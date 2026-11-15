# ai-ui-stream

Rust port of `assistant-stream`: typed stream chunks, the message accumulator that folds them into an assistant message, incremental partial JSON, state operations, the `assistant-transport` SSE serialization, and decoders for the AI SDK UI message stream and data stream protocols.

```rust,ignore
// Wrap a response byte stream; each wake-up yields the cumulative message.
let results = ai_ui_stream::run_stream_from_sse(bytes, MessageAccumulator::default());
```

Slices that are already available are folded into one result, so a burst costs one update, not one per slice.

Part of [assistant-ui for Rust](../../README.md), a port of [assistant-ui](https://github.com/assistant-ui/assistant-ui) (MIT). Not affiliated with the assistant-ui project. Status and gaps: [docs/STATUS.md](../../docs/STATUS.md).
