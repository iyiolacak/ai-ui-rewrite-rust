//! The data-stream adapter against a recording HTTP double, and the message
//! conversion rules, after upstream's react-data-stream tests.

use ai_ui_core::adapter::ToolDefinition;
use ai_ui_core::*;
use ai_ui_data_stream::*;
use futures_core::Stream;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

struct Body(Vec<Vec<u8>>);
impl Stream for Body {
    type Item = Result<Vec<u8>, HttpError>;
    fn poll_next(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready((!self.0.is_empty()).then(|| Ok(self.0.remove(0))))
    }
}

/// Answers every POST with one canned response and records the request body.
#[derive(Clone)]
struct Mock {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
    requests: Rc<RefCell<Vec<Value>>>,
}

impl Mock {
    fn new(status: u16, headers: &[(&str, &str)], body: &str) -> Self {
        Self {
            status,
            headers: headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            body: body.into(),
            requests: Rc::default(),
        }
    }
}

impl HttpClient for Mock {
    fn post(&self, request: HttpRequest, _signal: AbortSignal) -> ResponseFuture {
        self.requests
            .borrow_mut()
            .push(serde_json::from_slice(&request.body).unwrap());
        assert!(request
            .headers
            .iter()
            .any(|(k, v)| k == "Content-Type" && v == "application/json"));
        let response = HttpResponse {
            status: self.status,
            headers: self.headers.clone(),
            body: Box::pin(Body(
                self.body.as_bytes().chunks(7).map(<[u8]>::to_vec).collect(),
            )),
        };
        Box::pin(async move { Ok(response) })
    }
}

fn run(mock: &Mock, options: DataStreamOptions) -> (LocalThreadRuntime, Result<(), RunError>) {
    let runtime = LocalThreadRuntime::new(
        DataStreamAdapter::new(mock.clone(), options),
        LocalRuntimeOptions::default(),
    );
    let mut context = ModelContext {
        system: Some("Be brief.".into()),
        ..Default::default()
    };
    context.tools.insert(
        "weather".into(),
        ToolDefinition {
            description: Some("Weather".into()),
            parameters: json!({ "type": "object" }),
        },
    );
    runtime.set_model_context(context);
    let result = futures_executor::block_on(
        runtime
            .append(AppendMessage::user_text(None, "Weather?"))
            .unwrap(),
    );
    (runtime, result)
}

const UI_BODY: &str = "data: {\"type\":\"text-start\",\"id\":\"t\"}\n\ndata: {\"type\":\"text-delta\",\"id\":\"t\",\"delta\":\"Sunny\"}\n\ndata: {\"type\":\"finish\",\"finishReason\":\"stop\"}\n\ndata: [DONE]\n\n";

#[test]
fn posts_upstreams_request_body_and_streams_a_ui_message_reply() {
    let mock = Mock::new(200, &[("x-vercel-ai-ui-message-stream", "v1")], UI_BODY);
    let mut options = DataStreamOptions::new("https://example.com/api/chat");
    options.body = Some(Rc::new(|_thread| {
        serde_json::from_value(json!({ "model": "small" })).unwrap()
    }));
    let (runtime, result) = run(&mock, options);
    result.unwrap();
    assert_eq!(runtime.messages()[1].text(), "Sunny");
    assert_eq!(
        runtime.messages()[1].status(),
        Some(&MessageStatus::Complete {
            reason: CompleteReason::Stop
        })
    );

    let body = mock.requests.borrow()[0].clone();
    assert_eq!(body["system"], "Be brief.");
    assert_eq!(
        body["messages"],
        json!([{ "role": "user", "content": [{ "type": "text", "text": "Weather?" }] }]),
        "the empty in-progress assistant message adds nothing"
    );
    assert_eq!(
        body["tools"],
        json!({ "weather": { "description": "Weather", "parameters": { "type": "object" } } })
    );
    assert_eq!(
        body["unstable_assistantMessageId"],
        json!(runtime.messages()[1].id())
    );
    assert_eq!(body["parentId"], json!(runtime.messages()[0].id()));
    assert_eq!(body["model"], "small", "extra body fields are merged");
}

#[test]
fn detects_the_data_stream_protocol_from_the_header() {
    let mock = Mock::new(
        200,
        &[("X-Vercel-AI-Data-Stream", " v1 ")],
        "0:\"Rain\"\nd:{\"finishReason\":\"stop\"}\n",
    );
    let (runtime, result) = run(&mock, DataStreamOptions::new("https://e/api"));
    result.unwrap();
    assert_eq!(runtime.messages()[1].text(), "Rain");
}

#[test]
fn an_explicit_protocol_overrides_detection_and_the_fallback_is_ui_messages() {
    let mut options = DataStreamOptions::new("https://e/api");
    options.protocol = Some(Protocol::DataStream);
    let (runtime, _) = run(&Mock::new(200, &[], "0:\"Explicit\"\n"), options);
    assert_eq!(runtime.messages()[1].text(), "Explicit");
    let (runtime, _) = run(
        &Mock::new(200, &[], UI_BODY),
        DataStreamOptions::new("https://e/api"),
    );
    assert_eq!(runtime.messages()[1].text(), "Sunny");
}

