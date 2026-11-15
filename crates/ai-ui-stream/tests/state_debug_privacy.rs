use ai_ui_stream::state::Operation;
use serde_json::json;

#[test]
fn operations_do_not_debug_print_paths_or_values() {
    let operations = [
        Operation::Set {
            path: vec!["secret-path".into()],
            value: json!({"nested": "secret-value"}),
        },
        Operation::AppendText {
            path: vec!["secret-path".into()],
            value: "secret-value".into(),
        },
    ];
    let text = format!("{operations:?}");
    assert!(!text.contains("secret-path"));
    assert!(!text.contains("secret-value"));
    assert!(text.contains("Set"));
    assert!(text.contains("AppendText"));
}
