# ai-ui-rewrite-rust

A Rust port of [assistant-ui](https://github.com/assistant-ui/assistant-ui): chat runtime, AI SDK and assistant-stream
wire decoders, and a Slint thread UI, for native and WebAssembly apps.

- Runtime crates (`ai-ui-core`, `-stream`, `-data-stream`, `-http`, `-state`) have no UI dependency; the gate builds
  them for `wasm32-unknown-unknown`.
- Follows upstream module for module at one pinned commit, so a backend that serves assistant-ui clients needs no change.
- Per-update cost does not grow with thread length: a bounded change journal projects only changed rows
  ([how](docs/guide/07-performance.md)).
- Markdown is formatted on a worker-thread pool, settled blocks once and the live tail on each delta.
- Native demo is one Rust binary with no JavaScript engine: `cargo run -p ai-ui-example-thread`.

Against upstream assistant-ui on the same workload, 1000 messages with a 9 KB prompt, median of 5 interleaved pairs,
release build, software renderer on the Rust side, headless Chromium for React, 2026-11-11 (ranges in
[RESULTS](docs/RESULTS.md)):

| | Rust | React | Ratio |
|---|---|---|---|
| Open the thread, Markdown formatted and painted | 105.3 ms | 2940 ms | 27.9x |
| Longest iteration or frame while streaming | 34 ms | 916.6 ms | 26.9x |
| Memory at the end (Rust RSS, React PSS) | 30.1 MiB | 850.3 MiB | 28.2x |
| Process CPU while streaming 60 s | 17160 ms | 73260 ms | 4.2x |

Rust "open" is until the last message's Markdown is formatted and painted; the first rows appear after 12.9 ms, which is
not compared. Ratios are floored.

> Status: early and unpublished (nothing is on crates.io; the Slint crates wait for femtovg 0.28). Not yet ported:
> assistant-transport as a runtime, attachments, speech, custom per-tool components ([STATUS](docs/STATUS.md)). A port,
> not affiliated with the assistant-ui or Vercel AI SDK projects.

Start with [Why ai-ui](docs/guide/00-why-ai-ui.md) or [Getting started](docs/guide/01-getting-started.md); every
number is in [RESULTS](docs/RESULTS.md); the browser build is described in [WEB](docs/WEB.md).

## Quickstart

Not on crates.io yet, so depend on the repository:

```toml
[dependencies]
ai-ui-core = { git = "https://github.com/iyiolacak/ai-ui-rewrite-rust" }
```

```rust
use ai_ui_core::{adapter, AppendMessage, ChatModelRunResult, LocalRuntimeOptions, LocalThreadRuntime};

let runtime = LocalThreadRuntime::new(
    |_options| adapter::from_results([Ok(ChatModelRunResult::text("Hello!"))]),
    LocalRuntimeOptions::default(),
);
let run = runtime.append(AppendMessage::user_text(None, "Hi"))?;
// Drive `run` on any executor: slint::spawn_local, a Tokio LocalSet, wasm-bindgen-futures...
```

That snippet is built as [`crates/ai-ui-core/examples/quickstart.rs`](crates/ai-ui-core/examples/quickstart.rs).
For a full app, run the Slint example: `cargo run -p ai-ui-example-thread` (add `--api URL` for an AI SDK endpoint).

## Crates

| Crate | Upstream | Contents | wasm32 |
|---|---|---|---|
| `ai-ui-core` | `@assistant-ui/core` | Message model (upstream JSON shape), branching `MessageRepository`, `ChatModelAdapter`, `ThreadRuntime` with local and external-store runtimes, `ThreadList`, tools and approvals, composers | yes |
| `ai-ui-stream` | `assistant-stream` | Stream chunks, message accumulator, state operations, incremental partial JSON, `assistant-transport` SSE | yes |
| `ai-ui-data-stream` | `@assistant-ui/react-data-stream` | `ChatModelAdapter` for AI SDK backends: request conversion, UI message stream and data stream responses | yes |
| `ai-ui-sdk-stream` | AI SDK `readUIMessageStream` | Exact AI SDK 7 UI-message-stream reducer, checked against the published package | yes |
| `ai-ui-http` | (fetch in upstream) | HTTP clients for the adapter: native (ureq + rustls) and browser `fetch` | yes |
| `ai-ui-state` | scoped state (`MessageState`...) | `ThreadView`: UI-agnostic state and actions over any runtime, smooth streaming clock | yes |
| `ai-ui-markdown` | `@assistant-ui/markdown` | Bounded Markdown parsing and the streaming block splitter | yes |
| `ai-ui-clipboard` | (browser clipboard) | Result-aware copy for native and browser | yes |
| `ai-ui-markdown-slint` | (react-markdown renderers) | Markdown to Slint rich text, off-thread formatting, streaming cache | via the example |
| `ai-ui-slint` | `@assistant-ui/react` UI | Slint thread, messages, composer and actions | via the example |
| `examples/thread` | assistant-ui default thread | Demo app: built-in streaming model or any AI SDK endpoint; native and browser | demo build |

The first eight have no UI dependency and are the crates intended for release; the two Slint crates and the example
are `publish = false` for now. Portions are ported from assistant-ui (MIT, (c) AgentbaseAI Inc.); see [NOTICE](NOTICE).

## Performance

The latest measured Rust improvements and their reproduction commands are in
[`docs/PERFORMANCE.md`](docs/PERFORMANCE.md): incremental long-thread projection,
faster stream decoding, cooperative scheduling and bounded large-prompt layout.

Against upstream assistant-ui (React) on the same workload, measured with
[`benchmarks/react-baseline`](benchmarks/react-baseline) (whole-process CPU and
memory, same streamed reply, 60 s window; median of 5 interleaved pairs, release build, software renderer on the Rust side,
2026-11-11):

| | Rust | React |
|---|---|---|
| CPU while streaming into a 1000-message thread | 17.2 s | 73.3 s |
| Opening a 1000-message thread, until Markdown is formatted and shown | 0.11 s | 2.9 s |
| Memory with 1000 messages | 30 MiB | 1.4 GiB RSS, 0.8 GiB PSS (9 processes) |
| Longest stall while streaming a 40 KB prompt (200 messages) | 0.11 s | 2.4 s |

The open row compares like with like: Rust until the last message's Markdown is formatted and painted (105 ms; the rows
are visible after 13 ms, but that is not what React's 2.9 s measures), React until all messages and their Markdown are
rendered. Method, caveats and more numbers: [`docs/RESULTS.md`](docs/RESULTS.md) and [`docs/STATUS.md`](docs/STATUS.md#performance).

## Using the crates

`ai-ui-slint`, `ai-ui-markdown-slint` and the `examples/thread` app add a Slint UI. Until Slint ships femtovg 0.28,
an app using them adds the `[patch.crates-io] femtovg` entry from this workspace's `Cargo.toml` (see
[patches/README.md](patches/README.md)).

**Slint licence.** This project's code is MIT. Slint itself is available under GPL-3.0, the Slint Royalty-Free
licence (requires an attribution badge) or a commercial licence; an application using the Slint crates must comply
with the one it chooses. See [THIRD_PARTY.md](THIRD_PARTY.md). Contributions: [CONTRIBUTING.md](CONTRIBUTING.md);
security reports: [SECURITY.md](SECURITY.md); changes: [CHANGELOG.md](CHANGELOG.md).

