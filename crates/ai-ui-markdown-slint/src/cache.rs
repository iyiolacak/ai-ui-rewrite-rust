//! Bounded UI-thread cache for scoped, asynchronously parsed Markdown.
//!
//! A host calls `observe` with its visible, non-truncated parts in priority
//! order, then `poll` on its UI thread. The backend owns parsing and revision
//! fencing. The cache never accepts an obsolete revision as the current view.
use std::collections::{HashMap, HashSet};
use std::hash::Hash;

use crate::{convert, Factory};
use ai_ui_markdown::presentation::{Presentation, RenderBlock};
use slint::{ModelRc, VecModel};

pub const PENDING: &str = "Formatting Markdown…";
pub const UNAVAILABLE: &str = "Markdown could not be formatted; showing the source.";
pub const CAPACITY: &str = "Markdown display capacity reached; showing the source.";
pub const TRUNCATED: &str = "Earlier text was truncated; Markdown is shown literally.";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendError {
    Rejected,
    ProjectionFailed,
    ExecutorFailed,
}

/// The backend must fence revisions across cancellation and clear, be
/// nonblocking, and report unexpected failure rather than remaining pending.
pub trait Backend<K> {
    type Revision: Copy + Eq;
    fn submit(&mut self, key: K, source: &str) -> Result<Self::Revision, BackendError>;
    fn cancel(&mut self, key: &K);
    fn clear(&mut self);
    fn poll(&mut self) -> Vec<Event<K, Self::Revision>>;
    /// Include a terminal error that has not yet been delivered.
    fn has_pending(&self) -> bool;
}

pub enum Event<K, R> {
    Completed {
        key: K,
        revision: R,
        is_current: bool,
        result: Result<Presentation, BackendError>,
    },
    ExecutorFailed,
}

/// `presentation_units` counts serialized strings plus an allowance per node;
/// it is not a measurement of total Slint, allocator, or renderer memory.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub keys: usize,
    pub presentation_units: usize,
}

struct Slot<R, B> {
    source: String,
    revision: Option<R>,
    blocks: Option<ModelRc<B>>,
    warning: &'static str,
    cost: usize,
}

pub struct Cache<K, F: Factory, B: Backend<K>> {
    factory: F,
    backend: Option<B>,
    limits: Limits,
    slots: HashMap<K, Slot<B::Revision, F::Block>>,
    cache_units: usize,
    /// Some slot is waiting for capacity (so releasing units is worth a retry).
    blocked: bool,
    /// Keys whose formatting finished since the last `take_changed`.
    changed: Vec<K>,
}