#[test]
fn an_http_error_fails_the_run_and_marks_the_message() {
    let (runtime, result) = run(
        &Mock::new(500, &[], "boom"),
        DataStreamOptions::new("https://e/api"),
    );
    assert!(matches!(result, Err(RunError::Failed { message, .. }) if message == "Status 500"));
    assert!(matches!(
        runtime.messages()[1].status(),
        Some(MessageStatus::Incomplete {
            reason: IncompleteReason::Error,
            ..
        })
    ));
}

// ---------------------------------------------------------------- conversion

fn assistant(content: Value, status: Value) -> ThreadMessage {
    serde_json::from_value(json!({ "role": "assistant", "id": "a", "createdAt": 0, "content": content, "status": status })).unwrap()
}
fn user(content: Value, attachments: Value) -> ThreadMessage {
    serde_json::from_value(json!({ "role": "user", "id": "u", "createdAt": 0, "content": content, "attachments": attachments })).unwrap()
}
fn complete() -> Value {
    json!({ "type": "complete", "reason": "stop" })
}

#[test]
fn settled_tool_calls_become_assistant_and_tool_messages() {
    let message = assistant(
        json!([
            { "type": "text", "text": "Checking" },
            { "type": "tool-call", "toolCallId": "c", "toolName": "weather", "args": { "city": "NYC" }, "argsText": "", "result": { "temp": 70 } },
            { "type": "text", "text": "It is warm." }
        ]),
        complete(),
    );
    assert_eq!(
        to_model_messages([&message]),
        json!([
            { "role": "assistant", "content": [{ "type": "text", "text": "Checking" }, { "type": "tool-call", "toolCallId": "c", "toolName": "weather", "input": { "city": "NYC" } }] },
            { "role": "tool", "content": [{ "type": "tool-result", "toolCallId": "c", "toolName": "weather", "output": { "type": "json", "value": { "temp": 70 } } }] },
            { "role": "assistant", "content": [{ "type": "text", "text": "It is warm." }] }
        ])
        .as_array()
        .unwrap()
        .clone()
    );
}

#[test]
fn unfinished_calls_are_errors_unless_the_last_message_awaits_the_user() {
    let call = json!([{ "type": "tool-call", "toolCallId": "c", "toolName": "t", "args": {}, "argsText": "", "approval": { "id": "ap" } }]);
    let earlier = assistant(call.clone(), complete());
    let converted = to_model_messages([&earlier]);
    assert_eq!(
        converted[1]["content"][0]["output"],
        json!({ "type": "error-json", "value": { "error": "Tool call was not completed" } })
    );
    let waiting = assistant(
        call,
        json!({ "type": "requires-action", "reason": "tool-calls" }),
    );
    let converted = to_model_messages([&waiting]);
    assert_eq!(
        converted.len(),
        1,
        "a pending approval on the in-flight message sends no result"
    );
}

#[test]
fn decided_approvals_add_request_and_response_parts_and_denials_are_execution_denied() {
    let message = assistant(
        json!([{ "type": "tool-call", "toolCallId": "c", "toolName": "delete", "args": {}, "argsText": "", "result": "skipped",
                 "approval": { "id": "ap", "approved": false, "reason": "Not now", "requestReason": "Deletes files" } }]),
        complete(),
    );
    let converted = to_model_messages([&message]);
    assert_eq!(
        converted[0]["content"][1],
        json!({ "type": "tool-approval-request", "approvalId": "ap", "toolCallId": "c", "reason": "Deletes files" })
    );
    assert_eq!(
        converted[1]["content"][0],
        json!({ "type": "tool-approval-response", "approvalId": "ap", "approved": false, "reason": "Not now" })
    );
    assert_eq!(
        converted[1]["content"][1]["output"],
        json!({ "type": "execution-denied", "reason": "Not now" })
    );
}

#[test]
fn user_images_files_and_attachments_become_file_parts() {
    let message = user(
        json!([{ "type": "text", "text": "See" }, { "type": "image", "image": "https://e.com/cat.JPG?x=1" }, { "type": "file", "data": "data:text/csv;base64,YQ==", "mimeType": "" }]),
        json!([{ "id": "1", "type": "image", "name": "dog", "contentType": "image/webp", "content": [{ "type": "image", "image": "https://e.com/dog" }] }]),
    );
    let converted = to_model_messages([&message]);
    let parts = converted[0]["content"].as_array().unwrap();
    assert_eq!(
        parts[1],
        json!({ "type": "file", "data": "https://e.com/cat.JPG?x=1", "mediaType": "image/jpeg" })
    );
    assert_eq!(parts[2]["mediaType"], "text/csv");
    assert_eq!(
        parts[3]["mediaType"], "image/webp",
        "the attachment's declared type wins"
    );
}
