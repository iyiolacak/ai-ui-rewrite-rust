# assistant-ui → Rust: architecture of the port

Decision, 2026-11-08: the port follows upstream's own architecture, module for
module, rather than the independent event/effect core built earlier
(`crates/assistant-core`). Upstream reference:
[assistant-ui](https://github.com/assistant-ui/assistant-ui) at
`eb65bd4558b769550093003c95e5867ad0ec8eba` (MIT, © AgentbaseAI Inc.).

## Why

The earlier crates are carefully tested, but they model a different API (scoped
run keys, effects and events) and much of the effort went to edge lanes (a
custom long-draft editor with forked text shaping, storage encryption, upload
replay). No upstream package had a Rust equivalent, and nothing could be used the
way assistant-ui is used: pick a runtime, plug in a model adapter, render the
thread with primitives. The port now starts from that shape.

## Layering

| Upstream | Rust | Notes |
|---|---|---|
| `@assistant-ui/core` `types/` | `ai-ui-core::types` | Same JSON shape (`type`/`role` tags, camelCase, `unstable_*` names). Unmodelled fields round-trip through `extra`. `createdAt` is epoch milliseconds. |
| `runtime/utils/message-repository.ts` | `ai-ui-core::repository` | Branch tree with selected path, export/import, optimistic messages. Errors are `Result`s, not exceptions. |
| `runtime/utils/chat-model-adapter.ts` | `ai-ui-core::adapter` | `ChatModelAdapter::run` returns a stream of cumulative `ChatModelRunResult`s; a single result is a one-item stream. |
| `AbortSignal` | `ai-ui-core::abort` | Thread-safe signal an adapter can await or poll. |
| `runtimes/local/` | `ai-ui-core::local` | `LocalThreadRuntime`: append, run loop, cancel, branches, tool results, `maxSteps`, human tools. |
| `runtime/base/default-{thread,edit}-composer-runtime-core.ts` | `ai-ui-core::local::Composer` | Thread and edit composers; attachment adapters and dictation not yet. |
| Thread list, external store, tool registry (frontend `execute`), adapters (attachments, feedback, speech, suggestions, history) | planned | Next increments. |
| `assistant-stream` (chunks, accumulator, serializations) | `ai-ui-stream`, built on the existing `assistant-transport`, `assistant-state` and `assistant-partial-json` | `run_stream_from_sse` turns any response byte stream into a runtime `RunStream`; HTTP transports plug in there. |
| `@assistant-ui/react` primitives | `assistant-slint` (existing) | Slint components bound to runtime state. |
| `@assistant-ui/markdown` | `assistant-markdown` (existing) | Unchanged. |

## Rust conventions

- **Executor-agnostic.** Runtime methods do their synchronous work immediately
  (the user message and the running assistant message appear at once) and return
  a future that the host spawns: `slint::spawn_local`, a Tokio `LocalSet`, or
  `wasm-bindgen-futures`. The runtime itself is single-threaded (`Rc`), like
  upstream; messages are `Arc` so adapters can move them to worker threads.
- **No ambient clock or randomness.** Timestamps and ids come from options, so
  the core compiles for `wasm32-unknown-unknown`, where `SystemTime` is
  unavailable.
- **Subscriptions** are callbacks with drop-to-unsubscribe guards, notified after
  each state change and never while internal state is borrowed.
- **Behaviour parity is tested** by porting upstream test cases for each module.

## Frozen lanes

The long-draft editor stack (`assistant-editor*`, `assistant-harfrust`,
`assistant-parley`), storage encryption and upload adapters are kept but not
extended until the runtime, composer and primitives reach parity.

## Review against upstream (2026-11-09)

Compared with upstream's layering (runtimes, a scoped state layer, headless
primitives, adapters), measured on the Rust side in release mode.

**Where the port is sound.** The message model, branching repository, local
runtime semantics, tool and approval flow, and the AI SDK and
assistant-transport decoders follow upstream and are checked by tests derived
from upstream's. The engine is not a performance concern: streaming 400 KB of
text through decoding and folding takes about 120 ms in total, and one streamed
update through the runtime costs 5 us on a 10-message thread and 70 us on a
1000-message thread (a 16 ms frame is 16,000 us). `cargo test --release -p
ai-ui-stream --test perf -- --ignored --nocapture` reproduces this.

**Structural gaps, in priority order, and how each was closed.**

1. *UI-agnostic state layer* -> `ai-ui-state` (`ThreadView`): scoped
   message, thread and composer state (`isLast`, branch number and count,
   `isCopied`, parts, tool presentation) and the scoped actions, with a row
   cache keyed by `Arc` identity and a caller-supplied clock. The example no
   longer reads the runtime.
2. *Runtime abstraction* -> the `ThreadRuntime` trait; UIs and `ThreadView`
   bind to it, so every runtime below works with the same UI.
3. *External-store runtime* -> `ExternalStoreRuntime` and the `ExternalStore`
   callbacks (`on_new`, `on_edit`, `on_reload`, `on_cancel`, ...). Messages the
   app re-sends unchanged keep their `Arc`.
4. *Thread list* -> `ThreadList` with a `ThreadListAdapter` (list, initialize,
   rename, archive, delete, titles, load); optimistic changes roll back on
   failure. The example has a thread bar with generated titles.
5. *Streaming cost* -> Markdown no longer re-parses from the start:
   `StreamSplit` + `StreamingCache` format settled blocks once and only the
   live tail per frame (equivalent to whole-document formatting, tested over
   every prefix of five documents). `run_stream` folds all slices that are
   ready into one cumulative result, so `result()` runs once per wake-up
   instead of once per slice (per-slice it was 135 ms in total for 400 KB).
6. *Tool UI* -> tools render through a generic disclosure by default, and an
   app registers a presenter per tool (`ThreadView::set_tool_presenter`) to
   choose title, summary and status. A fully custom Slint component per tool
   (generative UI) is not ported.

**Rust-specific choices worth keeping or revisiting.** Keep: executor-agnostic
futures, `Arc` message snapshots, lossless unknown JSON fields, no ambient
clock. Revisit: the runtime is `Rc`-based (single-threaded, like the UI thread;
multi-threaded agent servers would need a `Send` variant). The selected path is
now patched in place when uniquely held; retained snapshots preserve copy-on-write
behavior. A bounded repository change journal lets `ThreadView` update affected
rows while the shape/global state are stable. The optional runtime hook retains
a full-scan fallback for other runtimes. See [`PERFORMANCE.md`](PERFORMANCE.md)
for measurements and [`../AGENTS.md`](../AGENTS.md) for invariants to preserve.
