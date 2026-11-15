//! Coalesce complete Markdown snapshots before dispatching projection work.
//!
//! The host owns this scheduler on one thread and dispatches at most one job to
//! an executor such as a native worker thread. Each accepted snapshot is
//! parsed in full: later reference definitions can change earlier blocks. A
//! newer snapshot, cancellation, or scope reset invalidates a running result.
//! This module neither starts workers nor publishes results to a UI.
//!
//! Pending input and the number of pending keys are bounded. One additional
//! source of at most [`MAX_SOURCE_BYTES`] may be held by the executor, and its
//! result can use the limits documented by `presentation`. The scheduler does
//! not bound key allocations, executor, renderer, or application memory. Browser
//! workers require a separate message transport, which this module does not provide.
//! `clear` invalidates but cannot interrupt running work. A host can abandon a
//! lost job by its unique ticket; see `Scheduler::abandon` for executor and
//! resource-lifetime requirements.

use crate::presentation::{project, Presentation, ProjectionError};
use crate::MAX_SOURCE_BYTES;
use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;

/// Limits for sources waiting to be dispatched, excluding the one running job.
#[derive(Clone, Copy, Debug)]
pub struct QueueLimits {
    /// Most parts waiting at once.
    pub pending_keys: usize,
    /// Most source bytes waiting at once.
    pub pending_bytes: usize,
}

/// Why a source was not accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitError {
    /// The source exceeds the size limit.
    SourceTooLarge,
    /// Too many sources are waiting.
    QueueFull,
    /// The revision counter ran out; reset and start again.
    RevisionExhausted,
}

/// A revision is unique within one scheduler, including across `clear` calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Revision(u64);

struct Pending<K> {
    key: K,
    revision: Revision,
    source: String,
}

struct Running<K> {
    key: K,
    revision: Revision,
    invalidated: bool,
}

/// Own one scheduler per user/session scope. Keys must identify the exact
/// message part within that scope. On sign-out or scope change, call `clear`
/// before accepting new snapshots, or drop the scheduler with its executor.
pub struct Scheduler<K> {
    identity: Arc<()>,
    limits: QueueLimits,
    next_revision: u64,
    pending: VecDeque<Pending<K>>,
    pending_bytes: usize,
    running: Option<Running<K>>,
}

/// An owned job can be moved to another thread. Run it exactly once and return
/// the completed value to the originating scheduler on its owner thread.
pub struct Job<K> {
    identity: Arc<()>,
    key: K,
    revision: Revision,
    source: String,
}

/// Identifies one dispatch from one scheduler. Retain this separately from the
/// owned job so a failed executor can release its slot without a completion.
#[derive(Clone)]
pub struct JobTicket {
    identity: Arc<()>,
    revision: Revision,
}

/// The outcome of a job, to hand back to `Scheduler::complete`.
pub struct Finished<K> {
    identity: Arc<()>,
    key: K,
    revision: Revision,
    result: Result<Presentation, WorkError>,
}

/// Why a job produced no presentation.
#[derive(Debug)]
pub enum WorkError {
    /// The document could not be projected.
    Projection(ProjectionError),
    /// An unwind during pure projection was caught on the worker. Builds that
    /// abort on panic cannot recover from a panic this way.
    Panicked,
    /// The platform executor failed or returned an invalid response.
    ExecutorFailed,
}
impl fmt::Display for WorkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Projection(error) => error.fmt(f),
            Self::Panicked => f.write_str("Markdown projection worker panicked"),
            Self::ExecutorFailed => f.write_str("Markdown projection executor failed"),
        }
    }
}
impl std::error::Error for WorkError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Projection(error) => Some(error),
            Self::Panicked | Self::ExecutorFailed => None,
        }
    }
}

/// What a finished job means for the host.
pub enum Completion<K> {
    /// Publish only while the host's own current scope and part still match.
    Ready {
        /// The part the result belongs to.
        key: K,
        /// Which version of the source was projected.
        revision: Revision,
        /// The projection, or why it failed.
        result: Result<Presentation, WorkError>,
    },
    /// A newer submission, cancellation, or reset made the result obsolete.
    Stale,
    /// The completion belongs to a different scheduler or is not in flight.
    Unknown,
}

