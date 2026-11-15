//! Several [`Coordinator`]s, one worker thread each, behind the same API.
//!
//! Every key always goes to the same shard (by hash), so each key keeps the
//! coordinator's guarantees unchanged: one parse in flight, only the newest
//! source retained while it runs, older completions labelled as previews, and
//! cancel and clear invalidate in-flight work. Different keys (messages, or the
//! segments of one streaming document) are formatted in parallel, so opening a
//! long thread uses every core instead of one.
//!
//! Limits are enforced for the pool as a whole, exactly as one coordinator
//! would: at most `keys` keys and `source_bytes` bytes across all shards.

use crate::coordinator::{Coordinator, Event, Limits, SourceRevision, SubmitError};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io;

/// Most worker threads a pool starts.
pub const MAX_WORKERS: usize = 8;

/// Worker threads for this machine: one per core, leaving one for the UI
/// thread, between 1 and [`MAX_WORKERS`].
pub fn default_workers() -> usize {
    std::thread::available_parallelism()
        .map_or(1, |n| n.get().saturating_sub(1))
        .clamp(1, MAX_WORKERS)
}

/// A pool of coordinators sharded by key. All methods are nonblocking.
pub struct CoordinatorPool<K> {
    shards: Vec<Coordinator<K>>,
    limits: Limits,
    /// Bytes held per key, for the pool-wide limits.
    sizes: HashMap<K, usize>,
    bytes: usize,
    failed: bool,
    report_failure: bool,
}

