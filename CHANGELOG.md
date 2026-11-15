# Changelog

All notable changes are listed here. The project follows [Semantic Versioning](https://semver.org/);
before 1.0, minor versions may break the API.

## 0.1.0 (2026-11-15)

First crates.io release of the eight runtime crates.

Naming: the repository is `ai-ui-rewrite-rust` and the crates use the `ai-ui-` prefix (they were `assistant-ui-*` while the
project was a working copy inside another repository). The AI SDK stream crate is `ai-ui-sdk-stream`. Environment
variables for tests are `AI_UI_*`.

First public version, ported from assistant-ui `eb65bd4` (MIT).

- `ai-ui-core`: message model, branching repository, `ChatModelAdapter`, `ThreadRuntime` trait with local
  and external-store runtimes, `ThreadList`, tools with approvals, composers, smooth streaming.
- `ai-ui-stream`: chunks, accumulator, partial JSON, `assistant-transport`, AI SDK UI message stream and
  data stream decoders; ready slices are coalesced into one result per wake-up.
- `ai-ui-data-stream`, `ai-ui-http`: AI SDK backend adapter with native and browser HTTP clients.
- `ai-ui-state`: UI-agnostic thread, message and composer state and actions.
- `ai-ui-markdown`: bounded Markdown parsing and projection, incremental block splitter.
- `ai-ui-clipboard`: result-aware clipboard for native and browser.
- Slint UI crates (`ai-ui-slint`, `ai-ui-markdown-slint`) and the `examples/thread` app; not part
  of the first crates.io release.
- Web demo package: `pixel-snap.js` lays the canvas out on whole device pixels, which keeps text sharp in Firefox on
  fractional-scale displays and fractional layouts; see `docs/WEB.md`.

### API notes for 0.1.0

- `ThreadRuntime::message_changes_since` is an optional, defaulted change-journal
  hook. `LocalThreadRuntime` implements it; `ThreadView` uses it to avoid scanning
  unchanged history. Retained snapshots, missed-history and structural/global
  changes keep the full-read fallback.
- Ready stream input yields cooperatively after bounded batches, including
  heartbeat-only and partial-frame reads. SSE parsing and common text/reasoning
  deltas avoid redundant allocations and JSON clones.
- The thread example pages unusually large user messages while retaining full
  source for copy/edit/send. Navigation rejects stale source/thread/page actions.
- Benchmark evidence and agent handoff guidance are in `docs/PERFORMANCE.md` and
  `AGENTS.md`; the simulated-read benchmark's quadratic `Vec::remove(0)` overhead
  has been corrected on both sides of the recorded comparisons.

- Error enums (`RuntimeError`, `RepositoryError`, `RunError`, the decoder and parser errors) and the protocol enums
  that follow upstream's evolving wire formats (`ChunkKind`, `PartInit`, `FinishReason`, `Operation`, `Protocol`) are
  `#[non_exhaustive]`: matching them needs a wildcard arm, and new variants are not breaking.
- `ai_ui_stream::AccumulatorLimits` is the root name of the accumulator's limits (the module path is
  `accumulator::Limits`, beside the other modules' own `Limits`).
- Message, part and status types mirror upstream's JSON and are deliberately exhaustive.

