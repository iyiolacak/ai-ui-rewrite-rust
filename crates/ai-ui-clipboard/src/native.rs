use crate::{Completion, CopyError, ReadCompletion, ReadError, MAX_COPY_BYTES, MAX_READ_BYTES};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::Arc;

enum Request {
    Copy { text: String, complete: Completion },
    Read { complete: ReadCompletion },
}
impl Request {
    fn cancel(self) {
        match self {
            Self::Copy { complete, .. } => complete(Err(CopyError::Cancelled)),
            Self::Read { complete } => complete(Err(ReadError::Cancelled)),
        }
    }
}

/// A single desktop clipboard owner and a bounded, nonblocking request queue.
/// Retain this object while clipboard ownership must persist (especially X11).
pub struct Clipboard {
    sender: SyncSender<Request>,
    active: Arc<AtomicBool>,
}

impl Clipboard {
    /// Start the worker without opening or writing the operating-system
    /// clipboard on the caller's thread. Startup failure is returned directly;
    /// clipboard access failures are delivered through each request completion.
    pub fn new() -> Result<Self, CopyError> {
        Self::start_with_read(
            || arboard::Clipboard::new().map_err(|_| CopyError::Unavailable),
            |clipboard, text| clipboard.set_text(text).map_err(|_| CopyError::Rejected),
            |clipboard| clipboard.get_text().map_err(|_| ReadError::Rejected),
        )
    }

    #[cfg(test)]
    fn start<T: 'static>(
        create: impl FnMut() -> Result<T, CopyError> + Send + 'static,
        write: impl FnMut(&mut T, String) -> Result<(), CopyError> + Send + 'static,
    ) -> Result<Self, CopyError> {
        Self::start_with_read(create, write, |_| Err(ReadError::Unavailable))
    }

    fn start_with_read<T: 'static>(
        mut create: impl FnMut() -> Result<T, CopyError> + Send + 'static,
        mut write: impl FnMut(&mut T, String) -> Result<(), CopyError> + Send + 'static,
        mut read: impl FnMut(&mut T) -> Result<String, ReadError> + Send + 'static,
    ) -> Result<Self, CopyError> {
        let (sender, receiver) = mpsc::sync_channel::<Request>(1);
        let active = Arc::new(AtomicBool::new(true));
        let worker_active = Arc::clone(&active);
        std::thread::Builder::new()
            .name("assistant-clipboard".into())
            .spawn(move || {
                // Create lazily and retry initialization on a later request if
                // the environment was unavailable (e.g. display not ready).
                let mut clipboard = None;
                while let Ok(request) = receiver.recv() {
                    if !worker_active.load(Ordering::SeqCst) {
                        request.cancel();
                        continue;
                    }
                    if clipboard.is_none() {
                        clipboard = create().ok();
                    }
                    // This check admits a request to the platform. A shutdown
                    // after this point cannot revoke the in-flight operation.
                    if !worker_active.load(Ordering::SeqCst) {
                        request.cancel();
                        continue;
                    }
                    match request {
                        Request::Copy { text, complete } => complete(match clipboard.as_mut() {
                            Some(owner) => write(owner, text),
                            None => Err(CopyError::Unavailable),
                        }),
                        Request::Read { complete } => complete(match clipboard.as_mut() {
                            Some(owner) => read(owner).and_then(|text| {
                                if text.len() > MAX_READ_BYTES {
                                    Err(ReadError::TooLarge)
                                } else {
                                    Ok(text)
                                }
                            }),
                            None => Err(ReadError::Unavailable),
                        }),
                    }
                }
            })
            .map_err(|_| CopyError::Unavailable)?;
        Ok(Self { sender, active })
    }

    /// Queue a write without blocking the calling thread. Immediate errors mean
    /// the write was not accepted and the completion will not run. Success
    /// means only that it was queued; observe the completion for the outcome.
    pub fn copy(&self, text: String, complete: Completion) -> Result<(), CopyError> {
        if !self.active.load(Ordering::SeqCst) {
            return Err(CopyError::Cancelled);
        }
        if text.len() > MAX_COPY_BYTES {
            return Err(CopyError::TooLarge);
        }
        self.sender
            .try_send(Request::Copy { text, complete })
            .map_err(|error| match error {
                TrySendError::Full(_) => CopyError::Busy,
                TrySendError::Disconnected(_) => CopyError::WorkerStopped,
            })
    }
    /// Queue a text read without blocking. `Ok` means queued, not completed;
    /// immediate errors do not invoke the completion. Platform reads already
    /// admitted when shutdown occurs may still return sensitive text.
    pub fn read_text(&self, complete: ReadCompletion) -> Result<(), ReadError> {
        if !self.active.load(Ordering::SeqCst) {
            return Err(ReadError::Cancelled);
        }
        self.sender
            .try_send(Request::Read { complete })
            .map_err(|error| match error {
                TrySendError::Full(_) => ReadError::Busy,
                TrySendError::Disconnected(_) => ReadError::WorkerStopped,
            })
    }
    /// Cancel requests not yet admitted to the platform without waiting for
    /// the worker. An operation admitted before this call cannot be revoked. Queued
    /// completions are delivered when the worker can drain them; a stalled OS
    /// call can delay them indefinitely. Retain the handle while X11 clipboard
    /// ownership is required. Scope late completion notifications in the host.
    pub fn shutdown(&self) {
        self.active.store(false, Ordering::SeqCst);
    }
}

