use ai_ui_markdown::{parse, Alignment, Block, Inline, ParseError, MAX_SOURCE_BYTES};

#[test]
fn parses_mixed_content_and_inert_untrusted_values() {
    let doc = parse("# Hello **world**\n\n[go](javascript:alert%281%29 \"title\") ![alt *em*](https://example.com/a.png) <b>raw</b> ~~gone~~\n\n```rust\nlet x = 1;\n```\n\n---\n").unwrap();
    assert_eq!(
        doc.blocks[0],
        Block::Heading {
            level: 1,
            content: vec![
                Inline::Text("Hello ".into()),
                Inline::Strong(vec![Inline::Text("world".into())])
            ]
        }
    );
    let Block::Paragraph(parts) = &doc.blocks[1] else {
        panic!("expected paragraph")
    };
    assert!(parts.iter().any(|p| matches!(p, Inline::Link { destination, title, .. } if destination == "javascript:alert%281%29" && title == "title")));
    assert!(parts.iter().any(|p| matches!(p, Inline::Image { destination, alt, .. } if destination == "https://example.com/a.png" && alt.len() == 2)));
    assert!(parts
        .iter()
        .any(|p| matches!(p, Inline::Html(s) if s == "<b>")));
    assert!(parts.iter().any(|p| matches!(p, Inline::Strikethrough(_))));
    assert_eq!(
        doc.blocks[2],
        Block::CodeBlock {
            language: Some("rust".into()),
            text: "let x = 1;\n".into()
        }
    );
    assert_eq!(doc.blocks[3], Block::ThematicBreak);
}

#[test]
fn nested_list_quote_and_task_markers_keep_structure() {
    let doc = parse("> 3. first\n>    - [x] done\n>    - [ ] later\n> 4. second\n").unwrap();
    let Block::Quote(quote) = &doc.blocks[0] else {
        panic!("quote")
    };
    let Block::List { start, items } = &quote[0] else {
        panic!("ordered list")
    };
    assert_eq!(*start, Some(3));
    assert_eq!(items.len(), 2);
    let Block::List { items: nested, .. } = &items[0].blocks[1] else {
        panic!("nested list")
    };
    assert!(
        matches!(&nested[0].blocks[0], Block::Paragraph(items) if matches!(items[0], Inline::TaskMarker(true)))
    );
    assert!(
        matches!(&nested[1].blocks[0], Block::Paragraph(items) if matches!(items[0], Inline::TaskMarker(false)))
    );
}

#[test]
fn table_keeps_alignment_and_cell_inlines() {
    let doc = parse("| A | B |\n| :--- | ---: |\n| **one** | `two` |\n").unwrap();
    let Block::Table {
        alignments,
        head,
        rows,
    } = &doc.blocks[0]
    else {
        panic!("table")
    };
    assert_eq!(alignments, &[Alignment::Left, Alignment::Right]);
    assert_eq!(head.len(), 2);
    assert!(matches!(&rows[0][0][0], Inline::Strong(_)));
    assert_eq!(rows[0][1][0], Inline::Code("two".into()));
}

#[test]
fn html_is_untrusted_text_and_incomplete_fence_reparses() {
    let html = parse("<script>alert(1)</script>\n").unwrap();
    assert!(matches!(&html.blocks[0], Block::Html(s) if s == "<script>alert(1)</script>\n"));
    let partial = parse("```rust\nlet a =").unwrap();
    assert_eq!(
        partial.blocks[0],
        Block::CodeBlock {
            language: Some("rust".into()),
            text: "let a =".into()
        }
    );
    let complete = parse("```rust\nlet a = 3;\n```\n\nnext").unwrap();
    assert!(matches!(&complete.blocks[0], Block::CodeBlock { text, .. } if text == "let a = 3;\n"));
    assert_eq!(complete.blocks.len(), 2);
}

#[test]
fn input_limits_fail_without_partial_document() {
    let source = "a".repeat(MAX_SOURCE_BYTES + 1);
    assert!(matches!(
        parse(&source),
        Err(ParseError::SourceTooLarge { .. })
    ));
}

#[test]
fn footnotes_and_disabled_math_remain_readable() {
    let doc = parse("Footnote [^id] and math $x$.\n\n[^id]: explanation\n").unwrap();
    assert!(
        matches!(&doc.blocks[0], Block::Paragraph(parts) if parts.iter().any(|part| matches!(part, Inline::FootnoteReference(label) if label == "id")))
    );
    assert!(matches!(&doc.blocks[1], Block::FootnoteDefinition { label, .. } if label == "id"));
    assert!(
        matches!(&doc.blocks[0], Block::Paragraph(parts) if parts.iter().any(|part| matches!(part, Inline::Text(text) if text.contains("$x$"))))
    );
}
