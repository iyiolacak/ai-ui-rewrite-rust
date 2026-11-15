//! Portable, bounded Markdown presentation data for assistant messages.
//!
//! Parsing is pure: links, images, and HTML are untrusted inert data. This crate
//! never fetches images, opens links, renders HTML, or performs I/O. A host may
//! reparse the currently received source on every stream update; unfinished
//! fences follow CommonMark's end-of-input rules and may change on the next parse.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

// Parsed and projected messages can contain private text, code and URLs.
// Callers that need the content can deliberately inspect the fields.
macro_rules! redacted_debug {
    ($($ty:ty),* $(,)?) => {$ (
        impl std::fmt::Debug for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($ty)).finish_non_exhaustive()
            }
        }
    )* };
}

#[cfg(not(target_arch = "wasm32"))]
pub mod coordinator;
#[cfg(not(target_arch = "wasm32"))]
pub mod pool;
pub mod presentation;
pub mod scheduling;
pub mod split;
#[cfg(not(target_arch = "wasm32"))]
pub mod worker;

use pulldown_cmark::{Alignment as ParserAlignment, CodeBlockKind, Event, Options, Parser, Tag};
use std::fmt;

/// Maximum UTF-8 bytes accepted for a single document.
pub const MAX_SOURCE_BYTES: usize = 1024 * 1024;
/// Maximum parser events accepted, including wrapper events.
pub const MAX_EVENTS: usize = 100_000;
/// Maximum nesting of markup containers.
pub const MAX_NESTING: usize = 128;
/// Maximum bytes copied into the resulting strings, including resolved references.
pub const MAX_OUTPUT_BYTES: usize = 4 * 1024 * 1024;

/// A parsed Markdown document.
#[derive(Clone, PartialEq, Eq)]
pub struct Document {
    /// The top-level blocks.
    pub blocks: Vec<Block>,
}

/// A block-level element.
#[derive(Clone, PartialEq, Eq)]
pub enum Block {
    /// A paragraph.
    Paragraph(Vec<Inline>),
    /// A heading.
    Heading {
        /// 1 to 6.
        level: u8,
        /// The heading's inline content.
        content: Vec<Inline>,
    },
    /// A fenced or indented code block.
    CodeBlock {
        /// The info string's language, if any.
        language: Option<String>,
        /// The code, verbatim.
        text: String,
    },
    /// A block quote.
    Quote(Vec<Block>),
    /// A list.
    List {
        /// The first number of an ordered list; `None` for a bulleted list.
        start: Option<u64>,
        /// The items.
        items: Vec<ListItem>,
    },
    /// A table.
    Table {
        /// The alignment of each column.
        alignments: Vec<Alignment>,
        /// The header cells.
        head: Vec<Vec<Inline>>,
        /// The body rows, each a list of cells.
        rows: Vec<Vec<Vec<Inline>>>,
    },
    /// A horizontal rule.
    ThematicBreak,
    /// Raw, untrusted markup; display literally, never execute it.
    Html(String),
    /// The text of a footnote.
    FootnoteDefinition {
        /// The footnote's label.
        label: String,
        /// The footnote's content.
        blocks: Vec<Block>,
    },
}

/// One list item.
#[derive(Clone, PartialEq, Eq)]
pub struct ListItem {
    /// The item's content.
    pub blocks: Vec<Block>,
}

/// How a table column aligns its cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Alignment {
    /// No alignment given.
    None,
    /// Left.
    Left,
    /// Centred.
    Center,
    /// Right.
    Right,
}

