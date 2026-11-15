//! Native, nonblocking latest-snapshot coordinator for streamed Markdown.
//!
//! The owner calls [`Coordinator::submit`] on source changes and [`Coordinator::poll`]
//! on its own event loop. One worker parses complete snapshots. While it runs,
//! the coordinator retains only the latest source per key; it does not invalidate
//! an active parse on every chunk. Older completions are explicitly identified as
//! previews. The host must check its current scope and part before publishing.

use crate::presentation::Presentation;
use crate::scheduling::{Completion, JobTicket, QueueLimits, Scheduler, WorkError};
use crate::worker::Worker;
use crate::MAX_SOURCE_BYTES;
use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::io;
use std::sync::mpsc::{sync_channel, Receiver, TryRecvError, TrySendError};
use std::sync::Arc;

/// Bounds current source slots; key storage, one in-flight source, its parse
/// output, and host-held presentations are additional allocations.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Most message parts tracked at once.
    pub keys: usize,
    /// Most source bytes accepted for one part.
    pub source_bytes: usize,
}

/// Monotonically allocated within one coordinator, including across resets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceRevision(u64);

/// Why a source was not accepted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubmitError {
    /// The source exceeds the size limit.
    SourceTooLarge,
    /// Too many parts are tracked or too many source bytes are held.
    CapacityExceeded,
    /// The revision counter ran out; reset and start again.
    RevisionExhausted,
    /// The worker stopped and cannot take work.
    ExecutorFailed,
}

/// An older result is usable only as an explicitly marked preview. The owner
/// must additionally verify that the scope and part are still visible.
pub enum Event<K> {
    /// A source finished projecting.
    Completed {
        /// The message part the source belongs to.
        key: K,
        /// Which version of the source was projected.
        revision: SourceRevision,
        /// Whether it is still the newest version; if not, it is only a preview.
        is_current: bool,
        /// The projection, or why it failed.
        result: Result<Presentation, WorkError>,
    },
    /// The worker stopped or could not accept a job. All tracked sources were
    /// discarded; show literal source and create a new coordinator explicitly.
    ExecutorFailed,
}

struct Entry {
    source: String,
    revision: SourceRevision,
    queued: bool,
}

struct Running<K> {
    key: K,
    revision: SourceRevision,
    scope: Arc<()>,
    ticket: JobTicket,
    invalidated: bool,
}

/// Single-owner state. All methods are nonblocking; only the worker parses.
/// Keys must be bounded by the caller and identify a unique message part.
pub struct Coordinator<K> {
    limits: Limits,
    scope: Arc<()>,
    next_revision: u64,
    entries: HashMap<K, Entry>,
    source_bytes: usize,
    pending: VecDeque<K>,
    running: Option<Running<K>>,
    scheduler: Scheduler<K>,
    worker: Worker<K>,
    receiver: Receiver<crate::scheduling::Finished<K>>,
    failed: bool,
    report_failure: bool,
}