impl<K: Eq + Hash + Clone + Send + 'static> CoordinatorPool<K> {
    /// Start `workers` coordinators (at least one), each with one worker thread.
    pub fn start(limits: Limits, workers: usize) -> io::Result<Self> {
        let shards = (0..workers.max(1))
            .map(|_| Coordinator::start(limits))
            .collect::<io::Result<Vec<_>>>()?;
        Ok(Self {
            shards,
            limits,
            sizes: HashMap::new(),
            bytes: 0,
            failed: false,
            report_failure: false,
        })
    }

    /// Number of worker threads.
    pub fn workers(&self) -> usize {
        self.shards.len()
    }

    fn shard(&self, key: &K) -> usize {
        if self.shards.len() == 1 {
            return 0;
        }
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        (hasher.finish() % self.shards.len() as u64) as usize
    }

    /// See [`Coordinator::submit`]; the limits apply to the whole pool.
    pub fn submit(&mut self, key: K, source: &str) -> Result<SourceRevision, SubmitError> {
        if self.failed {
            return Err(SubmitError::ExecutorFailed);
        }
        let previous = self.sizes.get(&key).copied();
        let others = self.bytes - previous.unwrap_or(0);
        if (previous.is_none() && self.sizes.len() >= self.limits.keys)
            || source.len() > self.limits.source_bytes.saturating_sub(others)
        {
            self.cancel(&key);
            return Err(SubmitError::CapacityExceeded);
        }
        let shard = self.shard(&key);
        match self.shards[shard].submit(key.clone(), source) {
            Ok(revision) => {
                self.bytes = others + source.len();
                self.sizes.insert(key, source.len());
                Ok(revision)
            }
            Err(error) => {
                // The shard forgot the key; so does the pool.
                self.forget(&key);
                if error == SubmitError::ExecutorFailed {
                    self.fail();
                }
                Err(error)
            }
        }
    }

    fn forget(&mut self, key: &K) {
        if let Some(size) = self.sizes.remove(key) {
            self.bytes -= size;
        }
    }

    /// Forget one key and invalidate its in-flight result.
    pub fn cancel(&mut self, key: &K) {
        self.forget(key);
        let shard = self.shard(key);
        self.shards[shard].cancel(key);
    }

    /// Invalidate all work (see [`Coordinator::clear`]).
    pub fn clear(&mut self) {
        self.sizes.clear();
        self.bytes = 0;
        for shard in &mut self.shards {
            shard.clear();
        }
    }

    /// Whether any shard has queued or in-flight work, or a failure is unreported.
    pub fn has_pending(&self) -> bool {
        self.report_failure || self.shards.iter().any(Coordinator::has_pending)
    }

    /// Poll every shard: at most one completion each, and dispatch next work.
    /// A failed shard fails the pool, like a single coordinator: all sources
    /// are discarded and [`Event::ExecutorFailed`] is reported once.
    pub fn poll(&mut self) -> Vec<Event<K>> {
        let mut events = Vec::new();
        if !self.failed {
            for shard in &mut self.shards {
                for event in shard.poll() {
                    match event {
                        Event::ExecutorFailed => self.failed = true,
                        completed => events.push(completed),
                    }
                }
            }
            if self.failed {
                self.fail();
                events.clear();
            }
        }
        if self.report_failure {
            self.report_failure = false;
            events.push(Event::ExecutorFailed);
        }
        events
    }

    fn fail(&mut self) {
        self.failed = true;
        self.report_failure = true;
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presentation::project;
    use std::time::{Duration, Instant};

    fn drain(pool: &mut CoordinatorPool<String>) -> Vec<Event<String>> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut events = Vec::new();
        while pool.has_pending() {
            events.extend(pool.poll());
            assert!(Instant::now() < deadline, "workers did not finish");
            std::thread::yield_now();
        }
        events
    }

    #[test]
    fn many_keys_complete_once_each_with_the_right_projection() {
        let mut pool = CoordinatorPool::start(
            Limits {
                keys: 1000,
                source_bytes: 1 << 20,
            },
            4,
        )
        .unwrap();
        assert_eq!(pool.workers(), 4);
        let sources: Vec<(String, String)> = (0..200)
            .map(|i| (format!("m{i}"), format!("**{i}** and `code {i}`")))
            .collect();
        for (key, source) in &sources {
            pool.submit(key.clone(), source).unwrap();
        }
        let mut seen = HashMap::new();
        for event in drain(&mut pool) {
            match event {
                Event::Completed {
                    key,
                    is_current,
                    result,
                    ..
                } => {
                    assert!(is_current);
                    assert!(seen.insert(key, result.unwrap()).is_none());
                }
                Event::ExecutorFailed => panic!("worker failed"),
            }
        }
        assert_eq!(seen.len(), sources.len());
        for (key, source) in &sources {
            assert_eq!(seen[key], project(source).unwrap());
        }
    }

    #[test]
    fn a_key_keeps_newest_wins_on_its_shard() {
        let mut pool = CoordinatorPool::start(
            Limits {
                keys: 4,
                source_bytes: 1024,
            },
            3,
        )
        .unwrap();
        pool.submit("part".into(), "old").unwrap();
        pool.poll(); // dispatch "old"; the next two arrive while it runs
        pool.submit("part".into(), "middle").unwrap();
        let latest = pool.submit("part".into(), "**latest**").unwrap();
        let last = drain(&mut pool)
            .into_iter()
            .filter_map(|e| match e {
                Event::Completed {
                    revision,
                    is_current,
                    result,
                    ..
                } => Some((revision, is_current, result.unwrap())),
                Event::ExecutorFailed => None,
            })
            .next_back()
            .unwrap();
        assert_eq!(last, (latest, true, project("**latest**").unwrap()));
    }

    #[test]
    fn limits_apply_to_the_whole_pool() {
        let mut pool: CoordinatorPool<String> = CoordinatorPool::start(
            Limits {
                keys: 2,
                source_bytes: 10,
            },
            4,
        )
        .unwrap();
        pool.submit("a".into(), "12345").unwrap();
        pool.submit("b".into(), "12345").unwrap();
        assert_eq!(
            pool.submit("c".into(), "1"),
            Err(SubmitError::CapacityExceeded)
        );
        assert_eq!(
            pool.submit("a".into(), "123456"),
            Err(SubmitError::CapacityExceeded)
        );
        // The rejected replacement freed "a".
        pool.submit("c".into(), "12345").unwrap();
        pool.clear();
        pool.submit("d".into(), "1234567890").unwrap();
    }
}
