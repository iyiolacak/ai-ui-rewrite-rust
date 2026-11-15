//! Bounded layout for unusually large user messages.
//!
//! The runtime keeps the complete source for copy, edit and model requests.
//! Here we retain only its `Arc` and byte ranges, then convert the visible page
//! to a Slint string. Every page is a verbatim UTF-8 slice of that source.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

// Upstream shows a user message whole, so paging starts only where one Text
// would become a problem: Slint's software renderer works in 16-bit
// coordinates (~32k px), and laying out the whole text costs ~0.3 ms per KB
// once. Up to 64 KB or 1000 lines (about 20k px at 20 px a line) a message stays
// one block, as in upstream; a pasted log or file is not split into pages.
// Pages of 16 KB or 400 lines stay far below the limit.
const UNPAGED_BYTES: usize = 64 * 1024;
const UNPAGED_LINES: usize = 1000;
const PAGE_BYTES: usize = 16 * 1024;
const PAGE_LINES: usize = 400;

struct Entry {
    ranges: Vec<Range<usize>>,
    selected: usize,
    token: u64,
}

struct MessagePages<S> {
    source: Arc<S>,
    parts: HashMap<usize, Entry>,
}

pub struct Page<'a> {
    pub text: &'a str,
    pub number: usize,
    pub count: usize,
    pub token: u64,
}

/// `source` must change identity whenever the text changes. Keeping an `Arc`
/// prevents an allocator-reused address from impersonating an older source.
pub struct Pages<S> {
    entries: HashMap<String, MessagePages<S>>,
    scope: i32,
    next_token: u64,
}

impl<S> Pages<S> {
    pub fn new(scope: i32) -> Self {
        Self {
            entries: HashMap::new(),
            scope,
            next_token: 0,
        }
    }

    /// Thread switches discard presentation state even if message IDs repeat.
    pub fn reset(&mut self, scope: i32) {
        self.entries.clear();
        self.scope = scope;
    }

    pub fn retain(&mut self, keep: impl Fn(&str) -> bool) {
        self.entries.retain(|id, _| keep(id));
    }

    /// Remove every old part when a message changes, including parts that no
    /// longer exist and therefore will never be passed to `project` again.
    pub fn reconcile_source(&mut self, id: &str, source: &Arc<S>) {
        if self
            .entries
            .get(id)
            .is_some_and(|message| !Arc::ptr_eq(&message.source, source))
        {
            self.entries.remove(id);
        }
    }

    fn token(&mut self) -> u64 {
        self.next_token = self
            .next_token
            .checked_add(1)
            .expect("page token exhausted");
        self.next_token
    }

    pub fn project<'a>(
        &mut self,
        id: &str,
        part: usize,
        source: &Arc<S>,
        text: &'a str,
    ) -> Option<Page<'a>> {
        self.reconcile_source(id, source);
        if !needs_paging(text) {
            if let Some(message) = self.entries.get_mut(id) {
                message.parts.remove(&part);
                if message.parts.is_empty() {
                    self.entries.remove(id);
                }
            }
            return None;
        }
        if !self
            .entries
            .get(id)
            .is_some_and(|message| message.parts.contains_key(&part))
        {
            let token = self.token();
            self.entries
                .entry(id.to_owned())
                .or_insert_with(|| MessagePages {
                    source: source.clone(),
                    parts: HashMap::new(),
                })
                .parts
                .insert(
                    part,
                    Entry {
                        ranges: ranges(text),
                        selected: 0,
                        token,
                    },
                );
        }
        let entry = &self.entries[id].parts[&part];
        Some(Page {
            text: &text[entry.ranges[entry.selected].clone()],
            number: entry.selected + 1,
            count: entry.ranges.len(),
            token: entry.token,
        })
    }

    /// Validate the exact conversation, source and displayed page before
    /// navigating. Replayed clicks and callbacks from replaced rows do nothing.
    pub fn advance(
        &mut self,
        scope: i32,
        key: (&str, usize),
        source: &Arc<S>,
        token: &str,
        page: i32,
        delta: i32,
    ) -> bool {
        if scope != self.scope || !matches!(delta, -1 | 1) {
            return false;
        }
        let Some(message) = self.entries.get(key.0) else {
            return false;
        };
        if !Arc::ptr_eq(&message.source, source) {
            return false;
        }
        let Some(entry) = message.parts.get(&key.1) else {
            return false;
        };
        if token.parse::<u64>().ok() != Some(entry.token)
            || usize::try_from(page).ok() != Some(entry.selected + 1)
        {
            return false;
        }
        let next = if delta < 0 {
            entry.selected.checked_sub(1)
        } else {
            entry.selected.checked_add(1)
        };
        let Some(next) = next.filter(|&index| index < entry.ranges.len()) else {
            return false;
        };
        let token = self.token();
        let entry = self
            .entries
            .get_mut(key.0)
            .and_then(|message| message.parts.get_mut(&key.1))
            .expect("checked above");
        entry.selected = next;
        entry.token = token;
        true
    }
}

fn needs_paging(text: &str) -> bool {
    text.len() > UNPAGED_BYTES
        || text
            .bytes()
            .filter(|&byte| byte == b'\n')
            .take(UNPAGED_LINES)
            .count()
            == UNPAGED_LINES
}

