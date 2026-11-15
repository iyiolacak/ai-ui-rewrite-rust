---
title: Coming from assistant-ui
description: What is ported from assistant-ui, how packages map to crates, where the Rust API differs, and what is not there yet.
order: 8
section: Reference
---

# Coming from assistant-ui

**What this page gives you.** This page maps assistant-ui packages to crates, lists what is not ported, shows where the Rust API differs from the TypeScript one, states what a backend needs, and ends with a migration checklist.

ai-ui follows upstream's architecture module for module. The reference is assistant-ui commit `eb65bd4` (MIT). A
feature counts as ported when it behaves as upstream does, checked by tests derived from upstream's. The
authoritative list is [STATUS](../STATUS.md); this page organises it for someone who knows the TypeScript packages.

## Package to crate map

| Upstream | Crate | Status |
|---|---|---|
| `@assistant-ui/core` types and `MessageRepository` | `ai-ui-core` | Ported. Same JSON shape; unknown fields round-trip. |
| `ChatModelAdapter`, local thread runtime | `ai-ui-core` | Ported: append, edit, regenerate, branches, cancel, tool-result continuation, `maxSteps`, human tools, feedback, export and import. |
| Thread and edit composers | `ai-ui-core` | Ported: send, cancel, edit branching. No attachment adapter or dictation. |
| `useExternalStoreRuntime` | `ai-ui-core` | Ported. App-owned messages with edit, reload, cancel, tool and feedback callbacks; `Arc` identity kept for unchanged messages. |
| Thread list, `useRemoteThreadListRuntime` | `ai-ui-core` (`ThreadList`) | Ported: switch, new, rename, archive, unarchive, delete, generated titles, load; optimistic with rollback. The host supplies the `Spawner`. |
| `assistant-stream` (chunks, accumulator, state operations, partial JSON, assistant-transport SSE) | `ai-ui-stream` | Ported. Timing metadata is not computed. |
| AI SDK UI message stream and data stream | `ai-ui-stream` | Both protocols, including assistant-ui's `aui-*` data-stream extensions. |
| AI SDK `readUIMessageStream` | `ai-ui-sdk-stream` | A separate strict AI SDK 7 reducer, checked against the published package. |
| `@assistant-ui/react-data-stream` | `ai-ui-data-stream` | Ported: request conversion, protocol detection, error statuses (without the response text). |
| `fetch` in upstream | `ai-ui-http` | Native (ureq and rustls) and browser `fetch` clients. |
| `useSmooth` | `ai-ui-core` (`SmoothText`) | Same pacing as upstream. |
| Frontend tools and approvals | `ai-ui-core` (`Tool`, `ToolRegistry`) | Ported with approvals; tools run sequentially. |
| Scoped state (`MessageState`, `ThreadState`, `ComposerState`) | `ai-ui-state` | `ThreadView`, a UI-agnostic projection with scoped actions. |
| `@assistant-ui/react` thread look | `ai-ui-slint` | Thread, messages, composer, suggestions, reasoning and tool disclosure, branch picker, copy, edit, regenerate and feedback actions. |
| `@assistant-ui/markdown` | `ai-ui-markdown`, `ai-ui-markdown-slint` | Parsed off the UI thread; settled blocks plus a live tail. |
| Clipboard (browser API) | `ai-ui-clipboard` | Reports whether the copy actually worked, on native and in the browser. |

## What is not ported yet

These gaps are listed in STATUS, not hidden:

- **Runtimes.** assistant-transport (server-owned state) as a runtime; a concrete HTTP remote thread-list adapter;
  LangGraph and other framework adapters. The assistant-transport wire format is decoded; what is missing is the
  runtime that owns state on the server.
- **Generative UI.** Custom per-tool components. Tools render through a generic disclosure by default, and an app
  can register a presenter per tool (`ThreadView::set_tool_presenter`) to choose the title, summary and status. A
  custom Slint component per tool is not available.
