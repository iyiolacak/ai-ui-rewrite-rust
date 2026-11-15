---
name: ai-ui
description: ai-ui is a Rust port of assistant-ui (crates.io, version 0.1): a chat runtime with branching threads, tools and approvals, decoders for the AI SDK UI message stream and data stream, bounded streaming Markdown, and an optional Slint chat UI, for native and WebAssembly apps. Use this skill when a user wants to add a chat runtime, AI SDK streaming, Markdown for streamed replies or a Slint chat UI to a Rust app, connect a Vercel AI SDK backend from Rust, port an assistant-ui React app to Rust, or debug the ai-ui-* crates. Triggers - ai-ui, assistant-ui in Rust, Slint chat UI, AI SDK stream in Rust, Vercel AI SDK backend from Rust, ai-ui-core, ThreadView.
---

# ai-ui

ai-ui is a Rust port of [assistant-ui](https://github.com/assistant-ui/assistant-ui). Ten library crates, all 0.1 on crates.io, plus `examples/thread`, a complete Slint app. It is early software and not affiliated with the assistant-ui or Vercel AI SDK projects.

Do not guess APIs. Read the reference files linked at the end, and the guides they point to, before writing code.

## Pick a mode

Choose from the user's request.

**Do the work.** The user wants ai-ui added, wired or fixed.
1. Inspect the project (Step 1) and report what you found and which crates you plan to add.
2. If the user asked to confirm first, stop and wait. Otherwise continue.
3. Implement in small steps. Prefer code copied from the guides and `examples/thread` over new code.
4. Verify (see the checklist) and report.

**Discuss or plan only.** The user asks whether it fits, what it would take, or for a plan. Make no file edits and run no `cargo add`. Read the project, then cover: fit, which crates, how it plugs into their executor and UI, the Slint and licence choices, known limits, and a numbered step plan. Wait for a go-ahead.

If the request is unclear, treat it as discuss mode and ask.

## Step 1: inspect the project

Report these before editing anything:
- Is it a Rust crate or a workspace? Which edition and Rust version (`rustc --version`, `rust-version` in Cargo.toml)?
- Which async runtime drives futures: Tokio, the Slint event loop, `wasm-bindgen-futures`, or none?
- Which UI toolkit, if any? Is Slint already used, and at which version?
- Does it target `wasm32-unknown-unknown`?
- Is there an AI SDK (Vercel) backend endpoint? What URL, and how does it authenticate?
- Does the project need a system prompt, tools or approvals?

## Pick crates

Add only what is needed, with `cargo add`.

| Need | Crates |
|---|---|
| Chat runtime, messages, branches, tools, no UI | `ai-ui-core` (always) |
| Talk to an AI SDK backend | `ai-ui-data-stream` + `ai-ui-http` |
| State and actions for any UI toolkit (`ThreadView`) | `ai-ui-state` |
| Markdown for streamed replies in your own renderer | `ai-ui-markdown` |
| Result-aware copy to the clipboard | `ai-ui-clipboard` |
| The ready-made Slint chat UI | `ai-ui-slint` (build-dependency) + `ai-ui-markdown-slint` + `ai-ui-state` |
| Custom wire protocol work, assistant-transport SSE | `ai-ui-stream` |
| Strict AI SDK 7 UI message stream reducer, standalone | `ai-ui-sdk-stream` |

Most apps need `ai-ui-core`, `ai-ui-state`, `ai-ui-data-stream` and `ai-ui-http`. Details per crate: [reference/crates.md](reference/crates.md).

## Hard rules

**Executor.** The runtime has no executor of its own. Runtime methods do their synchronous work at once and return a future (`RunFuture`) that you must spawn. Drive it with what the project already uses: `slint::spawn_local`, a Tokio `LocalSet`, or `wasm-bindgen-futures`. Use `futures_executor::block_on` only in tests and scripts. If the project has no executor and no UI, say so and ask which to use. See [reference/runtime.md](reference/runtime.md).

**Single thread.** Runtimes are `Rc`-based and single-threaded by design. Never wrap them in `Arc` or `Mutex`, and never move them to a multi-threaded executor. Messages are `Arc` snapshots, so adapters may move them to worker threads.

**Rust versions.** Minimum Rust 1.85 for the runtime crates. `ai-ui-clipboard` needs 1.86, `ai-ui-markdown` 1.88, `ai-ui-slint` and `ai-ui-markdown-slint` 1.92. Do not lower these. If the project's toolchain is older, tell the user.

**Slint.** Pin `slint` and `slint-build` to `=1.18.1`, with the feature `compat-1-18` on `slint`. `ai-ui-slint` is a build-dependency. In `build.rs`, register `ai_ui_slint::ui_dir()` as the library path named `assistant`, then import from `@assistant/lib.slint`. Full steps: [reference/slint-ui.md](reference/slint-ui.md).

**femtovg patch.** Until a Slint release uses femtovg 0.28, add this to the workspace root `Cargo.toml`. Without it a streaming chat uses about twice the CPU with Slint's FemtoVG-WGPU renderer. Slint's software and Skia renderers are not affected.

```toml
[patch.crates-io]
femtovg = { git = "https://github.com/iyiolacak/ai-ui-rewrite-rust" }
```

**Linux.** Slint needs the fontconfig development package: `libfontconfig1-dev` on Debian and Ubuntu, `fontconfig-devel` on Fedora. The runtime and state crates do not need it.

**Licences.** The ai-ui crates are MIT, except `ai-ui-sdk-stream`, which is "MIT AND Apache-2.0". Slint is GPL-3.0, or royalty-free with an attribution badge, or commercial. Ask the user which Slint licence applies before shipping anything that uses the Slint crates. Do not choose for them. See [THIRD_PARTY.md](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/THIRD_PARTY.md).

**AI SDK backend.** The client posts model messages. The server passes `body.messages` straight to `streamText` (no `convertToModelMessages`) and returns `result.toUIMessageStreamResponse()`. The system prompt is a top-level `system` field, not a system-role message, because AI SDK 7 rejects system-role messages inside `messages` by default. Proxies must not buffer the response. See [reference/backend.md](reference/backend.md).

**Cumulative results.** A `ChatModelAdapter` yields the whole reply so far in each item, not a delta. A hand-written adapter must mark streaming text parts `PartStatus::Running` or the view will not pace them. The wire decoders do this for you.

**wasm32.** Use `FetchClient` instead of `NativeClient`; pick with `cfg(target_arch = "wasm32")`. The Slint crates reach the browser only through an app built like `examples/thread`. See [reference/web.md](reference/web.md).

## Verification checklist

Run each, in the user's project:
1. `cargo build` (and `cargo check --target wasm32-unknown-unknown` for the crates that should build there, if the project targets the web).
2. `cargo clippy -- -D warnings`
3. `cargo test`
4. Run the app or a small example that sends one message and shows the streamed reply. Without a live backend, use a canned `ChatModelAdapter` or the loopback client in [reference/backend.md](reference/backend.md).

Report three lists: what you changed, what you ran (with results), and what you could not verify (for example a live backend, a GPU window, a licence decision).

## Known limits

Say these up front when they matter to the user's plan.
- Version 0.1, early. The repository does not call it production ready.
- Not yet ported: assistant-transport as a runtime (the wire format is decoded), a concrete HTTP remote thread-list adapter, LangGraph and other framework adapters.
- Not yet ported: custom per-tool components (generative UI). Tools render through a generic disclosure; an app can set a presenter per tool with `ThreadView::set_tool_presenter`.
- Not yet ported: attachment, feedback, speech, dictation, suggestion and history adapters, the message queue, composer attachments and quotes.
- Tools run sequentially.
- The Slint crates need the femtovg patch above. Markdown styling fidelity in a real GPU window is marked unverified in [STATUS](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/docs/STATUS.md).
- The interop test covers an AI SDK 7 `streamText` server only. Other backends, authentication and retries are not covered.
- In the browser build of the example, there are no Markdown worker threads and the example runs only its demo model; use `FetchClient` in your own app.
- Performance figures in the README are measured on one machine. Do not quote them as guarantees. See the README and `docs/RESULTS.md`.

## Where to look

Skill files:
- [reference/crates.md](reference/crates.md): the ten crates, key types, MSRV, wasm32.
- [reference/runtime.md](reference/runtime.md): first runtime, streaming, `ThreadView`, executors.
- [reference/backend.md](reference/backend.md): AI SDK backend, request and wire formats, errors, cancellation.
- [reference/slint-ui.md](reference/slint-ui.md): the Slint thread UI in your app.
- [reference/web.md](reference/web.md): WebAssembly notes.
- [reference/troubleshooting.md](reference/troubleshooting.md): common failures.

Online guides (repository `https://github.com/iyiolacak/ai-ui-rewrite-rust`):
- [README](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/README.md)
- [Getting started](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/docs/guide/01-getting-started.md)
- [Concepts](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/docs/guide/02-concepts.md)
- [Streaming and backends](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/docs/guide/03-streaming-and-backends.md)
- [Building a UI with Slint](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/docs/guide/04-ui-slint.md)
- [Running on the web](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/docs/guide/05-web.md)
- [Crate reference](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/docs/guide/06-crates.md)
- [Coming from assistant-ui](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/docs/guide/08-from-assistant-ui.md)
- [FAQ](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/docs/guide/10-faq.md)
- [STATUS](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/docs/STATUS.md), [PROTOCOL](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/docs/PROTOCOL.md), [WEB](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/docs/WEB.md), [patches/README.md](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/patches/README.md)
- A complete app: [examples/thread](https://github.com/iyiolacak/ai-ui-rewrite-rust/tree/main/examples/thread)

API reference on docs.rs: [ai-ui-core](https://docs.rs/ai-ui-core), [ai-ui-stream](https://docs.rs/ai-ui-stream), [ai-ui-data-stream](https://docs.rs/ai-ui-data-stream), [ai-ui-sdk-stream](https://docs.rs/ai-ui-sdk-stream), [ai-ui-http](https://docs.rs/ai-ui-http), [ai-ui-state](https://docs.rs/ai-ui-state), [ai-ui-markdown](https://docs.rs/ai-ui-markdown), [ai-ui-clipboard](https://docs.rs/ai-ui-clipboard), [ai-ui-markdown-slint](https://docs.rs/ai-ui-markdown-slint), [ai-ui-slint](https://docs.rs/ai-ui-slint). Slint: [slint.dev](https://slint.dev). AI SDK: [ai-sdk.dev](https://ai-sdk.dev).
