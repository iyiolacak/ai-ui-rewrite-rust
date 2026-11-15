//! Optional native single-thread executor for [`crate::scheduling::Scheduler`].
//!
//! Completion is delivered on the worker thread: a UI host must marshal it to
//! its event loop, then call `Scheduler::complete` before publishing and dispatch
//! the next pending job. Never touch thread-affine widgets from this callback.
//! The executor performs no I/O and does not create or manage the scheduler.

use crate::scheduling::{Finished, Job};
use std::io;
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};

/// Dropping the worker closes its queue without waiting on the UI thread. Any
/// already dispatched work finishes before the detached thread exits; clear
/// the scheduler first so its result cannot update a closed session. The host
/// must account for that lifetime when managing private data.
pub struct Worker<K> {
    sender: SyncSender<Job<K>>,
    thread: JoinHandle<()>,
}

impl<K: Send + 'static> Worker<K> {
    /// Start the worker thread; `on_finish` runs on it for every finished job.
    pub fn start(mut on_finish: impl FnMut(Finished<K>) + Send + 'static) -> io::Result<Self> {
        let (sender, receiver) = sync_channel::<Job<K>>(1);
        let thread = thread::Builder::new()
            .name("assistant-markdown".into())
            .spawn(move || {
                while let Ok(job) = receiver.recv() {
                    on_finish(job.run());
                }
            })?;
        Ok(Self { sender, thread })
    }

    /// Nonblocking delivery. If refused, return the enclosed job to
    /// `Scheduler::return_undispatched` and display an explicit failure.
    pub fn try_dispatch(&self, job: Job<K>) -> Result<(), TrySendError<Job<K>>> {
        self.sender.try_send(job)
    }

    /// A finished thread indicates a failed callback or unexpected teardown.
    /// Never silently wait for a completion from a dead worker.
    pub fn is_finished(&self) -> bool {
        self.thread.is_finished()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduling::{Completion, QueueLimits, Scheduler};
    use std::sync::mpsc::channel;
    use std::time::{Duration, Instant};

    #[test]
    fn worker_delivers_result_for_owner_thread_to_validate() {
        let (sender, receiver) = channel();
        let worker = Worker::start(move |result| sender.send(result).unwrap()).unwrap();
        let mut scheduler = Scheduler::new(QueueLimits {
            pending_keys: 1,
            pending_bytes: 100,
        });
        scheduler.submit("part", "**hello**").unwrap();
        worker
            .try_dispatch(scheduler.take_next().unwrap())
            .unwrap_or_else(|_| panic!("dispatch"));
        let result = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(
            scheduler.complete(result),
            Completion::Ready {
                key: "part",
                result: Ok(_),
                ..
            }
        ));
    }

    #[test]
    fn failed_callback_can_be_abandoned_and_replaced() {
        let (notifier, observed) = channel();
        let worker = Worker::start(move |_result| {
            notifier.send(()).unwrap();
            panic!("simulated completion callback failure");
        })
        .unwrap();
        let mut scheduler = Scheduler::new(QueueLimits {
            pending_keys: 1,
            pending_bytes: 100,
        });
        scheduler.submit("part", "old").unwrap();
        let job = scheduler.take_next().unwrap();
        let ticket = job.ticket();
        worker
            .try_dispatch(job)
            .unwrap_or_else(|_| panic!("dispatch"));
        observed.recv_timeout(Duration::from_secs(5)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !worker.is_finished() && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(worker.is_finished());
        assert!(scheduler.abandon(&ticket));
        drop(worker);

        let (sender, receiver) = channel();
        let replacement = Worker::start(move |result| sender.send(result).unwrap()).unwrap();
        scheduler.submit("part", "new").unwrap();
        replacement
            .try_dispatch(scheduler.take_next().unwrap())
            .unwrap_or_else(|_| panic!("dispatch replacement"));
        let result = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(
            scheduler.complete(result),
            Completion::Ready { result: Ok(_), .. }
        ));
    }
}