/// Inline content.
#[derive(Clone, PartialEq, Eq)]
pub enum Inline {
    /// Plain text.
    Text(String),
    /// Inline code.
    Code(String),
    /// Emphasis (italics).
    Emphasis(Vec<Inline>),
    /// Strong emphasis (bold).
    Strong(Vec<Inline>),
    /// Struck-through text.
    Strikethrough(Vec<Inline>),
    /// Destinations are untrusted strings and must not be opened automatically.
    Link {
        /// Where the link points; untrusted.
        destination: String,
        /// The link's title, possibly empty.
        title: String,
        /// The link text.
        content: Vec<Inline>,
    },
    /// Destination and alt text are untrusted; no fetching is performed.
    Image {
        /// The image address; untrusted and never fetched.
        destination: String,
        /// The image's title, possibly empty.
        title: String,
        /// The alternative text.
        alt: Vec<Inline>,
    },
    /// A line break inside a paragraph that renders as a space.
    SoftBreak,
    /// A forced line break.
    HardBreak,
    /// Raw, untrusted markup; display literally, never execute it.
    Html(String),
    /// A task list marker; `true` when checked.
    TaskMarker(bool),
    /// A reference to a footnote by label.
    FootnoteReference(String),
}

redacted_debug!(Document, Block, ListItem, Inline);

/// Why a document could not be parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// The source exceeds the size limit.
    SourceTooLarge {
        /// The limit, in bytes.
        limit: usize,
    },
    /// The document produces more parser events than allowed.
    TooManyEvents {
        /// The limit.
        limit: usize,
    },
    /// Blocks nest deeper than allowed.
    NestingTooDeep {
        /// The limit.
        limit: usize,
    },
    /// The parsed document is larger than allowed.
    OutputTooLarge {
        /// The limit.
        limit: usize,
    },
    /// The parser produced a construct for which this API has no representation.
    UnsupportedSyntax,
}
impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceTooLarge { limit } => write!(f, "Markdown source exceeds {limit} bytes"),
            Self::TooManyEvents { limit } => write!(f, "Markdown exceeds {limit} parser events"),
            Self::NestingTooDeep { limit } => write!(f, "Markdown nesting exceeds {limit}"),
            Self::OutputTooLarge { limit } => write!(f, "Markdown output exceeds {limit} bytes"),
            Self::UnsupportedSyntax => write!(f, "unsupported Markdown parser construct"),
        }
    }
}
impl std::error::Error for ParseError {}

// Keep the borrowed parser tree bounded independently from the output. A stack
// avoids recursively consuming parser events; conversion depth is then limited.
enum Node<'a> {
    Container(Tag<'a>, Vec<Node<'a>>),
    Leaf(Event<'a>),
}

/// Parse one complete current source snapshot. Limits fail atomically: callers
/// receive an error rather than a partial or silently truncated document.
pub fn parse(source: &str) -> Result<Document, ParseError> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err(ParseError::SourceTooLarge {
            limit: MAX_SOURCE_BYTES,
        });
    }
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES;
    let mut stack: Vec<(Tag<'_>, Vec<Node<'_>>)> = Vec::new();
    let mut root = Vec::new();
    for (index, event) in Parser::new_ext(source, options).enumerate() {
        if index >= MAX_EVENTS {
            return Err(ParseError::TooManyEvents { limit: MAX_EVENTS });
        }
        match event {
            Event::Start(tag) => {
                if stack.len() >= MAX_NESTING {
                    return Err(ParseError::NestingTooDeep { limit: MAX_NESTING });
                }
                stack.push((tag, Vec::new()));
            }
            Event::End(end) => {
                let (tag, children) = stack.pop().ok_or(ParseError::UnsupportedSyntax)?;
                if tag.to_end() != end {
                    return Err(ParseError::UnsupportedSyntax);
                }
                let node = Node::Container(tag, children);
                if let Some((_, nodes)) = stack.last_mut() {
                    nodes.push(node);
                } else {
                    root.push(node);
                }
            }
            leaf => {
                if let Some((_, nodes)) = stack.last_mut() {
                    nodes.push(Node::Leaf(leaf));
                } else {
                    root.push(Node::Leaf(leaf));
                }
            }
        }
    }
    if !stack.is_empty() {
        return Err(ParseError::UnsupportedSyntax);
    }
    Ok(Document {
        blocks: Convert { output_bytes: 0 }.blocks(root)?,
    })
}

