//! AI SDK data streams (`CODE:JSON` lines) through the decoder and
//! accumulator, after upstream's `DataStream` tests.

use ai_ui_core::*;
use ai_ui_stream::*;
use serde_json::{json, Value};

fn fold(body: &str) -> Result<MessageAccumulator, String> {
    let mut decoder = DataStreamDecoder::default();
    let mut accumulator = MessageAccumulator::default();
    // Odd slice sizes split lines and UTF-8 sequences across pushes.
    for slice in body.as_bytes().chunks(5) {
        decoder
            .push(slice)?
            .into_iter()
            .for_each(|c| accumulator.push(c));
    }
    decoder
        .finish()?
        .into_iter()
        .for_each(|c| accumulator.push(c));
    Ok(accumulator)
}

#[test]
fn an_ai_sdk_v4_response_folds_into_the_message() {
    let body = r#"f:{"messageId":"m1"}
g:"Let me think"
0:"Hello"
0:" wörld"
b:{"toolCallId":"c1","toolName":"weather"}
c:{"toolCallId":"c1","argsTextDelta":"{\"city\":"}
c:{"toolCallId":"c1","argsTextDelta":"\"NYC\"}"}
9:{"toolCallId":"c1","toolName":"weather","args":{"city":"NYC"}}
a:{"toolCallId":"c1","result":{"temp":70}}
h:{"sourceType":"url","id":"s1","url":"https://example.com","title":"Example"}
2:[{"x":1}]
8:[{"note":"a"}]
e:{"finishReason":"tool-calls","usage":{"inputTokens":1,"outputTokens":2},"isContinued":false}
f:{"messageId":"m2"}
0:"Done"
e:{"finishReason":"stop","usage":{"inputTokens":3,"outputTokens":4},"isContinued":false}
d:{"finishReason":"stop","usage":{"inputTokens":4,"outputTokens":6}}
"#;
    let accumulator = fold(body).unwrap();
    let parts = accumulator.parts();
    assert_eq!(parts.len(), 5);
    assert!(matches!(&parts[0], AssistantMessagePart::Reasoning(r) if r.text == "Let me think"));
    assert!(matches!(&parts[1], AssistantMessagePart::Text(t) if t.text == "Hello wörld"));
    let call = parts[2].as_tool_call().unwrap();
    assert_eq!(
        call.args_text, r#"{"city":"NYC"}"#,
        "the final 9: does not repeat streamed arguments"
    );
    assert_eq!(Value::Object(call.args.clone()), json!({ "city": "NYC" }));
    assert_eq!(call.result, Some(json!({ "temp": 70 })));
    assert!(
        matches!(&parts[3], AssistantMessagePart::Source(SourcePart::Url { id, .. }) if id == "s1")
    );
    assert!(
        matches!(&parts[4], AssistantMessagePart::Text(t) if t.text == "Done" && t.status == Some(PartStatus::Complete))
    );
    assert_eq!(
        accumulator.status(),
        &MessageStatus::Complete {
            reason: CompleteReason::Stop
        }
    );
    let metadata = accumulator.result().metadata.unwrap();
    assert_eq!(metadata.data, Some(vec![json!({ "x": 1 })]));
    assert_eq!(metadata.annotations, Some(vec![json!({ "note": "a" })]));
    let steps = metadata.steps.unwrap();
    assert_eq!(
        steps
            .iter()
            .map(|s| s.message_id.as_deref())
            .collect::<Vec<_>>(),
        [Some("m1"), Some("m2")]
    );
    assert_eq!(
        steps[1].usage,
        Some(Usage {
            input_tokens: 3,
            output_tokens: 4
        })
    );
}

#[test]
fn a_tool_call_without_streamed_arguments_takes_them_from_the_call() {
    let accumulator = fold("9:{\"toolCallId\":\"c\",\"toolName\":\"t\",\"args\":{\"a\":1}}\nd:{\"finishReason\":\"tool-calls\"}\n").unwrap();
    let call = accumulator.parts()[0].as_tool_call().unwrap();
    assert_eq!(call.args_text, r#"{"a":1}"#);
    assert_eq!(
        accumulator.status(),
        &MessageStatus::RequiresAction {
            reason: RequiresActionReason::ToolCalls
        }
    );
}

#[test]
fn assistant_ui_extensions_group_parts_and_update_state() {
    let body = r#"aui-text-delta:{"textDelta":"In group A","parentId":"A"}
aui-text-delta:{"textDelta":" still A","parentId":"A"}
aui-text-delta:{"textDelta":"Group B","parentId":"B"}
aui-reasoning-part-start:{"unstable_summary":"Plan"}
aui-reasoning-delta:{"reasoningDelta":"step 1","parentId":"B"}
aui-data:{"name":"chart","data":{"points":[1,2]}}
aui-state:[{"type":"set","path":["mode"],"value":"done"}]
3:{"error":"Quota low","code":"quota","severity":"warning"}
"#;
    let accumulator = fold(body).unwrap();
    let parts = accumulator.parts();
    assert!(
        matches!(&parts[0], AssistantMessagePart::Text(t) if t.text == "In group A still A" && t.parent_id.as_deref() == Some("A"))
    );
    assert!(
        matches!(&parts[1], AssistantMessagePart::Text(t) if t.text == "Group B" && t.parent_id.as_deref() == Some("B"))
    );
    assert!(
        matches!(&parts[2], AssistantMessagePart::Reasoning(r) if r.summary.as_deref() == Some("Plan"))
    );
    assert!(
        matches!(&parts[3], AssistantMessagePart::Reasoning(r) if r.text == "step 1" && r.parent_id.as_deref() == Some("B"))
    );
    assert!(matches!(&parts[4], AssistantMessagePart::Data(d) if d.name == "chart"));
    assert_eq!(
        accumulator.result().metadata.unwrap().state,
        Some(json!({ "mode": "done" }))
    );
    assert!(
        matches!(accumulator.status(), MessageStatus::Incomplete { reason: IncompleteReason::Error, error: Some(e) } if e["code"] == "quota" && e["severity"] == "warning")
    );
}

#[test]
fn strict_decoding_rejects_unknown_tool_calls_bad_json_and_unknown_codes() {
    assert!(fold("a:{\"toolCallId\":\"missing\",\"result\":1}\n")
        .unwrap_err()
        .contains("unknown id"));
    assert!(fold("0:not-json\n").unwrap_err().contains("Invalid JSON"));
    assert!(fold("z:{}\n")
        .unwrap_err()
        .contains("unsupported chunk type"));
    assert!(fold("0:{\"not\":\"a string\"}\n")
        .unwrap_err()
        .contains("Invalid value"));
    assert!(fold("b:{\"toolCallId\":\"c\",\"toolName\":\"t\"}\nb:{\"toolCallId\":\"c\",\"toolName\":\"t\"}\n").unwrap_err().contains("duplicate"));
}

#[test]
fn a_final_line_without_newline_and_open_text_are_completed_at_the_end() {
    let accumulator = fold("0:\"tail\"").unwrap();
    assert_eq!(
        accumulator.parts(),
        [AssistantMessagePart::Text(TextPart {
            text: "tail".into(),
            status: Some(PartStatus::Complete),
            ..Default::default()
        })]
    );
}
