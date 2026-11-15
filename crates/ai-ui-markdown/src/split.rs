//! Splitting a growing Markdown source into stable top-level blocks.
//!
//! While a reply streams, only the end of the text can still change.
//! [`StreamSplit`] keeps the blocks before the last two ("stable") so a host
//! formats each of them once, and re-parses only the live tail on every
//! update. Two blocks stay live, not one, because a half-typed line can look
//! like its own block and be absorbed by the block before it a moment later
//! (a lone `|` after a table, then `| 1`).
//!
//! This is exact for documents whose blocks do not depend on later text. A
//! link reference or footnote definition can change how earlier blocks render,
//! so once the source contains one the splitter stops stabilising and the
//! whole source is the tail. Hosts should format the finished text in one
//! piece, which is exact in every case.

use pulldown_cmark::{Event, Options, Parser};
use std::ops::Range;

const OPTIONS: Options = Options::ENABLE_TABLES
    .union(Options::ENABLE_STRIKETHROUGH)
    .union(Options::ENABLE_TASKLISTS)
    .union(Options::ENABLE_FOOTNOTES);

/// Byte ranges of the top-level blocks, each running from the end of the
/// previous block (the first from 0) to its own end, so the ranges tile the
/// source and trailing whitespace belongs to the last block.
pub fn top_level_blocks(source: &str) -> Vec<Range<usize>> {
    let mut ends: Vec<usize> = Vec::new();
    let mut depth = 0usize;
    for (event, range) in Parser::new_ext(source, OPTIONS).into_offset_iter() {
        match event {
            Event::Start(_) => depth += 1,
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    ends.push(range.end);
                }
            }
            _ if depth == 0 => ends.push(range.end),
            _ => {}
        }
    }
    if source.is_empty() {
        return Vec::new();
    }
    let mut blocks = Vec::with_capacity(ends.len().max(1));
    let mut start = 0;
    for end in ends {
        let end = end.clamp(start, source.len());
        if end > start {
            blocks.push(start..end);
            start = end;
        }
    }
    match blocks.last_mut() {
        Some(last) => last.end = source.len(),
        None => blocks.push(0..source.len()),
    }
    blocks
}

/// Blocks at the end that are never frozen.
const LIVE_BLOCKS: usize = 2;

/// A definition that later text could use to change earlier blocks.
fn has_definitions(text: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with('[') && line.contains("]:")
    })
}

/// The stable blocks and live tail of one growing document.
#[derive(Clone, Debug, Default)]
pub struct StreamSplit {
    stable: Vec<String>,
    /// The concatenation of `stable`.
    prefix: String,
    /// Definitions seen: nothing is stabilised any more.
    sticky_tail: bool,
}