struct Convert {
    output_bytes: usize,
}
impl Convert {
    fn string(&mut self, value: &str) -> Result<String, ParseError> {
        self.output_bytes =
            self.output_bytes
                .checked_add(value.len())
                .ok_or(ParseError::OutputTooLarge {
                    limit: MAX_OUTPUT_BYTES,
                })?;
        if self.output_bytes > MAX_OUTPUT_BYTES {
            return Err(ParseError::OutputTooLarge {
                limit: MAX_OUTPUT_BYTES,
            });
        }
        Ok(value.to_owned())
    }
    fn blocks(&mut self, nodes: Vec<Node<'_>>) -> Result<Vec<Block>, ParseError> {
        let mut out = Vec::new();
        let mut inline_nodes = Vec::new();
        for node in nodes {
            match node {
                Node::Container(tag, children) => match tag {
                    Tag::Paragraph => {
                        self.flush_inline(&mut inline_nodes, &mut out)?;
                        out.push(Block::Paragraph(self.inlines(children)?));
                    }
                    Tag::Heading { level, .. } => {
                        self.flush_inline(&mut inline_nodes, &mut out)?;
                        out.push(Block::Heading {
                            level: level as u8,
                            content: self.inlines(children)?,
                        });
                    }
                    Tag::BlockQuote(_) => {
                        self.flush_inline(&mut inline_nodes, &mut out)?;
                        out.push(Block::Quote(self.blocks(children)?));
                    }
                    Tag::CodeBlock(kind) => {
                        self.flush_inline(&mut inline_nodes, &mut out)?;
                        let language = match kind {
                            CodeBlockKind::Indented => None,
                            CodeBlockKind::Fenced(info) => info
                                .split_whitespace()
                                .next()
                                .filter(|x| !x.is_empty())
                                .map(|x| self.string(x))
                                .transpose()?,
                        };
                        let text = self.raw_text(children)?;
                        out.push(Block::CodeBlock { language, text });
                    }
                    Tag::HtmlBlock => {
                        self.flush_inline(&mut inline_nodes, &mut out)?;
                        let text = self.raw_text(children)?;
                        out.push(Block::Html(text));
                    }
                    Tag::FootnoteDefinition(label) => {
                        self.flush_inline(&mut inline_nodes, &mut out)?;
                        out.push(Block::FootnoteDefinition {
                            label: self.string(&label)?,
                            blocks: self.blocks(children)?,
                        });
                    }
                    Tag::List(start) => {
                        self.flush_inline(&mut inline_nodes, &mut out)?;
                        let mut items = Vec::new();
                        for child in children {
                            if let Node::Container(Tag::Item, content) = child {
                                items.push(ListItem {
                                    blocks: self.blocks(content)?,
                                });
                            } else {
                                return Err(ParseError::UnsupportedSyntax);
                            }
                        }
                        out.push(Block::List { start, items });
                    }
                    Tag::Table(alignments) => {
                        self.flush_inline(&mut inline_nodes, &mut out)?;
                        let mut head = Vec::new();
                        let mut rows = Vec::new();
                        for child in children {
                            match child {
                                Node::Container(Tag::TableHead, cells) => {
                                    head = self.cells(cells)?
                                }
                                Node::Container(Tag::TableRow, cells) => {
                                    rows.push(self.cells(cells)?)
                                }
                                _ => return Err(ParseError::UnsupportedSyntax),
                            }
                        }
                        out.push(Block::Table {
                            alignments: alignments
                                .into_iter()
                                .map(|a| match a {
                                    ParserAlignment::None => Alignment::None,
                                    ParserAlignment::Left => Alignment::Left,
                                    ParserAlignment::Center => Alignment::Center,
                                    ParserAlignment::Right => Alignment::Right,
                                })
                                .collect(),
                            head,
                            rows,
                        });
                    }
                    other => inline_nodes.push(Node::Container(other, children)),
                },
                Node::Leaf(Event::Rule) => {
                    self.flush_inline(&mut inline_nodes, &mut out)?;
                    out.push(Block::ThematicBreak);
                }
                node => inline_nodes.push(node),
            }
        }
        self.flush_inline(&mut inline_nodes, &mut out)?;
        Ok(out)
    }
    fn flush_inline<'a>(
        &mut self,
        nodes: &mut Vec<Node<'a>>,
        out: &mut Vec<Block>,
    ) -> Result<(), ParseError> {
        if !nodes.is_empty() {
            out.push(Block::Paragraph(self.inlines(std::mem::take(nodes))?));
        }
        Ok(())
    }
    fn cells(&mut self, nodes: Vec<Node<'_>>) -> Result<Vec<Vec<Inline>>, ParseError> {
        nodes
            .into_iter()
            .map(|node| match node {
                Node::Container(Tag::TableCell, contents) => self.inlines(contents),
                _ => Err(ParseError::UnsupportedSyntax),
            })
            .collect()
    }
    fn raw_text(&mut self, nodes: Vec<Node<'_>>) -> Result<String, ParseError> {
        let mut out = String::new();
        for node in nodes {
            match node {
                Node::Leaf(Event::Text(s) | Event::Html(s) | Event::InlineHtml(s)) => {
                    self.output_bytes = self.output_bytes.checked_add(s.len()).ok_or(
                        ParseError::OutputTooLarge {
                            limit: MAX_OUTPUT_BYTES,
                        },
                    )?;
                    if self.output_bytes > MAX_OUTPUT_BYTES {
                        return Err(ParseError::OutputTooLarge {
                            limit: MAX_OUTPUT_BYTES,
                        });
                    }
                    out.push_str(&s);
                }
                _ => return Err(ParseError::UnsupportedSyntax),
            }
        }
        Ok(out)
    }
    fn inlines(&mut self, nodes: Vec<Node<'_>>) -> Result<Vec<Inline>, ParseError> {
        let mut out = Vec::new();
        for node in nodes {
            let inline = match node {
                Node::Leaf(Event::Text(s)) => Inline::Text(self.string(&s)?),
                Node::Leaf(Event::Code(s)) => Inline::Code(self.string(&s)?),
                Node::Leaf(Event::InlineHtml(s) | Event::Html(s)) => Inline::Html(self.string(&s)?),
                Node::Leaf(Event::SoftBreak) => Inline::SoftBreak,
                Node::Leaf(Event::HardBreak) => Inline::HardBreak,
                Node::Leaf(Event::TaskListMarker(checked)) => Inline::TaskMarker(checked),
                Node::Leaf(Event::FootnoteReference(label)) => {
                    Inline::FootnoteReference(self.string(&label)?)
                }
                Node::Container(Tag::Emphasis, children) => {
                    Inline::Emphasis(self.inlines(children)?)
                }
                Node::Container(Tag::Strong, children) => Inline::Strong(self.inlines(children)?),
                Node::Container(Tag::Strikethrough, children) => {
                    Inline::Strikethrough(self.inlines(children)?)
                }
                Node::Container(
                    Tag::Link {
                        dest_url, title, ..
                    },
                    children,
                ) => Inline::Link {
                    destination: self.string(&dest_url)?,
                    title: self.string(&title)?,
                    content: self.inlines(children)?,
                },
                Node::Container(
                    Tag::Image {
                        dest_url, title, ..
                    },
                    children,
                ) => Inline::Image {
                    destination: self.string(&dest_url)?,
                    title: self.string(&title)?,
                    alt: self.inlines(children)?,
                },
                _ => return Err(ParseError::UnsupportedSyntax),
            };
            out.push(inline);
        }
        Ok(out)
    }
}
