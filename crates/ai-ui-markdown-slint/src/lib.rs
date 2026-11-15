//! Conversion from the portable Markdown presentation to host-generated Slint
//! records. This crate performs no link, clipboard, image or network actions.
//!
//! Slint generates distinct record types per application. A small [`Factory`]
//! lets each host map the common conversion into its own generated types. The
//! caller must run this conversion on its Slint thread; parsing can occur on a
//! worker, but constructing `StyledText` is UI-side work.
#![forbid(unsafe_code)]

pub mod cache;
pub mod streaming;

use ai_ui_markdown::{
    presentation::{Presentation, ProjectionError, RenderBlock},
    Alignment as PortableAlignment,
};
use slint::StyledText;

/// The corresponding variant of the shared Slint Markdown component.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockKind {
    Paragraph,
    Heading,
    Code,
    Rule,
    TableHeader,
    TableRow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Alignment {
    Left,
    Center,
    Right,
}

/// A block ready for an application-specific generated Slint constructor.
/// Content may contain private conversation data; it is deliberately not Debug.
pub struct Block<C> {
    pub kind: BlockKind,
    pub depth: i32,
    pub quote_depth: i32,
    pub heading_level: i32,
    pub text: StyledText,
    pub code: String,
    pub language: String,
    pub cells: Vec<C>,
}

/// Maps the shared Slint-facing records into an application's generated types.
/// Hosts own actions and must not install link or copy handlers implicitly.
pub trait Factory {
    type Block;
    type Cell;

    fn cell(&self, text: StyledText, alignment: Alignment) -> Self::Cell;
    fn block(&self, block: Block<Self::Cell>) -> Self::Block;
}

/// Rendered blocks and an explicit warning when inline formatting fell back.
pub struct Rendered<B> {
    pub blocks: Vec<B>,
    pub warning: Option<&'static str>,
}

fn blank<C>(kind: BlockKind, depth: u16, quote_depth: u16) -> Block<C> {
    Block {
        kind,
        depth: i32::from(depth),
        quote_depth: i32::from(quote_depth),
        heading_level: 0,
        text: StyledText::default(),
        code: String::new(),
        language: String::new(),
        cells: Vec::new(),
    }
}

fn styled(markdown: &str, plain: &str, degraded: &mut bool) -> StyledText {
    match StyledText::from_markdown(markdown) {
        Ok(text) => text,
        Err(_) => {
            *degraded = true;
            StyledText::from_plain_text(plain)
        }
    }
}

/// Converts a complete, size-bounded presentation. Fails before creating
/// generated UI records if its total nodes or serialized strings exceed the
/// presentation limits. The presentation must come from the trusted portable
/// projector; limits alone do not sanitize forged input. The caller must still
/// guard source revision and scope.
pub fn convert<F: Factory>(
    presentation: Presentation,
    factory: &F,
) -> Result<Rendered<F::Block>, ProjectionError> {
    presentation.validate_limits()?;
    let mut degraded = false;
    let mut blocks = Vec::with_capacity(presentation.blocks.len());
    for item in presentation.blocks {
        let block = match item {
            RenderBlock::Text {
                markdown,
                plain,
                depth,
                quote_depth,
                heading,
            } => {
                let mut block = blank(
                    if heading.is_some() {
                        BlockKind::Heading
                    } else {
                        BlockKind::Paragraph
                    },
                    depth,
                    quote_depth,
                );
                block.heading_level = i32::from(heading.unwrap_or(0));
                block.text = styled(&markdown, &plain, &mut degraded);
                block
            }
            RenderBlock::Code {
                language,
                code,
                depth,
                quote_depth,
            } => {
                let mut block = blank(BlockKind::Code, depth, quote_depth);
                block.language = language.unwrap_or_default();
                block.code = code;
                block
            }
            RenderBlock::Rule { depth, quote_depth } => blank(BlockKind::Rule, depth, quote_depth),
            RenderBlock::TableRow {
                header,
                cells,
                depth,
                quote_depth,
            } => {
                let mut block = blank(
                    if header {
                        BlockKind::TableHeader
                    } else {
                        BlockKind::TableRow
                    },
                    depth,
                    quote_depth,
                );
                block.cells = cells
                    .into_iter()
                    .map(|cell| {
                        let alignment = match cell.alignment {
                            PortableAlignment::None | PortableAlignment::Left => Alignment::Left,
                            PortableAlignment::Center => Alignment::Center,
                            PortableAlignment::Right => Alignment::Right,
                        };
                        factory.cell(
                            styled(&cell.markdown, &cell.plain, &mut degraded),
                            alignment,
                        )
                    })
                    .collect();
                block
            }
        };
        blocks.push(factory.block(block));
    }
    Ok(Rendered {
        blocks,
        warning: degraded.then_some("Some inline formatting could not be shown."),
    })
}

