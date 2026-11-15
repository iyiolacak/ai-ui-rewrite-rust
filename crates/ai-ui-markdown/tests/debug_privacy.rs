use ai_ui_markdown::{parse, presentation::project, Block, Inline, ListItem};

#[test]
fn markdown_and_presentation_debug_hide_nested_text_code_and_urls() {
    let source = "- secret-text [link](https://secret.example/path)\n\n```secret-language\nsecret-code\n```\n";
    let document = parse(source).unwrap();
    let presentation = project(source).unwrap();
    let item = ListItem {
        blocks: vec![Block::Paragraph(vec![Inline::Text("secret-text".into())])],
    };
    let mut formatted = vec![
        format!("{document:?}"),
        format!("{presentation:?}"),
        format!("{item:?}"),
    ];
    formatted.extend(document.blocks.iter().map(|block| format!("{block:?}")));
    formatted.extend(presentation.blocks.iter().map(|block| format!("{block:?}")));
    formatted.push(format!("{:?}", Inline::Text("secret-text".into())));
    for text in formatted {
        for secret in [
            "secret-text",
            "secret.example",
            "secret-code",
            "secret-language",
        ] {
            assert!(!text.contains(secret), "leaked {secret}");
        }
    }
    let table = project("| secret-cell |\n| --- |\n| secret-value |").unwrap();
    if let ai_ui_markdown::presentation::RenderBlock::TableRow { cells, .. } = &table.blocks[0] {
        assert!(!format!("{:?}", cells[0]).contains("secret-cell"));
    } else {
        panic!("expected table row");
    }
}
