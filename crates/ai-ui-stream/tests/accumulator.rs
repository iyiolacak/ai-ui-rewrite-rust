//! The accumulator against upstream's Python encoder output and focused cases
//! after upstream's `assistant-message-accumulator` tests.

use ai_ui_core::*;
use ai_ui_stream::*;
use serde_json::json;

fn fold(chunks: Vec<serde_json::Value>) -> MessageAccumulator {
    let mut accumulator = MessageAccumulator::default();
    for chunk in chunks {
        accumulator.push(serde_json::from_value(chunk).expect("valid chunk"));
    }
    accumulator
}

#[test]
fn upstream_python_encoder_stream_folds_into_the_expected_message() {
    let bytes = include_bytes!("fixtures/python-encoder.sse");
    let mut decoder = SseDecoder::default();
    let mut accumulator = MessageAccumulator::default();
    // Feed in small slices to exercise frame reassembly.
    let mut done = false;
    for slice in bytes.chunks(7) {
        let decoded = decoder.push(slice);
        assert_eq!(decoded.error, None);
        decoded.chunks.into_iter().for_each(|c| accumulator.push(c));
        done |= decoded.done;
    }
    assert!(done);
    decoder.finish().unwrap();
    assert_eq!(accumulator.warnings(), [] as [String; 0]);

    let parts = accumulator.parts();
    assert_eq!(parts.len(), 6);
    let AssistantMessagePart::Reasoning(reasoning) = &parts[0] else {
        panic!("reasoning first")
    };
    assert_eq!(reasoning.text, "Let me check the weather.");
    assert_eq!(reasoning.status, Some(PartStatus::Complete));
    assert_eq!(
        parts[1],
        AssistantMessagePart::Text(TextPart {
            text: "Checking now.".into(),
            status: Some(PartStatus::Complete),
            ..Default::default()
        })
    );

    let call = parts[2].as_tool_call().unwrap();
    assert_eq!(
        (call.tool_call_id.as_str(), call.tool_name.as_str()),
        ("call_1", "get_weather")
    );
    assert_eq!(call.args_text, "{\"city\": \"NYC\"}");
    assert_eq!(
        serde_json::Value::Object(call.args.clone()),
        json!({ "city": "NYC" })
    );
    assert_eq!(
        call.result,
        Some(json!({ "temp": 70 })),
        "the final result replaces the preliminary one"
    );
    assert_eq!(call.is_preliminary, None);
    assert_eq!(
        call.extra["messages"][0]["content"][0]["text"],
        json!("Sunny")
    );

    assert!(
        matches!(&parts[3], AssistantMessagePart::Source(SourcePart::Url { url, .. }) if url == "https://example.com")
    );
    assert_eq!(
        parts[4],
        AssistantMessagePart::Text(TextPart {
            text: "It is sunny.".into(),
            status: Some(PartStatus::Complete),
            ..Default::default()
        })
    );
    assert!(matches!(&parts[5], AssistantMessagePart::File(f) if f.mime_type == "image/png"));

    // A later error chunk overrides the status (upstream behaviour).
    assert_eq!(
        accumulator.status(),
        &MessageStatus::Incomplete {
            reason: IncompleteReason::Error,
            error: Some(
                json!({ "code": "weather_delay", "message": "Forecast delayed", "severity": "warning" })
            ),
        }
    );
    let result = accumulator.result();
    let metadata = result.metadata.unwrap();
    assert_eq!(metadata.state, Some(json!({ "status": "running" })));
    assert_eq!(metadata.data, Some(vec![json!({ "progress": 1 })]));
    assert_eq!(
        metadata.annotations,
        Some(vec![json!({ "type": "citation", "id": "a1" })])
    );
    let steps = metadata.steps.unwrap();
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].message_id.as_deref(), Some("msg_1"));
    assert_eq!(
        steps[0].usage,
        Some(Usage {
            input_tokens: 12,
            output_tokens: 34
        })
    );
}

#[test]
fn tool_arguments_parse_incrementally_while_streaming() {
    let mut accumulator = fold(vec![
        json!({ "type": "part-start", "part": { "type": "tool-call", "toolCallId": "c", "toolName": "t" }, "path": [] }),
        json!({ "type": "text-delta", "textDelta": "{\"query\": \"rus", "path": [0] }),
    ]);
    let call = accumulator.parts()[0].as_tool_call().unwrap().clone();
    assert_eq!(
        serde_json::Value::Object(call.args),
        json!({ "query": "rus" }),
        "partial strings are exposed"
    );
    accumulator.push(
        serde_json::from_value(
            json!({ "type": "text-delta", "textDelta": "t\", \"n\": 3}", "path": [0] }),
        )
        .unwrap(),
    );
    let call = accumulator.parts()[0].as_tool_call().unwrap();
    assert_eq!(
        serde_json::Value::Object(call.args.clone()),
        json!({ "query": "rust", "n": 3 })
    );
}

