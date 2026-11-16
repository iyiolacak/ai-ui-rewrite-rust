---
title: Running on the web
description: Build the thread UI for WebAssembly, embed the demo in a page, and know what works in the browser and what does not.
order: 5
section: Build
---

# Running on the web

**What this page gives you.** This page covers the WebAssembly build: which crates run in the browser, the build command, embedding the demo in a page, the size of the output, and the known limits (WebGL required, small-text softness).

The runtime crates have no I/O, clock or thread dependencies, so they compile for `wasm32-unknown-unknown` unchanged. The Slint example also builds for the browser: the same `examples/thread` application, drawn with Slint's femtovg renderer on WebGL, with winit driving a `<canvas>`. The reference is [WEB.md](../WEB.md); this page explains it and adds the embedding details.

## What runs in the browser

| Piece | In the browser |
|---|---|
| `ai-ui-core`, `ai-ui-stream`, `ai-ui-sdk-stream`, `ai-ui-data-stream`, `ai-ui-state`, `ai-ui-markdown` | Yes. No I/O, clock or thread dependencies. |
| `ai-ui-http` | Yes. `FetchClient` (browser `fetch`) replaces `NativeClient` (ureq) on wasm32. |
| `ai-ui-clipboard` | Yes. `navigator.clipboard` on `wasm32-unknown-unknown`; arboard on desktop. |
| `ai-ui-markdown-slint`, `ai-ui-slint` | Through the example only. They are published Slint crates (crates.io 0.1.0); for the web they are built and run via `examples/thread`. |
| `examples/thread` | Yes, as the demo. |

Check any single crate with:

```sh
rustup target add wasm32-unknown-unknown
cargo check --target wasm32-unknown-unknown -p ai-ui-core
```

`scripts/gate.sh` builds the non-Slint crates for wasm32 as part of the full gate (`--quick` skips the wasm builds).

## Building the demo

`scripts/build-web-demo.sh` does the whole build. From a checkout:

```sh
scripts/build-web-demo.sh
```

It reads the `wasm-bindgen` version from `Cargo.lock` (`0.2.129`), finds the binary on `PATH` or in `WASM_BINDGEN`, and fails with a clear message if the two versions differ. Then it:

1. Runs `cargo build --locked --release --target wasm32-unknown-unknown -p ai-ui-example-thread --features gpu-window`. The `gpu-window` feature makes the Slint build embed fonts and images as files, which the GPU renderer wants, instead of as pixels for the software renderer.
2. Runs `wasm-bindgen --target web --no-typescript --out-name ai_ui_example_thread` over the `.wasm`.
3. Writes `dist-web/ai-ui-web-demo.tar.gz` containing `ai_ui_example_thread.js`, `ai_ui_example_thread_bg.wasm` and `pixel-snap.js`, and `dist-web/ai-ui-web-demo.sha256`.

If `wasm-bindgen` is not on `PATH`:

```sh
WASM_BINDGEN=/path/to/wasm-bindgen scripts/build-web-demo.sh
```

The version must match exactly because the glue code and the `.wasm` are generated as a pair; a mismatch fails at load time with an unhelpful error, which is why the script checks first. Unpack the tarball next to your page:

```sh
mkdir -p public/wasm
tar -xzf dist-web/ai-ui-web-demo.tar.gz -C public/wasm
```

## Embedding the demo in a page

Serve the files and a page with a `<canvas>` inside a container that has a definite size. Slint's winit backend looks for a canvas with the id `canvas`, so give it that id. `pixel-snap.js` (in the tarball) lays the canvas out on whole device pixels; call it before `init()`:

```html
<div id="stage" style="width: 100%; height: 640px">
  <canvas id="canvas" tabindex="0"></canvas>
</div>
<script type="module">
  import { pixelSnap } from "./wasm/pixel-snap.js";
  import init from "./wasm/ai_ui_example_thread.js";
  pixelSnap(document.getElementById("canvas"));
  await init();
</script>
```

`tabindex="0"` lets the canvas take keyboard focus. The canvas fills its container's padding box, so size the container with CSS. Do not size the canvas itself; the helper does, and winit rewrites its inline size.

**Why the helper.** The canvas backing store is a whole number of device pixels, but a container that is centred, sized in `vh`, or shown on a 1.25x or 1.5x display makes the canvas box fractional. Firefox then resamples the canvas bilinearly and all text softens; Chrome snaps the box and hides it. The helper insets the canvas to whole device pixels so the two match 1:1. Details and measurements are in [WEB.md](../WEB.md).

### Passing options

The app reads the same options as the desktop command line, in either of two places, with the first taking priority:

- `window.__AUI_ARGS`, set before `init()` runs: `window.__AUI_ARGS = "history=1000&light"`.
- The page's query string: `?history=1000&light`.

Each `key=value` pair becomes `--key value`, and a bare `key` becomes `--key`. So `history=1000&light` is `--history 1000 --light`. Values are URL-decoded. Useful options in the browser:

| Option | Effect |
|---|---|
| `history=N` | Start with a generated thread of N messages. |
| `light`, `theme=system\|light\|dark` | Theme. |
| `text-scale=1.15`, `reading-width=wide` | Text size and reading width for this session. |
| `long-reply-kb=9`, `token-delay-ms=4` | Shape of the demo model's replies. |

`--api` (an AI SDK endpoint) is not available in the browser build of the example. There is no settings file in the browser build, so settings last for the page session.

