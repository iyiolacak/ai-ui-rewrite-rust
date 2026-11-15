//! Long streaming text as consecutive chunks for one Slint model.
//!
//! A `Text` element re-wraps its whole string whenever it changes, so a
//! streamed reasoning part of tens of kilobytes cost more on every update as it
//! grew. Here the text is cut at line breaks into chunks of a few kilobytes;
//! finished chunks are appended once and never touched again, and only the
//! last one is rewritten per update. Slint keeps the layout of unchanged
//! chunks, and the text is converted to Slint strings once instead of whole on
//! every update.

use slint::{Model, ModelRc, SharedString, VecModel};
use std::rc::Rc;

/// Text shorter than this stays one string.
pub const MIN_CHUNKED: usize = 4 * 1024;
/// A chunk ends at the first line break after this many bytes...
const CHUNK: usize = 2 * 1024;
/// ...or, in a line longer than this, at the last space before it (the only
/// case where chunking adds a line break the text does not have).
const MAX_CHUNK: usize = 16 * 1024;

pub struct Chunks {
    model: Rc<VecModel<SharedString>>,
    /// Bytes of the source in finished chunks, including the line break that
    /// ended the last one.
    frozen: usize,
    /// The source's first `frozen` bytes, to tell an extended text (the usual
    /// streaming case) from a replaced one. Comparing is one memcmp.
    prefix: Vec<u8>,
}

impl Chunks {
    pub fn new() -> Self {
        Self {
            model: Rc::new(VecModel::default()),
            frozen: 0,
            prefix: Vec::new(),
        }
    }

    fn extends(&self, text: &str) -> bool {
        text.as_bytes().starts_with(&self.prefix)
    }

    /// Bring the chunks up to date with `text` and return the model (always the
    /// same one, updated in place).
    pub fn update(&mut self, text: &str) -> ModelRc<SharedString> {
        if self.frozen > 0 && !self.extends(text) {
            self.model.set_vec(Vec::new());
            self.frozen = 0;
            self.prefix.clear();
        }
        // The last row is the live tail. Rows are written in place, so Slint
        // keeps the tail's element and only re-wraps its text.
        let mut row = self.model.row_count().saturating_sub(1);
        let write = |row: &mut usize, text: &str| {
            let value = SharedString::from(text);
            if *row < self.model.row_count() {
                self.model.set_row_data(*row, value);
            } else {
                self.model.push(value);
            }
            *row += 1;
        };
        let mut start = self.frozen;
        while let Some((end, next)) = cut(text, start) {
            write(&mut row, &text[start..end]);
            start = next;
        }
        write(&mut row, &text[start..]);
        if start != self.frozen {
            self.prefix
                .extend_from_slice(&text.as_bytes()[self.frozen..start]);
            self.frozen = start;
        }
        ModelRc::from(self.model.clone())
    }
}

/// Where the chunk starting at `start` ends (exclusive) and the next begins,
/// if the text past `start` is long enough to finish a chunk.
fn cut(text: &str, start: usize) -> Option<(usize, usize)> {
    let rest = &text.as_bytes()[start..];
    if rest.len() <= CHUNK {
        return None;
    }
    let window = &rest[CHUNK..rest.len().min(MAX_CHUNK)];
    if let Some(i) = window.iter().position(|&b| b == b'\n') {
        let end = start + CHUNK + i;
        return Some((end, end + 1));
    }
    if rest.len() <= MAX_CHUNK {
        return None; // the line may still end before MAX_CHUNK
    }
    let space = rest[..MAX_CHUNK].iter().rposition(|&b| b == b' ')?;
    let end = start + space;
    Some((end, end + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn joined(model: &ModelRc<SharedString>) -> Vec<String> {
        model.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn chunks_rejoin_to_the_text_and_only_the_tail_changes() {
        let line = "word ".repeat(100) + "\n"; // 501 bytes
        let mut text = String::new();
        let mut chunks = Chunks::new();
        let mut previous: Vec<String> = Vec::new();
        for _ in 0..40 {
            text.push_str(&line);
            let parts = joined(&chunks.update(&text));
            assert_eq!(parts.join("\n"), text);
            // Every chunk but the last is unchanged from the previous update.
            for (i, part) in previous
                .iter()
                .enumerate()
                .take(previous.len().saturating_sub(1))
            {
                assert_eq!(&parts[i], part);
            }
            previous = parts;
        }
        assert!(previous.len() > 5);
    }

    #[test]
    fn a_replaced_text_starts_over() {
        let mut chunks = Chunks::new();
        let first = "a".repeat(100) + &"line\n".repeat(1000);
        chunks.update(&first);
        let second = "b".repeat(100) + &"line\n".repeat(1000);
        let parts = joined(&chunks.update(&second));
        assert_eq!(parts.join("\n"), second);
    }

    #[test]
    fn a_very_long_line_is_cut_at_a_space() {
        let text = "word ".repeat(10_000); // 50 KB, no line break
        let parts = joined(&Chunks::new().update(&text));
        assert!(parts.len() >= 3);
        assert_eq!(parts.join(" "), text);
    }
}