- **Attachments.** Composer attachments with an adapter, and quotes.
- **Dictation and speech.** No speech or dictation adapters.
- **Other adapters.** Feedback, suggestion and history adapters, and the message queue.
- **Look.** The Slint UI follows upstream's default thread; Markdown styling fidelity in a real GPU window is marked
  unverified in STATUS, because offscreen screenshots use the software renderer.

## Where the Rust API differs

**No React hooks.** Upstream exposes state through hooks that re-render components. Here `ai-ui-state::ThreadView`
wraps any `ThreadRuntime`, projects it into plain data (message rows, parts, `isLast`, branch number and count,
`isCopied`, composer state) and offers the scoped actions. A UI subscribes, reads the view, and updates its own
widgets. The Slint example does this; another toolkit can too. `ThreadView` replaces `useThread`, `useMessage`
and `useComposer`.

**One runtime trait.** `ThreadRuntime` is a trait. The local runtime, the external-store runtime and thread-list
threads implement it, so the same UI works with all of them.

**Ownership, not ambient state.** Messages are immutable snapshots held in `Arc`. The view keeps a row cache keyed by
`Arc` identity, so an unchanged message is a pointer comparison and not a deep compare. An external-store app that
re-sends an unchanged message keeps its `Arc` and costs nothing downstream. Errors are `Result`s, not exceptions.

**No executor of its own.** Runtime methods do their synchronous work at once (the user message and the running
assistant message appear immediately) and return a future that you spawn: `slint::spawn_local`, a Tokio `LocalSet`,
or `wasm-bindgen-futures`. The runtime is single-threaded (`Rc`), like upstream's. Messages are `Arc` so adapters can
move them to worker threads. Do not wrap it in locks to make it look concurrent.

**No ambient clock or randomness.** Timestamps and ids come from options, and `ThreadView` takes a caller-supplied
clock for smooth streaming. That is what lets the core compile for `wasm32-unknown-unknown`. `createdAt` is epoch
milliseconds.

**Subscriptions** are callbacks with a guard that unsubscribes when dropped, called after each state change and never
while internal state is borrowed.

**`AbortSignal`** becomes `ai-ui-core::abort`, a thread-safe signal an adapter can await or poll.

**Streaming adapters.** `ChatModelAdapter::run` returns a stream of cumulative `ChatModelRunResult`s, as upstream
does. A single result is a one-item stream. `run_stream` folds all slices that are ready into one result per wake-up.

## What a backend needs

Nothing new. The port reads and writes the wire protocols upstream's clients use: the AI SDK UI message stream, the
AI SDK data stream with assistant-ui's extensions, and the assistant-transport SSE format. A backend that serves
assistant-ui over these needs no change. A real AI SDK 7 `streamText` server over a mock model is exercised end to end
by `crates/ai-ui-http/tests/interop_ai_sdk.rs`: reasoning, a server tool call, a second step, and a follow-up turn
that replays the tool call. Not covered: other backends, authentication and retries. Details are in
[Streaming and backends](03-streaming-and-backends.md).

## Migration checklist

1. Pick the runtime: `LocalThreadRuntime` with a `ChatModelAdapter` (for example `ai-ui-data-stream` against your AI
   SDK endpoint), or `ExternalStoreRuntime` if your app owns the messages.
2. Replace hook calls with a `ThreadView` over that runtime, and call its actions where you called the hook's.
3. Spawn the futures the runtime returns on your executor.
4. Convert message storage, if any. The JSON shape is upstream's, so exported threads import without a mapping.
5. Re-create tools as `Tool` values with `execute`, or mark them `requires_approval`. Replace custom tool
   components with a presenter, or accept the generic disclosure.
6. Replace anything that used attachments, dictation or assistant-transport-as-runtime. These are the gaps above.
7. For a thread list, implement `ThreadListAdapter` and supply the `Spawner`.
8. Check wasm if you target the web: `cargo check --target wasm32-unknown-unknown -p <crate>`.

## Licence of the toolkit

The Slint crates need a Slint licence of your choice; the runtime crates do not depend on Slint. See
[Building a UI with Slint](04-ui-slint.md) and [FAQ](10-faq.md).

Next: [Contributing and testing](09-contributing.md), or the [FAQ](10-faq.md) for short answers.
