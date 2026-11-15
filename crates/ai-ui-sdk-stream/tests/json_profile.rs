//! Deliberate current JSON-profile boundaries, not claims of JavaScript parity.

use ai_ui_sdk_stream::{Config, RunState, Runtime, StreamError};
use serde_json::json;

#[test]
fn json_profile_valid_surrogate_pair_decodes_to_unicode_scalar() {
    let mut runtime = Runtime::new("m", Config::default()).unwrap();
    runtime
        .push(
            runtime.ticket(),
            b"data: {\"type\":\"data-x\",\"data\":\"\\ud83d\\ude42\"}\n\n",
            |_| {},
        )
        .unwrap();
    assert_eq!(runtime.view().snapshot()["parts"][0]["data"], "🙂");
}

#[test]
fn json_profile_lone_surrogate_and_overflow_fail_without_erasing_prior_content() {
    for value in ["\"\\ud800\"", "1e400"] {
        let mut runtime = Runtime::new("m", Config::default()).unwrap();
        let ticket = runtime.ticket();
        runtime
            .push(
                ticket,
                b"data: {\"type\":\"data-x\",\"data\":1}\n\n",
                |_| {},
            )
            .unwrap();
        let before = runtime.view().snapshot();
        let wire = format!("data: {{\"type\":\"data-x\",\"data\":{value}}}\n\n");
        assert!(matches!(
            runtime.push(ticket, wire.as_bytes(), |_| {}),
            Err(StreamError::Json(_))
        ));
        assert!(matches!(
            runtime.state(),
            RunState::Failed(StreamError::Json(_))
        ));
        assert_eq!(runtime.view().snapshot(), before);
    }
}

#[test]
fn json_profile_exact_large_integer_is_not_javascript_number_parity() {
    let mut runtime = Runtime::new("m", Config::default()).unwrap();
    runtime
        .push(
            runtime.ticket(),
            b"data: {\"type\":\"data-x\",\"data\":9007199254740993}\n\n",
            |_| {},
        )
        .unwrap();
    assert_eq!(
        runtime.view().snapshot()["parts"][0]["data"].as_u64(),
        Some(9_007_199_254_740_993)
    );
}

#[test]
fn json_profile_parser_depth_is_bounded_before_reducer_publication() {
    let nested = format!("{}0{}", "[".repeat(140), "]".repeat(140));
    let wire = format!("data: {{\"type\":\"data-x\",\"data\":{nested}}}\n\n");
    let mut runtime = Runtime::new("m", Config::default()).unwrap();
    let mut publications = 0;
    assert!(matches!(
        runtime.push(runtime.ticket(), wire.as_bytes(), |_| publications += 1),
        Err(StreamError::Json(_))
    ));
    assert_eq!(publications, 0);
    assert_eq!(runtime.view().snapshot()["parts"], json!([]));
}
