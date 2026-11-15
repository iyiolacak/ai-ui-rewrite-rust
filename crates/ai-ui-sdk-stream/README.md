# ai-ui-sdk-stream

Renderer-independent Rust assistant-message runtime. The initial profile targets
the **AI SDK 7.0.166 UI-message SSE protocol**, usable by native and Wasm code.
It folds the stream the way the AI SDK's own reducer does (strict lifecycle errors, exact data and tool semantics), with
incremental updates instead of a full snapshot per event. It is the AI SDK profile; `ai-ui-stream` decodes the
same wire into assistant-ui's message model.

```rust
use ai_ui_sdk_stream::{Config, Runtime, Update};

let mut stream = Runtime::new("response-1", Config::default())?;
let ticket = stream.ticket(); // capture BEFORE starting asynchronous network work

stream.push(ticket, b"data: {\"type\":\"text-start\",\"id\":\"t\"}\n\n", |delivery| {
    for update in &delivery.applied.updates {
        if let Update::PartAdded { index } | Update::PartChanged { index } = update {
            if let Some(handle) = delivery.view.handle(*index) {
                // Queue a scoped dirty handle for the presentation adapter.
                // Read this part only; do not snapshot the whole conversation.
                let _part = delivery.view.part(handle);
            }
        }
        // A real host must also consume control/transient events, not just paint.
    }
})?;
stream.end(ticket)?;
let cumulative_message = stream.view().snapshot(); // explicit owned export
# Ok::<(), ai_ui_sdk_stream::StreamError>(())
```

The host owns networking, credentials, UI scheduling and transport cancellation.
The runtime owns framing and folding. `restart` creates a new ticket and old
callbacks cannot publish. Reset-invalidated handles cannot alias a reused index.

Full snapshots and partial tool-argument previews are explicit materialization
operations. Incoming text/argument deltas mutate only their indexed part and
buffer; no cumulative message clone is needed for ordinary change notifications.

Legacy data-stream, assistant-transport, request bodies, Markdown and Slint chat
components remain follow-up work. Defaults impose explicit configurable event,
run-byte and part limits. The `v1` response header is not an SDK-major selector.

[Architecture, compatibility policy and tests](../../docs/AI-SDK-STREAM.md)
describe the pinned source behavior, lazy preview contract and remaining scope.