impl<K, F, B> Cache<K, F, B>
where
    K: Eq + Hash + Clone,
    F: Factory,
    F::Block: Clone + 'static,
    B: Backend<K>,
{
    pub fn new(factory: F, backend: Option<B>, limits: Limits) -> Self {
        Self {
            factory,
            backend,
            limits,
            slots: HashMap::new(),
            cache_units: 0,
            blocked: false,
            changed: Vec::new(),
        }
    }
    /// Invalidates all outstanding work before scope or identity changes.
    pub fn clear(&mut self) {
        if let Some(backend) = &mut self.backend {
            backend.clear();
        }
        self.slots.clear();
        self.cache_units = 0;
        self.blocked = false;
        self.changed.clear();
    }
    /// Inputs must be unique keys and listed highest priority first. A host
    /// should omit truncated parts; `view` renders those as literal text.
    pub fn observe<'a>(&mut self, wanted: impl IntoIterator<Item = (K, &'a str)>) {
        let mut wanted: Vec<(K, &str)> = wanted.into_iter().take(self.limits.keys).collect();
        let keys: HashSet<&K> = wanted.iter().map(|(key, _)| key).collect();
        let before = self.cache_units;
        let removed: Vec<K> = self
            .slots
            .keys()
            .filter(|key| !keys.contains(key))
            .cloned()
            .collect();
        for key in removed {
            if let Some(old) = self.slots.remove(&key) {
                self.cache_units -= old.cost;
            }
            if let Some(backend) = &mut self.backend {
                backend.cancel(&key);
            }
        }
        // Decreases first prevent a temporary source-budget rejection when
        // the final visible total fits. Keep active work for coalescing.
        wanted.sort_by_key(|(key, source)| {
            source
                .len()
                .saturating_sub(self.slots.get(key).map_or(0, |slot| slot.source.len()))
        });
        for (key, source) in wanted {
            if self
                .slots
                .get(&key)
                .is_some_and(|slot| slot.source == source)
            {
                continue;
            }
            self.replace(key, source);
        }
        if self.cache_units < before {
            self.retry_capacity();
        }
    }

    /// Observe one part and leave every other untouched: the update for a part
    /// whose text changed. Cost does not depend on how many parts are cached,
    /// unlike [`observe`](Self::observe), which takes the full visible set. A
    /// new part beyond the key limit is ignored (the host shows it as text).
    pub fn observe_one(&mut self, key: K, source: &str) {
        if self
            .slots
            .get(&key)
            .is_some_and(|slot| slot.source == source)
        {
            return;
        }
        if self.slots.len() >= self.limits.keys && !self.slots.contains_key(&key) {
            return;
        }
        let before = self.cache_units;
        self.replace(key, source);
        if self.cache_units < before {
            self.retry_capacity();
        }
    }

    /// Stop tracking one part.
    pub fn forget(&mut self, key: &K) {
        if let Some(old) = self.slots.remove(key) {
            self.cache_units -= old.cost;
            if let Some(backend) = &mut self.backend {
                backend.cancel(key);
            }
            self.retry_capacity();
        }
    }

    /// Stop tracking every part for which `keep` is false.
    pub fn retain(&mut self, keep: impl Fn(&K) -> bool) {
        let removed: Vec<K> = self
            .slots
            .keys()
            .filter(|key| !keep(key))
            .cloned()
            .collect();
        for key in &removed {
            if let Some(old) = self.slots.remove(key) {
                self.cache_units -= old.cost;
            }
            if let Some(backend) = &mut self.backend {
                backend.cancel(key);
            }
        }
        if !removed.is_empty() {
            self.retry_capacity();
        }
    }

    /// Submit `source` for `key`, replacing whatever was there.
    fn replace(&mut self, key: K, source: &str) {
        if let Some(old) = self.slots.remove(&key) {
            self.cache_units -= old.cost;
        }
        let revision = self
            .backend
            .as_mut()
            .and_then(|backend| backend.submit(key.clone(), source).ok());
        self.slots.insert(
            key,
            Slot {
                source: source.to_owned(),
                revision,
                blocks: None,
                warning: if revision.is_some() {
                    PENDING
                } else {
                    UNAVAILABLE
                },
                cost: 0,
            },
        );
    }
    /// The keys whose view changed because formatting finished, since the last
    /// call. After `poll` returns true a host rebuilds only these.
    pub fn take_changed(&mut self) -> Vec<K> {
        std::mem::take(&mut self.changed)
    }
    /// Whether the owner needs to continue polling its backend.
    pub fn has_pending(&self) -> bool {
        self.backend.as_ref().is_some_and(Backend::has_pending)
    }
    /// Drain ready completions without waiting; returns whether visible state changed.
    pub fn poll(&mut self) -> bool {
        let Some(backend) = &mut self.backend else {
            return false;
        };
        let events = backend.poll();
        let mut changed = false;
        let before = self.cache_units;
        let mut failed = false;
        for event in events {
            match event {
                Event::Completed {
                    key,
                    revision,
                    is_current: true,
                    result,
                } => {
                    let Some(slot) = self.slots.get_mut(&key) else {
                        continue;
                    };
                    if slot.revision != Some(revision) {
                        continue;
                    }
                    self.cache_units -= slot.cost;
                    slot.cost = 0;
                    slot.blocks = None;
                    match result {
                        Ok(presentation) => {
                            let cost = presentation_cost(&presentation);
                            if cost
                                > self
                                    .limits
                                    .presentation_units
                                    .saturating_sub(self.cache_units)
                            {
                                slot.warning = CAPACITY;
                                self.blocked = true;
                            } else if let Ok(rendered) = convert(presentation, &self.factory) {
                                slot.blocks = Some(ModelRc::new(VecModel::from(rendered.blocks)));
                                slot.warning = rendered.warning.unwrap_or("");
                                slot.cost = cost;
                                self.cache_units += cost;
                            } else {
                                slot.warning = UNAVAILABLE;
                            }
                        }
                        Err(_) => slot.warning = UNAVAILABLE,
                    }
                    self.changed.push(key);
                    changed = true;
                }
                Event::Completed { .. } => {}
                Event::ExecutorFailed => {
                    for slot in self.slots.values_mut() {
                        slot.blocks = None;
                        slot.revision = None;
                        slot.warning = UNAVAILABLE;
                        slot.cost = 0;
                    }
                    self.cache_units = 0;
                    self.changed.extend(self.slots.keys().cloned());
                    changed = true;
                    failed = true;
                }
            }
        }
        if !failed && self.cache_units < before {
            changed |= self.retry_capacity();
        }
        changed
    }
    // A capacity failure is retried only on a real release, never on timer ticks.
    fn retry_capacity(&mut self) -> bool {
        if !self.blocked {
            return false;
        }
        let Some(backend) = &mut self.backend else {
            return false;
        };
        self.blocked = false;
        let mut changed = false;
        for (key, slot) in &mut self.slots {
            if slot.warning != CAPACITY {
                continue;
            }
            backend.cancel(key);
            slot.revision = backend.submit(key.clone(), &slot.source).ok();
            slot.warning = if slot.revision.is_some() {
                PENDING
            } else {
                UNAVAILABLE
            };
            changed = true;
        }
        changed
    }
    /// Returns formatted blocks only for the exact current source. Truncated
    /// suffixes are always literal because they are not complete documents.
    pub fn view(
        &self,
        key: &K,
        source: &str,
        truncated: bool,
    ) -> (Option<ModelRc<F::Block>>, &'static str) {
        if truncated {
            return (None, TRUNCATED);
        }
        let Some(slot) = self.slots.get(key) else {
            return (None, CAPACITY);
        };
        if slot.source != source {
            return (None, PENDING);
        }
        (slot.blocks.clone(), slot.warning)
    }
}

