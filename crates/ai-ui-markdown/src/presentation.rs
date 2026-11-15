//! UI-independent projection of a Markdown snapshot. This module performs no I/O.
//!
//! `markdown` is a newly serialized, sanitized representation suitable for a
//! Markdown-capable text widget; `plain` preserves the corresponding visible
//! text for a host that cannot format it. Raw HTML, images and unsupported URLs
//! are text, never executable content. Links require an explicit host action.
use crate::{parse, Alignment, Block, Inline, ParseError};
use std::fmt;
use std::fmt::Write as _;
use url::Url;

/// Bound the total number of independently rendered blocks and table cells.
pub const MAX_PRESENTATION_NODES: usize = 4096;
/// Bound the size of all serialized and plain presentation strings together.
pub const MAX_PRESENTATION_BYTES: usize = 16 * 1024 * 1024;

/// A document reduced to blocks a UI can lay out.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Presentation {
    /// The blocks, in order.
    pub blocks: Vec<RenderBlock>,
}

impl Presentation {
    /// Validate a presentation received across a trusted executor boundary.
    /// This checks structural resource limits, not source equivalence or safety
    /// against a malicious executor. Deserialization has already allocated it.
    pub fn validate_limits(&self) -> Result<(), ProjectionError> {
        let (mut nodes, mut bytes) = (0, 0);
        for block in &self.blocks {
            account(block, &mut nodes, &mut bytes)?;
        }
        Ok(())
    }
}

/// One block of a presentation.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RenderBlock {
    /// A paragraph, heading or other run of text.
    Text {
        /// The text as sanitized Markdown for a widget that formats it.
        markdown: String,
        /// The same text without formatting.
        plain: String,
        /// List nesting depth.
        depth: u16,
        /// Block quote nesting depth.
        quote_depth: u16,
        /// The heading level (1 to 6) when the block is a heading.
        heading: Option<u8>,
    },
    /// A code block.
    Code {
        /// The language, if given.
        language: Option<String>,
        /// The code, verbatim.
        code: String,
        /// List nesting depth.
        depth: u16,
        /// Block quote nesting depth.
        quote_depth: u16,
    },
    /// A horizontal rule.
    Rule {
        /// List nesting depth.
        depth: u16,
        /// Block quote nesting depth.
        quote_depth: u16,
    },
    /// One row of a table.
    TableRow {
        /// Whether this is the header row.
        header: bool,
        /// The row's cells.
        cells: Vec<RenderCell>,
        /// List nesting depth.
        depth: u16,
        /// Block quote nesting depth.
        quote_depth: u16,
    },
}

/// One cell of a table row.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RenderCell {
    /// The cell text as sanitized Markdown.
    pub markdown: String,
    /// The cell text without formatting.
    pub plain: String,
    /// The column's alignment.
    pub alignment: Alignment,
}

// Presentation strings and code retain conversation content, including URLs.
redacted_debug!(Presentation, RenderBlock, RenderCell);

/// Why a document could not be turned into a presentation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectionError {
    /// The document could not be parsed.
    Parse(ParseError),
    /// The presentation would have more blocks and cells than allowed.
    TooManyNodes {
        /// The limit.
        limit: usize,
    },
    /// The presentation would be larger than allowed.
    OutputTooLarge {
        /// The limit, in bytes.
        limit: usize,
    },
}
impl fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(error) => write!(f, "{error}"),
            Self::TooManyNodes { limit } => {
                write!(f, "Markdown presentation exceeds {limit} nodes")
            }
            Self::OutputTooLarge { limit } => {
                write!(f, "Markdown presentation exceeds {limit} bytes")
            }
        }
    }
}
impl std::error::Error for ProjectionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Parse(error) => Some(error),
            _ => None,
        }
    }
}

/// Produce a complete snapshot, or an error with no partial presentation.
/// Consumers should show the original source literally if this fails.
pub fn project(source: &str) -> Result<Presentation, ProjectionError> {
    let document = parse(source).map_err(ProjectionError::Parse)?;
    let mut context = Context {
        blocks: Vec::new(),
        bytes: 0,
        nodes: 0,
    };
    context.walk(&document.blocks, 0, 0, None)?;
    Ok(Presentation {
        blocks: context.blocks,
    })
}

