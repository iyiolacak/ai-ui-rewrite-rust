# ai-ui-data-stream

Rust port of `@assistant-ui/react-data-stream`: a `ChatModelAdapter` for AI SDK backends, with request conversion (`toModelMessages`), protocol detection and error statuses.

```rust,ignore
let adapter = DataStreamAdapter::new(my_http_client, DataStreamOptions::new("https://example.com/api/chat"));
let runtime = LocalThreadRuntime::new(adapter, LocalRuntimeOptions::default());
```

Bring your own `HttpClient`, or use [`ai-ui-http`](../ai-ui-http).

Part of [assistant-ui for Rust](../../README.md), a port of [assistant-ui](https://github.com/assistant-ui/assistant-ui) (MIT). Not affiliated with the assistant-ui project. Status and gaps: [docs/STATUS.md](../../docs/STATUS.md).
