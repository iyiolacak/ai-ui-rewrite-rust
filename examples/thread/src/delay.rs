//! An async sleep on Slint's event loop, for pacing the demo model.

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

#[derive(Default)]
struct Shared {
    done: bool,
    waker: Option<Waker>,
}

/// Resolves after `duration`; zero resolves on first poll.
pub struct Delay {
    duration: Duration,
    shared: Option<Rc<RefCell<Shared>>>,
}

impl Delay {
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            shared: None,
        }
    }
}

impl Future for Delay {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.duration.is_zero() {
            return Poll::Ready(());
        }
        let duration = self.duration;
        let shared = self
            .shared
            .get_or_insert_with(|| {
                let shared = Rc::new(RefCell::new(Shared::default()));
                let timer = shared.clone();
                slint::Timer::single_shot(duration, move || {
                    let waker = {
                        let mut state = timer.borrow_mut();
                        state.done = true;
                        state.waker.take()
                    };
                    if let Some(waker) = waker {
                        waker.wake();
                    }
                });
                shared
            })
            .clone();
        let mut state = shared.borrow_mut();
        if state.done {
            Poll::Ready(())
        } else {
            state.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}