impl Drop for Clipboard {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn read_returns_exact_text_and_rejects_oversized_or_failed_results() {
        let mut attempt = 0;
        let clipboard = Clipboard::start_with_read(
            || Ok(()),
            |_, _| Ok(()),
            move |_| {
                attempt += 1;
                match attempt {
                    1 => Ok("first\r\n雪👩‍💻".into()),
                    2 => Ok("x".repeat(MAX_READ_BYTES + 1)),
                    _ => Err(ReadError::Rejected),
                }
            },
        )
        .unwrap();
        for expected in [
            Ok("first\r\n雪👩‍💻".to_owned()),
            Err(ReadError::TooLarge),
            Err(ReadError::Rejected),
        ] {
            let (tx, rx) = mpsc::channel();
            clipboard
                .read_text(Box::new(move |result| tx.send(result).unwrap()))
                .unwrap();
            assert_eq!(rx.recv_timeout(Duration::from_secs(1)).unwrap(), expected);
        }
    }

    #[test]
    fn read_shares_bounded_queue_and_shutdown_cancels_pending_read() {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let clipboard = Clipboard::start_with_read(
            || Ok(()),
            move |_, _| {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(())
            },
            |_| Ok("must not be read".to_owned()),
        )
        .unwrap();
        clipboard
            .copy("in flight".into(), Box::new(|_| {}))
            .unwrap();
        entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        clipboard
            .read_text(Box::new(move |result| result_tx.send(result).unwrap()))
            .unwrap();
        assert_eq!(
            clipboard.read_text(Box::new(|_| panic!("not queued"))),
            Err(ReadError::Busy)
        );
        clipboard.shutdown();
        assert_eq!(
            clipboard.read_text(Box::new(|_| panic!("not queued"))),
            Err(ReadError::Cancelled)
        );
        release_tx.send(()).unwrap();
        assert_eq!(
            result_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            Err(ReadError::Cancelled)
        );
    }

    #[test]
    fn admitted_read_completes_after_shutdown_for_the_host_to_fence() {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let clipboard = Clipboard::start_with_read(
            || Ok(()),
            |_, _| Ok(()),
            move |_| {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok("synthetic".to_owned())
            },
        )
        .unwrap();
        clipboard
            .read_text(Box::new(move |result| result_tx.send(result).unwrap()))
            .unwrap();
        entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        clipboard.shutdown();
        release_tx.send(()).unwrap();
        assert_eq!(
            result_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            Ok("synthetic".to_owned())
        );
    }

    #[test]
    fn results_follow_worker_write_and_do_not_touch_system_clipboard() {
        let clipboard = Clipboard::start(
            || Ok(()),
            |_, text| {
                if text == "ok" {
                    Ok(())
                } else {
                    Err(CopyError::Rejected)
                }
            },
        )
        .unwrap();
        for (text, expected) in [("ok", Ok(())), ("fail", Err(CopyError::Rejected))] {
            let (tx, rx) = mpsc::channel();
            clipboard
                .copy(
                    text.into(),
                    Box::new(move |result| tx.send(result).unwrap()),
                )
                .unwrap();
            assert_eq!(rx.recv_timeout(Duration::from_secs(1)).unwrap(), expected);
        }
    }

