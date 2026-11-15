use ai_ui_stream::partial_json::{Limits, Parser};

#[test]
fn parser_and_snapshot_do_not_debug_print_partial_arguments_or_keys() {
    let mut parser = Parser::new(Limits::default()).unwrap();
    parser.append(r#"{"secret-key":"secret-value"#).unwrap();
    for text in [format!("{parser:?}"), format!("{:?}", parser.snapshot())] {
        assert!(!text.contains("secret-key"));
        assert!(!text.contains("secret-value"));
    }
}
