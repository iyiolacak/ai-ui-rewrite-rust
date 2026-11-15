//! AI SDK UI message streams through the decoder and accumulator, after
//! upstream's `UIMessageStream` tests.

use ai_ui_core::*;
use ai_ui_stream::*;
use serde_json::{json, Value};

fn sse(chunks: &[Value]) -> Vec<u8> {
    let mut body = String::new();
    for chunk in chunks {
        body.push_str(&format!("data: {chunk}\n\n"));
    }
    body.push_str("data: [DONE]\n\n");
    body.into_bytes()
}

/// Decode `body` one byte at a time and fold the chunks.
fn fold(body: &[u8]) -> (MessageAccumulator, UiMessageStreamDecoder) {
    let mut decoder = UiMessageStreamDecoder::default();
    let mut accumulator = MessageAccumulator::default();
    for byte in body {
        decoder
            .push(std::slice::from_ref(byte))
            .unwrap()
            .into_iter()
            .for_each(|c| accumulator.push(c));
    }
    decoder
        .finish()
        .unwrap()
        .into_iter()
        .for_each(|c| accumulator.push(c));
    (accumulator, decoder)
}

#[test]
fn an_ai_sdk_response_folds_into_the_message() {
    let body = sse(&[
        json!({ "type": "start", "messageId": "m1" }),
        json!({ "type": "start-step" }),
        json!({ "type": "reasoning-start", "id": "r1" }),
        json!({ "type": "reasoning-delta", "id": "r1", "delta": "Thinking" }),
        json!({ "type": "reasoning-end", "id": "r1" }),
        json!({ "type": "text-start", "id": "t1" }),
        json!({ "type": "text-delta", "id": "t1", "delta": "Hello" }),
        json!({ "type": "text-delta", "id": "t1", "delta": " world" }),
        json!({ "type": "text-end", "id": "t1" }),
        json!({ "type": "tool-input-start", "toolCallId": "c1", "toolName": "weather" }),
        json!({ "type": "tool-input-delta", "toolCallId": "c1", "inputTextDelta": "{\"city\":" }),
        json!({ "type": "tool-input-delta", "toolCallId": "c1", "inputTextDelta": "\"NYC\"}" }),
        json!({ "type": "tool-input-available", "toolCallId": "c1", "toolName": "weather", "input": { "city": "NYC" } }),
        json!({ "type": "tool-output-available", "toolCallId": "c1", "output": { "temp": 70 } }),
        json!({ "type": "source-url", "sourceId": "s1", "url": "https://example.com", "title": "Example" }),
        json!({ "type": "file", "url": "https://example.com/a.png", "mediaType": "image/png" }),
        json!({ "type": "data-weather", "data": { "city": "NYC" } }),
        json!({ "type": "data-progress", "data": 1, "transient": true }),
        json!({ "type": "finish-step", "finishReason": "stop", "usage": { "inputTokens": 3, "outputTokens": 5 } }),
        json!({ "type": "finish" }),
    ]);
    let (accumulator, decoder) = fold(&body);
    assert!(decoder.warnings().is_empty());
    let parts = accumulator.parts();
    assert_eq!(parts.len(), 5);
    assert!(
        matches!(&parts[0], AssistantMessagePart::Reasoning(r) if r.text == "Thinking" && r.status == Some(PartStatus::Complete))
    );
    assert!(
        matches!(&parts[1], AssistantMessagePart::Text(t) if t.text == "Hello world" && t.status == Some(PartStatus::Complete))
    );
    let call = parts[2].as_tool_call().unwrap();
    assert_eq!(call.tool_name, "weather");
    assert_eq!(call.args_text, r#"{"city":"NYC"}"#);
    assert_eq!(Value::Object(call.args.clone()), json!({ "city": "NYC" }));
    assert_eq!(call.result, Some(json!({ "temp": 70 })));
    assert!(
        matches!(&parts[3], AssistantMessagePart::Source(SourcePart::Url { url, title: Some(t), .. }) if url == "https://example.com" && t == "Example")
    );
    assert!(matches!(&parts[4], AssistantMessagePart::File(f) if f.mime_type == "image/png"));

    assert_eq!(
        accumulator.status(),
        &MessageStatus::Complete {
            reason: CompleteReason::Unknown
        }
    );
    let metadata = accumulator.result().metadata.unwrap();
    assert_eq!(
        metadata.data,
        Some(vec![
            json!({ "name": "weather", "data": { "city": "NYC" } })
        ]),
        "transient data is not kept"
    );
    let steps = metadata.steps.unwrap();
    assert_eq!(steps.len(), 1, "start and start-step open one step");
    assert_eq!(steps[0].message_id.as_deref(), Some("m1"));
    assert_eq!(
        steps[0].usage,
        Some(Usage {
            input_tokens: 3,
            output_tokens: 5
        })
    );
}

#[test]
fn tool_calls_needing_a_client_result_end_in_requires_action() {
    let body = sse(&[
        json!({ "type": "start", "messageId": "m" }),
        json!({ "type": "tool-input-available", "toolCallId": "c", "toolName": "ask", "input": { "q": "?" } }),
        json!({ "type": "finish", "finishReason": "tool-calls" }),
    ]);
    let (accumulator, _) = fold(&body);
    let call = accumulator.parts()[0].as_tool_call().unwrap();
    assert!(call.is_open());
    assert_eq!(
        accumulator.status(),
        &MessageStatus::RequiresAction {
            reason: RequiresActionReason::ToolCalls
        }
    );
}

#[test]
fn tool_errors_and_preliminary_outputs() {
    let body = sse(&[
        json!({ "type": "tool-input-available", "toolCallId": "a", "toolName": "t", "input": {} }),
        json!({ "type": "tool-output-available", "toolCallId": "a", "output": "partial", "preliminary": true }),
        json!({ "type": "tool-output-available", "toolCallId": "a", "output": "done" }),
        json!({ "type": "tool-input-error", "toolCallId": "b", "toolName": "t", "input": "{bad", "errorText": "invalid input" }),
        json!({ "type": "tool-output-error", "toolCallId": "c", "errorText": "boom" }),
    ]);
    let (accumulator, _) = fold(&body);
    let calls: Vec<_> = accumulator
        .parts()
        .iter()
        .filter_map(AssistantMessagePart::as_tool_call)
        .collect();
    assert_eq!(calls.len(), 3);
    assert_eq!(
        (calls[0].result.clone(), calls[0].is_preliminary),
        (Some(json!("done")), None)
    );
    assert_eq!(
        (calls[1].args_text.as_str(), calls[1].is_error),
        ("{bad", Some(true))
    );
    assert_eq!(
        (calls[2].result.clone(), calls[2].is_error),
        (Some(json!("boom")), Some(true)),
        "an output alone opens the call"
    );
}

#[test]
fn approval_request_response_and_denial() {
    let body = sse(&[
        json!({ "type": "tool-input-available", "toolCallId": "c", "toolName": "delete", "input": {} }),
        json!({ "type": "tool-approval-request", "toolCallId": "c", "approvalId": "ap", "reason": "Deletes files" }),
        json!({ "type": "tool-approval-response", "approvalId": "ap", "approved": false, "reason": "Not now" }),
        json!({ "type": "tool-output-denied", "toolCallId": "c" }),
    ]);
    let (accumulator, _) = fold(&body);
    let call = accumulator.parts()[0].as_tool_call().unwrap();
    let approval = call.approval.as_ref().unwrap();
    assert_eq!(
        (
            approval.id.as_str(),
            approval.prompt.as_deref(),
            approval.approved
        ),
        ("ap", Some("Deletes files"), Some(false))
    );
    assert_eq!(call.result, Some(json!({ "error": "Not now" })));
    assert_eq!(call.is_error, Some(true));
}

#[test]
fn abort_finishes_as_incomplete_and_errors_are_reported() {
    let (aborted, _) = fold(&sse(&[
        json!({ "type": "text-delta", "delta": "Hi" }),
        json!({ "type": "abort" }),
    ]));
    assert_eq!(
        aborted.status(),
        &MessageStatus::Incomplete {
            reason: IncompleteReason::Other,
            error: None
        }
    );
    assert!(
        matches!(&aborted.parts()[0], AssistantMessagePart::Text(t) if t.text == "Hi"),
        "a delta without text-start opens a part"
    );

    let (errored, _) = fold(&sse(&[
        json!({ "type": "error", "errorText": "rate limited" }),
    ]));
    assert!(
        matches!(errored.status(), MessageStatus::Incomplete { reason: IncompleteReason::Error, error: Some(e) } if e["message"] == "rate limited")
    );
}

#[test]
fn text_delta_aliases_keep_precedence_ids_and_provider_metadata() {
    let body = sse(&[
        json!({ "type": "text-start", "id": "t" }),
        json!({ "type": "reasoning-start", "id": "r" }),
        json!({ "type": "text-delta", "id": "t", "textDelta": "legacy ", "delta": "ignored" }),
        json!({ "type": "text-delta", "id": "t", "delta": "é🦀", "providerMetadata": { "provider": { "signature": "retained" } } }),
        json!({ "type": "text-delta", "id": "t", "textDelta": null, "delta": "ignored" }),
        json!({ "type": "reasoning-delta", "id": "r", "delta": "thinking" }),
        json!({ "type": "text-delta", "delta": " active" }),
        json!({ "type": "text-end", "id": "t" }),
        json!({ "type": "reasoning-end", "id": "r" }),
    ]);
    let (reference, _) = fold(&body);
    assert!(
        matches!(&reference.parts()[0], AssistantMessagePart::Text(text) if text.text == "legacy é🦀 active")
    );
    assert!(
        matches!(&reference.parts()[1], AssistantMessagePart::Reasoning(text) if text.text == "thinking")
    );
    assert_eq!(
        serde_json::to_value(&reference.parts()[0]).unwrap()["providerMetadata"],
        json!({ "provider": { "signature": "retained" } })
    );

    for width in [7, 64, body.len()] {
        let mut decoder = UiMessageStreamDecoder::default();
        let mut accumulator = MessageAccumulator::default();
        for slice in body.chunks(width) {
            for chunk in decoder.push(slice).unwrap() {
                accumulator.push(chunk);
            }
        }
        for chunk in decoder.finish().unwrap() {
            accumulator.push(chunk);
        }
        assert_eq!(accumulator.result(), reference.result(), "width={width}");
    }
}

#[test]
fn malformed_chunks_are_dropped_and_unknown_results_fail_the_stream() {
    let mut decoder = UiMessageStreamDecoder::default();
    decoder
        .push(b"data: not json\n\ndata: {\"no\":\"type\"}\n\n")
        .unwrap();
    assert_eq!(decoder.warnings().len(), 2);
    let error = decoder.push(b"data: {\"type\":\"tool-call-start\",\"toolCallId\":\"x\",\"toolName\":\"t\"}\n\ndata: {\"type\":\"tool-result\",\"toolCallId\":\"y\",\"result\":1}\n\n");
    assert!(error.unwrap_err().contains("unknown id"));
}

#[test]
fn a_ui_message_stream_drives_the_runtime() {
    struct Body(Vec<Vec<u8>>);
    impl futures_core::Stream for Body {
        type Item = Result<Vec<u8>, String>;
        fn poll_next(
            mut self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Self::Item>> {
            std::task::Poll::Ready((!self.0.is_empty()).then(|| Ok(self.0.remove(0))))
        }
    }
    let runtime = LocalThreadRuntime::new(
        |_options: ChatModelRunOptions| {
            let body = sse(&[
                json!({ "type": "text-start", "id": "t" }),
                json!({ "type": "text-delta", "id": "t", "delta": "Streamed " }),
                json!({ "type": "text-delta", "id": "t", "delta": "answer" }),
                json!({ "type": "finish", "finishReason": "stop" }),
            ]);
            run_stream(
                Body(body.chunks(16).map(<[u8]>::to_vec).collect()),
                UiMessageStreamDecoder::default(),
                MessageAccumulator::default(),
            )
        },
        LocalRuntimeOptions::default(),
    );
    futures_executor::block_on(
        runtime
            .append(AppendMessage::user_text(None, "Hi"))
            .unwrap(),
    )
    .unwrap();
    let answer = runtime.messages()[1].as_assistant().unwrap().clone();
    assert_eq!(
        answer.content,
        [AssistantMessagePart::Text(TextPart {
            text: "Streamed answer".into(),
            status: Some(PartStatus::Complete),
            ..Default::default()
        })]
    );
    assert_eq!(
        answer.status,
        MessageStatus::Complete {
            reason: CompleteReason::Stop
        }
    );
}
