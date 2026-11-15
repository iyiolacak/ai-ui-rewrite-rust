# WebAssembly and the browser

Source: [Running on the web](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/docs/guide/05-web.md) and [WEB](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/docs/WEB.md).

## Which crates build for wasm32

Add the target once: `rustup target add wasm32-unknown-unknown`. Check a crate with `cargo check --target wasm32-unknown-unknown -p <crate>`.

| Crate | In the browser |
|---|---|
| `ai-ui-core`, `ai-ui-stream`, `ai-ui-data-stream`, `ai-ui-sdk-stream`, `ai-ui-state`, `ai-ui-markdown` | Yes. No I/O, clock or thread dependencies. |
| `ai-ui-http` | Yes. `FetchClient` (browser `fetch`) replaces `NativeClient` (ureq). |
| `ai-ui-clipboard` | Yes. `navigator.clipboard` on `wasm32-unknown-unknown`; arboard on desktop. |
| `ai-ui-markdown-slint`, `ai-ui-slint` | Through an app built like `examples/thread` only. |
| `examples/thread` | Yes, as the demo: Slint with the femtovg renderer on WebGL, winit on a `<canvas>`. |

The core has no ambient clock or randomness; ids and timestamps come from `LocalRuntimeOptions` (`clock`, `ids`) because `SystemTime` does not exist on `wasm32-unknown-unknown`.

## Backend calls from the browser

Use `FetchClient` with `DataStreamAdapter`. It uses `credentials: same-origin`; set `include_credentials` for cross-origin cookies. Dropping the response body cancels the reader and aborts the request. Code: [backend.md](backend.md). The server must allow the browser's request (CORS, if cross-origin).

Pick the client per target with `cfg(target_arch = "wasm32")`; `NativeClient` does not exist on wasm32 and `FetchClient` exists only there.

Spawn run futures with `wasm-bindgen-futures`.

## Clipboard

Call `Clipboard::copy` synchronously inside the user gesture (a click handler); browsers reject writes outside one. The API needs a secure context (HTTPS or localhost). One write may be pending at a time; a second returns `CopyError::Busy`.

## Building and embedding the demo

`scripts/build-web-demo.sh` builds `examples/thread` for wasm32 in release with `--features gpu-window`, runs `wasm-bindgen --target web --no-typescript`, and writes `dist-web/ai-ui-web-demo.tar.gz` (`ai_ui_example_thread.js`, `ai_ui_example_thread_bg.wasm`, `pixel-snap.js`).

- The `wasm-bindgen` command-line tool must match the version in `Cargo.lock` exactly (0.2.129 at the time of writing). Otherwise point `WASM_BINDGEN=/path/to/wasm-bindgen` at the right one.
- Embed with a `<canvas id="canvas" tabindex="0">` inside a container that has a definite size. Call `pixelSnap(canvas)` from `pixel-snap.js` before `init()`. Do not size the canvas itself.
- Options come from the query string (`?history=1000&light`) or `window.__AUI_ARGS`.
- Serve the `.wasm` compressed. The size of the demo build is recorded in WEB.md; read it there rather than assuming.

## Limits

- No Markdown worker threads: the browser build formats Markdown inline on the UI thread under a 4 ms budget per poll.
- The example's browser build runs only its demo model; `--api` is not available there.
- WebGL is expected. Check for it before loading.
- Small text, especially code, can look softer than DOM text on 1x displays. A fractional canvas box softens text in Firefox unless the page uses `pixel-snap.js`. Safari was not tested.
- Frame rates measured in headless Chromium use software WebGL. Treat them as a floor.