// One accounting rule for generated and externally received presentations.
fn account(
    block: &RenderBlock,
    nodes: &mut usize,
    bytes: &mut usize,
) -> Result<(), ProjectionError> {
    let count = 1usize.saturating_add(match block {
        RenderBlock::TableRow { cells, .. } => cells.len(),
        _ => 0,
    });
    let next_nodes = nodes
        .checked_add(count)
        .filter(|n| *n <= MAX_PRESENTATION_NODES)
        .ok_or(ProjectionError::TooManyNodes {
            limit: MAX_PRESENTATION_NODES,
        })?;
    let block_bytes = match block {
        RenderBlock::Text {
            markdown, plain, ..
        } => markdown.len().saturating_add(plain.len()),
        RenderBlock::Code { language, code, .. } => code
            .len()
            .saturating_add(language.as_ref().map_or(0, String::len)),
        RenderBlock::Rule { .. } => 0,
        RenderBlock::TableRow { cells, .. } => cells.iter().fold(0usize, |n, c| {
            n.saturating_add(c.markdown.len())
                .saturating_add(c.plain.len())
        }),
    };
    let next_bytes = bytes
        .checked_add(block_bytes)
        .filter(|n| *n <= MAX_PRESENTATION_BYTES)
        .ok_or(ProjectionError::OutputTooLarge {
            limit: MAX_PRESENTATION_BYTES,
        })?;
    *nodes = next_nodes;
    *bytes = next_bytes;
    Ok(())
}

struct Context {
    blocks: Vec<RenderBlock>,
    bytes: usize,
    nodes: usize,
}
impl Context {
    fn check_nodes(&self, count: usize) -> Result<(), ProjectionError> {
        if count > MAX_PRESENTATION_NODES.saturating_sub(self.nodes) {
            return Err(ProjectionError::TooManyNodes {
                limit: MAX_PRESENTATION_NODES,
            });
        }
        Ok(())
    }
    fn push(&mut self, block: RenderBlock) -> Result<(), ProjectionError> {
        account(&block, &mut self.nodes, &mut self.bytes)?;
        self.blocks.push(block);
        Ok(())
    }
    fn text(
        &mut self,
        parts: &[Inline],
        prefix: &str,
        depth: u16,
        quote_depth: u16,
        heading: Option<u8>,
    ) -> Result<(), ProjectionError> {
        let (mut markdown, mut plain) = (String::new(), String::new());
        append_text(prefix, &mut markdown, &mut plain);
        render_inline(parts, &mut markdown, &mut plain);
        self.push(RenderBlock::Text {
            markdown,
            plain,
            depth,
            quote_depth,
            heading,
        })
    }
    fn prefix(
        &mut self,
        prefix: &str,
        depth: u16,
        quote_depth: u16,
    ) -> Result<(), ProjectionError> {
        self.text(&[], prefix, depth, quote_depth, None)
    }
    fn walk(
        &mut self,
        blocks: &[Block],
        depth: u16,
        quote_depth: u16,
        prefix: Option<String>,
    ) -> Result<(), ProjectionError> {
        let mut prefix = prefix;
        for block in blocks {
            match block {
                Block::Paragraph(parts) => self.text(
                    parts,
                    &prefix.take().unwrap_or_default(),
                    depth,
                    quote_depth,
                    None,
                )?,
                Block::Heading { level, content } => self.text(
                    content,
                    &prefix.take().unwrap_or_default(),
                    depth,
                    quote_depth,
                    Some(*level),
                )?,
                Block::Html(html) => {
                    let (mut markdown, mut plain) = (String::new(), String::new());
                    append_text(
                        &prefix.take().unwrap_or_default(),
                        &mut markdown,
                        &mut plain,
                    );
                    append_text(html, &mut markdown, &mut plain);
                    self.push(RenderBlock::Text {
                        markdown,
                        plain,
                        depth,
                        quote_depth,
                        heading: None,
                    })?;
                }
                Block::CodeBlock { language, text } => {
                    if let Some(p) = prefix.take() {
                        self.prefix(&p, depth, quote_depth)?;
                    }
                    self.push(RenderBlock::Code {
                        language: language.clone(),
                        code: text.clone(),
                        depth,
                        quote_depth,
                    })?;
                }
                Block::ThematicBreak => {
                    if let Some(p) = prefix.take() {
                        self.prefix(&p, depth, quote_depth)?;
                    }
                    self.push(RenderBlock::Rule { depth, quote_depth })?;
                }
                Block::Quote(children) => {
                    if let Some(p) = prefix.take() {
                        self.prefix(&p, depth, quote_depth)?;
                    }
                    self.walk(children, depth, quote_depth.saturating_add(1), None)?;
                }
                Block::List { start, items } => {
                    if let Some(p) = prefix.take() {
                        self.prefix(&p, depth, quote_depth)?;
                    }
                    for (index, item) in items.iter().enumerate() {
                        let marker = start
                            .map(|n| format!("{}. ", n.saturating_add(index as u64)))
                            .unwrap_or_else(|| "• ".into());
                        if item.blocks.is_empty() {
                            self.prefix(&marker, depth.saturating_add(1), quote_depth)?;
                        } else {
                            self.walk(
                                &item.blocks,
                                depth.saturating_add(1),
                                quote_depth,
                                Some(marker),
                            )?;
                        }
                    }
                }
                Block::Table {
                    alignments,
                    head,
                    rows,
                } => {
                    if let Some(p) = prefix.take() {
                        self.prefix(&p, depth, quote_depth)?;
                    }
                    self.table(head, alignments, true, depth, quote_depth)?;
                    for row in rows {
                        self.table(row, alignments, false, depth, quote_depth)?;
                    }
                }
                Block::FootnoteDefinition { label, blocks } => {
                    if let Some(p) = prefix.take() {
                        self.prefix(&p, depth, quote_depth)?;
                    }
                    self.walk(blocks, depth, quote_depth, Some(format!("[^{label}]: ")))?;
                }
            }
        }
        if let Some(p) = prefix {
            self.prefix(&p, depth, quote_depth)?;
        }
        Ok(())
    }
    fn table(
        &mut self,
        cells: &[Vec<Inline>],
        alignments: &[Alignment],
        header: bool,
        depth: u16,
        quote_depth: u16,
    ) -> Result<(), ProjectionError> {
        // Reject a very wide row before allocating or formatting its cells.
        self.check_nodes(cells.len().saturating_add(1))?;
        let values = cells
            .iter()
            .enumerate()
            .map(|(index, cell)| {
                let (mut markdown, mut plain) = (String::new(), String::new());
                render_inline(cell, &mut markdown, &mut plain);
                RenderCell {
                    markdown,
                    plain,
                    alignment: alignments.get(index).copied().unwrap_or(Alignment::None),
                }
            })
            .collect();
        self.push(RenderBlock::TableRow {
            header,
            cells: values,
            depth,
            quote_depth,
        })
    }
}

