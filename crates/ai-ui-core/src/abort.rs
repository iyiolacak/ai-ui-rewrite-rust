//! Cancellation for model runs, standing in for the web `AbortSignal`.
//!
//! The controller lives in the runtime; adapters receive a signal they can
//! poll (`is_aborted`) or await (`aborted()`), on any thread. The runtime also
//! stops polling an adapter's stream as soon as the signal fires, so an adapter
//! that ignores the signal still stops affecting the thread.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

#[derive(Default)]
struct Shared {
    aborted: AtomicBool,
    wakers: Mutex<Vec<Waker>>,
}

/// Owner side: aborts its signal once. Clones control the same signal.
#[derive(Clone, Default)]
pub struct AbortController {
    shared: Arc<Shared>,
}

impl AbortController {
    /// A controller whose signal has not fired.
    pub fn new() -> Self {
        Self::default()
    }
    /// A signal that observes this controller; clone it freely.
    pub fn signal(&self) -> AbortSignal {
        AbortSignal {
            shared: self.shared.clone(),
        }
    }
    /// Abort and wake every task awaiting the signal. Idempotent.
    pub fn abort(&self) {
        if !self.shared.aborted.swap(true, Ordering::SeqCst) {
            let wakers =
                std::mem::take(&mut *self.shared.wakers.lock().unwrap_or_else(|e| e.into_inner()));
            for waker in wakers {
                waker.wake();
            }
        }
    }
}

/// Observer side, cheap to clone and `Send`.
#[derive(Clone, Default)]
pub struct AbortSignal {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for AbortSignal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AbortSignal")
            .field("aborted", &self.is_aborted())
            .finish()
    }
}

impl AbortSignal {
    /// Whether the run was aborted.
    pub fn is_aborted(&self) -> bool {
        self.shared.aborted.load(Ordering::SeqCst)
    }
    /// Resolves once the signal is aborted.
    pub fn aborted(&self) -> Aborted {
        Aborted {
            signal: self.clone(),
        }
    }
    fn poll_aborted(&self, cx: &mut Context<'_>) -> Poll<()> {
        if self.is_aborted() {
            return Poll::Ready(());
        }
        let mut wakers = self.shared.wakers.lock().unwrap_or_else(|e| e.into_inner());
        // Re-check under the lock: `abort` drains wakers after setting the flag.
        if self.is_aborted() {
            return Poll::Ready(());
        }
        if !wakers.iter().any(|w| w.will_wake(cx.waker())) {
            wakers.push(cx.waker().clone());
        }
        Poll::Pending
    }
}

/// Future returned by [`AbortSignal::aborted`].
pub struct Aborted {
    signal: AbortSignal,
}

impl Future for Aborted {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.signal.poll_aborted(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abort_wakes_a_waiting_task_and_is_idempotent() {
        let controller = AbortController::new();
        let signal = controller.signal();
        let waiter = std::thread::spawn(move || futures_executor::block_on(signal.aborted()));
        controller.abort();
        controller.abort();
        waiter.join().unwrap();
        assert!(controller.signal().is_aborted());
    }

    #[test]
    fn an_already_aborted_signal_resolves_immediately() {
        let controller = AbortController::new();
        controller.abort();
        futures_executor::block_on(controller.signal().aborted());
    }
}