impl<K> Job<K> {
    /// Source for a platform executor; keep this job on the owner thread so
    /// the scheduler's identity and revision never cross the process boundary.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Complete a dispatched job with a platform executor's result. The host
    /// must trust or validate the result before publishing it.
    pub fn finish(self, result: Result<Presentation, WorkError>) -> Finished<K> {
        Finished {
            identity: self.identity,
            key: self.key,
            revision: self.revision,
            result,
        }
    }
    /// A handle for abandoning this job if its worker dies.
    pub fn ticket(&self) -> JobTicket {
        JobTicket {
            identity: Arc::clone(&self.identity),
            revision: self.revision,
        }
    }

    /// Project the source, catching a panic as `WorkError::Panicked`.
    pub fn run(self) -> Finished<K> {
        Finished {
            identity: self.identity,
            key: self.key,
            revision: self.revision,
            result: std::panic::catch_unwind(|| project(&self.source))
                .map_err(|_| WorkError::Panicked)
                .and_then(|result| result.map_err(WorkError::Projection)),
        }
    }
}

impl<K: Eq + Clone> Scheduler<K> {
    /// A scheduler with the given queue limits.
    pub fn new(limits: QueueLimits) -> Self {
        Self {
            identity: Arc::new(()),
            limits,
            next_revision: 0,
            pending: VecDeque::new(),
            pending_bytes: 0,
            running: None,
        }
    }

    /// Submit the authoritative current source. An earlier pending source for
    /// this key is replaced and a running source invalidated even on error, so
    /// an old result cannot be mistaken for the rejected newer snapshot. On
    /// error the host should explicitly display its own failure or fallback.
    pub fn submit(&mut self, key: K, source: &str) -> Result<Revision, SubmitError> {
        self.cancel(&key);
        if source.len() > MAX_SOURCE_BYTES {
            return Err(SubmitError::SourceTooLarge);
        }
        if self.pending.len() >= self.limits.pending_keys
            || source.len() > self.limits.pending_bytes.saturating_sub(self.pending_bytes)
        {
            return Err(SubmitError::QueueFull);
        }
        let next = self
            .next_revision
            .checked_add(1)
            .ok_or(SubmitError::RevisionExhausted)?;
        self.next_revision = next;
        let revision = Revision(next);
        self.pending_bytes += source.len();
        self.pending.push_back(Pending {
            key,
            revision,
            source: source.to_owned(),
        });
        Ok(revision)
    }

    /// Invalidate this key's running work and remove its queued snapshot.
    pub fn cancel(&mut self, key: &K) {
        if let Some(running) = &mut self.running {
            if &running.key == key {
                running.invalidated = true;
            }
        }
        if let Some(index) = self.pending.iter().position(|entry| &entry.key == key) {
            let removed = self.pending.remove(index).expect("position exists");
            self.pending_bytes -= removed.source.len();
        }
    }

    /// Remove all queued data and invalidate the running job. Revisions are
    /// not reused, so an old result cannot revive after a scope change.
    pub fn clear(&mut self) {
        self.pending.clear();
        self.pending_bytes = 0;
        if let Some(running) = &mut self.running {
            running.invalidated = true;
        }
    }

    /// Dispatch in submission order; replacement moves a key to the back.
    /// Returns `None` while a previous job is still outstanding.
    pub fn take_next(&mut self) -> Option<Job<K>> {
        if self.running.is_some() {
            return None;
        }
        let pending = self.pending.pop_front()?;
        self.pending_bytes -= pending.source.len();
        self.running = Some(Running {
            key: pending.key.clone(),
            revision: pending.revision,
            invalidated: false,
        });
        Some(Job {
            identity: Arc::clone(&self.identity),
            key: pending.key,
            revision: pending.revision,
            source: pending.source,
        })
    }