/// Accept only explicit absolute HTTP(S) URLs without credentials or controls.
/// Parsing and canonicalizing a URL does not authorize or perform its activation.
pub fn safe_destination(input: &str) -> Option<String> {
    if input.trim() != input || input.chars().any(char::is_control) || input.contains('\\') {
        return None;
    }
    let (scheme, rest) = input.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    let url = Url::parse(input).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    Some(url.to_string())
}

fn append_text(text: &str, markdown: &mut String, plain: &mut String) {
    plain.push_str(text);
    // Most text consists of runs that need no escaping. Append those runs at
    // once while using UTF-8 character boundaries for the exceptional cases.
    let mut start = 0;
    for (offset, ch) in text.char_indices() {
        if !ch.is_whitespace() && !ch.is_ascii_punctuation() {
            continue;
        }
        markdown.push_str(&text[start..offset]);
        match ch {
            // An entity keeps newlines and tabs in text nodes, so indentation,
            // headings or raw HTML cannot be reparsed as block syntax.
            '\n' => markdown.push_str("&#10;"),
            '\r' => markdown.push_str("&#13;"),
            '\t' => markdown.push_str("&#9;"),
            ' ' => markdown.push_str("&#32;"),
            // Literal spaces at a line or style boundary can become Markdown
            // indentation or stop a delimiter from opening/closing. Entities
            // preserve the displayed character without changing that grammar.
            ch if ch.is_whitespace() => {
                write!(markdown, "&#{};", u32::from(ch)).expect("writing to a string cannot fail");
            }
            _ => {
                markdown.push('\\');
                markdown.push(ch);
            }
        }
        start = offset + ch.len_utf8();
    }
    markdown.push_str(&text[start..]);
}
fn plain_inlines(parts: &[Inline]) -> String {
    let (mut markup, mut text) = (String::new(), String::new());
    render_inline(parts, &mut markup, &mut text);
    text
}

