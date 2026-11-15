# Running in the browser (WebAssembly)

Most crates build for `wasm32-unknown-unknown`. Add the target once with `rustup target add wasm32-unknown-unknown`.

## Which crates build for wasm32

| Crate | wasm32 | Notes |
|---|---|---|
| `ai-ui-core`, `ai-ui-stream`, `ai-ui-data-stream`, `ai-ui-state`, `ai-ui-markdown`, `ai-ui-sdk-stream` | yes | No I/O, clock or thread dependencies. |
| `ai-ui-http` | yes | `FetchClient` (browser `fetch`) replaces `NativeClient` (ureq) on wasm32. |
| `ai-ui-clipboard` | yes | `navigator.clipboard` on `wasm32-unknown-unknown`; arboard on desktop. |
| `ai-ui-markdown-slint`, `ai-ui-slint` | through the example only | Slint crates, `publish = false`; built and run via `examples/thread`. |
| `examples/thread` | yes, as the demo | Slint with the femtovg renderer on WebGL, winit on a canvas. |

Check any of them with `cargo check --target wasm32-unknown-unknown -p <crate>`; `scripts/gate.sh` builds the non-Slint ones.

## The web demo

`scripts/build-web-demo.sh` builds `examples/thread` for wasm32 (release, `--features gpu-window`), runs `wasm-bindgen --target web --no-typescript` and writes:

- `dist-web/ai-ui-web-demo.tar.gz`: `ai_ui_example_thread.js`, `ai_ui_example_thread_bg.wasm` and `pixel-snap.js`
- `dist-web/ai-ui-web-demo.sha256`

`wasm-bindgen` must match the version in `Cargo.lock` (`0.2.129`); point `WASM_BINDGEN=/path/to/wasm-bindgen` at it if it is not on `PATH`.

To host it, serve the files next to a page that has a `<canvas id="canvas">` inside a sized container, and load the glue. `pixel-snap.js` lays the canvas out on whole device pixels; call it before `init()` (see "Sharp text in Firefox" below):

```html
<div id="stage" style="height: 640px"><canvas id="canvas" tabindex="0"></canvas></div>
<script type="module">
  import { pixelSnap } from "./pixel-snap.js";
  import init from "./ai_ui_example_thread.js";
  pixelSnap(document.getElementById("canvas"));
  await init();
</script>
```

### Sharp text in Firefox

The canvas backing store is a whole number of device pixels (Slint sizes it from winit's `devicePixelContentBoxSize`). If the canvas *box* is fractional in device pixels, which a centred container, a `vh` height or a 1.25x/1.5x display easily produces, Firefox composites it with bilinear resampling and every glyph softens; Chrome snaps the box and hides the problem. Measured 2026-11-13 in Firefox 159 with a `min(88.37vh, 900px)` by `min(100% - 2.3rem, 1151.7px)` container at 1.25x: layout rect 1071x707.667 CSS px = 1338.75x884.58 device px against a 1339x885 backing store, so a stretch on both axes. `pixel-snap.js` insets the canvas to whole device pixels, at a whole-pixel offset, through CSS custom properties and one injected `!important` rule (winit rewrites the canvas's inline `width`/`height`, so inline sizes do not stick). The same page then measures 21,27 / 1337x883 device px against a 1337x883 backing store at 1.25x, 25,32 / 1605x1061 at 1.5x and 18,24 / 1167x771 at 12/11x, in Firefox and in Chrome. It loses under two device pixels per axis. This was verified on the geometry; headless Firefox cannot screenshot a WebGL canvas, so the on-screen sharpness was not compared pixel by pixel. The maths is covered by `node --test examples/thread/web/pixel-snap.test.mjs`.

The app takes its options from the page's query string (`?history=1000&light` is `--history 1000 --light`) or from `window.__AUI_ARGS`. It publishes frame counts and milestones on `window.__aui` and routes panics to the console.

### Size of the demo build

Measured 2026-11-11 on the release build behind release `web-demo-v0.1.0` (`scripts/build-web-demo.sh`, not processed
with wasm-opt): `ai_ui_example_thread_bg.wasm` is 9,500,548 bytes (9.06 MiB); the `.js` glue is 92,655 bytes; the
release tarball is 2,973,264 bytes. Compressed with gzip -9 the wasm is about 2.8 MiB; with brotli level 11 it is
2,005,710 bytes (1.91 MiB). A size-optimised build (`opt-level = "s"`) was 7,572,692 bytes raw but did not reach a first
frame in headless Chromium, so it is not shipped.

## HTTP and clipboard

`ai-ui-http::FetchClient` implements the `HttpClient` trait that `ai-ui-data-stream` uses, over `fetch` with `credentials: same-origin` (set `include_credentials` for cross-origin cookies). Dropping the response body cancels the reader and aborts the request. It is exercised in headless Chromium by `crates/ai-ui-http/tests/browser/run.py`.

`ai-ui-clipboard` reports the real outcome of a copy. Call `Clipboard::copy` synchronously inside the user gesture (a click handler); browsers reject writes outside one, and the API needs a secure context (HTTPS or localhost). One write may be pending at a time (`CopyError::Busy`).

## Limits

- No Markdown worker threads: native builds format Markdown off the UI thread; the browser build runs a wasm-only inline backend on the UI thread.
- The demo runs the built-in demo model only. `--api` (an AI SDK endpoint) is not available in the example's browser build; use `FetchClient` with `ai-ui-data-stream` in your own app.
- HiDPI: the canvas backing store follows `devicePixelRatio` (winit's `ResizeObserver` reports `devicePixelContentBoxSize` and Slint sizes the canvas to it). Checked 2026-11-13 in Chrome at real scale factors 1.25, 1.5 and 2 (`--force-device-scale-factor`): backing stores 1213x787, 1452x941 and 1936x1255 for CSS widths of about 970, 968 and 968. Playwright's `deviceScaleFactor` emulation does not change `devicePixelContentBoxSize`, so headless runs that use it show a 1x canvas that a real display does not. Without `pixel-snap.js`, a fractional canvas box still resamples in Firefox (above). Even when snapped, small text, code blocks especially (unhinted grayscale glyphs, thin 13px mono), can look softer than DOM text on 1x displays. Safari lacks `devicePixelContentBoxSize` and was not tested.
