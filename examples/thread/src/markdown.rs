//! Markdown for assistant text, parsed off the UI thread. While a newer source
//! is being formatted the previous formatting stays on screen, so streaming
//! text never flashes back to plain text.

use crate::{MarkdownAlignment, MarkdownBlock, MarkdownBlockKind, MarkdownCell};
use ai_ui_markdown_slint::{self as adapter, cache::Limits, streaming::StreamingCache, Factory};
use slint::{ModelRc, StyledText, VecModel};

#[cfg(not(target_arch = "wasm32"))]
use ai_ui_markdown_slint::cache::NativeBackend;

/// (message id, part index)
pub type Key = (String, usize);
/// Cache entries: a streaming reply uses one per top-level block.
const MAX_PARTS: usize = 2048;
#[cfg(not(target_arch = "wasm32"))]
const MAX_SOURCE_BYTES: usize = 256 * 1024;

struct SlintFactory;
impl Factory for SlintFactory {
    type Block = MarkdownBlock;
    type Cell = MarkdownCell;
    fn cell(&self, text: StyledText, alignment: adapter::Alignment) -> MarkdownCell {
        MarkdownCell {
            text,
            alignment: match alignment {
                adapter::Alignment::Left => MarkdownAlignment::Left,
                adapter::Alignment::Center => MarkdownAlignment::Center,
                adapter::Alignment::Right => MarkdownAlignment::Right,
            },
        }
    }
    fn block(&self, block: adapter::Block<MarkdownCell>) -> MarkdownBlock {
        MarkdownBlock {
            kind: match block.kind {
                adapter::BlockKind::Paragraph => MarkdownBlockKind::Paragraph,
                adapter::BlockKind::Heading => MarkdownBlockKind::Heading,
                adapter::BlockKind::Code => MarkdownBlockKind::Code,
                adapter::BlockKind::Rule => MarkdownBlockKind::Rule,
                adapter::BlockKind::TableHeader => MarkdownBlockKind::TableHeader,
                adapter::BlockKind::TableRow => MarkdownBlockKind::TableRow,
            },
            depth: block.depth,
            quote_depth: block.quote_depth,
            heading_level: block.heading_level,
            text: block.text,
            code: block.code.into(),
            language: block.language.into(),
            cells: ModelRc::new(VecModel::from(block.cells)),
            copy_token: "".into(),
            copied: false,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
type PlatformBackend = NativeBackend<(Key, usize)>;
#[cfg(target_arch = "wasm32")]
type PlatformBackend = inline::InlineBackend<(Key, usize)>;

/// The browser has no worker threads here: the same projection runs on the UI
/// thread, a few milliseconds per poll, newest source of a key only.
#[cfg(target_arch = "wasm32")]
mod inline {
    use ai_ui_markdown::presentation::project;
    use ai_ui_markdown_slint::cache::{Backend, BackendError, Event};
    use std::collections::{HashMap, VecDeque};
    use std::hash::Hash;
    use web_time::{Duration, Instant};

    const BUDGET: Duration = Duration::from_millis(4);

    pub struct InlineBackend<K> {
        queue: VecDeque<(K, u64, String)>,
        latest: HashMap<K, u64>,
        next: u64,
    }

    impl<K: Eq + Hash + Clone> InlineBackend<K> {
        pub fn new() -> Self {
            Self {
                queue: VecDeque::new(),
                latest: HashMap::new(),
                next: 0,
            }
        }
    }

    impl<K: Eq + Hash + Clone> Backend<K> for InlineBackend<K> {
        type Revision = u64;
        fn submit(&mut self, key: K, source: &str) -> Result<u64, BackendError> {
            self.next += 1;
            self.latest.insert(key.clone(), self.next);
            self.queue.push_back((key, self.next, source.to_owned()));
            Ok(self.next)
        }
        fn cancel(&mut self, key: &K) {
            self.latest.remove(key);
            self.queue.retain(|(k, _, _)| k != key);
        }
        fn clear(&mut self) {
            self.latest.clear();
            self.queue.clear();
        }
        fn has_pending(&self) -> bool {
            !self.queue.is_empty()
        }
        fn poll(&mut self) -> Vec<Event<K, u64>> {
            let start = Instant::now();
            let mut events = Vec::new();
            while let Some((key, revision, source)) = self.queue.pop_front() {
                if self.latest.get(&key) != Some(&revision) {
                    continue;
                }
                self.latest.remove(&key);
                events.push(Event::Completed {
                    key,
                    revision,
                    is_current: true,
                    result: project(&source).map_err(|_| BackendError::ProjectionFailed),
                });
                if start.elapsed() >= BUDGET {
                    break;
                }
            }
            events
        }
    }
}

pub struct Markdown {
    cache: StreamingCache<Key, SlintFactory, PlatformBackend>,
}

impl Markdown {
    pub fn new() -> Self {
        #[cfg(target_arch = "wasm32")]
        let backend = Some(inline::InlineBackend::new());
        // `AI_UI_MARKDOWN_WORKERS=1` restores one worker thread (for comparisons).
        #[cfg(not(target_arch = "wasm32"))]
        let backend = match std::env::var("AI_UI_MARKDOWN_WORKERS")
            .ok()
            .and_then(|n| n.parse().ok())
        {
            Some(workers) => NativeBackend::with_workers(MAX_PARTS, MAX_SOURCE_BYTES, workers),
            None => NativeBackend::start(MAX_PARTS, MAX_SOURCE_BYTES),
        }
        .ok();
        Self {
            cache: StreamingCache::new(
                SlintFactory,
                backend,
                Limits {
                    keys: MAX_PARTS,
                    presentation_units: 8 * 1024 * 1024,
                },
            ),
        }
    }

    /// Declare one text that changed, with whether it is still streaming
    /// (formatted block by block) or finished (formatted whole).
    pub fn observe_one(&mut self, key: Key, text: &str, streaming: bool) {
        self.cache.observe_one(key, text, streaming);
    }

    /// Stop formatting every text for which `keep` is false.
    pub fn retain(&mut self, keep: impl Fn(&Key) -> bool) {
        self.cache.retain(keep);
    }

    /// True when finished work arrived and rows should be refreshed.
    pub fn poll(&mut self) -> bool {
        self.cache.poll()
    }

    /// The texts whose formatting finished since the last call.
    pub fn take_changed(&mut self) -> Vec<Key> {
        self.cache.take_changed()
    }
    pub fn pending(&self) -> bool {
        self.cache.has_pending()
    }

    /// Blocks for `source`, or the last formatting of this part while the new
    /// one is pending.
    pub fn view(
        &mut self,
        key: &Key,
        source: &str,
        streaming: bool,
    ) -> Option<ModelRc<MarkdownBlock>> {
        self.cache.view(key, source, streaming)
    }
}
