use ai_ui_markdown::presentation::{project, RenderBlock};
use i_slint_common::styled_text::{parse_interpolated, Style, StyledTextParagraph};

fn paragraph(source: &str) -> StyledTextParagraph {
    let presentation = project(source).unwrap();
    let RenderBlock::Text {
        markdown, plain, ..
    } = &presentation.blocks[0]
    else {
        panic!("expected paragraph")
    };
    let (mut paragraphs, errors) = parse_interpolated::<&[_]>(markdown, &[]);
    assert!(errors.is_empty(), "generated markup was rejected");
    assert_eq!(paragraphs.len(), 1);
    let paragraph = paragraphs.remove(0);
    assert_eq!(&paragraph.text, plain);
    paragraph
}

fn assert_link(paragraph: &StyledTextParagraph, range: std::ops::Range<usize>) {
    let mut coverage = vec![false; paragraph.text.len()];
    for (span, destination) in &paragraph.links {
        assert_eq!(destination, "https://example.com/");
        for byte in span.clone() {
            assert!(range.contains(&byte));
            coverage[byte] = true;
        }
    }
    for (byte, covered) in coverage.into_iter().enumerate() {
        assert_eq!(
            covered,
            range.contains(&byte),
            "link coverage at byte {byte}"
        );
    }
}

fn has_style(paragraph: &StyledTextParagraph, style: Style, range: std::ops::Range<usize>) {
    assert!(
        paragraph
            .formatting
            .iter()
            .any(|span| span.style == style && span.range == range),
        "missing {style:?} span"
    );
}

#[test]
fn standalone_styled_segments_remain_fully_clickable() {
    let p = paragraph("[**strong** and `code`](https://example.com/)");
    assert_eq!(p.text, "strong and code");
    assert_link(&p, 0..15);
    has_style(&p, Style::Strong, 0..6);
    has_style(&p, Style::Code, 11..15);

    let p = paragraph("[lead *café* tail](https://example.com/)");
    assert_eq!(p.text, "lead café tail");
    assert_link(&p, 0..15);
    has_style(&p, Style::Emphasis, 5..10);

    let p = paragraph("[~~struck~~](https://example.com/)");
    assert_eq!(p.text, "struck");
    assert_link(&p, 0..6);
    has_style(&p, Style::Strikethrough, 0..6);
}

#[test]
fn inline_code_survives_embedded_link_and_unsafe_urls_stay_inert() {
    let p = paragraph("before [`code`](https://example.com/) after");
    assert_eq!(p.text, "before code after");
    assert_link(&p, 7..11);
    has_style(&p, Style::Code, 7..11);

    let p = paragraph("before [**strong**](https://example.com/)after");
    assert_eq!(p.text, "before strongafter");
    assert_link(&p, 7..13);
    assert!(!p.formatting.iter().any(|span| span.style == Style::Strong));

    let p = paragraph("[**no**](javascript:alert)");
    assert!(p.links.is_empty());
    assert!(p.text.contains("javascript:alert"));
}

#[test]
fn adversarial_standalone_link_labels_preserve_text_and_coverage() {
    for left in ["", "lead ", "a", "世界 ", "!", "\\[", "☃ "] {
        for right in ["", " tail", "z", " 世界", "!", "\\]", " ☃"] {
            for marker in ["**", "*", "~~"] {
                let source = format!("[{left}{marker}styled{marker}{right}](https://example.com/)");
                let p = paragraph(&source);
                assert_link(&p, 0..p.text.len());
                let source_has_style = pulldown_cmark::Parser::new_ext(
                    &source,
                    pulldown_cmark::Options::ENABLE_STRIKETHROUGH,
                )
                .any(|event| match (marker, event) {
                    ("**", pulldown_cmark::Event::Start(pulldown_cmark::Tag::Strong))
                    | ("*", pulldown_cmark::Event::Start(pulldown_cmark::Tag::Emphasis))
                    | ("~~", pulldown_cmark::Event::Start(pulldown_cmark::Tag::Strikethrough)) => {
                        true
                    }
                    _ => false,
                });
                if source_has_style {
                    let style = match marker {
                        "**" => Style::Strong,
                        "*" => Style::Emphasis,
                        _ => Style::Strikethrough,
                    };
                    let start = p.text.find("styled").unwrap();
                    has_style(&p, style, start..start + "styled".len());
                }
            }
        }
    }
}

#[test]
fn nested_and_broken_markup_keep_fallback_text_safe() {
    for source in [
        "[***nested*** and `x`](https://example.com/)",
        "[**one** *two*](https://example.com/)",
        "before [**strong**](https://example.com/) after",
        "[**a**](https://example.com/)suffix",
        "[a\\]b `x`](https://example.com/)",
        "[one\ntwo](https://example.com/)",
    ] {
        let p = paragraph(source);
        let start = p.links.iter().map(|(range, _)| range.start).min().unwrap();
        let end = p.links.iter().map(|(range, _)| range.end).max().unwrap();
        assert_link(&p, start..end);
    }
    for source in [
        "[**no**](javascript:alert)",
        "[x](https://example.com@evil.test/)",
        "[x](data:text/plain,hi)",
    ] {
        let p = paragraph(source);
        assert!(p.links.is_empty());
    }
}
