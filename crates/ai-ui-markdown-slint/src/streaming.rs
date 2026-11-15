//! Markdown for text that is still streaming: each stable block is formatted
//! once and only the live tail is formatted again as text arrives.
//!
//! Wraps [`Cache`]. A streaming source is split with
//! [`StreamSplit`](ai_ui_markdown::split::StreamSplit); every piece is a
//! cache entry of its own, so a long reply costs the size of its tail per
//! update instead of its whole length. Finished text is formatted in one piece,
//! which is exact for every document (see the splitter's notes on definitions).

use crate::cache::{Backend, Cache, Limits};
use crate::Factory;
use ai_ui_markdown::split::StreamSplit;
use slint::{Model, ModelRc, VecModel};
use std::collections::HashMap;
use std::hash::Hash;
use std::rc::Rc;

/// Segment index of a finished document's single piece.
const WHOLE: usize = usize::MAX;

struct Doc<B> {
    split: StreamSplit,
    /// The rows a UI shows for this document, updated in place so it re-lays out
    /// only the rows that changed, not the whole reply.
    model: Rc<VecModel<B>>,
    /// Segments (from the start) already published as frozen rows, and their row count.
    frozen_segments: usize,
    frozen_rows: usize,
    /// The text was replaced: the shown rows are discarded once the new text is formatted,
    /// so the old content stays visible until then.
    reset: bool,
    /// Frozen blocks (append-only while the text only grows), then the tail.
    segments: Vec<String>,
    stable: usize,
    total_len: usize,
    /// Segment entries registered with the cache by `observe_one`.
    published: usize,
}

pub struct StreamingCache<K, F: Factory, B: Backend<(K, usize)>> {
    cache: Cache<(K, usize), F, B>,
    docs: HashMap<K, Doc<F::Block>>,
    /// The last complete formatting of each document, shown while a newer
    /// source is still being formatted.
    shown: HashMap<K, ModelRc<F::Block>>,
}