fn presentation_cost(presentation: &Presentation) -> usize {
    let mut bytes = 0usize;
    let mut nodes = 0usize;
    for block in &presentation.blocks {
        nodes = nodes.saturating_add(1);
        match block {
            RenderBlock::Text {
                markdown, plain, ..
            } => {
                bytes = bytes
                    .saturating_add(markdown.len())
                    .saturating_add(plain.len())
            }
            RenderBlock::Code { language, code, .. } => {
                bytes = bytes
                    .saturating_add(code.len())
                    .saturating_add(language.as_ref().map_or(0, String::len))
            }
            RenderBlock::Rule { .. } => {}
            RenderBlock::TableRow { cells, .. } => {
                for cell in cells {
                    nodes = nodes.saturating_add(1);
                    bytes = bytes
                        .saturating_add(cell.markdown.len())
                        .saturating_add(cell.plain.len());
                }
            }
        }
    }
    bytes.saturating_add(nodes.saturating_mul(256))
}

/// Native implementation: coalescing coordinators on a pool of worker threads
/// (one per core, leaving one for the UI), sharded by key.
#[cfg(not(target_arch = "wasm32"))]
pub struct NativeBackend<K>(ai_ui_markdown::pool::CoordinatorPool<K>);
#[cfg(not(target_arch = "wasm32"))]
impl<K: Eq + Hash + Clone + Send + 'static> NativeBackend<K> {
    /// Start with [`ai_ui_markdown::pool::default_workers`] worker threads.
    pub fn start(keys: usize, source_bytes: usize) -> std::io::Result<Self> {
        Self::with_workers(keys, source_bytes, ai_ui_markdown::pool::default_workers())
    }

    /// Start with an explicit number of worker threads (at least one).
    pub fn with_workers(keys: usize, source_bytes: usize, workers: usize) -> std::io::Result<Self> {
        ai_ui_markdown::pool::CoordinatorPool::start(
            ai_ui_markdown::coordinator::Limits { keys, source_bytes },
            workers,
        )
        .map(Self)
    }
}
#[cfg(not(target_arch = "wasm32"))]
impl<K: Eq + Hash + Clone + Send + 'static> Backend<K> for NativeBackend<K> {
    type Revision = ai_ui_markdown::coordinator::SourceRevision;
    fn submit(&mut self, key: K, source: &str) -> Result<Self::Revision, BackendError> {
        self.0.submit(key, source).map_err(|error| match error {
            ai_ui_markdown::coordinator::SubmitError::ExecutorFailed => {
                BackendError::ExecutorFailed
            }
            _ => BackendError::Rejected,
        })
    }
    fn cancel(&mut self, key: &K) {
        self.0.cancel(key);
    }
    fn clear(&mut self) {
        self.0.clear();
    }
    fn has_pending(&self) -> bool {
        self.0.has_pending()
    }
    fn poll(&mut self) -> Vec<Event<K, Self::Revision>> {
        self.0
            .poll()
            .into_iter()
            .map(|event| match event {
                ai_ui_markdown::coordinator::Event::Completed {
                    key,
                    revision,
                    is_current,
                    result,
                } => Event::Completed {
                    key,
                    revision,
                    is_current,
                    result: result.map_err(|error| match error {
                        ai_ui_markdown::scheduling::WorkError::ExecutorFailed => {
                            BackendError::ExecutorFailed
                        }
                        _ => BackendError::ProjectionFailed,
                    }),
                },
                ai_ui_markdown::coordinator::Event::ExecutorFailed => Event::ExecutorFailed,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Alignment, Block, BlockKind};
    use slint::StyledText;
    use std::{cell::RefCell, rc::Rc};

    struct TestFactory;
    impl Factory for TestFactory {
        type Block = &'static str;
        type Cell = ();
        fn cell(&self, _: StyledText, _: Alignment) {}
        fn block(&self, block: Block<()>) -> Self::Block {
            match block.kind {
                BlockKind::Heading => "heading",
                _ => "block",
            }
        }
    }
    #[derive(Default)]
    struct State {
        next: u64,
        revisions: HashMap<&'static str, u64>,
        events: Vec<Event<&'static str, u64>>,
    }
    struct TestBackend(Rc<RefCell<State>>);
    impl Backend<&'static str> for TestBackend {
        type Revision = u64;
        fn submit(&mut self, key: &'static str, _: &str) -> Result<u64, BackendError> {
            let mut state = self.0.borrow_mut();
            state.next += 1;
            let revision = state.next;
            state.revisions.insert(key, revision);
            Ok(revision)
        }
        fn cancel(&mut self, key: &&'static str) {
            self.0.borrow_mut().revisions.remove(key);
        }
        fn clear(&mut self) {
            self.0.borrow_mut().revisions.clear();
        }
        fn poll(&mut self) -> Vec<Event<&'static str, u64>> {
            std::mem::take(&mut self.0.borrow_mut().events)
        }
        fn has_pending(&self) -> bool {
            !self.0.borrow().events.is_empty()
        }
    }
    fn complete(state: &Rc<RefCell<State>>, key: &'static str, source: &str) {
        let mut state = state.borrow_mut();
        let revision = state.revisions[key];
        state.events.push(Event::Completed {
            key,
            revision,
            is_current: true,
            result: Ok(ai_ui_markdown::presentation::project(source).unwrap()),
        });
    }
    #[test]
    fn capacity_failure_retries_only_after_other_content_is_released() {
        let state = Rc::new(RefCell::new(State::default()));
        let mut cache = Cache::new(
            TestFactory,
            Some(TestBackend(state.clone())),
            Limits {
                keys: 2,
                presentation_units: 400,
            },
        );
        cache.observe([("a", "alpha"), ("b", "bravo")]);
        complete(&state, "a", "alpha");
        complete(&state, "b", "bravo");
        assert!(cache.has_pending());
        assert!(cache.poll());
        assert_eq!(cache.view(&"b", "bravo", false).1, CAPACITY);
        let next_before = state.borrow().next;
        for _ in 0..3 {
            assert!(!cache.poll());
        }
        assert_eq!(state.borrow().next, next_before, "no timer-driven retry");
        cache.observe([("b", "bravo")]);
        assert_eq!(cache.view(&"b", "bravo", false).1, PENDING);
        assert_eq!(state.borrow().next, next_before + 1);
        complete(&state, "b", "bravo");
        assert!(cache.poll());
        assert!(cache.view(&"b", "bravo", false).0.is_some());
    }
    #[test]
    fn stale_revision_and_truncated_source_never_publish() {
        let state = Rc::new(RefCell::new(State::default()));
        let mut cache = Cache::new(
            TestFactory,
            Some(TestBackend(state.clone())),
            Limits {
                keys: 1,
                presentation_units: 1024,
            },
        );
        cache.observe([("a", "old")]);
        complete(&state, "a", "old");
        cache.observe([("a", "new")]);
        assert!(!cache.poll());
        assert_eq!(cache.view(&"a", "new", false).1, PENDING);
        assert_eq!(cache.view(&"a", "new", true).1, TRUNCATED);
        complete(&state, "a", "new");
        assert!(cache.poll());
        assert!(cache.view(&"a", "new", false).0.is_some());
    }
}