/// Scan the source once. Prefer existing line/word boundaries in the latter
/// half of a page; the hard byte/line limits also handle unbroken and dense text.
fn ranges(text: &str) -> Vec<Range<usize>> {
    let mut pages = Vec::with_capacity(text.len() / PAGE_BYTES + 1);
    let mut start = 0;
    while start < text.len() {
        let mut end = (start + PAGE_BYTES).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let window = &text[start..end];
        if let Some((position, _)) = window
            .bytes()
            .enumerate()
            .filter(|(_, byte)| *byte == b'\n')
            .nth(PAGE_LINES - 1)
        {
            end = start + position + 1;
        } else if end < text.len() {
            let halfway = window.len() / 2;
            let boundary = window
                .char_indices()
                .rev()
                .find(|&(position, character)| position >= halfway && character == '\n')
                .or_else(|| {
                    window.char_indices().rev().find(|&(position, character)| {
                        position >= halfway && character.is_whitespace()
                    })
                });
            if let Some((position, character)) = boundary {
                end = start + position + character.len_utf8();
            }
            // Keep CRLF together even when the byte budget lands between it.
            if text.as_bytes()[end - 1] == b'\r' && text.as_bytes().get(end) == Some(&b'\n') {
                end -= 1;
            }
        }
        pages.push(start..end);
        start = end;
    }
    pages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_preserve_every_byte_and_bound_dense_unbroken_and_unicode_text() {
        for text in [
            "line\r\n".repeat(5000),
            "x".repeat(512 * 1024),
            "🦀 café 東京 e\u{301}\n".repeat(5000),
            "\n".repeat(20_000),
        ] {
            let pages = ranges(&text);
            assert!(pages.len() > 1);
            assert_eq!(pages.first().unwrap().start, 0);
            assert_eq!(pages.last().unwrap().end, text.len());
            let mut previous = 0;
            for range in pages {
                assert_eq!(range.start, previous);
                let page = &text[range.clone()];
                assert!(!page.is_empty());
                assert!(page.len() <= PAGE_BYTES);
                assert!(page.bytes().filter(|&b| b == b'\n').count() <= PAGE_LINES);
                assert!(!(page.ends_with('\r') && text[range.end..].starts_with('\n')));
                previous = range.end;
            }
        }
    }

    #[test]
    fn ordinary_messages_are_not_paged() {
        let mut pages = Pages::new(1);
        let source = Arc::new(());
        for text in ["".into(), "Hello, world!".into(), "x".repeat(UNPAGED_BYTES)] {
            assert!(pages.project("m", 0, &source, &text).is_none());
        }
        assert!(pages
            .project("m", 0, &source, &"\n".repeat(UNPAGED_LINES))
            .is_some());
    }

    #[test]
    fn navigation_rejects_stale_pages_sources_and_conversations() {
        let text = "a word ".repeat(10_000);
        let source = Arc::new(());
        let mut pages = Pages::new(1);
        let first = pages.project("m", 0, &source, &text).unwrap();
        let token = first.token.to_string();
        assert!(!pages.advance(1, ("m", 0), &source, &token, 1, -1));
        assert!(!pages.advance(1, ("m", 0), &source, &token, 1, 2));
        assert!(!pages.advance(1, ("m", 1), &source, &token, 1, 1));
        assert!(!pages.advance(2, ("m", 0), &source, &token, 1, 1));
        assert!(pages.advance(1, ("m", 0), &source, &token, 1, 1));
        assert!(!pages.advance(1, ("m", 0), &source, &token, 1, 1));
        let second = pages.project("m", 0, &source, &text).unwrap();
        assert_eq!(second.number, 2);
        let second_token = second.token.to_string();
        let replacement = Arc::new(());
        assert!(!pages.advance(1, ("m", 0), &replacement, &second_token, 2, 1));
        let replaced = pages.project("m", 0, &replacement, &text).unwrap();
        assert_eq!(replaced.number, 1);
        assert_ne!(replaced.token.to_string(), second_token);
        pages.reset(2);
        assert!(!pages.advance(1, ("m", 0), &source, &token, 1, 1));
        let new_thread = pages.project("m", 0, &source, &text).unwrap();
        assert_eq!(new_thread.number, 1);
        assert_ne!(new_thread.token.to_string(), token);
    }

    #[test]
    fn edited_short_text_and_removed_messages_release_cached_sources() {
        let source = Arc::new(());
        let mut pages = Pages::new(1);
        let text = "x".repeat(UNPAGED_BYTES + 1);
        pages.project("m", 0, &source, &text);
        assert_eq!(Arc::strong_count(&source), 2);
        assert!(pages.project("m", 0, &Arc::new(()), "short edit").is_none());
        assert_eq!(Arc::strong_count(&source), 1);
        pages.project("m", 0, &source, &text);
        pages.retain(|id| id != "m");
        assert_eq!(Arc::strong_count(&source), 1);
    }

    #[test]
    fn replacing_a_message_releases_removed_parts_without_rescanning_other_messages() {
        let source = Arc::new(());
        let mut pages = Pages::new(1);
        let text = "x".repeat(UNPAGED_BYTES + 1);
        pages.project("m", 0, &source, &text);
        pages.project("m", 1, &source, &text);
        assert_eq!(Arc::strong_count(&source), 2);
        let replacement = Arc::new(());
        pages.reconcile_source("m", &replacement);
        assert_eq!(Arc::strong_count(&source), 1);
        assert!(pages.entries.is_empty());
        pages.project("m", 0, &replacement, &text);
        assert!(!pages.entries["m"].parts.contains_key(&1));
    }
}