fn render_inline(parts: &[Inline], markdown: &mut String, plain: &mut String) {
    // Slint 1.18 consumes its current URL at the first nested style end. A
    // standalone link with one styled run can instead put the links inside
    // that style; keep this narrow because CommonMark delimiter recognition
    // depends on surrounding text. Other styled labels retain the safe flat
    // fallback below.
    if markdown.is_empty() && parts.len() == 1 {
        if let Inline::Link {
            destination,
            content,
            ..
        } = &parts[0]
        {
            if destination.len() <= 512
                && content.iter().any(|part| {
                    matches!(
                        part,
                        Inline::Emphasis(_) | Inline::Strong(_) | Inline::Strikethrough(_)
                    )
                })
            {
                if let Some(destination) = safe_destination(destination) {
                    if render_standalone_styled_link(content, &destination, markdown, plain) {
                        return;
                    }
                }
            }
        }
    }
    render_inline_with_styles(parts, markdown, plain, &mut Vec::new());
}

// Text and code do not close a nested style while Slint is inside a link.
// Keep other inline nodes flattened, including images and nested links.
fn link_leaf_parts(parts: &[Inline]) -> bool {
    parts
        .iter()
        .all(|part| matches!(part, Inline::Text(_) | Inline::Code(_) | Inline::SoftBreak))
}

fn link_leaf_has_text(parts: &[Inline]) -> bool {
    parts.iter().any(|part| match part {
        Inline::Text(text) | Inline::Code(text) => !text.is_empty(),
        Inline::SoftBreak => true,
        _ => false,
    })
}

fn render_link_leaf(
    parts: &[Inline],
    destination: &str,
    markdown: &mut String,
    plain: &mut String,
) {
    markdown.push('[');
    render_inline_with_styles(parts, markdown, plain, &mut Vec::new());
    markdown.push_str("](<");
    markdown.push_str(destination);
    markdown.push_str(">)");
}

