# Connecting an AI SDK backend

Source: [Streaming and backends](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/docs/guide/03-streaming-and-backends.md) and [PROTOCOL](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/docs/PROTOCOL.md). Add `ai-ui-core`, `ai-ui-data-stream` and `ai-ui-http`.

## The pipeline

1. Transport: `HttpClient` (`ai-ui-data-stream`) sends a request and returns status, headers and a `ByteStream`. Reads can split anywhere, even inside a UTF-8 character.
2. Decoder: `StreamDecoder` (`ai-ui-stream`) turns bytes into `Chunk`s.
3. Accumulator: `MessageAccumulator` folds chunks into a cumulative `ChatModelRunResult`.
4. Run stream: `run_stream` ties them together and yields results to the runtime.

`DataStreamAdapter` does all four for AI SDK backends.

## Server shape

Any server that accepts a JSON POST and answers with an AI SDK stream works. With the AI SDK it is a handler like this (shortened from the repository's interop server):

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

- `messages` are model messages. Do not run `convertToModelMessages` over them; that is for a client that posts UI messages.
- AI SDK 7 rejects system-role messages inside `messages` by default, so the system prompt travels as the top-level `system` field.
- `tools` in the body are schemas only. The backend decides whether to use them.
- Proxies must not buffer the response, or the reply arrives in one piece.

## Client

Build a runtime whose adapter is `DataStreamAdapter`. This compiles; set `AI_UI_API` to run it against a live endpoint.

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

In an app, spawn the run on the project's executor instead of `block_on` (see [runtime.md](runtime.md)). In the browser build use `FetchClient` instead of `NativeClient`:

```rust,ignore
use ai_ui_core::{LocalRuntimeOptions, LocalThreadRuntime};
use ai_ui_data_stream::{DataStreamAdapter, DataStreamOptions};
use ai_ui_http::FetchClient;

let adapter = DataStreamAdapter::new(
    FetchClient::default(),
    DataStreamOptions::new("/api/chat"),
);
let runtime = LocalThreadRuntime::new(adapter, LocalRuntimeOptions::default());
```

## Headers, body and protocol options

`DataStreamOptions::new(api)` has public fields `headers`, `body` and `protocol`.

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

`Content-Type: application/json` is always set. Fields from `body` are applied last and can override anything. The request body carries `messages`, `system` (when set), `tools`, `unstable_assistantMessageId`, `threadId`, `parentId`, `runConfig`, `state`, and flattened call settings. `ai_ui_data_stream::request_body` is public if you need the same body in your own transport. Field table: the Streaming guide, section "Requests".

## What must come back

Status 2xx and a streamed body. With the AI SDK the response carries `x-vercel-ai-ui-message-stream: v1` and SSE events (`data: {...}` separated by a blank line). Without that header the adapter assumes the UI message stream. If you serve the data stream and cannot send `x-vercel-ai-data-stream: v1`, set `DataStreamOptions::protocol`.

## The three wire formats

| Format | Marker | Framing | Decoder | Selected by |
|---|---|---|---|---|
| AI SDK UI message stream (v5 to v7) | `x-vercel-ai-ui-message-stream: v1` | SSE, one JSON object per `data:` event, optional `[DONE]` | `UiMessageStreamDecoder` | `DataStreamAdapter` (default) |
| AI SDK data stream (v4) with assistant-ui's `aui-*` extensions | `x-vercel-ai-data-stream: v1` | `CODE:JSON` lines, no SSE | `DataStreamDecoder` | `DataStreamAdapter`, by header |
| assistant-transport (assistant-stream) | none | SSE, `data: <JSON>`, ends `data: [DONE]` | `SseDecoder`, `run_stream_from_sse` | your own adapter |

The UI message stream header is the same for AI SDK v5, v6 and v7, so it does not tell you the SDK version. `DataStreamAdapter` does not select assistant-transport: wrap your response bytes with `run_stream_from_sse(bytes, MessageAccumulator::default())` inside your own `ChatModelAdapter`. A complete example is in the Streaming guide, section "assistant-transport". Assistant-transport as a runtime with server-owned state is not ported.

`ai-ui-stream` is lenient: it keeps content that arrived and records warnings. The strict AI SDK 7 reducer is a separate crate, `ai-ui-sdk-stream`; for valid streams both give the same content. Byte-level detail: [PROTOCOL](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/docs/PROTOCOL.md) and [AI-SDK-STREAM](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/docs/AI-SDK-STREAM.md).

## Errors

| What happens | What the thread shows |
|---|---|
| The request cannot be made (DNS, refused, TLS) | The `HttpError` text; the message becomes `incomplete/error`. |
| A non-2xx status | `RunError::Failed` with the text `Status 503`. The response body is not read into the message. |
| The stream is malformed or cut inside an event | The run fails; text received before the failure stays on the message. |
| An `error` event inside the stream | The message becomes `incomplete/error` with `{ code, message }` as detail; content so far stays. |
| `cancel_run`, or dropping the run future | `incomplete/cancelled`, text so far kept. |

`ThreadView` turns the stored error into a readable `MessageState::error`, and the run future returns the same `RunError`.

## Cancellation

`runtime.cancel_run()`, `ThreadView::stop` or the composer's cancel fires the run's `AbortSignal`, which reaches the client as the second argument of `post`. The runtime stops polling the stream at once and marks the message `incomplete/cancelled`. The native worker thread stops at its next read; the browser client aborts the fetch.

## Testing without a network

Implement `HttpClient` with a loopback that records the request and returns a canned stream. A full compiling version (a `Loopback` client with 16-byte reads) is in the Streaming guide, section "Testing without a network". For the real client, bind a `TcpListener` on `127.0.0.1:0` and point `NativeClient` at it; the same section has that example, and `crates/ai-ui-http/tests/native.rs` covers an error status and cancellation.

## Multi-step runs and tools

If the model calls a tool you registered with `execute`, the runtime runs it and sends a second request whose `messages` end with the assistant's tool call and its result. A backend that ran the tool itself sends the result in the same stream, and no second request is needed. A `Tool` with `requires_approval` pauses the run until the host calls `respond_to_tool_approval` (`ThreadView::respond_to_approval`). Tools run sequentially.

## Checking the connection

`crates/ai-ui-http/tests/interop_ai_sdk.rs` runs this path against a real AI SDK 7 server over a mock model. It needs Node and `npm ci --ignore-scripts` in `benchmarks/ai-sdk-stream`. It covers the UI message stream only. Other backends, authentication and retries are not covered ([STATUS](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/docs/STATUS.md)).

The example app can target an endpoint directly: `cargo run -p ai-ui-example-thread --release -- --api http://localhost:3000/api/chat`, with `--header NAME:VALUE` for credentials (native build only).