    /// Release a job that an executor refused before taking ownership. This
    /// preserves any newer queued revision. Dispatch failures must be surfaced
    /// by the host; silently dropping a job would stall the scheduler.
    pub fn return_undispatched(&mut self, job: Job<K>) -> bool {
        let matches = self.running.as_ref().is_some_and(|running| {
            Arc::ptr_eq(&self.identity, &job.identity)
                && running.revision == job.revision
                && running.key == job.key
        });
        if matches {
            self.running = None;
        }
        matches
    }

    /// Release a lost or deliberately abandoned dispatch. An old or foreign
    /// ticket cannot release a newer job, and an eventual late completion is
    /// rejected. The host must report lost work and stop using or replace its
    /// executor before dispatching again, unless it knows the job has ended.
    /// Abandonment cannot stop a thread: replacing a still-busy worker may run
    /// old and new jobs concurrently, so the host must bound such workers and
    /// their memory and retain the old session's data-lifetime obligations.
    pub fn abandon(&mut self, ticket: &JobTicket) -> bool {
        let matches = self.running.as_ref().is_some_and(|running| {
            Arc::ptr_eq(&self.identity, &ticket.identity) && running.revision == ticket.revision
        });
        if matches {
            self.running = None;
        }
        matches
    }

    /// Reject foreign, superseded and cancelled results before publication.
    /// After any matching completion the host may call `take_next` again.
    pub fn complete(&mut self, finished: Finished<K>) -> Completion<K> {
        let Some(running) = &self.running else {
            return Completion::Unknown;
        };
        if !Arc::ptr_eq(&self.identity, &finished.identity)
            || running.revision != finished.revision
            || running.key != finished.key
        {
            return Completion::Unknown;
        }
        let running = self.running.take().expect("matched running job");
        if running.invalidated {
            Completion::Stale
        } else {
            Completion::Ready {
                key: finished.key,
                revision: finished.revision,
                result: finished.result,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scheduler() -> Scheduler<&'static str> {
        Scheduler::new(QueueLimits {
            pending_keys: 2,
            pending_bytes: 64,
        })
    }

    #[test]
    fn superseded_work_is_discarded_and_only_latest_snapshot_is_dispatched() {
        let mut queue = scheduler();
        queue.submit("a", "one").unwrap();
        let first = queue.take_next().unwrap();
        queue.submit("a", "two").unwrap();
        let latest = queue.submit("a", "three").unwrap();
        assert!(queue.take_next().is_none());
        assert!(matches!(queue.complete(first.run()), Completion::Stale));
        let job = queue.take_next().unwrap();
        match queue.complete(job.run()) {
            Completion::Ready {
                key,
                revision,
                result,
            } => {
                assert_eq!(key, "a");
                assert_eq!(revision, latest);
                assert_eq!(result.unwrap(), project("three").unwrap());
            }
            _ => panic!("latest snapshot must be ready"),
        }
        assert!(queue.take_next().is_none());
    }

    #[test]
    fn replacement_is_fair_and_queue_limits_reject_without_stale_publish() {
        let mut queue = Scheduler::new(QueueLimits {
            pending_keys: 2,
            pending_bytes: 5,
        });
        queue.submit("a", "1").unwrap();
        let running = queue.take_next().unwrap();
        queue.submit("a", "22").unwrap();
        queue.submit("b", "33").unwrap();
        assert_eq!(queue.submit("a", "4444"), Err(SubmitError::QueueFull));
        assert!(matches!(queue.complete(running.run()), Completion::Stale));
        let b = queue.take_next().unwrap();
        assert!(matches!(
            queue.complete(b.run()),
            Completion::Ready { key: "b", .. }
        ));
        assert!(queue.take_next().is_none());

        queue.submit("a", "1").unwrap();
        queue.submit("b", "2").unwrap();
        queue.submit("a", "3").unwrap();
        let first = queue.take_next().unwrap();
        assert!(matches!(
            queue.complete(first.run()),
            Completion::Ready { key: "b", .. }
        ));
        let second = queue.take_next().unwrap();
        assert!(matches!(
            queue.complete(second.run()),
            Completion::Ready { key: "a", .. }
        ));
    }

    #[test]
    fn reset_cancellation_and_foreign_completions_do_not_publish() {
        let mut one = scheduler();
        let mut other = scheduler();
        one.submit("a", "secret").unwrap();
        let old = one.take_next().unwrap();
        other.submit("a", "other").unwrap();
        let foreign = other.take_next().unwrap();
        assert!(matches!(one.complete(foreign.run()), Completion::Unknown));
        one.clear();
        one.submit("a", "new session").unwrap();
        assert!(matches!(one.complete(old.run()), Completion::Stale));
        let next = one.take_next().unwrap();
        one.cancel(&"a");
        assert!(matches!(one.complete(next.run()), Completion::Stale));
        assert!(one.take_next().is_none());
    }

    #[test]
    fn rejected_new_snapshot_invalidates_running_and_preserves_other_keys() {
        let mut queue = scheduler();
        queue.submit("a", "old").unwrap();
        let old = queue.take_next().unwrap();
        queue.submit("b", "kept").unwrap();
        assert_eq!(
            queue.submit("a", &"x".repeat(MAX_SOURCE_BYTES + 1)),
            Err(SubmitError::SourceTooLarge)
        );
        assert!(matches!(queue.complete(old.run()), Completion::Stale));
        let kept = queue.take_next().unwrap();
        assert!(matches!(
            queue.complete(kept.run()),
            Completion::Ready { key: "b", .. }
        ));
    }

    #[test]
    fn externally_computed_completion_still_obeys_scope_and_revision() {
        let mut queue = scheduler();
        queue.submit("part", "old").unwrap();
        let old = queue.take_next().unwrap();
        assert_eq!(old.source(), "old");
        queue.clear();
        queue.submit("part", "new scope").unwrap();
        assert!(matches!(
            queue.complete(old.finish(Ok(project("old").unwrap()))),
            Completion::Stale
        ));
        let current = queue.take_next().unwrap();
        assert!(matches!(
            queue.complete(current.finish(Ok(project("new scope").unwrap()))),
            Completion::Ready {
                key: "part",
                result: Ok(_),
                ..
            }
        ));
    }

    #[test]
    fn refused_dispatch_can_release_slot_without_losing_a_newer_snapshot() {
        let mut queue = scheduler();
        queue.submit("a", "old").unwrap();
        let undispatched = queue.take_next().unwrap();
        queue.submit("a", "new").unwrap();
        assert!(queue.return_undispatched(undispatched));
        let next = queue.take_next().unwrap();
        let Completion::Ready { result, .. } = queue.complete(next.run()) else {
            panic!("newer source should be ready");
        };
        assert_eq!(result.unwrap(), project("new").unwrap());
    }

    #[test]
    fn abandoned_and_foreign_tickets_cannot_release_or_publish_newer_work() {
        let mut queue = scheduler();
        let mut other = scheduler();
        queue.submit("a", "old").unwrap();
        other.submit("a", "foreign").unwrap();
        let old = queue.take_next().unwrap();
        let ticket = old.ticket();
        let foreign = other.take_next().unwrap();
        assert!(!queue.abandon(&foreign.ticket()));

        queue.clear();
        queue.submit("a", "new session").unwrap();
        assert!(queue.abandon(&ticket));
        let next = queue.take_next().unwrap();
        assert!(!queue.abandon(&ticket));
        assert!(matches!(queue.complete(old.run()), Completion::Unknown));
        assert!(matches!(
            queue.complete(next.run()),
            Completion::Ready { result: Ok(_), .. }
        ));
        assert!(!queue.abandon(&ticket));
    }

    #[test]
    fn worker_computes_full_snapshot_including_later_reference_definitions() {
        let mut queue = scheduler();
        queue
            .submit("a", "[link][id]\n\n[id]: https://example.com")
            .unwrap();
        let job = queue.take_next().unwrap();
        let finished = std::thread::spawn(move || job.run()).join().unwrap();
        let Completion::Ready { result, .. } = queue.complete(finished) else {
            panic!("expected result")
        };
        let projected = result.unwrap();
        assert_eq!(
            projected,
            project("[link][id]\n\n[id]: https://example.com").unwrap()
        );
    }
}