impl StreamSplit {
    /// A splitter with nothing settled yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed the full current source. Returns the stable blocks (unchanged
    /// since earlier calls unless the text was replaced) and the live tail;
    /// together they are exactly `source`.
    pub fn update<'s, 'a>(&'s mut self, source: &'a str) -> (&'s [String], &'a str) {
        if !source.starts_with(&self.prefix) {
            // The text was replaced, not extended.
            *self = Self::default();
        }
        let tail = &source[self.prefix.len()..];
        if !self.sticky_tail {
            if has_definitions(tail) {
                self.sticky_tail = true;
            } else {
                let blocks = top_level_blocks(tail);
                if blocks.len() > LIVE_BLOCKS {
                    let frozen = &blocks[..blocks.len() - LIVE_BLOCKS];
                    for range in frozen {
                        self.stable.push(tail[range.clone()].to_owned());
                    }
                    let end = frozen[frozen.len() - 1].end;
                    self.prefix.push_str(&tail[..end]);
                }
            }
        }
        (&self.stable, &source[self.prefix.len()..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn blocks_of(source: &str) -> Vec<&str> {
        top_level_blocks(source)
            .into_iter()
            .map(|r| &source[r])
            .collect()
    }

    #[test]
    fn blocks_tile_the_source() {
        let source = "# Title\n\nFirst paragraph\nstill first.\n\n- a\n- b\n\n```rust\nlet x = 1;\n\nlet y = 2;\n```\n\n---\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n> quote\n";
        let blocks = blocks_of(source);
        assert_eq!(blocks.concat(), source);
        assert_eq!(blocks.len(), 7, "{blocks:?}");
        assert!(
            blocks[3].contains("let y = 2;"),
            "a fenced block with a blank line stays whole"
        );
        assert!(
            blocks[2].trim_start().starts_with("- a"),
            "a list is one block"
        );
        assert_eq!(top_level_blocks(""), Vec::<Range<usize>>::new());
    }

    #[test]
    fn a_loose_list_is_never_split() {
        let source = "1. one\n\n2. two\n\n3. three\n\nafter";
        let blocks = blocks_of(source);
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].contains("3. three"));
    }

    const DOCUMENT: &str = "# Heading\n\nA paragraph with **bold** and `code`.\n\n1. one\n2. two\n\n```rust\nfn main() {}\n\n// blank above\n```\n\n> quoted\n> text\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\nClosing paragraph.\n";

    #[test]
    fn growing_text_keeps_stable_blocks_and_matches_a_full_parse() {
        let mut split = StreamSplit::new();
        let mut seen_stable: Vec<String> = Vec::new();
        for end in (1..=DOCUMENT.len()).filter(|e| DOCUMENT.is_char_boundary(*e)) {
            let source = &DOCUMENT[..end];
            let (stable, tail) = split.update(source);
            assert_eq!(
                format!("{}{tail}", stable.concat()),
                source,
                "stable + tail is the source"
            );
            // Stable blocks only ever grow in number and never change.
            assert!(stable.len() >= seen_stable.len());
            assert_eq!(&stable[..seen_stable.len()], &seen_stable[..]);
            seen_stable = stable.to_vec();
            // Formatting the pieces equals formatting the whole.
            let mut pieces = Vec::new();
            for part in stable.iter().map(String::as_str).chain([tail]) {
                pieces.extend(parse(part).unwrap().blocks);
            }
            assert_eq!(
                pieces,
                parse(source).unwrap().blocks,
                "differs at {end} bytes: {source:?}"
            );
        }
        assert!(seen_stable.len() >= 4);
    }

    /// Streaming prefix by prefix, the pieces always format like the whole.
    #[test]
    fn equivalence_holds_for_other_structures() {
        let documents = [
            "Setext title\n============\n\nText after.\n\n- [x] done\n- [ ] todo\n  - nested **item**\n\n~~struck~~ words\n\n---\n\nEnd.",
            "> quote with list:\n> - one\n> - two\n>\n> ```\n> code\n> ```\n\nnext paragraph\n\n    indented code\n\nlast",
            "<div>\nhtml block\n</div>\n\n| h1 | h2 |\n|:---|---:|\n| a | b |\n| c | d |\n\n1. first\n\n   continued paragraph\n\n2. second\n\nDone.\n",
            "Hard  \nbreak and a [link](https://example.com) and ![img](a.png).\n\n## Sub\n\n```\nunterminated fence\n\nstill code",
        ];
        for document in documents {
            let mut split = StreamSplit::new();
            for end in (1..=document.len()).filter(|e| document.is_char_boundary(*e)) {
                let source = &document[..end];
                let (stable, tail) = split.update(source);
                assert_eq!(format!("{}{tail}", stable.concat()), source);
                let mut pieces = Vec::new();
                for part in stable.iter().map(String::as_str).chain([tail]) {
                    pieces.extend(parse(part).unwrap().blocks);
                }
                assert_eq!(
                    pieces,
                    parse(source).unwrap().blocks,
                    "differs at {end} bytes of {document:?}"
                );
            }
        }
    }

    #[test]
    fn definitions_stop_stabilising_and_replacement_resets() {
        let mut split = StreamSplit::new();
        split.update("One.\n\nTwo.\n\nThree.");
        assert!(!split.update("One.\n\nTwo.\n\nThree.").0.is_empty());
        let source = "One.\n\nTwo.\n\nSee [x][ref].\n\n[ref]: https://example.com\n\nAfter.";
        let mut split = StreamSplit::new();
        let (stable, tail) = split.update(source);
        assert!(
            stable.is_empty(),
            "a definition keeps everything in the tail"
        );
        assert_eq!(tail, source);
        // New, unrelated text starts over.
        let (stable, tail) = split.update("Different.\n\nStart.");
        assert_eq!((stable.len(), tail), (0, "Different.\n\nStart."));
    }
}
