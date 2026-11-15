# The ten crates

All are version 0.1 on crates.io. Full guide: [Crate reference](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/docs/guide/06-crates.md). API docs: docs.rs/<crate>.

None of the library crates defines Cargo features; platform differences are `cfg` switches. (The example app has one, `gpu-window`.)

| Crate | MSRV | wasm32 | Upstream counterpart |
|---|---|---|---|
| `ai-ui-core` | 1.85 | yes | `@assistant-ui/core` |
| `ai-ui-stream` | 1.85 | yes | `assistant-stream` |
| `ai-ui-data-stream` | 1.85 | yes | `@assistant-ui/react-data-stream` |
| `ai-ui-sdk-stream` | 1.85 | yes | AI SDK `readUIMessageStream` |
| `ai-ui-http` | 1.85 | yes (`FetchClient`) | `fetch` |
| `ai-ui-state` | 1.85 | yes | scoped state (`MessageState` and friends) |
| `ai-ui-markdown` | 1.88 | yes, without the thread-based modules | `@assistant-ui/markdown` |
| `ai-ui-clipboard` | 1.86 | yes (`navigator.clipboard`) | browser clipboard |
| `ai-ui-markdown-slint` | 1.92 | only through the example app | react-markdown renderers |
| `ai-ui-slint` | 1.92 | only through the example app | `@assistant-ui/react` UI |

MSRV values come from each crate's `Cargo.toml` (`rust-version`; the workspace default is 1.85).

## ai-ui-core

Add: always. The message model (upstream JSON shape), the branching repository, the model adapter contract and the runtimes. No UI, I/O, clock or randomness.

- `ThreadMessage` and part types (`TextPart`, `PartStatus`, `AssistantMessagePart`).
- `MessageRepository`: branch tree with a selected path.
- `ChatModelAdapter` (trait), `ChatModelRunResult` (`text`, `content`), `RunStream`, `RunError`, `adapter::once`, `adapter::from_results`.
- `ThreadRuntime` (trait), `RunFuture`, `LocalThreadRuntime`, `LocalRuntimeOptions`, `ExternalStoreRuntime` with `ExternalStore`.
- `AppendMessage::user_text`.
- `ThreadList`, `ThreadListAdapter`, `Spawner`.
- `Tool`, `ToolRegistry`; `SmoothText`; `AbortController`, `AbortSignal`; `Subscription` (drop to unsubscribe).

Use `LocalThreadRuntime` when the library drives the model. Use `ExternalStoreRuntime` when your app owns messages and persistence.

## ai-ui-stream

Add: custom wire-protocol work or assistant-transport SSE. The `assistant-stream` port.

- `MessageAccumulator`: folds chunks into an assistant message.
- `StreamDecoder` (trait) with `SseDecoder` (assistant-transport), `DataStreamDecoder` (AI SDK data stream), `UiMessageStreamDecoder` (AI SDK UI message stream).
- `run_stream`, `run_stream_from_sse`: response bytes to a `RunStream`.
- Module `partial_json` for tool arguments that arrive in fragments.

Lenient by design: it keeps content that arrived and records warnings. For the strict reducer see `ai-ui-sdk-stream`.

## ai-ui-data-stream

Add: with `ai-ui-http`, to talk to an AI SDK backend. A `ChatModelAdapter` that builds the request body, POSTs it, picks the decoder from the response headers and returns the run stream.

- `DataStreamAdapter`, `DataStreamOptions` (fields `api`, `headers`, `body`, `protocol`), `Protocol`.
- `HttpClient` (trait), `HttpRequest`, `HttpResponse`, `HttpError`, `ByteStream`, `ResponseFuture`.
- `request_body`, `to_model_messages`.

Details: [backend.md](backend.md).

## ai-ui-sdk-stream

Add: only if you need an exact AI SDK 7 UI message stream reducer on its own. Licence "MIT AND Apache-2.0". Depends only on `serde_json`. Checked against the published JavaScript package. Do not merge its semantics with `ai-ui-stream`.

- `Runtime` (`new`, `ticket`, `push`, `end`, `cancel`, `restart`, `view`), `RunTicket`, `Config`, `Limits`, `FeedOutcome`, `MessageView`, `Delivery`.

Reference: [AI-SDK-STREAM](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/docs/AI-SDK-STREAM.md).

## ai-ui-http

Add: with `ai-ui-data-stream`, for a ready-made HTTP client.

- `NativeClient` (not wasm32): `ureq` with rustls. `NativeClient::new()`, `NativeClient::with_agent(agent)` for proxies or timeouts.
- `FetchClient` (wasm32 only): browser `fetch`. Field `include_credentials` for cross-origin cookies.

## ai-ui-state

Add: for any UI. Toolkit-independent projection and actions.

- `ThreadView`: `new(runtime, ViewOptions)`, `for_list`, `subscribe`, `state`, `rebind`, `tick`, `needs_frame`.
- Actions: `send`, `stop`, `reload`, `switch_branch`, `begin_edit`, `copy_text`, `mark_copied`, `toggle_part`, `respond_to_approval`, `add_tool_result`, `submit_feedback`, `set_tool_presenter`.
- `ThreadState`, `MessageState`, `PartKind`.

Walkthrough: [runtime.md](runtime.md).

## ai-ui-markdown

Add: Markdown for streamed replies in your own renderer. Bounded, safe parsing; raw HTML, images and unsupported URLs become inert text.

- `parse(source) -> Result<Document, ParseError>`.
- `presentation::project(source)`: UI-independent render blocks.
- `split::StreamSplit`, `split::top_level_blocks`: settled blocks versus the live tail.
- Native only: `pool::CoordinatorPool` (`default_workers()`, `MAX_WORKERS` is 8).

Hard limits fail with an error; they never truncate.

## ai-ui-clipboard

Add: when the host must know whether a copy worked. `Clipboard::new()`, `copy(text, completion)`, `read_text`, `shutdown`; `CopyError`; `MAX_COPY_BYTES` (1 MiB), `MAX_READ_BYTES` (4 MiB). On native the completion runs on a worker thread, so hop back to the UI thread. In the browser call `copy` synchronously inside the user gesture.

## ai-ui-markdown-slint

Add: with the Slint UI. Maps portable Markdown blocks to the record types your own `.slint` generates and keeps the UI-thread cache.

- `Factory` (trait you implement), `cache::Cache`, `streaming::StreamingCache` (`observe_one`, `poll`, `take_changed`, `view`, `retain`, `has_pending`).

The compiling reference is `examples/thread/src/markdown.rs`.

## ai-ui-slint

Add: as a `[build-dependencies]` entry for the Slint thread UI. The Rust API is one function, `ai_ui_slint::ui_dir()`. The rest is `.slint` source: `AssistantThread`, `AssistantComposer`, `AssistantThreadList`, `AssistantMarkdown`, `AssistantTheme`, `ChatRow` and more, all exported from `lib.slint`. See [slint-ui.md](slint-ui.md).
