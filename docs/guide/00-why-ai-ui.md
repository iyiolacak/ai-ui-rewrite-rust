---
title: Why ai-ui
description: What ai-ui is, what it gives you that you can check, how it compares with upstream assistant-ui on the same workload, and where its limits are.
order: 0
section: Start
---

# Why ai-ui

**What this page gives you.** The case for ai-ui in one place: the problem it addresses, what you can verify about it,
a small like-for-like results table with its conditions, and the limits. Each claim points to a file or a command.

## The problem

A chat thread grows. A reply streams in as many small updates, a few bytes at a time. In a UI where each update
re-renders, re-lays out or re-parses work that scales with the whole thread, the cost of one update grows with the
number of messages, not with the size of the change.

In upstream assistant-ui's default React thread, which renders every message into the DOM and does not virtualise the
list, that looks like this on our benchmark: with 1000 messages the longest frame interval while streaming was 916.6 ms
(median of 5 interleaved pairs, range 866.6 to 1183.3 ms), and the main thread was busy for about the whole 60 s window.
With 10,000 messages, opening the thread took 26.1 s. Upstream documents a virtualised thread for long conversations;
it has not been measured here yet. Conditions and caveats are in the table below and in [RESULTS](../RESULTS.md).

## What ai-ui is

ai-ui is a Rust port of [assistant-ui](https://github.com/assistant-ui/assistant-ui), module for module at one pinned
upstream commit: the message model with branching, a chat runtime, tool calls with approvals, the AI SDK and
assistant-stream wire protocols, a UI-agnostic state layer, and a Slint thread UI with Markdown. The runtime crates
have no UI dependency, so a native app, a WebAssembly app or your own UI toolkit can use them. A backend that already
serves assistant-ui clients needs no change.

## What you get

- **Runtime crates with no UI dependency.** `ai-ui-core`, `ai-ui-stream`, `ai-ui-data-stream`, `ai-ui-http` and
  `ai-ui-state` do not depend on Slint or any windowing code. See the [crate reference](06-crates.md).
- **wasm32 is checked by the gate.** `scripts/gate.sh` runs `cargo build --locked --target wasm32-unknown-unknown` for
  the eight toolkit-independent crates (the full gate, not `--quick`). The Slint demo has its own web build; see
  [Running on the web](05-web.md).
- **Wire-compatible decoders.** The AI SDK UI message stream, the AI SDK data stream and assistant-transport are decoded
  incrementally, with reads that may split anywhere, including inside a UTF-8 character. 96 protocol vectors are saved in
  `crates/ai-ui-stream/tests/fixtures/protocol-vectors.json`; the counts are below.
- **Per-update work limited to the rows that changed.** A bounded change journal in `ai-ui-core` lets a view
  project only the rows that changed, and the host no longer scans every row. The runtime snapshot and state projection
  still grow about linearly with history, so per-update cost is lower but not constant
  ([PERFORMANCE](../PERFORMANCE.md#remaining-costs-and-next-experiments)). The mechanism is in
  [Performance](07-performance.md); the end-to-end measurements are in the table.
- **Markdown formatted off the UI thread.** `ai-ui-markdown` uses a pool of worker threads, formats settled blocks once
  and re-formats only the live tail while text streams (`crates/ai-ui-markdown/src/pool.rs`).
- **One native binary, no JavaScript engine.** `cargo run -p ai-ui-example-thread` builds and runs the demo; there is no
  Node or browser process in the native case.
- **Measured against upstream, with ranges.** Every figure comes from `docs/results.json`, is generated into
  [RESULTS](../RESULTS.md), and is checked by `python3 scripts/results.py check`.
- **Your own backend or none.** `LocalThreadRuntime` with a custom `ChatModelAdapter` needs no network; an
  [AI SDK endpoint](03-streaming-and-backends.md) is one flag in the example (`--api URL`).

## Measured against upstream assistant-ui

Rust is this repository's example app with the software renderer (`--bench-render`); React is upstream assistant-ui's
default thread (`@assistant-ui/react` 0.15.33) in headless Chromium, which renders every message into the DOM and does
not virtualise the list. A virtualised React thread has not been measured yet; it would narrow the gap on long threads,
the 10,000-message row most of all. Same history, same demo reply, 60 s window. Medians of 5
interleaved pairs, measured 2026-11-11, machine load about 4-6. Ranges are min to max over the 5 runs. A second, independent run (9 pairs, lighter load) is in
[Performance](07-performance.md#second-run) and [RESULTS](../RESULTS.md). Across all four runs of that day the ratios were 16.6x to 27.9x for open, 23.8x to 42.7x for the longest stall, 28.2x to 35.4x for memory and 4.2x to 14.0x for CPU (medians per run; [RUNS](../RUNS.md)).

| Workload | Measure | Rust | React (default, not virtualised) | Ratio (range) | RESULTS row |
|---|---|---|---|---|---|
| 1000 messages | open the thread, Markdown formatted and painted | 105.3 ms (100.8 to 150.9) | 2940 ms (2794 to 2987) | 27.9x (18.5x to 29.6x) | 15 |
| 1000 messages | longest iteration or frame while streaming | 34 ms (17.5 to 35.4) | 916.6 ms (866.6 to 1183.3) | 26.9x (24.4x to 67.6x) | 7 |
| 1000 messages | memory at the end, Rust RSS vs React PSS | 30.1 MiB (29.7 to 30.4) | 850.3 MiB (841.8 to 895.4) | 28.2x (27.6x to 30.1x) | 14 |
| 1000 messages | process CPU while streaming 60 s | 17160 ms (13700 to 17650) | 73260 ms (73050 to 73540) | 4.2x (4.1x to 5.3x) | 18 |
| 10,000 messages | open the thread, Markdown formatted and painted | 435.5 ms (382.9 to 472.1) | 26081 ms (23591 to 27196) | 59.8x (49.9x to 71x) | 6 |

Conditions that apply to every row:

- **Like for like.** "Open" is the same end state on both sides. Rust: until the last message's Markdown is formatted
  and painted at least once. React: until all messages are in the DOM with their Markdown rendered. Rust's first sync
  alone (12.9 ms at 1000 messages) is an earlier event and is not compared.
- **Memory.** PSS is the conservative figure for React because it discounts shared pages. React RSS summed over all
  Chromium processes was 1481.5 MiB for the same run (row 11).
- **Rust CPU includes software rasterisation.** React CPU is the whole Chromium process tree.
- **Ratios** are computed from the stated values and floored to one decimal. The range pairs the best Rust run with the
  worst React run and the reverse.
- **Not measured here:** upstream's virtualised React thread; the default thread with the kit's Tailwind and Radix
  styling and its `content-visibility: auto` rule on message roots (the React page uses plain CSS without it); a GPU
  window against React with a GPU, other browsers, other machines, and scenarios other than these two prompts. Stalls from single events are noisy; read the range, not just the median.

Method and reproduction commands: [Performance](07-performance.md). The earlier "24x" open figure, which compared
Rust before Markdown was formatted with React after, was withdrawn and is explained in RESULTS.

## How it works

A streamed update changes one message. The repository records which selected-path message changed in a bounded journal;
each `ThreadView` reads its own cursor and projects only those rows, falling back to a full projection when the thread's
shape changed or the cursor expired. The Slint host compares row identities (messages are `Arc`, so that is a pointer
comparison) and updates changed rows in place. The details, with file names, are in [Performance](07-performance.md).

Markdown is the other large cost. Parsing happens on worker threads, the newest source per key wins, and the UI keeps
showing formatted text while newer text is pending. Visible parts are submitted first. In the browser there are no
threads, so a wasm-only inline backend is used.

The design rules that make this possible (immutable `Arc` messages, a runtime free of I/O and clocks, adapters that yield
cumulative results) are described in [Concepts](02-concepts.md).

## Who it is for

- Rust apps, native or WebAssembly, that need a chat thread with branching, tools and approvals.
- Teams with a backend that already speaks the AI SDK protocols or assistant-transport.
- People who want the chat runtime without a UI toolkit and will write their own view on `ai-ui-state`.

## Who it is not for

- Projects that need features on the not-yet-ported list: assistant-transport as a runtime, a concrete HTTP remote
  thread-list adapter, LangGraph and other framework adapters, custom per-tool components, attachment, speech,
  dictation, suggestion and history adapters, message queue.
- Web apps that already run well on React and have no reason to change. The port does not remove the need for a Rust
  toolchain.

## Status and limits

- **Early.** All ten library crates are on crates.io as 0.1 (minor versions may break the API before 1.0). With Slint's
  FemtoVG-WGPU renderer, an app adds a one-line femtovg patch until a Slint release includes the fix
  (`patches/README.md`).
- **Browser demo.** It needs WebGL. On a 2x display the canvas renders at 1x and looks soft (known issue). Frame rates
  measured in headless Chromium use software WebGL and are a floor.
- **Fidelity.** Markdown styling in a real GPU window is marked unverified in [STATUS](../STATUS.md).
- **Protocol conformance.** As of 2026-11-09 ([PROTOCOL](../PROTOCOL.md#conformance-status)): 22 of 23 assistant-stream
  vectors that were run match (2 skipped), and 52 of 65 AI SDK vectors match, with each of the 13 differences recorded
  with its reason. The tests are the current truth.
- **Slint licence.** The code is MIT. Slint is GPL-3.0, a royalty-free licence with a badge, or commercial; apps that use
  the Slint crates must comply with the one they choose.
- **Independent project.** A port, not affiliated with the assistant-ui or Vercel AI SDK projects.

## Where to start

- [Getting started](01-getting-started.md): a first runtime in 20 lines, a first window, a first web build.
- [Concepts](02-concepts.md): the message model and runtimes.
- [Performance](07-performance.md): why the numbers look like this, and how to reproduce them.
- [Coming from assistant-ui](08-from-assistant-ui.md): parity table and differences.
- [RESULTS](../RESULTS.md) and [STATUS](../STATUS.md): the full tables and the ported and not-ported lists.