#[test]
fn finish_reasons_map_to_message_status() {
    for (reason, status) in [
        (
            "tool-calls",
            MessageStatus::RequiresAction {
                reason: RequiresActionReason::ToolCalls,
            },
        ),
        (
            "stop",
            MessageStatus::Complete {
                reason: CompleteReason::Stop,
            },
        ),
        (
            "unknown",
            MessageStatus::Complete {
                reason: CompleteReason::Unknown,
            },
        ),
        (
            "length",
            MessageStatus::Incomplete {
                reason: IncompleteReason::Length,
                error: None,
            },
        ),
        (
            "content-filter",
            MessageStatus::Incomplete {
                reason: IncompleteReason::ContentFilter,
                error: None,
            },
        ),
    ] {
        let accumulator = fold(vec![
            json!({ "type": "message-finish", "finishReason": reason, "usage": { "inputTokens": 1, "outputTokens": 1 } }),
        ]);
        assert_eq!(accumulator.status(), &status, "{reason}");
    }
}

#[test]
fn message_finish_does_not_clear_an_error() {
    let accumulator = fold(vec![
        json!({ "type": "error", "error": "boom" }),
        json!({ "type": "message-finish", "finishReason": "stop", "usage": { "inputTokens": 0, "outputTokens": 0 } }),
    ]);
    assert!(matches!(
        accumulator.status(),
        MessageStatus::Incomplete {
            reason: IncompleteReason::Error,
            ..
        }
    ));
}

#[test]
fn chunks_for_missing_or_wrong_parts_are_dropped_with_one_warning_each() {
    let accumulator = fold(vec![
        json!({ "type": "text-delta", "textDelta": "x", "path": [3] }),
        json!({ "type": "text-delta", "textDelta": "y", "path": [4] }),
        json!({ "type": "part-start", "part": { "type": "text" }, "path": [] }),
        json!({ "type": "result", "result": 1, "isError": false, "path": [0] }),
    ]);
    assert_eq!(
        accumulator.parts(),
        [AssistantMessagePart::Text(TextPart {
            status: Some(PartStatus::Running),
            ..Default::default()
        })]
    );
    assert_eq!(accumulator.warnings().len(), 2, "deduplicated by cause");
}

#[test]
fn an_unknown_part_type_keeps_later_indices_aligned() {
    let accumulator = fold(vec![
        json!({ "type": "part-start", "part": { "type": "hologram" }, "path": [] }),
        json!({ "type": "part-start", "part": { "type": "text" }, "path": [] }),
        json!({ "type": "text-delta", "textDelta": "ok", "path": [1] }),
    ]);
    assert_eq!(accumulator.parts().len(), 2);
    assert!(matches!(&accumulator.parts()[1], AssistantMessagePart::Text(t) if t.text == "ok"));
}

#[test]
fn a_preliminary_result_never_replaces_a_final_one() {
    let accumulator = fold(vec![
        json!({ "type": "part-start", "part": { "type": "tool-call", "toolCallId": "c", "toolName": "t" }, "path": [] }),
        json!({ "type": "result", "result": "final", "isError": false, "path": [0] }),
        json!({ "type": "result", "result": "late", "isError": false, "isPreliminary": true, "path": [0] }),
    ]);
    assert_eq!(
        accumulator.parts()[0].as_tool_call().unwrap().result,
        Some(json!("final"))
    );
}

#[test]
fn state_operations_start_from_the_initial_state() {
    let mut accumulator = MessageAccumulator::default().with_state(json!({ "count": 1 }));
    accumulator.push(serde_json::from_value(json!({ "type": "update-state", "operations": [ { "type": "set", "path": ["done"], "value": true } ] })).unwrap());
    assert_eq!(
        accumulator.result().metadata.unwrap().state,
        Some(json!({ "count": 1, "done": true }))
    );
}

#[test]
fn malformed_and_unknown_chunks_are_dropped_and_the_stream_continues() {
    // As upstream assistant-stream (protocol vector AT05).
    let mut decoder = SseDecoder::default();
    let decoded = decoder.push(b"data: {\"type\": \"part-start\", \"part\": {\"type\": \"text\"}, \"path\": []}\n\ndata: {not json}\n\ndata: {\"type\": \"future\", \"path\": []}\n\ndata: {\"type\": \"text-delta\", \"textDelta\": \"ok\", \"path\": [0]}\n\n");
    assert_eq!(decoded.chunks.len(), 2);
    assert_eq!(decoded.dropped.len(), 2);
    assert!(decoded.error.is_none());
}

#[test]
fn a_framing_error_is_reported_with_the_chunks_before_it() {
    let mut decoder = SseDecoder::default();
    let decoded = decoder.push(b"data: {\"type\": \"part-start\", \"part\": {\"type\": \"text\"}, \"path\": []}\n\nevent: surprise\ndata: {}\n\n");
    assert_eq!(decoded.chunks.len(), 1);
    assert!(matches!(decoded.error, Some(DecodeError::Transport(_))));
}