impl<K: Eq + Hash + Clone + Send + 'static> Coordinator<K> {
    /// Starts exactly one worker. Dropping this value does not join a running
    /// parse; callers must account for that bounded thread and source lifetime.
    pub fn start(limits: Limits) -> io::Result<Self> {
        let (sender, receiver) = sync_channel(1);
        let worker = Worker::start(move |finished| {
            // At most one dispatch can be outstanding. A disconnected owner
            // needs no result; the callback must never panic or block.
            let _ = sender.try_send(finished);
        })?;
        Ok(Self {
            limits,
            scope: Arc::new(()),
            next_revision: 0,
            entries: HashMap::new(),
            source_bytes: 0,
            pending: VecDeque::new(),
            running: None,
            scheduler: Scheduler::new(QueueLimits {
                pending_keys: 1,
                pending_bytes: MAX_SOURCE_BYTES,
            }),
            worker,
            receiver,
            failed: false,
            report_failure: false,
        })
    }

    /// Retain the newest authoritative source. Identical input keeps its
    /// revision and does not schedule work again. A rejected update invalidates
    /// any old work for that key; the host must display a failure or literal
    /// fallback. Call `poll` to dispatch and receive completions.
    pub fn submit(&mut self, key: K, source: &str) -> Result<SourceRevision, SubmitError> {
        if self.failed || self.worker.is_finished() {
            self.fail();
            return Err(SubmitError::ExecutorFailed);
        }
        if source.len() > MAX_SOURCE_BYTES {
            self.cancel(&key);
            return Err(SubmitError::SourceTooLarge);
        }
        if let Some(entry) = self.entries.get(&key) {
            if entry.source == source {
                return Ok(entry.revision);
            }
        }
        let previous_bytes = self.entries.get(&key).map_or(0, |entry| entry.source.len());
        if (!self.entries.contains_key(&key) && self.entries.len() >= self.limits.keys)
            || source.len()
                > self
                    .limits
                    .source_bytes
                    .saturating_sub(self.source_bytes - previous_bytes)
        {
            self.cancel(&key);
            return Err(SubmitError::CapacityExceeded);
        }
        let Some(next) = self.next_revision.checked_add(1) else {
            self.cancel(&key);
            return Err(SubmitError::RevisionExhausted);
        };
        self.next_revision = next;
        let revision = SourceRevision(next);
        self.source_bytes = self.source_bytes - previous_bytes + source.len();
        let running_this_key = self.running.as_ref().is_some_and(|running| {
            !running.invalidated && Arc::ptr_eq(&running.scope, &self.scope) && running.key == key
        });
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.source = source.to_owned();
            entry.revision = revision;
            if !entry.queued && !running_this_key {
                entry.queued = true;
                self.pending.push_back(key);
            }
        } else {
            self.entries.insert(
                key.clone(),
                Entry {
                    source: source.to_owned(),
                    revision,
                    queued: !running_this_key,
                },
            );
            if !running_this_key {
                self.pending.push_back(key);
            }
        }
        Ok(revision)
    }

    /// Forget one key and invalidate its in-flight result.
    pub fn cancel(&mut self, key: &K) {
        if let Some(entry) = self.entries.remove(key) {
            self.source_bytes -= entry.source.len();
        }
        self.pending.retain(|pending| pending != key);
        if let Some(running) = self.running.as_mut() {
            if &running.key == key && Arc::ptr_eq(&running.scope, &self.scope) {
                running.invalidated = true;
                self.scheduler.cancel(key);
            }
        }
    }

    /// Invalidate all work before changing scope. In-flight work is allowed to
    /// finish before new work is dispatched, so no replacement worker overlaps.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.source_bytes = 0;
        self.pending.clear();
        self.scope = Arc::new(());
        self.scheduler.clear();
        if let Some(running) = self.running.as_mut() {
            running.invalidated = true;
        }
    }

    /// Whether polling is needed for queued work, an in-flight parse, or a
    /// terminal failure that has not yet been reported.
    pub fn has_pending(&self) -> bool {
        self.running.is_some() || !self.pending.is_empty() || self.report_failure
    }

    /// Drain at most one completion and dispatch the next snapshot without
    /// blocking. The bounded completion channel holds at most one result.
    pub fn poll(&mut self) -> Vec<Event<K>> {
        let mut events = Vec::new();
        match self.receiver.try_recv() {
            Ok(finished) => {
                let running = self.running.take();
                match (running, self.scheduler.complete(finished)) {
                    (Some(running), Completion::Ready { key, result, .. }) => {
                        if !running.invalidated && Arc::ptr_eq(&running.scope, &self.scope) {
                            if let Some(entry) = self.entries.get_mut(&key) {
                                let is_current = entry.revision == running.revision;
                                if !is_current && !entry.queued {
                                    entry.queued = true;
                                    self.pending.push_back(key.clone());
                                }
                                events.push(Event::Completed {
                                    key,
                                    revision: running.revision,
                                    is_current,
                                    result,
                                });
                            }
                        }
                    }
                    (Some(_), Completion::Stale) => {}
                    _ => self.fail(),
                }
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => self.fail(),
        }
        if self.worker.is_finished() {
            self.fail();
        }
        if !self.failed && self.running.is_none() {
            self.dispatch_next();
        }
        if self.report_failure {
            self.report_failure = false;
            events.push(Event::ExecutorFailed);
        }
        events
    }

    fn dispatch_next(&mut self) {
        let Some(key) = self.pending.pop_front() else {
            return;
        };
        let Some(entry) = self.entries.get_mut(&key) else {
            self.fail();
            return;
        };
        entry.queued = false;
        let revision = entry.revision;
        if self.scheduler.submit(key.clone(), &entry.source).is_err() {
            self.fail();
            return;
        }
        let Some(job) = self.scheduler.take_next() else {
            self.fail();
            return;
        };
        let ticket = job.ticket();
        match self.worker.try_dispatch(job) {
            Ok(()) => {
                self.running = Some(Running {
                    key,
                    revision,
                    scope: Arc::clone(&self.scope),
                    ticket,
                    invalidated: false,
                })
            }
            Err(TrySendError::Full(job) | TrySendError::Disconnected(job)) => {
                self.scheduler.return_undispatched(job);
                self.fail();
            }
        }
    }

    fn fail(&mut self) {
        if self.failed {
            return;
        }
        self.failed = true;
        self.report_failure = true;
        if let Some(running) = self.running.take() {
            self.scheduler.abandon(&running.ticket);
        }
        self.scheduler.clear();
        self.entries.clear();
        self.pending.clear();
        self.source_bytes = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presentation::project;
    use std::time::{Duration, Instant};

    fn until_event(coordinator: &mut Coordinator<&'static str>) -> Event<&'static str> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(event) = coordinator.poll().into_iter().next() {
                return event;
            }
            assert!(Instant::now() < deadline, "worker did not return");
            std::thread::yield_now();
        }
    }

    #[test]
    fn streaming_coalesces_latest_and_labels_older_completion() {
        let mut coordinator = Coordinator::start(Limits {
            keys: 2,
            source_bytes: 128,
        })
        .unwrap();
        assert!(!coordinator.has_pending());
        let first = coordinator.submit("part", "old").unwrap();
        assert!(coordinator.has_pending());
        assert!(coordinator.poll().is_empty()); // dispatch, do not wait for the worker
        assert!(coordinator.has_pending());
        coordinator.submit("part", "intermediate").unwrap();
        let latest = coordinator.submit("part", "**latest**").unwrap();
        assert_eq!(coordinator.submit("part", "**latest**"), Ok(latest));
        match until_event(&mut coordinator) {
            Event::Completed {
                key,
                revision,
                is_current,
                result,
            } => {
                assert_eq!(key, "part");
                assert_eq!(revision, first);
                assert!(!is_current);
                assert_eq!(result.unwrap(), project("old").unwrap());
            }
            Event::ExecutorFailed => panic!("worker failed"),
        }
        match until_event(&mut coordinator) {
            Event::Completed {
                key,
                revision,
                is_current,
                result,
            } => {
                assert_eq!(key, "part");
                assert_eq!(revision, latest);
                assert!(is_current);
                assert_eq!(result.unwrap(), project("**latest**").unwrap());
            }
            Event::ExecutorFailed => panic!("worker failed"),
        }
        assert!(coordinator.poll().is_empty());
        assert!(!coordinator.has_pending());
    }

    #[test]
    fn reset_and_cancel_reject_old_completions_even_when_ids_are_reused() {
        let mut coordinator = Coordinator::start(Limits {
            keys: 2,
            source_bytes: 128,
        })
        .unwrap();
        coordinator.submit("part", "old scope").unwrap();
        assert!(coordinator.poll().is_empty());
        coordinator.clear();
        let new = coordinator.submit("part", "new scope").unwrap();
        match until_event(&mut coordinator) {
            Event::Completed {
                revision,
                is_current,
                result,
                ..
            } => {
                assert_eq!(revision, new);
                assert!(is_current);
                assert_eq!(result.unwrap(), project("new scope").unwrap());
            }
            Event::ExecutorFailed => panic!("worker failed"),
        }
        coordinator.submit("part", "cancelled").unwrap();
        assert!(coordinator.poll().is_empty());
        coordinator.cancel(&"part");
        let replacement = coordinator.submit("part", "replacement").unwrap();
        match until_event(&mut coordinator) {
            Event::Completed {
                revision,
                is_current,
                result,
                ..
            } => {
                assert_eq!(revision, replacement);
                assert!(is_current);
                assert_eq!(result.unwrap(), project("replacement").unwrap());
            }
            Event::ExecutorFailed => panic!("worker failed"),
        }
    }

    #[test]
    fn unexpected_worker_death_is_reported_and_never_silently_retried() {
        let mut coordinator = Coordinator::start(Limits {
            keys: 2,
            source_bytes: 128,
        })
        .unwrap();
        let (sender, receiver) = sync_channel(1);
        let worker = Worker::start(move |_result| {
            let _keep_sender_alive = &sender;
            panic!("synthetic worker failure");
        })
        .unwrap();
        coordinator.worker = worker;
        coordinator.receiver = receiver;
        coordinator.submit("part", "source").unwrap();
        assert!(coordinator.poll().is_empty());
        assert!(matches!(
            until_event(&mut coordinator),
            Event::ExecutorFailed
        ));
        assert_eq!(
            coordinator.submit("part", "new"),
            Err(SubmitError::ExecutorFailed)
        );
        coordinator.clear();
        assert_eq!(
            coordinator.submit("part", "new"),
            Err(SubmitError::ExecutorFailed)
        );
        assert!(coordinator.poll().is_empty());
    }

    #[test]
    fn rejected_replacement_invalidates_prior_work_and_frees_capacity() {
        let mut coordinator = Coordinator::start(Limits {
            keys: 2,
            source_bytes: 8,
        })
        .unwrap();
        coordinator.submit("a", "old").unwrap();
        assert!(coordinator.poll().is_empty());
        assert_eq!(
            coordinator.submit("a", "too long!"),
            Err(SubmitError::CapacityExceeded)
        );
        let kept = coordinator.submit("b", "new").unwrap();
        match until_event(&mut coordinator) {
            Event::Completed {
                key,
                revision,
                is_current,
                result,
            } => {
                assert_eq!(key, "b");
                assert_eq!(revision, kept);
                assert!(is_current);
                assert_eq!(result.unwrap(), project("new").unwrap());
            }
            Event::ExecutorFailed => panic!("worker failed"),
        }
        assert_eq!(
            coordinator.submit("oversize", &"x".repeat(MAX_SOURCE_BYTES + 1)),
            Err(SubmitError::SourceTooLarge)
        );
    }
}
