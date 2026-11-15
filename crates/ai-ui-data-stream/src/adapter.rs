//! `DataStreamRuntimeAdapter`: a [`ChatModelAdapter`] that POSTs the thread to
//! an AI SDK endpoint and streams the reply.

use crate::convert::to_model_messages;
use crate::http::{HttpClient, HttpRequest, HttpResponse, ResponseFuture};
use ai_ui_core::adapter::{ChatModelRunOptions, RunError, RunStream};
use ai_ui_core::{ChatModelAdapter, ChatModelRunResult, JsonObject, ThreadMessage};
use ai_ui_stream::{run_stream, DataStreamDecoder, MessageAccumulator, UiMessageStreamDecoder};
use futures_core::Stream;
use serde_json::{json, Map, Value};
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::task::{Context, Poll};

/// The response format (`DataStreamProtocol`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Protocol {
    /// AI SDK v5+ UI message stream (SSE JSON).
    UiMessageStream,
    /// AI SDK v4 data stream (`CODE:JSON` lines).
    DataStream,
}

/// Explicit choice, else the AI SDK response headers, else the UI message stream.
pub fn resolve_protocol(response: &HttpResponse, explicit: Option<Protocol>) -> Protocol {
    let v1 = |name| response.header(name).is_some_and(|v| v.trim() == "v1");
    explicit.unwrap_or(if v1("x-vercel-ai-data-stream") {
        Protocol::DataStream
    } else {
        Protocol::UiMessageStream
    })
}

type HeadersFn = dyn Fn() -> Vec<(String, String)>;
type BodyFn = dyn Fn(Option<&str>) -> JsonObject;

/// Options of [`DataStreamAdapter`] (`useDataStreamRuntime` minus React).
pub struct DataStreamOptions {
    /// The endpoint, for example `https://example.com/api/chat`.
    pub api: String,
    /// Extra request headers, evaluated per request (for example a fresh token).
    pub headers: Option<Rc<HeadersFn>>,
    /// Extra body fields, evaluated per request with the thread id.
    pub body: Option<Rc<BodyFn>>,
    /// Skip header detection.
    pub protocol: Option<Protocol>,
}

impl DataStreamOptions {
    /// Options for the endpoint at `api`, with defaults for everything else.
    pub fn new(api: impl Into<String>) -> Self {
        Self {
            api: api.into(),
            headers: None,
            body: None,
            protocol: None,
        }
    }
}

/// A model adapter that talks to an AI SDK backend over HTTP.
pub struct DataStreamAdapter<C> {
    client: Rc<C>,
    options: Rc<DataStreamOptions>,
}

impl<C: HttpClient + 'static> DataStreamAdapter<C> {
    /// An adapter that sends requests through `client` as `options` say.
    pub fn new(client: C, options: DataStreamOptions) -> Self {
        Self {
            client: Rc::new(client),
            options: Rc::new(options),
        }
    }
}

/// The JSON request body upstream sends.
pub fn request_body(options: &ChatModelRunOptions, extra: Option<JsonObject>) -> Value {
    let mut thread: Vec<&ThreadMessage> = options.messages.iter().map(Arc::as_ref).collect();
    let current = ThreadMessage::Assistant(options.message.clone());
    thread.push(&current);
    let tools: Map<String, Value> = options
        .context
        .tools
        .iter()
        .map(|(name, tool)| {
            let mut entry = json!({ "parameters": tool.parameters });
            if let Some(description) = &tool.description {
                entry["description"] = Value::String(description.clone());
            }
            (name.clone(), entry)
        })
        .collect();
    let mut body = Map::new();
    if let Some(system) = &options.context.system {
        body.insert("system".into(), json!(system));
    }
    body.insert(
        "messages".into(),
        Value::Array(to_model_messages(thread.iter().copied())),
    );
    body.insert("tools".into(), Value::Object(tools));
    body.insert(
        "unstable_assistantMessageId".into(),
        json!(options.assistant_message_id),
    );
    if let Some(thread_id) = &options.thread_id {
        body.insert("threadId".into(), json!(thread_id));
    }
    body.insert("parentId".into(), json!(options.parent_id));
    body.insert(
        "runConfig".into(),
        serde_json::to_value(&options.run_config).unwrap_or(Value::Null),
    );
    if !options.message.metadata.state.is_null() {
        body.insert("state".into(), options.message.metadata.state.clone());
    }
    body.extend(options.context.call_settings.clone());
    body.extend(options.context.config.clone());
    body.extend(extra.unwrap_or_default());
    Value::Object(body)
}

impl<C: HttpClient + 'static> ChatModelAdapter for DataStreamAdapter<C> {
    fn run(&self, options: ChatModelRunOptions) -> RunStream {
        let extra = self
            .options
            .body
            .as_ref()
            .map(|body| body(options.thread_id.as_deref()));
        let body = request_body(&options, extra);
        let mut headers = self
            .options
            .headers
            .as_ref()
            .map(|h| h())
            .unwrap_or_default();
        headers.retain(|(name, _)| !name.eq_ignore_ascii_case("content-type"));
        headers.push(("Content-Type".into(), "application/json".into()));
        let request = HttpRequest {
            url: self.options.api.clone(),
            headers,
            body: body.to_string().into_bytes(),
        };
        let response = self.client.post(request, options.abort_signal.clone());
        let accumulator =
            MessageAccumulator::default().with_state(options.message.metadata.state.clone());
        Box::pin(Response {
            state: State::Waiting(response, Box::new(accumulator)),
            protocol: self.options.protocol,
        })
    }
}

enum State {
    Waiting(ResponseFuture, Box<MessageAccumulator>),
    Streaming(RunStream),
    Done,
}

struct Response {
    state: State,
    protocol: Option<Protocol>,
}

impl Stream for Response {
    type Item = Result<ChatModelRunResult, RunError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            match &mut self.state {
                State::Done => return Poll::Ready(None),
                State::Streaming(stream) => return stream.as_mut().poll_next(cx),
                State::Waiting(future, _) => {
                    let response = match future.as_mut().poll(cx) {
                        Poll::Pending => return Poll::Pending,
                        Poll::Ready(response) => response,
                    };
                    let State::Waiting(_, accumulator) =
                        std::mem::replace(&mut self.state, State::Done)
                    else {
                        unreachable!("matched above")
                    };
                    let response = match response {
                        Ok(response) => response,
                        Err(error) => return Poll::Ready(Some(Err(RunError::failed(error.0)))),
                    };
                    if !(200..300).contains(&response.status) {
                        return Poll::Ready(Some(Err(RunError::failed(format!(
                            "Status {}",
                            response.status
                        )))));
                    }
                    let stream = match resolve_protocol(&response, self.protocol) {
                        Protocol::UiMessageStream => run_stream(
                            response.body,
                            UiMessageStreamDecoder::default(),
                            *accumulator,
                        ),
                        Protocol::DataStream => {
                            run_stream(response.body, DataStreamDecoder::default(), *accumulator)
                        }
                    };
                    self.state = State::Streaming(stream);
                }
            }
        }
    }
}
