---
title: FAQ
description: Short answers to the questions people ask first about ai-ui, the Rust port of assistant-ui.
order: 10
section: Reference
---

# FAQ

**What this page gives you.** This page gives short answers to the questions people ask first, each with a pointer to the page or file that holds the detail.

## Is it production ready?

No. The repository describes itself as early (0.1). The runtime, the wire-protocol decoders and the Slint
thread are tested, and the gate (formatting, Clippy, documentation, tests, wasm builds, AI SDK interop) passes
locally. But it is a young port with listed gaps (see "What is not supported yet"), Markdown styling in a real GPU
window is marked unverified in [STATUS](../STATUS.md), and nothing is released. Evaluate it against your own needs
before you depend on it.

## Why Rust?

So that the same chat runtime can run in a native app and in WebAssembly, without a JavaScript engine in the
native case, and with predictable cost per streamed update. The core is free of UI, I/O, clocks and randomness, so
it compiles for `wasm32-unknown-unknown`. Where this shows up in speed is described in [Performance](07-performance.md).

## Why Slint?

Slint is the toolkit the bundled UI uses: it runs on native desktops and in the browser, and its software renderer
makes headless tests possible. It is a choice for the UI crates only. The runtime crates (`ai-ui-core`, `-stream`,
`-data-stream`, `-http`, `-state`, `-markdown`, `-clipboard`) do not depend on Slint.

## Does it work with my backend or the AI SDK?

If your backend already serves assistant-ui, yes without changes: the crates read the AI SDK UI message stream, the AI
SDK data stream with assistant-ui's extensions, and the assistant-transport SSE format. The integration test runs a
real AI SDK 7 `streamText` server over a mock model, covering reasoning, a server tool call, a second step and a
follow-up turn. Other backends, authentication and retries are not covered by that test. See
[Streaming and backends](03-streaming-and-backends.md).

## Can I use it on the web, and how big is it?

Yes. `examples/thread` builds for `wasm32-unknown-unknown` (`scripts/build-web-demo.sh`) and runs on a canvas with
Slint's femtovg renderer on WebGL. Browser `fetch` and the clipboard are supported. Limits: no Markdown worker threads
(Markdown is formatted inline on the UI thread), the demo runs only the built-in demo model, and small text,
especially code, can look softer than DOM text on 1x displays, and a fractional canvas box softens text in Firefox
unless the page uses the bundled `pixel-snap.js`. The guide does not state a size for the bundle, because no size is recorded in
the repository's reference documents. Build it and measure `dist-web/`. See [Running on the web](05-web.md).

## How does it compare with assistant-ui in speed, and what is the catch?

On the recorded workloads, Rust opened a 1,000-message thread, Markdown formatted and painted, in 105 ms against
2.94 s for React (27.9x), used 17.2 s of CPU against 73.3 s for 60 s of streaming (4.2x), and 30 MiB of memory
against 1481 MiB RSS or 850 MiB PSS. Those are medians of 5 interleaved pairs on 2026-11-11; across all four runs that day the ratios were 16.6x to 27.9x for open, 23.8x to 42.7x for the longest stall, 28.2x to 35.4x for memory and 4.2x to 14.0x for CPU ([RUNS](../RUNS.md)).

The catch: the comparison is against upstream's thread with plain CSS in headless Chromium on a shared, busy machine;
the Rust side was offscreen with software raster; it was one machine; no GPU window was compared; and the 2026-11-10
numbers had to be corrected. Read "How to read these numbers" in [Performance](07-performance.md).

## Is it on crates.io?

The eight toolkit-independent crates are: `ai-ui-core`, `ai-ui-stream`, `ai-ui-data-stream`, `ai-ui-sdk-stream`, `ai-ui-http`, `ai-ui-state`, `ai-ui-markdown` and `ai-ui-clipboard`, version 0.1.
The two Slint crates and the example are `publish = false` until Slint ships femtovg 0.28; depend on them with a `git`
dependency on the repository.

## Which licence?

The project's own code is MIT, with portions ported from assistant-ui (MIT). Slint, used by the UI crates, is
available under GPL-3.0, a royalty-free licence that requires an attribution badge, or a commercial licence. An app
using the Slint crates must comply with the one it picks. Details are in `THIRD_PARTY.md` and `NOTICE`.

## Can I use my own UI toolkit with the runtime crates?

Yes. `ai-ui-core` and `ai-ui-state` do not depend on a toolkit. `ThreadView` projects any `ThreadRuntime` into plain
data and actions; a UI subscribes to it and updates its widgets. The Slint crates are one consumer. See
[Coming from assistant-ui](08-from-assistant-ui.md) for how `ThreadView` replaces React hooks.

## How do branches, edit and regenerate work?

As in upstream. `MessageRepository` is a tree of messages with a selected path. Editing a user message creates a
sibling branch; regenerating an assistant message does the same. The branch picker shows the current branch number and
count (`ThreadView` exposes both) and selects between siblings. Thread export and import use upstream's JSON shape.

## How do I stream tool calls?

Tool calls arrive as parts of the streamed assistant message, with their arguments filled in incrementally (partial
JSON is repaired as it grows). A `Tool` with `execute` runs in your app after each model step and its result feeds the
next step, up to `maxSteps`. A tool with `requires_approval` pauses the run until you call
`respond_to_tool_approval`. Tools run sequentially. A tool executed by the server arrives already resolved in the
stream. See [Concepts](02-concepts.md).

## What is not supported yet?

Assistant-transport as a runtime (the format is decoded), a concrete HTTP remote thread-list adapter, LangGraph and
other framework adapters, custom per-tool components (generative UI), attachment, speech, dictation, suggestion and
history adapters, the message queue, and composer attachments and quotes. The full list is in
[Coming from assistant-ui](08-from-assistant-ui.md) and [STATUS](../STATUS.md).

## How do I reproduce the benchmarks?

Build `ai-ui-example-thread` in release, build `benchmarks/react-baseline` with `npm ci --ignore-scripts && npm run
build`, set `PLAYWRIGHT` and `CHROME`, then run `./compare.sh 1000 9 5` from that directory. Raw per-run files and
the summary scripts are described in [Performance](07-performance.md#reproduce-it-yourself). `python3
scripts/results.py check` confirms that [RESULTS](../RESULTS.md) matches `docs/results.json`.

## Do numbers in this guide come from somewhere I can check?

Yes. Every figure is copied from `docs/RESULTS.md` or `docs/results.json` with its conditions. If a figure is not
there, the guide does not state it, which is why there is no bundle size above.

## How do I contribute?

Run `scripts/gate.sh`, port tests from upstream for behaviour changes, and publish results only when they are measured
and reproducible. See [Contributing and testing](09-contributing.md).

## Who maintains it?

The repository is under the GitHub account `iyiolacak`; the licence names "The assistant-ui Rust port contributors".
It is not affiliated with the assistant-ui or Vercel AI SDK projects. `SECURITY.md` says the private security contact
will be set when the repository is published.

Next: [Why ai-ui](00-why-ai-ui.md) for the summary, or [Getting started](01-getting-started.md) to try it.