fn render_standalone_styled_link(
    content: &[Inline],
    destination: &str,
    markdown: &mut String,
    plain: &mut String,
) -> bool {
    // At most three copies of a short URL, so repeated fragments cannot turn
    // a small source into an unbounded projection.
    if destination.len() > 512 {
        return false;
    }
    let mut styled = None;
    for (index, part) in content.iter().enumerate() {
        let value = match part {
            Inline::Emphasis(children) => Some(("*", children)),
            Inline::Strong(children) => Some(("**", children)),
            Inline::Strikethrough(children) => Some(("~~", children)),
            Inline::Text(_) | Inline::Code(_) | Inline::SoftBreak => None,
            _ => return false,
        };
        if let Some((marker, children)) = value {
            if styled.is_some() || !link_leaf_parts(children) || !link_leaf_has_text(children) {
                return false;
            }
            styled = Some((index, marker, children));
        }
    }
    let Some((index, marker, children)) = styled else {
        return false;
    };
    let before = &content[..index];
    let after = &content[index + 1..];
    if link_leaf_has_text(before) {
        render_link_leaf(before, destination, markdown, plain);
    }
    markdown.push_str(marker);
    render_link_leaf(children, destination, markdown, plain);
    markdown.push_str(marker);
    if link_leaf_has_text(after) {
        render_link_leaf(after, destination, markdown, plain);
    }
    true
}
fn render_inline_with_styles(
    parts: &[Inline],
    markdown: &mut String,
    plain: &mut String,
    styles: &mut Vec<&'static str>,
) {
    for part in parts {
        match part {
            Inline::Text(text) | Inline::Html(text) => append_text(text, markdown, plain),
            Inline::Code(text) => {
                plain.push_str(text);
                let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
                let fence = "`".repeat(longest + 1);
                markdown.push_str(&fence);
                let pad = text.starts_with('`')
                    || text.ends_with('`')
                    || (text.starts_with(' ')
                        && text.ends_with(' ')
                        && text.chars().any(|c| c != ' '));
                if pad {
                    markdown.push(' ');
                }
                markdown.push_str(text);
                if pad {
                    markdown.push(' ');
                }
                markdown.push_str(&fence);
            }
            Inline::Emphasis(children)
            | Inline::Strong(children)
            | Inline::Strikethrough(children) => {
                let marker = match part {
                    Inline::Emphasis(_) => "*",
                    Inline::Strong(_) => "**",
                    _ => "~~",
                };
                markdown.push_str(marker);
                styles.push(marker);
                render_inline_with_styles(children, markdown, plain, styles);
                styles.pop();
                markdown.push_str(marker);
            }
            Inline::Link {
                destination,
                content,
                ..
            } => {
                if let Some(destination) = safe_destination(destination) {
                    if link_leaf_parts(content) {
                        render_link_leaf(content, &destination, markdown, plain);
                    } else {
                        let label = plain_inlines(content);
                        markdown.push('[');
                        let mut ignored = String::new();
                        append_text(&label, markdown, &mut ignored);
                        markdown.push_str("](<");
                        markdown.push_str(&destination);
                        markdown.push_str(">)");
                        plain.push_str(&label);
                    }
                } else {
                    let label = plain_inlines(content);
                    append_text(&format!("{label} ({destination})"), markdown, plain);
                }
            }
            Inline::Image {
                destination,
                title,
                alt,
            } => {
                append_text(
                    &format!("[Image: {}] ({destination})", plain_inlines(alt)),
                    markdown,
                    plain,
                );
                if !title.is_empty() {
                    append_text(&format!(" ({title})"), markdown, plain);
                }
            }
            Inline::SoftBreak => append_text(" ", markdown, plain),
            Inline::HardBreak => {
                // Slint 1.18 resets its paragraph at a hard break; keeping an
                // inline style open across it corrupts the next span offsets.
                for marker in styles.iter().rev() {
                    markdown.push_str(marker);
                }
                markdown.push_str("  \n");
                for marker in styles.iter() {
                    markdown.push_str(marker);
                }
                plain.push('\n');
            }
            Inline::TaskMarker(checked) => {
                append_text(if *checked { "☑ " } else { "☐ " }, markdown, plain)
            }
            Inline::FootnoteReference(label) => {
                append_text(&format!("[^{label}]"), markdown, plain)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn escaped_text_matches_scalar_encoding_across_unicode_boundaries() {
        let text = "A café\u{a0}世界\tline\nnext\r\u{2003}x.!_[] \\\0🙂";
        let mut expected = String::new();
        for ch in text.chars() {
            match ch {
                '\n' => expected.push_str("&#10;"),
                '\r' => expected.push_str("&#13;"),
                '\t' => expected.push_str("&#9;"),
                ch if ch.is_whitespace() => write!(expected, "&#{};", u32::from(ch)).unwrap(),
                _ => {
                    if ch.is_ascii_punctuation() {
                        expected.push('\\');
                    }
                    expected.push(ch);
                }
            }
        }
        let (mut markdown, mut plain) = (String::from("prefix"), String::from("prefix"));
        append_text(text, &mut markdown, &mut plain);
        assert_eq!(markdown, format!("prefix{expected}"));
        assert_eq!(plain, format!("prefix{text}"));
    }

    #[test]
    fn destinations_require_absolute_credential_free_http() {
        assert_eq!(
            safe_destination("https://example.com/p?q=a"),
            Some("https://example.com/p?q=a".into())
        );
        for url in [
            "javascript:alert(1)",
            "file:///tmp/a",
            "//example.com",
            "https://user:pass@example.com",
            "https://@example.com",
            " https://example.com",
            "https://example.com\n",
            "https:\\\\example.com",
            "https:///example.com",
            "https://example.com\\@evil.com",
        ] {
            assert_eq!(safe_destination(url), None, "{url:?}");
        }
    }
    #[test]
    fn html_and_unsafe_urls_remain_literal() {
        let presentation = project("<script>alert(1)</script>\n\n[bad](javascript:alert%281%29) ![alt](https://example.com/a.png)").unwrap();
        let all = presentation
            .blocks
            .iter()
            .filter_map(|block| match block {
                RenderBlock::Text { plain, .. } => Some(plain.as_str()),
                _ => None,
            })
            .collect::<String>();
        assert!(all.contains("<script>"));
        assert!(all.contains("javascript:alert"));
        assert!(all.contains("Image: alt"));
        for block in presentation.blocks {
            if let RenderBlock::Text { markdown, .. } = block {
                assert!(!markdown.contains("<script>"));
                assert!(!markdown.contains("](javascript:"));
                assert!(!markdown.contains("!["));
            }
        }
    }
    #[test]
    fn standalone_link_preserves_a_styled_run_and_code() {
        let p = project("[**strong** and `code`](https://example.com/)").unwrap();
        let RenderBlock::Text {
            markdown, plain, ..
        } = &p.blocks[0]
        else {
            panic!("expected text")
        };
        assert_eq!(plain, "strong and code");
        assert_eq!(
            markdown,
            "**[strong](<https://example.com/>)**[&#32;and&#32;`code`](<https://example.com/>)"
        );
    }
    #[test]
    fn link_title_does_not_change_visible_link_text() {
        let p = project("[reference](https://example.com/ \"description\")").unwrap();
        assert!(matches!(&p.blocks[0], RenderBlock::Text { plain, .. } if plain == "reference"));
    }
    #[test]
    fn inline_code_preserves_whitespace_and_backticks() {
        for source in ["`   `", "`` `x` ``", "` text `"] {
            let p = project(source).unwrap();
            let RenderBlock::Text {
                markdown, plain, ..
            } = &p.blocks[0]
            else {
                panic!("expected text")
            };
            let reparsed = parse(markdown).unwrap();
            let original = parse(source).unwrap();
            assert_eq!(
                reparsed, original,
                "{source:?} serialized as {markdown:?} ({plain:?})"
            );
        }
    }
    #[test]
    fn inline_styles_close_and_reopen_across_hard_breaks() {
        let p = project("prefix **first  \nlast**").unwrap();
        let RenderBlock::Text {
            markdown, plain, ..
        } = &p.blocks[0]
        else {
            panic!("expected text")
        };
        assert_eq!(plain, "prefix first\nlast");
        assert_eq!(markdown, "prefix&#32;**first**  \n**last**");
        for (source, expected_plain) in [
            ("**one&#32;  \n&#32;two**", "one \n two"),
            ("**one&#x2003;  \n&nbsp;two**", "one\u{2003}\n\u{a0}two"),
            ("&#32;&#32;&#32;&#32;hello **world**", "    hello world"),
        ] {
            let projected = project(source).unwrap();
            let RenderBlock::Text {
                markdown, plain, ..
            } = &projected.blocks[0]
            else {
                panic!("expected text")
            };
            assert_eq!(plain, expected_plain);
            assert_eq!(project(markdown).unwrap().blocks[0], projected.blocks[0]);
        }
    }
    #[test]
    fn nested_list_quote_table_and_footnote_are_projected_in_order() {
        let p = project("> - task\n>\n>   | a | b |\n>   | :- | -: |\n>   | x | y |\n\nnote[^a]\n\n[^a]: explanation").unwrap();
        assert!(p.blocks.iter().any(|b| matches!(b, RenderBlock::Text { plain, depth: 1, quote_depth: 1, .. } if plain == "• task")));
        assert!(p.blocks.iter().any(|b| matches!(b, RenderBlock::TableRow { header: true, cells, quote_depth: 1, .. } if cells.len() == 2 && cells[1].alignment == Alignment::Right)));
        assert!(p.blocks.iter().any(
            |b| matches!(b, RenderBlock::Text { plain, .. } if plain.contains("[^a]: explanation"))
        ));
    }
    #[test]
    fn limits_fail_atomically() {
        let source = "x\n\n".repeat(MAX_PRESENTATION_NODES + 1);
        assert_eq!(
            project(&source),
            Err(ProjectionError::TooManyNodes {
                limit: MAX_PRESENTATION_NODES
            })
        );
        let large = "x".repeat(crate::MAX_SOURCE_BYTES + 1);
        assert!(matches!(
            project(&large),
            Err(ProjectionError::Parse(ParseError::SourceTooLarge { .. }))
        ));
        let header = format!("|{}\n", "x|".repeat(MAX_PRESENTATION_NODES));
        let delimiter = format!("|{}\n", "---|".repeat(MAX_PRESENTATION_NODES));
        assert_eq!(
            project(&(header + &delimiter)),
            Err(ProjectionError::TooManyNodes {
                limit: MAX_PRESENTATION_NODES
            })
        );
    }
}