impl<K, F, B> StreamingCache<K, F, B>
where
    K: Eq + Hash + Clone,
    F: Factory,
    F::Block: Clone + 'static,
    B: Backend<(K, usize)>,
{
    pub fn new(factory: F, backend: Option<B>, limits: Limits) -> Self {
        Self {
            cache: Cache::new(factory, backend, limits),
            docs: HashMap::new(),
            shown: HashMap::new(),
        }
    }

    pub fn clear(&mut self) {
        self.cache.clear();
        self.docs.clear();
        self.shown.clear();
    }

    /// The documents on screen, highest priority first, each with its current
    /// source and whether it is still streaming.
    pub fn observe<'a>(&mut self, wanted: impl IntoIterator<Item = (K, &'a str, bool)>) {
        let mut order: Vec<(K, bool, &str)> = Vec::new();
        for (key, source, streaming) in wanted {
            if streaming {
                let doc = self.docs.entry(key.clone()).or_insert_with(|| Doc {
                    split: StreamSplit::new(),
                    model: Rc::new(VecModel::default()),
                    frozen_segments: 0,
                    frozen_rows: 0,
                    reset: false,
                    segments: Vec::new(),
                    stable: 0,
                    total_len: 0,
                    published: 0,
                });
                let (stable, tail) = doc.split.update(source);
                if stable.len() < doc.stable {
                    // The text was replaced rather than extended.
                    doc.segments.clear();
                    doc.stable = 0;
                    doc.reset = true;
                }
                doc.segments.truncate(doc.stable);
                doc.segments.extend(stable[doc.stable..].iter().cloned());
                doc.stable = stable.len();
                doc.segments.push(tail.to_owned());
                doc.total_len = source.len();
            } else {
                self.docs.remove(&key);
            }
            order.push((key, streaming, source));
        }
        let mut list: Vec<((K, usize), &str)> = Vec::new();
        for (key, streaming, source) in &order {
            if *streaming {
                let doc = &self.docs[key];
                // The live tail first, then the frozen blocks, newest first.
                for index in (0..doc.segments.len()).rev() {
                    list.push(((key.clone(), index), doc.segments[index].as_str()));
                }
            } else {
                list.push(((key.clone(), WHOLE), source));
            }
        }
        self.cache.observe(list);
        let live: std::collections::HashSet<&K> = order.iter().map(|(k, _, _)| k).collect();
        self.shown.retain(|key, _| live.contains(key));
    }

    /// Observe one document and leave the others untouched: the update for a
    /// part whose text changed, costing its own size rather than the thread's.
    /// Pair it with [`retain`](Self::retain) when parts leave the screen;
    /// [`observe`](Self::observe) takes the whole visible set instead, and the
    /// two should not be mixed on one cache.
    pub fn observe_one(&mut self, key: K, source: &str, streaming: bool) {
        if streaming {
            let doc = self.docs.entry(key.clone()).or_insert_with(|| Doc {
                split: StreamSplit::new(),
                model: Rc::new(VecModel::default()),
                frozen_segments: 0,
                frozen_rows: 0,
                reset: false,
                segments: Vec::new(),
                stable: 0,
                total_len: 0,
                published: 0,
            });
            if doc.published == 0 {
                self.cache.forget(&(key.clone(), WHOLE));
            }
            let (stable, tail) = doc.split.update(source);
            if stable.len() < doc.stable {
                doc.segments.clear();
                doc.stable = 0;
                doc.reset = true;
            }
            doc.segments.truncate(doc.stable);
            doc.segments.extend(stable[doc.stable..].iter().cloned());
            doc.stable = stable.len();
            doc.segments.push(tail.to_owned());
            doc.total_len = source.len();
            // Newest first: the live tail, then blocks that just froze.
            for index in (0..doc.segments.len()).rev() {
                self.cache
                    .observe_one((key.clone(), index), &doc.segments[index]);
            }
            for index in doc.segments.len()..doc.published {
                self.cache.forget(&(key.clone(), index));
            }
            doc.published = doc.segments.len();
        } else {
            if let Some(doc) = self.docs.remove(&key) {
                for index in 0..doc.published {
                    self.cache.forget(&(key.clone(), index));
                }
            }
            self.cache.observe_one((key, WHOLE), source);
        }
    }

    /// Stop tracking every document for which `keep` is false.
    pub fn retain(&mut self, keep: impl Fn(&K) -> bool) {
        self.docs.retain(|key, _| keep(key));
        self.shown.retain(|key, _| keep(key));
        self.cache.retain(|(key, _)| keep(key));
    }

    /// True when finished work arrived and views may have changed.
    pub fn poll(&mut self) -> bool {
        self.cache.poll()
    }

    /// The documents whose view changed because formatting finished, since the
    /// last call (each once).
    pub fn take_changed(&mut self) -> Vec<K> {
        let mut seen = std::collections::HashSet::new();
        self.cache
            .take_changed()
            .into_iter()
            .filter_map(|(key, _)| seen.insert(key.clone()).then_some(key))
            .collect()
    }

    pub fn has_pending(&self) -> bool {
        self.cache.has_pending()
    }

    /// The formatted blocks for the document's current source, or the last
    /// complete formatting while the new one is not ready. `None` before the
    /// first one exists.
    pub fn view(&mut self, key: &K, source: &str, streaming: bool) -> Option<ModelRc<F::Block>> {
        let fresh = if streaming {
            self.view_segments(key, source)
        } else {
            self.cache.view(&(key.clone(), WHOLE), source, false).0
        };
        match fresh {
            Some(blocks) => {
                self.shown.insert(key.clone(), blocks.clone());
                Some(blocks)
            }
            None => self.shown.get(key).cloned(),
        }
    }

    /// The document's rows, brought up to date in place: rows of blocks that froze
    /// are appended once, and only the live tail's rows are rewritten.
    fn view_segments(&mut self, key: &K, source: &str) -> Option<ModelRc<F::Block>> {
        let doc = self
            .docs
            .get_mut(key)
            .filter(|d| d.total_len == source.len())?;
        let stable = doc.stable;
        // After a replacement everything is published again, from the start.
        let from = if doc.reset { 0 } else { doc.frozen_segments };
        // Every segment not yet published must be formatted before anything changes.
        let mut views: Vec<ModelRc<F::Block>> = Vec::with_capacity(doc.segments.len());
        for index in from..doc.segments.len() {
            views.push(
                self.cache
                    .view(&(key.clone(), index), &doc.segments[index], false)
                    .0?,
            );
        }
        let tail = views.pop()?;
        if doc.reset {
            doc.model.set_vec(Vec::new());
            doc.frozen_segments = 0;
            doc.frozen_rows = 0;
            doc.reset = false;
        }
        if doc.frozen_segments < stable {
            // New blocks froze: drop the old tail rows, then append the frozen ones.
            while doc.model.row_count() > doc.frozen_rows {
                doc.model.remove(doc.model.row_count() - 1);
            }
            for view in &views {
                for block in view.iter() {
                    doc.model.push(block);
                    doc.frozen_rows += 1;
                }
            }
            doc.frozen_segments = stable;
        }
        // The tail: rewrite its rows, then trim or extend.
        let mut row = doc.frozen_rows;
        for block in tail.iter() {
            if row < doc.model.row_count() {
                doc.model.set_row_data(row, block);
            } else {
                doc.model.push(block);
            }
            row += 1;
        }
        while doc.model.row_count() > row {
            doc.model.remove(doc.model.row_count() - 1);
        }
        Some(ModelRc::from(doc.model.clone()))
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::cache::NativeBackend;
    use crate::{Alignment, Block, BlockKind};
    use slint::StyledText;
    use std::time::{Duration, Instant};

    type Shape = (BlockKind, i32, i32, String);

    struct Shapes;
    impl Factory for Shapes {
        type Block = Shape;
        type Cell = ();
        fn cell(&self, _: StyledText, _: Alignment) {}
        fn block(&self, block: Block<()>) -> Shape {
            (block.kind, block.depth, block.heading_level, block.code)
        }
    }

    type Cache = StreamingCache<&'static str, Shapes, NativeBackend<(&'static str, usize)>>;

    fn cache() -> Cache {
        let backend = NativeBackend::start(512, 256 * 1024).expect("worker thread");
        StreamingCache::new(
            Shapes,
            Some(backend),
            Limits {
                keys: 512,
                presentation_units: 1 << 20,
            },
        )
    }

    fn settle(cache: &mut Cache) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while cache.has_pending() && Instant::now() < deadline {
            cache.poll();
            std::thread::sleep(Duration::from_millis(1));
        }
        cache.poll();
        assert!(!cache.has_pending(), "formatting did not finish");
    }

    fn shapes(blocks: &ModelRc<Shape>) -> Vec<Shape> {
        blocks.iter().collect()
    }

    const DOCUMENT: &str = "# Streaming\n\nFirst paragraph with **bold** text.\n\n- one\n- two\n  - nested\n\n```rust\nfn main() {}\n\n// a blank line inside\n```\n\n> a quote\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\nClosing words.\n";

    #[test]
    fn streaming_ends_up_identical_to_formatting_the_finished_text() {
        let mut streaming = cache();
        let mut last = None;
        let mut pieces = 0;
        for end in (17..=DOCUMENT.len()).step_by(17).chain([DOCUMENT.len()]) {
            let source = &DOCUMENT[..end];
            streaming.observe([("reply", source, true)]);
            settle(&mut streaming);
            if let Some(blocks) = streaming.view(&"reply", source, true) {
                last = Some(shapes(&blocks));
                pieces += 1;
            }
        }
        assert!(pieces > 5, "a view is available as the text grows");

        // The same text, finished: formatted as a whole.
        streaming.observe([("reply", DOCUMENT, false)]);
        settle(&mut streaming);
        let whole = shapes(
            &streaming
                .view(&"reply", DOCUMENT, false)
                .expect("finished view"),
        );
        assert_eq!(last.expect("streamed view"), whole);
        assert!(whole.len() >= 7);
    }

    #[test]
    fn while_the_next_version_formats_the_previous_one_stays_visible() {
        let mut streaming = cache();
        streaming.observe([("reply", "# Title\n\nFirst.", true)]);
        settle(&mut streaming);
        let first = shapes(&streaming.view(&"reply", "# Title\n\nFirst.", true).unwrap());
        // More text arrives; before polling, the old formatting is returned.
        streaming.observe([("reply", "# Title\n\nFirst. More.", true)]);
        let during = shapes(
            &streaming
                .view(&"reply", "# Title\n\nFirst. More.", true)
                .unwrap(),
        );
        assert_eq!(first, during);
        // A view for a source that was never observed has nothing newer either.
        assert!(streaming.view(&"other", "x", true).is_none());
    }

    #[test]
    fn delta_updates_match_observing_the_whole_set() {
        let (mut delta, mut full) = (cache(), cache());
        for end in (17..=DOCUMENT.len()).step_by(17).chain([DOCUMENT.len()]) {
            let source = &DOCUMENT[..end];
            // Another document stays untouched while "reply" streams.
            full.observe([
                ("reply", source, true),
                ("done", "Settled **text**.", false),
            ]);
            delta.observe_one("reply", source, true);
            if end == 17 {
                delta.observe_one("done", "Settled **text**.", false);
            }
            settle(&mut full);
            settle(&mut delta);
            assert_eq!(
                full.view(&"reply", source, true).map(|b| shapes(&b)),
                delta.view(&"reply", source, true).map(|b| shapes(&b)),
                "at {end} bytes"
            );
            assert_eq!(
                full.view(&"done", "Settled **text**.", false)
                    .map(|b| shapes(&b)),
                delta
                    .view(&"done", "Settled **text**.", false)
                    .map(|b| shapes(&b)),
            );
        }
        // Finishing replaces the pieces with the whole text, and `retain`
        // drops what left the screen.
        full.observe([("reply", DOCUMENT, false)]);
        delta.observe_one("reply", DOCUMENT, false);
        settle(&mut full);
        settle(&mut delta);
        assert_eq!(
            full.view(&"reply", DOCUMENT, false).map(|b| shapes(&b)),
            delta.view(&"reply", DOCUMENT, false).map(|b| shapes(&b)),
        );
        delta.retain(|key| *key == "reply");
        assert!(delta.view(&"done", "Settled **text**.", false).is_none());
        assert!(delta.view(&"reply", DOCUMENT, false).is_some());
    }

    #[test]
    fn a_streaming_document_keeps_one_model_that_is_updated_in_place() {
        let mut streaming = cache();
        let mut handle: Option<ModelRc<Shape>> = None;
        for end in (17..=DOCUMENT.len()).step_by(17).chain([DOCUMENT.len()]) {
            let source = &DOCUMENT[..end];
            streaming.observe_one("reply", source, true);
            settle(&mut streaming);
            let view = streaming.view(&"reply", source, true).expect("formatted");
            // The handle taken at the start follows every later update.
            let first = handle.get_or_insert_with(|| view.clone());
            assert_eq!(shapes(first), shapes(&view), "at {end} bytes");
            assert_eq!(
                view.row_count(),
                shapes(&view).len(),
                "the model has no stale rows"
            );
        }
        assert!(handle.unwrap().row_count() >= 7);
    }

    #[test]
    fn replaced_text_keeps_the_old_rows_until_the_new_text_is_formatted() {
        let mut streaming = cache();
        streaming.observe_one("reply", "# Old title\n\nOld body.", true);
        settle(&mut streaming);
        let old = streaming
            .view(&"reply", "# Old title\n\nOld body.", true)
            .expect("formatted");
        let before = shapes(&old);
        // The reply is regenerated: new text that does not extend the old.
        let new = "A different answer.\n\nWith two paragraphs.";
        streaming.observe_one("reply", new, true);
        let during = streaming.view(&"reply", new, true).expect("old rows stay");
        assert_eq!(
            shapes(&during),
            before,
            "nothing is blanked while formatting"
        );
        settle(&mut streaming);
        let after = streaming.view(&"reply", new, true).expect("formatted");
        assert_ne!(shapes(&after), before);
        assert_eq!(
            shapes(&old),
            shapes(&after),
            "the shown model was updated in place"
        );
    }
}