    #[test]
    fn unavailability_and_oversize_are_explicit() {
        let clipboard = Clipboard::start(
            || -> Result<(), CopyError> { Err(CopyError::Unavailable) },
            |_, _| Ok(()),
        )
        .unwrap();
        let (tx, rx) = mpsc::channel();
        clipboard
            .copy(
                "test".into(),
                Box::new(move |result| tx.send(result).unwrap()),
            )
            .unwrap();
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            Err(CopyError::Unavailable)
        );
        assert_eq!(
            clipboard.copy(
                "x".repeat(MAX_COPY_BYTES + 1),
                Box::new(|_| panic!("not queued"))
            ),
            Err(CopyError::TooLarge)
        );
    }

    #[test]
    fn unavailable_clipboard_is_retried_on_a_later_request() {
        let mut attempts = 0;
        let clipboard = Clipboard::start(
            move || {
                attempts += 1;
                if attempts == 1 {
                    Err(CopyError::Unavailable)
                } else {
                    Ok(())
                }
            },
            |_, _| Ok(()),
        )
        .unwrap();
        for expected in [Err(CopyError::Unavailable), Ok(())] {
            let (tx, rx) = mpsc::channel();
            clipboard
                .copy(
                    "test".into(),
                    Box::new(move |result| tx.send(result).unwrap()),
                )
                .unwrap();
            assert_eq!(rx.recv_timeout(Duration::from_secs(1)).unwrap(), expected);
        }
    }

    #[test]
    fn shutdown_cancels_queued_write_but_not_the_in_flight_write() {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (results_tx, results_rx) = mpsc::channel();
        let clipboard = Clipboard::start(
            || Ok(()),
            move |_, text| {
                entered_tx.send(text).unwrap();
                release_rx.recv().unwrap();
                Ok(())
            },
        )
        .unwrap();
        let first = results_tx.clone();
        clipboard
            .copy(
                "first".into(),
                Box::new(move |result| first.send(result).unwrap()),
            )
            .unwrap();
        assert_eq!(
            entered_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            "first"
        );
        clipboard
            .copy(
                "second".into(),
                Box::new(move |result| results_tx.send(result).unwrap()),
            )
            .unwrap();
        clipboard.shutdown(); // Returns while the first platform write is still blocked.
        assert_eq!(
            clipboard.copy("third".into(), Box::new(|_| panic!("not queued"))),
            Err(CopyError::Cancelled)
        );
        release_tx.send(()).unwrap();
        assert_eq!(
            results_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            Ok(())
        );
        assert_eq!(
            results_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            Err(CopyError::Cancelled)
        );
        assert!(entered_rx.try_recv().is_err());
    }

    #[test]
    fn drop_during_initialization_cancels_before_platform_write() {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (write_tx, write_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let clipboard = Clipboard::start(
            move || {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(())
            },
            move |_, _| {
                write_tx.send(()).unwrap();
                Ok(())
            },
        )
        .unwrap();
        clipboard
            .copy(
                "secret".into(),
                Box::new(move |result| result_tx.send(result).unwrap()),
            )
            .unwrap();
        entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        drop(clipboard); // Does not wait for initialization.
        release_tx.send(()).unwrap();
        assert_eq!(
            result_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            Err(CopyError::Cancelled)
        );
        assert!(write_rx.try_recv().is_err());
    }

    #[test]
    fn queue_is_bounded_without_blocking_input() {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let clipboard = Clipboard::start(
            || Ok(()),
            move |_, _| {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(())
            },
        )
        .unwrap();
        clipboard.copy("first".into(), Box::new(|_| {})).unwrap();
        entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        clipboard.copy("second".into(), Box::new(|_| {})).unwrap();
        assert_eq!(
            clipboard.copy("third".into(), Box::new(|_| {})),
            Err(CopyError::Busy)
        );
        release_tx.send(()).unwrap();
        entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        release_tx.send(()).unwrap();
    }
}
