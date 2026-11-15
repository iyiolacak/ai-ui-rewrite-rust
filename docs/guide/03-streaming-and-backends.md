---
title: Streaming and backends
description: How model output becomes messages - chunks, decoders for the three wire protocols, the HTTP clients - and how to connect and test a real backend.
order: 3
section: Build
---

# Streaming and backends

**What this page gives you.** This page shows how bytes from a backend become messages: the four-stage pipeline, the three wire formats and their decoders, request building, the HTTP clients, a connection to a real backend, and how to test without a network.

[Concepts](02-concepts.md) described the runtime from the adapter inwards. This page covers the other side: how bytes from
a backend become the cumulative results an adapter yields, which wire formats are understood, how a request is built,
and how to connect and test a real backend. Byte-level detail lives in the reference documents
[PROTOCOL](../PROTOCOL.md) (the protocol audit and conformance record) and [AI-SDK-STREAM](../AI-SDK-STREAM.md) (the strict AI SDK 7
reducer); this page links to them rather than repeating them.

## The pipeline

Every network adapter does the same four things, and each has one type:

1. **Transport** (`HttpClient`, `ai-ui-data-stream`): send a request, return a status, headers and a `ByteStream` of body
   reads. The reads can split anywhere, including inside a UTF-8 character.
2. **Decoder** (`StreamDecoder`, `ai-ui-stream`): `push(bytes)` returns the `Chunk`s that are now complete, `finish()`
   flushes at end of input. Each wire format has one decoder.
3. **Accumulator** (`MessageAccumulator`): folds chunks into the assistant message and produces a `ChatModelRunResult`.
4. **Run stream** (`run_stream`): ties the three together and yields the cumulative result to the runtime.

### Chunks

A `Chunk` is the one internal vocabulary all wire formats are translated into, modelled on assistant-stream's
`AssistantStreamChunk`. A chunk has a `path` (the index of the part it applies to) and a `ChunkKind`: `PartStart` (a
`PartInit` of text, reasoning, tool call, source, file or data), `PartFinish`, `TextDelta`, `ToolCallArgsTextFinish`,
`ToolCallApproval`, `Annotations`, `Data`, `StepStart`, `StepFinish`, `MessageFinish`, `Result` (a tool result), `Error` and
`UpdateState`. Tool arguments arrive as `TextDelta`s on a tool-call part. One vocabulary means a new wire format needs only a
decoder, and everything after it (the accumulator, the runtime, tool continuation, the UI) is shared.

### The accumulator

`MessageAccumulator` applies chunks in order and never panics on bad input: an invalid chunk is dropped and recorded in
`warnings()`. `result()` builds the cumulative `ChatModelRunResult`, `status()` reports `Running` until the message
finishes, and `finish()` settles a message at a clean end of stream: `RequiresAction` if a tool call is still open,
otherwise `Complete`. Configurable `Limits` bound tool-argument size and state size so a hostile or broken stream cannot
allocate without limit.

`run_stream(bytes, decoder, accumulator)` does the reading. It folds every slice that is ready into one result, so a burst of
reads costs one `result()` call, and it yields to the executor after a bounded number of slices so a source that is always
ready cannot starve painting or cancellation. A transport or decoding error ends the run, but content accumulated before
it is delivered first and stays on the message.

### Partial JSON

Tool arguments are JSON that arrives in fragments. A UI wants to show `{"city":"Oslo"` as it grows, but nothing may act on
it until it is complete. `ai-ui-stream::partial_json::Parser` keeps an incremental parse: after each `append` the
`snapshot()` holds the best reading so far, a `State` of `Partial` or `Complete`, and the path of the value still being
written. It enforces limits on bytes, nodes and depth.

```rust
use ai_ui_stream::partial_json::{Limits, Parser, State};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut parser = Parser::new(Limits::default())?;
    for fragment in [r#"{"city":"Os"#, r#"lo","units":"#, r#""metric"}"#] {
        parser.append(fragment)?;
        let snapshot = parser.snapshot();
        let complete = snapshot.state == State::Complete;
        println!("{} (complete: {complete})", snapshot.value);
    }
    Ok(())
}
```