### Milestones on `window.__aui`

The demo publishes counters on `window.__aui`, which a page can poll:

- `frames`: number of frames drawn so far.
- `started`, `firstFrame`, `lastFrame`: `performance.now()` timestamps.
- `importMs` and `messages`, when a mock thread was loaded with `thread=URL`.

Panics are routed to `console.error`. `firstFrame` is the right signal for hiding a loading cover.

### Load on click

The WebAssembly file is large for a web page, so the landing site does not load it until the visitor asks. The pattern is small:

```ts
function hasWebGL(): boolean {
  try {
    const c = document.createElement("canvas");
    return !!(c.getContext("webgl2") || c.getContext("webgl"));
  } catch {
    return false;
  }
}

async function load() {
  if (!hasWebGL()) { showMessage("This browser has no WebGL."); return; }
  window.__AUI_ARGS = "history=10000";
  const mod = await import(new URL("wasm/ai_ui_example_thread.js", document.baseURI).href);
  try {
    await mod.default();
  } catch (e) {
    // winit unwinds out of run() as control flow; anything else is a real failure.
    if (!/control flow/.test(String(e))) { showMessage(`Could not start: ${e}`); return; }
  }
  const wait = () => window.__aui?.firstFrame ? hideCover() : setTimeout(wait, 50);
  wait();
}
button.addEventListener("click", load);
```

Two details come from the landing site's implementation. First, hovering or focusing the button can add `<link rel="prefetch">` tags for the glue and the `.wasm`, so the files are in the cache when the click comes, and nothing is compiled until then. Second, winit's `run()` throws a "control flow" exception on purpose to hand control back to the browser's event loop. Treat that one as success.

## Size

From the landing site's measurement of the release build (`lto`, `opt-level` 3), taken 2026-11-11: the `.wasm` file is 9,500,548 bytes raw (9.06 MiB), 2.81 MiB with `gzip -9` and 1.91 MiB with brotli at quality 11. Serve it with gzip or brotli compression.

These three numbers are not in `docs/RESULTS.md` or `docs/WEB.md` on this branch. They are in `site/data/benchmarks.json` on the landing branch, which is the site's single source for figures. Re-measure after any dependency change; the file is dominated by Slint, femtovg and the embedded fonts.

## HTTP and clipboard in your own app

The example cannot call a backend in the browser, but your app can. `ai-ui-http::FetchClient` implements the `HttpClient` trait that `ai-ui-data-stream` uses, over `fetch` with `credentials: same-origin`. Set `include_credentials` for cross-origin cookies. Dropping the response body cancels the reader and aborts the request.

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

The fetch client is exercised in headless Chromium by `crates/ai-ui-http/tests/browser/run.py`. It needs `wasm-bindgen` 0.2.129 (`WASM_BINDGEN`) and Playwright (`AI_UI_PLAYWRIGHT_MODULE`), and opens no window. `crates/ai-ui-clipboard/tests/browser/run.py` does the same for the clipboard.

`ai-ui-clipboard` reports the real outcome of a copy. Call `Clipboard::copy` synchronously inside the user gesture, for example a click handler, because browsers reject writes outside one. The API needs a secure context (HTTPS or `localhost`). One write may be pending at a time; a second returns `CopyError::Busy`.

## Limits

- **No Markdown worker threads.** Native builds format Markdown on a pool of worker threads. The browser build runs a wasm-only inline backend on the UI thread, with a 4 ms budget per poll and only the newest source per key. Long replies cost UI-thread time that native builds spend elsewhere.
- **Demo model only.** The example's browser build runs the built-in demo model. Use `FetchClient` with `ai-ui-data-stream` in your own app to reach a real backend.
- **HiDPI softness.** The canvas backing store follows `devicePixelRatio` (checked in Chrome at real scale factors 1.25, 1.5 and 2, see [WEB.md](../WEB.md)); Playwright's `deviceScaleFactor` emulation does not, so headless screenshots taken with it show a 1x canvas. A fractional canvas box softens text in Firefox unless you use `pixel-snap.js` (above). Small text, code blocks especially, can still look softer than DOM text on 1x displays because femtovg draws unhinted grayscale glyphs. Safari was not tested.
- **WebGL is expected.** The demo is built for the femtovg WebGL renderer, and the landing page refuses to start it in a browser without WebGL, hence the check above.
- **Frame rates in headless tests are a floor.** Headless Chromium uses software WebGL (SwiftShader), so a frame rate measured there is the GPU emulated on a busy CPU. Treat headless numbers as a floor, not as what a browser with a GPU gives, and do not compare them with native numbers.

## Cloudflare Pages notes

The landing site (`site/` on the landing branch, built with Astro) is configured for Cloudflare Pages; nothing is deployed from the repository.

| Setting | Value |
|---|---|
| Root directory | `site` |
| Build command | `npm ci && npm run build` |
| Build output directory | `dist` |

The Pages build image has no Rust toolchain. Build the wasm files first (`npm run wasm` in `site/`, which runs the cargo and `wasm-bindgen` steps and writes `public/wasm`), then run `npx wrangler pages deploy dist` from CI, or add the wasm step to a CI job that uploads `dist`. `public/_headers` sets `Cache-Control: public, max-age=3600` for `/wasm/*` and `X-Content-Type-Options: nosniff` for everything. A longer cache lifetime is safe only if the file names change with each build.

Next: the [crate reference](06-crates.md).