/// Preserve the exact original source as inert plain text when projection is
/// pending, fails, or cannot safely represent a truncated source.
pub fn literal<F: Factory>(source: &str, warning: &'static str, factory: &F) -> Rendered<F::Block> {
    let mut block = blank(BlockKind::Paragraph, 0, 0);
    block.text = StyledText::from_plain_text(source);
    Rendered {
        blocks: vec![factory.block(block)],
        warning: Some(warning),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ai_ui_markdown::presentation::{project, RenderCell};

    struct Raw;
    impl Factory for Raw {
        type Block = Block<(StyledText, Alignment)>;
        type Cell = (StyledText, Alignment);
        fn cell(&self, text: StyledText, alignment: Alignment) -> Self::Cell {
            (text, alignment)
        }
        fn block(&self, block: Block<Self::Cell>) -> Self::Block {
            block
        }
    }

    #[test]
    fn maps_headings_code_and_table_alignment_without_actions() {
        let presentation = project(
            "# Title\n\n```rust\nlet x = 1;\n```\n\n| Left | Right |\n| :--- | ---: |\n| a | b |\n",
        )
        .unwrap();
        let rendered = convert(presentation, &Raw).unwrap();
        assert!(rendered.warning.is_none());
        assert_eq!(rendered.blocks[0].kind, BlockKind::Heading);
        assert_eq!(rendered.blocks[0].heading_level, 1);
        assert_eq!(
            rendered.blocks[0].text,
            StyledText::from_plain_text("Title")
        );
        assert_eq!(rendered.blocks[1].kind, BlockKind::Code);
        assert_eq!(rendered.blocks[1].language, "rust");
        assert_eq!(rendered.blocks[1].code, "let x = 1;\n");
        assert_eq!(rendered.blocks[2].kind, BlockKind::TableHeader);
        assert_eq!(rendered.blocks[2].cells[0].1, Alignment::Left);
        assert_eq!(rendered.blocks[2].cells[1].1, Alignment::Right);
    }

    #[test]
    fn literal_fallback_preserves_source_and_warning() {
        let source = "<script>not executable</script>\n```partial";
        let rendered = literal(source, "Formatting unavailable.", &Raw);
        assert_eq!(rendered.warning, Some("Formatting unavailable."));
        assert_eq!(rendered.blocks.len(), 1);
        assert_eq!(rendered.blocks[0].text, StyledText::from_plain_text(source));
    }

    #[test]
    fn rejects_overlarge_presentations_before_calling_factory() {
        let cells = vec![
            RenderCell {
                markdown: String::new(),
                plain: String::new(),
                alignment: PortableAlignment::Left
            };
            ai_ui_markdown::presentation::MAX_PRESENTATION_NODES
        ];
        let presentation = Presentation {
            blocks: vec![RenderBlock::TableRow {
                header: false,
                cells,
                depth: 0,
                quote_depth: 0,
            }],
        };
        assert!(matches!(
            convert(presentation, &Raw),
            Err(ProjectionError::TooManyNodes { .. })
        ));
    }
}