```text
{"city":"Os"} (complete: false)
{"city":"Oslo"} (complete: false)
{"city":"Oslo","units":"metric"} (complete: true)
```

The accumulator keeps the raw argument text (`args_text`) separately from the parsed `args`, and the runtime runs a tool
only from complete arguments. A response that ends mid-arguments produces an error result for that call instead of
executing a repaired guess (see [Tools and approvals](02-concepts.md#tools-and-approvals)).

## Three wire formats

| Format | Marker | Framing | Decoder | Selected by |
|---|---|---|---|---|
| AI SDK UI message stream (v5 to v7) | `x-vercel-ai-ui-message-stream: v1` | SSE, one JSON object per `data:` event, optional `[DONE]` | `UiMessageStreamDecoder` | `DataStreamAdapter` (default) |
| AI SDK data stream (v4) with assistant-ui's `aui-*` extensions | `x-vercel-ai-data-stream: v1` | `CODE:JSON` lines, no SSE | `DataStreamDecoder` | `DataStreamAdapter`, by header |
| assistant-transport (assistant-stream) | none | SSE, `data: <JSON>`, ends `data: [DONE]` | `SseDecoder`, `run_stream_from_sse` | your own adapter |

The UI message stream header is the same for AI SDK v5, v6 and v7, so it does not tell you the SDK version. The format is
a superset over time: newer versions add optional fields.

### The AI SDK UI message stream

Events are typed JSON objects: `start`, `text-start` / `text-delta` / `text-end` (with an `id`), the `reasoning-*`
equivalents, `tool-input-start` / `tool-input-delta` / `tool-input-available` / `tool-input-error`,
`tool-output-available` / `tool-output-error` / `tool-output-denied`, `tool-approval-request` / `tool-approval-response`,
`source-url`, `source-document`, `file`, `data-<name>`, `start-step`, `finish-step`, `reset-step`, `finish`, `abort`,
`error` and `message-metadata`. The decoder below feeds the bytes in 11-byte reads to show that framing does not matter:

```rust
use ai_ui_stream::{MessageAccumulator, StreamDecoder, UiMessageStreamDecoder};

fn text_of(result: &ai_ui_core::ChatModelRunResult) -> String {
    result
        .content
        .iter()
        .flatten()
        .filter_map(|part| match part {
            ai_ui_core::AssistantMessagePart::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect()
}

fn main() -> Result<(), String> {
    let wire = concat!(
        "data: {\"type\":\"text-start\",\"id\":\"t1\"}\n\n",
        "data: {\"type\":\"text-delta\",\"id\":\"t1\",\"delta\":\"Hello, \"}\n\n",
        "data: {\"type\":\"text-delta\",\"id\":\"t1\",\"delta\":\"world\"}\n\n",
        "data: {\"type\":\"text-end\",\"id\":\"t1\"}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":\"stop\"}\n\n",
        "data: [DONE]\n\n",
    );
    let mut decoder = UiMessageStreamDecoder::default();
    let mut accumulator = MessageAccumulator::default();
    // Network reads can split anywhere, even inside a UTF-8 character.
    for read in wire.as_bytes().chunks(11) {
        for chunk in decoder.push(read)? {
            accumulator.push(chunk);
        }
        let result = accumulator.result();
        if !text_of(&result).is_empty() {
            println!(
                "{} -> {:?}",
                String::from_utf8_lossy(read).escape_debug(),
                text_of(&result)
            );
        }
    }
    for chunk in decoder.finish()? {
        accumulator.push(chunk);
    }
    accumulator.finish();
    println!("{:?}", accumulator.status());
    Ok(())
}
```

It prints the text as it grows (`Hello, `, then `Hello, world`) and ends with `Complete { reason: Stop }`. The same loop
with a tool call whose arguments stream in two pieces:

```rust
use ai_ui_stream::{MessageAccumulator, StreamDecoder, UiMessageStreamDecoder};

fn main() -> Result<(), String> {
    // A tool call whose arguments stream in pieces, as a real model produces them.
    let wire = concat!(
        "data: {\"type\":\"tool-input-start\",\"toolCallId\":\"c1\",\"toolName\":\"get_weather\"}\n\n",
        "data: {\"type\":\"tool-input-delta\",\"toolCallId\":\"c1\",\"inputTextDelta\":\"{\\\"city\\\":\"}\n\n",
        "data: {\"type\":\"tool-input-delta\",\"toolCallId\":\"c1\",\"inputTextDelta\":\"\\\"Oslo\\\"}\"}\n\n",
        "data: {\"type\":\"tool-input-available\",\"toolCallId\":\"c1\",\"toolName\":\"get_weather\",\"input\":{\"city\":\"Oslo\"}}\n\n",
    );
    let mut decoder = UiMessageStreamDecoder::default();
    let mut accumulator = MessageAccumulator::default();
    for chunk in decoder.push(wire.as_bytes())? {
        accumulator.push(chunk);
    }
    for chunk in decoder.finish()? {
        accumulator.push(chunk);
    }
    for part in accumulator.parts() {
        if let Some(call) = part.as_tool_call() {
            println!(
                "{}: args_text={} args={}",
                call.tool_name,
                call.args_text,
                serde_json::Value::Object(call.args.clone())
            );
        }
    }
    Ok(())
}
```

prints `get_weather: args_text={"city":"Oslo"} args={"city":"Oslo"}`. Note that, as in upstream's decoder, the UI-stream
decoder holds tool input back until `tool-input-available` and then writes it as one piece; the legacy data stream below
streams argument text progressively.

`ai-ui-stream` is deliberately lenient on this wire: content that arrived is kept, and a lifecycle violation such as a delta
before its start produces a warning, not a failed run. The strict reducer that mirrors AI SDK 7 error for error is a
separate crate, `ai-ui-sdk-stream`, described in [AI-SDK-STREAM](../AI-SDK-STREAM.md). For a valid stream both give the same
content; they differ only on malformed or unusual ones. Which events `ai-ui-stream` does not represent yet
(`reasoning-file`, `custom`, `steering-message`, citations, message metadata, tool `inputEncoding` and `title`, abort
reason) is tracked in [PROTOCOL](../PROTOCOL.md#conformance-status). Two deliberate improvements over assistant-ui's own
UI-stream adapter are worth knowing: `data-*` parts with an `id` replace the earlier part with that name and id, and
`reset-step` discards the parts of the current step.

### The data stream and the `aui-*` extensions

The older format is one `CODE:JSON` per line: `0` text, `g` reasoning, `b` / `c` / `9` tool input start, delta and complete
call, `a` tool result, `d` finish message, `e` finish step, `f` start step, `h` source, `k` file, `2` data, `8` annotations,
`3` error. assistant-ui adds codes of its own on the same framing: `aui-state` (state operations), `aui-text-delta`,
`aui-reasoning-delta`, `aui-data`, `aui-reasoning-part-start` and `aui-tool-call-approval`, and extends several stock codes
with fields such as `parentId`, `artifact`, `isPreliminary` and `modelContent`. `DataStreamDecoder` handles all of these. As
upstream's default decoder it is strict: a malformed value or a reference to an unknown tool call fails the stream.
The full code table is in [PROTOCOL](../PROTOCOL.md#a2-legacy-ai-sdk-data-stream-ai4319).

### assistant-transport

assistant-stream's own SSE format carries the chunk vocabulary almost directly: 13 event types, each with a numeric `path`
addressing a part (`part-start`, `text-delta`, `tool-call-approval`, `result`, `update-state`, and so on), ending with
`data: [DONE]`. Because it is not an AI SDK format, `DataStreamAdapter` does not select it. Wrap your response bytes with
`run_stream_from_sse` inside your own adapter:

```rust
use ai_ui_core::adapter::RunStream;
use ai_ui_core::{AppendMessage, ChatModelRunOptions, LocalRuntimeOptions, LocalThreadRuntime};
use ai_ui_stream::{run_stream_from_sse, MessageAccumulator};
use std::convert::Infallible;
use std::pin::Pin;
use std::task::{Context, Poll};

/// A response body that arrives in the given reads.
struct Reads(std::vec::IntoIter<Vec<u8>>);

impl futures_core::Stream for Reads {
    type Item = Result<Vec<u8>, Infallible>;
    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(self.0.next().map(Ok))
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // An assistant-transport response: part-start, text deltas addressed by part path, [DONE].
    let wire = concat!(
        "data: {\"type\":\"part-start\",\"part\":{\"type\":\"text\"},\"path\":[]}\n\n",
        "data: {\"type\":\"text-delta\",\"textDelta\":\"Served \",\"path\":[0]}\n\n",
        "data: {\"type\":\"text-delta\",\"textDelta\":\"by assistant-transport\",\"path\":[0]}\n\n",
        "data: [DONE]\n\n",
    );
    let runtime = LocalThreadRuntime::new(
        move |_options: ChatModelRunOptions| -> RunStream {
            // In an app, `bytes` is the body of your own HTTP request.
            let bytes = Reads(
                wire.as_bytes()
                    .chunks(24)
                    .map(<[u8]>::to_vec)
                    .collect::<Vec<_>>()
                    .into_iter(),
            );
            run_stream_from_sse(bytes, MessageAccumulator::default())
        },
        LocalRuntimeOptions::default(),
    );
    futures_executor::block_on(runtime.append(AppendMessage::user_text(None, "Hi"))?)?;
    println!("{}", runtime.messages()[1].text());
    println!("{:?}", runtime.messages()[1].status());
    Ok(())
}
```

Malformed or unknown events are dropped with a warning, as in assistant-stream. `update-state` carries state operations
(`set`, `append-text`) with string paths, which the accumulator applies to the message's `unstable_state` metadata. The
result is delivered to the next request as `state`. A runtime for assistant-transport with server-owned state is not
ported yet ([STATUS](../STATUS.md#not-yet-ported)).

## Requests: `ai-ui-data-stream`

`DataStreamAdapter<C: HttpClient>` is a `ChatModelAdapter`. For each model step it builds a JSON body, POSTs it, checks the
status, picks a decoder and returns the run stream. `request_body(options, extra)` is public if you need the same body in your
own transport. The body follows `useDataStreamRuntime`:

| Field | Content |
|---|---|
| `messages` | The selected path plus the assistant message being continued, converted to model-style `role` / `content` messages (`to_model_messages`). |
| `system` | The system prompt, when the app set one. |
| `tools` | Tool name to `{ parameters, description }`. Schemas only; there is no `execute` on the wire. |
| `unstable_assistantMessageId` | The id of the assistant message being filled. |
| `threadId` | The thread's remote id, when it has one. |
| `parentId` | The id of the message being answered (`null` for the first). |
| `runConfig` | The run configuration, normally `{}`. |
| `state` | The message's `unstable_state`, when not null. |
| call settings and config | Flattened into the top level (for example `temperature`, `modelName`). |
| `DataStreamOptions::body` | Extra fields from your callback, applied last, so they can override anything above. |

`Content-Type: application/json` is always set; other headers come from `DataStreamOptions::headers`, a closure called per
request so a refreshed token is picked up. The conversion drops what a model request cannot carry (reasoning, sources,
timestamps) and gives an old tool call that never completed the error result `{"error":"Tool call was not completed"}`, as upstream does, instead of leaving it dangling.

```rust
use ai_ui_data_stream::{DataStreamOptions, Protocol};
use std::rc::Rc;

fn main() {
    let token = "secret-token".to_owned();
    let mut options = DataStreamOptions::new("https://example.com/api/chat");
    // Evaluated for every request, so a refreshed token is picked up.
    options.headers = Some(Rc::new(move || {
        vec![("Authorization".into(), format!("Bearer {token}"))]
    }));
    // Extra top-level body fields, given the thread's remote id when it has one.
    options.body = Some(Rc::new(|thread_id| {
        let mut extra = serde_json::Map::new();
        extra.insert("model".into(), "my-model".into());
        extra.insert("threadId".into(), thread_id.into());
        extra
    }));
    // Skip header detection when you know what the server sends.
    options.protocol = Some(Protocol::UiMessageStream);
    println!("{}", options.api);
}
```

### Errors

| What happens | What the thread shows |
|---|---|
| The request cannot be made (DNS, refused, TLS) | The `HttpError` text; the message becomes `incomplete/error`. |
| A non-2xx status | `RunError::Failed` with the text `Status 503`. The response body is not read into the message. |
| The stream is malformed or cut inside an event | The run fails; text received before the failure stays on the message. |
| An `error` event inside the stream | The message becomes `incomplete/error` with `{ code, message }` (and `severity` when valid) as detail; content so far stays. |
| `cancel_run`, or dropping the run future | `incomplete/cancelled`, text so far kept. |

`ThreadView` turns the stored error into a readable `MessageState::error`, and the run future returns the same `RunError`
to your executor.

## HTTP clients: `ai-ui-http`

`HttpClient` has one method, `post(request, signal) -> ResponseFuture`. Two implementations ship:

- **`NativeClient`** (not `wasm32`): `ureq` with rustls. Each request runs on its own worker thread, and the body is passed
  through a bounded queue (64 reads of up to 16 KiB), so a slow consumer slows reading instead of buffering the response.
  It returns non-2xx responses to the adapter instead of failing, which is how `Status 503` is reported. Use
  `NativeClient::with_agent(agent)` for proxies or timeouts, and keep `http_status_as_error(false)` on that agent.
- **`FetchClient`** (`wasm32`): browser `fetch` with `credentials: same-origin`; set `include_credentials` for cross-origin
  cookies. Dropping the response body cancels the reader and aborts the request.

A client that owns your auth, retries or a different TLS stack only has to implement the trait.

### Cancellation

`runtime.cancel_run()` (or `ThreadView::stop`, or the composer's cancel) fires the run's `AbortSignal`. The signal reaches
the client as the second argument of `post`. The runtime stops polling the adapter's stream immediately and marks the message
`incomplete/cancelled`, so the UI reacts at once even if the server keeps the connection open. The native worker thread stops
at its next read and when the consumer is gone; the browser client aborts the fetch. The test
`cancelling_stops_a_stream_the_server_keeps_open` in `crates/ai-ui-http/tests/native.rs` checks that a cancelled run ends in
under three seconds against a server that holds the connection for five.

## Connect a real backend

Step by step, for an AI SDK backend.

**1. The server.** Any server that accepts a JSON POST and answers with an AI SDK stream works. With the AI SDK it is
a handler that passes the body to `streamText` and returns the UI message stream. This is the shape of the repository's interop
server, `benchmarks/ai-sdk-stream/interop-server.mjs` (shortened here):

```js
const body = JSON.parse(await readBody(req));
const result = streamText({
  model,
  messages: body.messages,   // model-style messages, passed straight through
  system: body.system,
  tools,                     // your tools, with their own execute
});
const response = result.toUIMessageStreamResponse();
// copy response.status, response.headers and the streamed body to the client
```

Two details from [PROTOCOL](../PROTOCOL.md#b3-request-body-contract) matter. `messages` are model messages, so do not
run `convertToModelMessages` over them (that is for a client that posts UI messages). And AI SDK 7 rejects system-role
messages inside `messages` by default, which is why the system prompt travels as a top-level field. `tools` in the body
are schemas only; the backend decides whether to use them.

**2. The client.** Build a runtime whose adapter is `DataStreamAdapter`. This compiles; set `AI_UI_API` to run it against a
live endpoint.

```rust
use ai_ui_core::{AppendMessage, LocalRuntimeOptions, LocalThreadRuntime};
use ai_ui_data_stream::{DataStreamAdapter, DataStreamOptions};
use ai_ui_http::NativeClient;
use std::rc::Rc;

/// A runtime whose model is an AI SDK endpoint.
fn chat_runtime(api: &str, token: String) -> LocalThreadRuntime {
    let mut options = DataStreamOptions::new(api);
    options.headers = Some(Rc::new(move || {
        vec![("Authorization".into(), format!("Bearer {token}"))]
    }));
    LocalThreadRuntime::new(
        DataStreamAdapter::new(NativeClient::new(), options),
        LocalRuntimeOptions::default(),
    )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set AI_UI_API to a running endpoint to try it; without it the example only builds the runtime.
    let Ok(api) = std::env::var("AI_UI_API") else {
        println!("set AI_UI_API=http://localhost:3000/api/chat to send a message");
        return Ok(());
    };
    let runtime = chat_runtime(&api, std::env::var("AI_UI_TOKEN").unwrap_or_default());
    let run = runtime.append(AppendMessage::user_text(None, "Say hello in five words."))?;
    match futures_executor::block_on(run) {
        Ok(()) => println!("{}", runtime.messages().last().unwrap().text()),
        Err(error) => println!("run failed: {error}"),
    }
    Ok(())
}
```

In the example app the same thing is `--api URL`, with `--header NAME:VALUE` for credentials.

**3. What goes out.** A POST to the URL with `Content-Type: application/json` and a body like this (printed by the loopback
example below for the message "Hello"):

```json
{
  "messages": [{ "content": [{ "text": "Hello", "type": "text" }], "role": "user" }],
  "parentId": "1a1cb95041d-1",
  "runConfig": {},
  "tools": {},
  "unstable_assistantMessageId": "1a1cb95041d-2"
}
```

The ids come from the runtime's `ids` option and are only examples.

**4. What must come back.** Status 2xx, then a streamed body. With the AI SDK, the response carries
`x-vercel-ai-ui-message-stream: v1` and a body of SSE events (`data: {...}` separated by a blank line). Without
that header the adapter assumes the UI message stream; set `DataStreamOptions::protocol` if you serve the data stream and
cannot send its `x-vercel-ai-data-stream: v1` header. Proxies must not buffer the response, or the reply arrives in one piece.

**5. Multi-step runs.** If the model calls a tool you registered with `execute`, the runtime runs it and sends a second request
whose `messages` end with the assistant's tool call and its result. A backend that already executed the tool itself (the
interop server does) sends the result in the same stream, and no second request is needed.

**6. Check it.** The repository's `crates/ai-ui-http/tests/interop_ai_sdk.rs` runs this exact path against a real AI SDK 7
server over a mock model, with reasoning, a streamed tool call executed by the server, a second step, and a follow-up turn.
It needs Node and `npm ci --ignore-scripts` in `benchmarks/ai-sdk-stream`. The result covers the AI SDK UI message stream
only; other backends, authentication and retries are not covered ([STATUS](../STATUS.md#interop-with-a-real-backend)).

## Testing without a network

Because the adapter only needs an `HttpClient`, a backend is easy to fake. This client records the request and answers with
a canned stream, split into 16-byte reads to exercise the decoder:

```rust
use ai_ui_core::{AbortSignal, AppendMessage, LocalRuntimeOptions, LocalThreadRuntime};
use ai_ui_data_stream::{
    ByteStream, DataStreamAdapter, DataStreamOptions, HttpClient, HttpRequest, HttpResponse,
    ResponseFuture,
};
use std::cell::RefCell;
use std::rc::Rc;

/// A client that never touches the network: it records the request and
/// answers with a canned AI SDK UI message stream, split into small reads.
#[derive(Clone, Default)]
struct Loopback {
    last_request: Rc<RefCell<Option<serde_json::Value>>>,
}

impl HttpClient for Loopback {
    fn post(&self, request: HttpRequest, _signal: AbortSignal) -> ResponseFuture {
        *self.last_request.borrow_mut() = serde_json::from_slice(&request.body).ok();
        let wire = concat!(
            "data: {\"type\":\"text-start\",\"id\":\"t\"}\n\n",
            "data: {\"type\":\"text-delta\",\"id\":\"t\",\"delta\":\"Hi there\"}\n\n",
            "data: {\"type\":\"finish\",\"finishReason\":\"stop\"}\n\n",
            "data: [DONE]\n\n",
        );
        let reads: Vec<Result<Vec<u8>, _>> =
            wire.as_bytes().chunks(16).map(|c| Ok(c.to_vec())).collect();
        let body: ByteStream = Box::pin(Reads(reads.into_iter()));
        Box::pin(std::future::ready(Ok(HttpResponse {
            status: 200,
            headers: vec![("x-vercel-ai-ui-message-stream".into(), "v1".into())],
            body,
        })))
    }
}

struct Reads(std::vec::IntoIter<Result<Vec<u8>, ai_ui_data_stream::HttpError>>);
impl futures_core::Stream for Reads {
    type Item = Result<Vec<u8>, ai_ui_data_stream::HttpError>;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        std::task::Poll::Ready(self.0.next())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Loopback::default();
    let seen = client.last_request.clone();
    let runtime = LocalThreadRuntime::new(
        DataStreamAdapter::new(
            client,
            DataStreamOptions::new("http://backend.test/api/chat"),
        ),
        LocalRuntimeOptions::default(),
    );
    futures_executor::block_on(runtime.append(AppendMessage::user_text(None, "Hello"))?)?;
    println!("{}", runtime.messages()[1].text());
    println!(
        "{}",
        serde_json::to_string_pretty(seen.borrow().as_ref().unwrap())?
    );
    Ok(())
}
```

It prints `Hi there` followed by the request body shown above. To test the real client, bind a `TcpListener` on
`127.0.0.1:0` and point `NativeClient` at it; the example below does so for the data stream protocol, and
`crates/ai-ui-http/tests/native.rs` also covers an error status and cancellation against a server that never closes:

```rust
use ai_ui_core::{AppendMessage, LocalRuntimeOptions, LocalThreadRuntime};
use ai_ui_data_stream::{DataStreamAdapter, DataStreamOptions};
use ai_ui_http::NativeClient;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // A one-request HTTP server on a loopback port.
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let url = format!("http://{}/api/chat", listener.local_addr()?);
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = v.trim().parse().unwrap();
            }
            if line == "\r\n" {
                break;
            }
        }
        reader.read_exact(&mut vec![0; length]).unwrap();
        let body = "0:\"Hello from a socket\"\nd:{\"finishReason\":\"stop\"}\n";
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nx-vercel-ai-data-stream: v1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
    });

    let runtime = LocalThreadRuntime::new(
        DataStreamAdapter::new(NativeClient::new(), DataStreamOptions::new(url)),
        LocalRuntimeOptions::default(),
    );
    futures_executor::block_on(runtime.append(AppendMessage::user_text(None, "Hi"))?)?;
    println!("{}", runtime.messages()[1].text());
    Ok(())
}
```

For decoder work, replay bytes in arbitrary partitions: `crates/ai-ui-stream/tests/fixtures/protocol-vectors.json` holds 96 wire
vectors from the protocol audit, and `crates/ai-ui-stream/tests/protocol_vectors.rs` and `ai_sdk_vectors.rs` run them. As of
2026-11-09 the status in [PROTOCOL](../PROTOCOL.md#conformance-status) was 22 of the 23 assistant-stream vectors run matching and 52 of
65 AI SDK vectors matching, with each of the 13 differences recorded with its reason. Those figures are a snapshot; the tests are
the current truth.

Next: [Building a UI with Slint](04-ui-slint.md) puts a thread on screen, and [Running on the web](05-web.md) covers the browser.
