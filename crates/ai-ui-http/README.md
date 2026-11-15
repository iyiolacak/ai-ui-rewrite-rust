# ai-ui-http

HTTP clients for `ai-ui-data-stream`: a native client (ureq with rustls, one worker thread per request) and, on `wasm32`, a browser `fetch` client.

```rust,ignore
let client = ai_ui_http::NativeClient::new();
```

The native client is tested over loopback sockets. The browser client is tested in headless Chromium through the real adapter and runtime (`tests/browser/run.py`; needs `wasm-bindgen` 0.2.129 and Playwright, see the script).

Part of [assistant-ui for Rust](../../README.md), a port of [assistant-ui](https://github.com/assistant-ui/assistant-ui) (MIT). Not affiliated with the assistant-ui project. Status and gaps: [docs/STATUS.md](../../docs/STATUS.md).
