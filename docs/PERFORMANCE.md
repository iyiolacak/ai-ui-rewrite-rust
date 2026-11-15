# Performance engineering record

This document records measured changes, their correctness contracts, and the
remaining costs. `AGENTS.md` is the short guide for future coding agents.

## 2026-11-14: O(1) streaming tail — repository overlay + view fast path

Identified as "the next target" in the 2026-11-11 section below
(`2.9 → 21.3 µs` per update, 1k → 10k). Two photocopy machines remained:

1. **View side:** `ThreadView::build_into` called `runtime.messages()` on every
   notification, cloning `N` `Arc`s per token even when the journal said only
   the tail changed.
2. **Repository side:** `replace_in_path` fell back to `path.to_vec()` (N `Arc`
   clones) whenever `Arc::get_mut(path)` failed — i.e. whenever any snapshot
   was retained.

### What changed

- `ThreadRuntime` gains defaulted `message_at(i)` / `len_selected()` /
  `begin_overlay()` / `end_overlay()` (no-ops by default; safe for custom
  runtimes). `LocalThreadRuntime` and `ExternalStoreRuntime` override to
  hit `MessageRepository` directly.
- `MessageRepository` gains a tail **overlay** slice. `begin_overlay()` clones
  the cached prefix once per run; `replace_in_path` patches the overlay via
  `Arc::get_mut` when no snapshot is retained → O(1) per token. A retained
  prior snapshot is never mutated: the overlay is dropped and the next
  `messages()` rebuilds from `nodes`. Structural changes clear it via `dirty`.
- `local/run.rs`: `begin_overlay()` at step start, `end_overlay()` at step end.
- `ThreadView::build_into` fast lane: when shape/capabilities/running are
  unchanged and the journal yields `indices: Some`, project only those indices
  via `message_at` (including UI-stale ids). Otherwise materialize once.
  Smooth sync needs only the last message.
- New tests: `repository::tests::overlay_keeps_streaming_updates_o1_without_stale_snapshots`
  and `message_at`/`len_selected` assertions in the journal test.

### Validation

- `bash scripts/gate.sh --quick`: **gate passed**
- `cargo test -p ai-ui-state --test view`: **19/19** (run-end `None`, retained
  snapshots, smooth ticks, rebind barriers)
- `cargo test -p ai-ui-core --lib`: **38/38**

### Measured (busy box, load 4–8 — absolutes noisy, ratios indicative)

Release, `projection_cost_per_streamed_update`, 1000 updates, this machine:

| History | Before | After (this change alone) |
|---|---|---|
| 1k, smooth=false | 12.3 µs | 5.0 µs |
| 5k, smooth=false | 33.7 µs | 17.9 µs |
| 5k, smooth=true | 21.0 µs | 15.1 µs |

`host_reconciliation` feed median moved 31.7 → 24.9 µs at 10k (same noisy box;
not claimed as proof — see interleaved-pair caveat in earlier sections).

Why not flat yet: the remaining linear term is **content copying in the reply**
— `with_continuation` / `with_external_results` in `local/run.rs` and
`accumulator::result()` in `ai-ui-stream` clone the growing reply per token.
That is intentionally untouched: it changes adapter/snapshot contracts.

Reproduce:
```sh
cargo test --release -p ai-ui-state --test perf -- --ignored --nocapture projection_cost_per_streamed_update
bash scripts/gate.sh --quick
```

---

## 2026-11-11: projected changes reach the host

Base: `173765d8ac094941b85abe5ad241f7cfe6085370` on `main`. Accepted candidate:
`437fd45` (PR #3). Implementation and first source review were AI-assisted in an
environment without a Rust toolchain; compilation, tests, review, the memory fix
below and all measurements were done afterwards on a local machine. Raw samples,
system metadata and commands: [`benchmarks/performance-2026-11-11`](../benchmarks/performance-2026-11-11/README.md).
No earlier timing, React comparison or memory result is replaced by this work.

### Host reconciliation

`ThreadView::state_changes_since` exposes an independent cursor for each UI
consumer. It returns the full immutable snapshot and either sorted changed-row
indices or a full-reconciliation instruction. A second bounded journal retains
64 projected revisions, containing only indices. Ordinary `state()` and
`needs_frame()` reads do not consume another consumer's changes. Foreign or
expired cursors, structural/global changes, rebinds, unsupported runtime journals
and retained parent snapshots preserve the conservative full path.

`examples/thread/src/app.rs` reconciles those indices instead of comparing every
historical row on each steady-state update. Markdown completions use an
ID-to-row index and a separate local dirty list; page navigation uses that same
local invalidation path. The index is rebuilt on a structural change. Thread
switching resets the cursor, local dirty rows and presentation caches.

The host holds a `std::sync::Weak<MessageState>` per row and compares
allocation addresses. A live `Weak` keeps the allocation reserved, so a recycled
address cannot be mistaken for an unchanged row after missed updates, while the
row's contents are freed when the view replaces it. The first candidate held
strong `Arc`s instead: a full reconciliation (run start/end) then kept a second
copy of every row alive while the view rebuilt them, adding about 0.9 KB of peak
RSS per message (+3.7 MiB at 4,000 messages). That was rejected; see Memory below.

### Cooperative work and cancellation

The offscreen `Tasks::run` helper uses a deque and polls each task present at
entry at most once per call. Pending tasks and newly spawned tasks wait for the
next event-loop tick; the queue is not borrowed while polling. This prevents the
old until-empty loop from spinning on a pending future. An individual future
must still cooperate; this is not a time limit on its `poll` implementation.
Windowed execution continues to use Slint's executor.

The local runtime constructs its run drop guard before returning the future.
Dropping an unpolled future cancels its matching run, and dropping pending
adapter/tool work aborts the matching controller after releasing the state
borrow. A stale future cannot cancel its replacement. This is a resource-lifetime
fix; it carries no throughput claim.

### Validation

- `bash scripts/gate.sh` (fmt, Clippy `-D warnings`, docs, AI SDK interop, wasm32,
  workspace tests): passed, 61 test binaries. The candidate needed a rustfmt pass
  and moving `task_tests` after `main` (`clippy::items_after_test_module`).
- `cargo +1.85.0 test --locked -p ai-ui-core -p ai-ui-state`: passed (MSRV).
- Software-rendered screenshots of six states (empty, conversation, expanded
  reasoning/tools, a completed ask, an approval, a new thread) are
  pixel-identical between base and candidate. A real GPU window was not checked.

### Measurements

Host traversal in isolation (`perf.rs`, candidate state crate in both modes,
1,000 cumulative updates, 9 interleaved pairs, median µs per update, no rendering):

| History | Full host scan | Changed-row feed | Row entries examined |
|---|---|---|---|
| 1,000 | 4.14 | 2.91 | 1,006,008 → 3,006 |
| 10,000 | 31.39 | 21.30 | 10,042,008 → 21,006 |

The feed removes the host scan, but the remaining per-update cost still grows
about linearly with history (2.9 → 21.3 µs for 10× the messages). That cost is in
the runtime snapshot and state projection, not in the host.

Whole application, release, base vs candidate, 1 discarded warmup then 5
interleaved pairs per scenario. The machine was busy (load average 4–8); times
drifted by up to 35% within a session, so the figure is the median of per-pair
candidate/base ratios, with the range over pairs:

| Scenario | App work total | p95 iteration | Process CPU | Opening | Peak RSS |
|---|---|---|---|---|---|
| 1,000 messages, 9 KB reply, app work only | **0.72** [0.64–0.80] | **0.63** [0.55–0.71] | **0.79** [0.69–0.86] | 0.98 | 1.00 |
| 1,000 messages, 9 KB reply, layout + software paint | **0.95** [0.83–1.00] | 0.95 [0.82–1.01] | 0.96 [0.84–0.99] | 0.98 | 1.00 |
| 200 messages, 40 KB reply, layout + software paint | 1.00 [0.91–1.17] | 1.02 [0.91–1.20] | 1.00 [0.90–1.16] | 1.00 | 1.01 |

Every pair of the 1,000-message scenarios was at least as fast with the
candidate. The short-thread scenario is neutral, as expected: it has little
history to scan. An earlier session (before the memory fix, same binaries
otherwise) gave 0.63 and 0.87 for the first two rows; the gain is real but its
size is load-sensitive. Layout and paint dominate rendered iterations and are
unchanged by this work.

### Memory

RSS after opening a thread and at the end of a short run, mean of 3 runs per side
(`rss-by-size.txt`):

| Messages | Base open → end | Candidate open → end |
|---|---|---|
| 200 | 17.4 → 17.8 MiB | 17.5 → 17.9 MiB |
| 1,000 | 22.8 → 23.2 MiB | 23.0 → 23.6 MiB |
| 4,000 | 34.6 → 35.3 MiB | 35.0 → 36.2 MiB |

The remaining cost is about 0.2 KB per message (the reserved `Weak` allocations
and the ID index). With strong `Arc`s it was +3.7 MiB at 4,000 messages.

Two measurement hazards found during this work:

- **Binary placement changes RSS.** The same executable, copied to tmpfs at
  different times, showed up to 4 MiB difference in RSS right after opening, all of
  it file-backed pages of the executable itself; anonymous memory was identical
  (2,432 vs 2,436 kB). Copy both A/B binaries at the same moment, or compare RSS
  growth after opening rather than absolute values.
- **A shared target directory can mix revisions.** Building base in a worktree
  and then the candidate with the same `CARGO_TARGET_DIR` reused the base's
  `ai-ui-state` (the candidate failed to compile against it). Cargo judges path
  dependencies fresh by mtime, so a compatible API could link a mixed binary
  silently. `scripts/ab-bench.sh` only touches the Slint crates; build each side
  in its own target directory until that script is fixed.

## 2026-11-10: incremental projection, stream scheduling, large prompts

Baseline: `54c371fa44ad193b6ae8466d00c0ea0f4e8af50b` on `main`.
Candidate production source: `e9219baa25173b3240bd534f965b0d92b2721a1b`
(subsequent commits only complete this report and its measurement metadata).
Environment: Linux x86_64, Rust 1.99.1, the existing release profile
(`lto = true`, `codegen-units = 1`). Same machine and toolchain for each pair;
no dependency, compiler-profile or CPU-specific instruction changes.

### Runtime and state projection

The repository already patched uniquely held selected-path snapshots in place,
but `ThreadView` still compared every historical row after every content update.
A bounded repository journal now reports changed selected-message indices. The
view merges those with UI-local changes and projects only affected rows when
its cached output is uniquely held and the global state/shape are unchanged.

The journal retains 64 revisions, not message bodies. Every consumer has an
independent cursor; missed notifications do not lose updates. Structural changes,
expired history, unknown runtimes, retained output snapshots, running/capability
changes and a presenter replacement all preserve a full-projection fallback.
Runtime rebinding discards both version cursors. The trait addition has a default,
so existing custom runtimes continue to work without implementing a journal.

One warmup per executable, then five interleaved before/after pairs. Each
scenario sends 1,000 cumulative six-byte text updates; the benchmark reads the
view on every notification. Median reported microseconds per update:

| Existing messages | Smooth text | Before | After | Ratio |
|---:|:---:|---:|---:|---:|
| 10 | Off | 2.6 | 2.6 | 1.00x |
| 100 | Off | 1.9 | 1.3 | 1.46x |
| 1,000 | Off | 9.2 | 2.2 | 4.18x |
| 5,000 | Off | 40.4 | 7.1 | 5.69x |
| 10 | On | 1.3 | 1.1 | 1.18x |
| 100 | On | 1.9 | 1.2 | 1.58x |
| 1,000 | On | 8.5 | 1.7 | 5.00x |
| 5,000 | On | 38.4 | 4.7 | 8.17x |

The denominator is the number of streamed updates, but timing includes send,
run-start and run-end projections. Those global transitions still visit the
history. This is a runtime-plus-projection workload, not frame latency, GPU
rendering or network latency. The ten-message unsmoothed case is unchanged.
Numbers are rounded by the existing harness to 0.1 microseconds.

Raw samples and warmups: [`state.json`](../benchmarks/performance-2026-11-10/state.json).

```sh
cargo test --locked --release -p ai-ui-state --test perf \
  projection_cost_per_streamed_update -- --ignored --nocapture
```

### Stream decoding and executor fairness

Complete UI SSE lines now borrow incoming bytes. Fragmented lines reuse their
scratch allocation. Assistant-transport reuses its line/event buffers and moves
its owned JSON tree into typed deserialization instead of deep-cloning it.
Text/reasoning deltas avoid the normalization vector and redundant string copies.

The 64-read batch budget previously only returned when it had visible output.
Heartbeat-only, ignored or partial-frame reads could therefore monopolize the
executor. The budget now returns `Pending` with a wake in those cases. After a
full content batch, the next poll also yields so an eager consumer cannot keep
looping in the same task. Every semantic event is still decoded in order.

Five interleaved pairs, with each test invocation reporting a median of nine
repetitions. Workload: 10,000 deltas producing 240,000 bytes of text.

| Workload | Read size | Before | After | Less time |
|---|---:|---:|---:|---:|
| UI decoder + accumulator | 4,096 bytes | 7.975 ms | 4.725 ms | 40.8% |
| UI decoder + accumulator | 64 bytes | 9.384 ms | 4.832 ms | 48.5% |
| UI decoder + accumulator | 1 byte | 16.654 ms | 12.879 ms | 22.7% |
| Assistant transport + accumulator | 4,096 bytes | 12.062 ms | 9.589 ms | 20.5% |
| Assistant transport + accumulator | 64 bytes | 12.123 ms | 8.360 ms | 31.0% |
| Assistant transport + accumulator | 1 byte | 29.548 ms | 27.776 ms | 6.0% |
| Full `run_stream` | 4,096 bytes | 7.95 ms | 5.06 ms | 36.4% |
| Full `run_stream` | One event/read | 8.81 ms | 5.47 ms | 37.9% |

The full-stream cases returned the same 3 and 157 cumulative snapshots,
respectively. The isolated decoder test excludes simulated transport polling
and cumulative snapshot creation. It verifies the final accumulated text length.

**Benchmark correction:** the old full-stream test used `Vec::remove(0)` for
every simulated network read, adding quadratic vector-shifting cost. Both
before/after executables above use the corrected `IntoIter` harness. Older
figures from that harness in `docs/STATUS.md` cannot be compared directly or
attributed to the production-code improvement. The new isolated decoder test
was also present in both benchmark builds before production changes were made.

Raw samples, per-run output and method:
[`stream.json`](../benchmarks/performance-2026-11-10/stream.json).

```sh
cargo test --locked --release -p ai-ui-stream --test perf \
  protocol_decode_and_fold_throughput -- --ignored --nocapture
cargo test --locked --release -p ai-ui-stream --test perf \
  ui_stream_text_workload_like_the_ai_sdk_harness -- --ignored --nocapture
```

For a historical comparison, put the candidate's `tests/perf.rs` in the baseline
worktree before building it. This applies the same measurement harness to both
revisions. Build each executable once, warm it, then alternate executable runs
without concurrent compilation or other benchmarks.

### Large-message presentation

The example previously passed an entire user message to one Slint `Text`.
It now uses the existing controlled `AssistantPagedText` component for content
over 64 KiB or with at least 1000 newlines (16 KiB and 160 before 2026-11-10: ordinary pasted logs were split; upstream shows a message whole). Only the selected verbatim page enters
the Slint model, bounded to 16 KiB and 400 newlines. The cache computes byte ranges
once per source identity. It keeps Unicode boundaries and CRLF intact and
prefers line/word boundaries when possible.

The runtime still owns the complete message. Copy, editing and model requests
use that source, not the currently displayed page. Page state resets on source
replacement and thread switching. Callbacks are fenced by conversation, source,
part, page and token; the control captures identity when the gesture begins.
Replacing a multipart source releases cached pages for removed parts as well.

The example integration test renders a 740,000-byte, 20,000-line user message,
checks bounded page data, navigates, rejects stale callbacks, switches threads
and verifies full-source copy/edit. Its software-rendered screenshot was
inspected: page navigation, message actions and composer remain visible. The
standalone tests also reconstruct every byte of 512 KiB unbroken text and test
Unicode, combining characters, CRLF, dense newlines and ordinary messages.

The ignored `bench_large_user_layout` test compares the old raw-text policy and
the new paged policy in the same 900x760 software-rendered window. It is a layout
microbenchmark, separate from source import, page-range construction and runtime
projection. Do not treat it as a whole-application speedup.

With a 32,742-byte user message in the release build, one warmup per policy and
five interleaved first-layout/paint pairs measured **9.589 ms raw vs 5.554 ms
paged** (1.73x, 42.1% less time). The policy is the only varied input to the same
window. The summary and reproduction command are in
[`layout.json`](../benchmarks/performance-2026-11-10/layout.json); this test prints
the median, so individual layout samples were not retained. A prior attempted
512 KiB raw-text comparison hit a software-renderer integer-overflow panic in
the debug build; it was excluded from timing claims. The 740,000-byte paged
integration test succeeds.

```sh
cargo test --locked -p ai-ui-example-thread \
  large_user_pages_render_navigate_and_keep_full_source_for_copy_and_edit
cargo test --locked --release -p ai-ui-example-thread \
  bench_large_user_layout -- --ignored --nocapture
cargo test --locked -p ai-ui-slint --test paged_text
```

### Settings: text size, reading width, theme

The example's Settings row (sidebar footer) and Ctrl+= / Ctrl+- / Ctrl+0 change
`AssistantTheme.text-scale`, one global that multiplies the font sizes of message
text, Markdown, code, reasoning (header and body), tool calls, approvals, source
and attachment cards, buttons, the branch picker and the action row; icons in
scaled rows scale with the text, and hit targets grow with it but never fall
below their 28-32 px size (`AssistantTheme.hit-scale = max(1, text-scale)`). Nothing on the Rust side is involved in a
change: Markdown blocks are `StyledText` without sizes and the Markdown cache is
keyed by message and part, so no text is re-parsed or re-formatted. Slint
re-evaluates only the rows the `ListView` has instantiated. The reading width is
one `column-width` property on the same path. Reading width options are Compact 576,
Default 704, Wide 880, Extra wide 1100 and Full (the whole main area with 40 px side
padding); the pixel width is multiplied by the text scale (Default at 1.3 is about
915 px) and the layout clamps it to the available area. The settings file stores
`reading_width` as `compact|default|wide|xwide|full` (old files load unchanged), and
`--reading-width` takes the same names. The popover scrolls when the window is shorter
than its 420 px.

Re-measured after the scaling of reasoning, tool, action-row and width changes
(release, software renderer, 900x760, load 2-12, two interleaved pairs against the
`origin/main` binary): 20,000 messages, scale and width changes cost 0.02-0.03 ms to set and
1.2-1.5 ms layout and paint on both sides, 1,000 messages 1.4-1.7 ms; RSS 97.4-97.7 MiB
against 96.9 MiB. Streaming (`--bench 1000 --bench-reply-kb 9 --bench-render`): process
CPU 6.0 and 6.8 s against 7.0 and 6.2 s, p95 1.21 and 1.45 ms against 1.49 and 1.32 ms,
which two pairs on a loaded machine cannot separate.

Measured 2026-11-10, release, software renderer, 900x760, 20,000 messages
(`--bench 20000 --bench-render --bench-settings`): setting the property costs
0.05-0.10 ms and the layout and paint of the next frame 3.8-4.4 ms (text scale
1.0 to 1.3 and back, 1.3 to 0.9) and 3.8-3.9 ms (width 704 to 576 px and back);
RSS stays at 97.5 MiB. The cost does not grow with thread length because only
visible rows relayout.

Against origin/main (0c255ad), interleaved pairs, `--bench 20000 --bench-render`
open time (import and first sync), median of 6 pairs on a quieter machine: 169.6
ms before, 166.6 ms after. At 1,000 messages: 7.6 ms and 8.9 ms (one run of each
side 17-32 ms, so the machine was not quiet). Streaming scenarios
(`--bench 1000 --bench-reply-kb 9 --bench-render`, 3 pairs, load 7): process CPU
12.6 s before, 13.1 s after; p95 per iteration 2.77 and 2.84 ms; layout and paint
per frame (first 10 s) 1.73 and 2.70 ms with a spread of 1.7-3.0 and 2.2-3.3 ms,
which these three pairs cannot separate. `--bench 20000` streaming is bimodal
between runs (frames drawn 135-143, p50 18 us to 365 us) on both sides.

The software renderer draws glyphs pre-rendered at the font sizes it finds in
the source; a size computed at run time snaps to the nearest embedded one. The
example's `build.rs` therefore embeds the sizes the four scales produce
(`SLINT_FONT_SIZES`, software embedding only; 18 px was added for 14 px text at 1.3), which adds about 1.4 MB to the
binary. GPU renderers (`--features gpu-window`) draw any size from the font file.

Reproduce: build `origin/main` and the branch in release, then
`--bench 20000 --bench-render` (branch only: add `--bench-settings`) interleaved.

### Regression and compatibility gates

The full local workspace checks passed: formatting, Clippy for all targets with
warnings denied, and **305 tests/doctests passed, zero failed, seven ignored**
across 56 suites. The ignores are six manual performance tests and one existing
illustrative doctest; the release benchmarks described above ran separately.
The runtime quick gate also passed documentation with warnings denied and its
280 tests. Rust 1.85 checks for core/state/stream/SDK-stream/HTTP/data-stream and
the CI wasm package set passed.

The pinned AI SDK oracle passed 45/45 stored fixtures (150 emissions) with
`npm run verify`. The native loopback HTTP interop test passed with
`AI_UI_REQUIRE_INTEROP=1`, so it could not silently skip: reasoning, a server tool
call/result, a second model step and a follow-up history replay were exercised.
It uses a mock provider; live authentication/retries and the separate expanded
`differential.mjs` suite were not part of these checks.

Hosted CI did not execute during this validation. In
[Actions run 42752889486](https://github.com/iyiolacak/ai-ui-rewrite-rust/actions/runs/42752889486),
all nine jobs failed before any steps started; no job logs were available. The
connector did not expose a diagnostic for the cause. Local passing gates do not
turn that hosted check green. The remaining hosted matrix, supply-chain and
packaging jobs still need a functioning runner.

Behavior regressions cover journal overflow/rollover, independent consumers,
coalesced updates, old immutable snapshots, older-message feedback, copy state,
same-version rebinds and nested notifications. Stream tests cover every split
width of a Unicode/CRLF fixture, EOF, byte limits, metadata, alias precedence,
part routing and cooperative progress with/without visible output.

- ~~The view-to-host change feed (2026-11-11) removed the host's historical-row
  scan, but per-update cost still grows about linearly with history in the
  runtime snapshot and state projection (2.9 → 21.3 µs per update from 1,000 to
  10,000 messages in `perf.rs`). That is the next target for long threads.
  Slint's row/layout behavior and any virtualization experiment are unchanged.~~
  Done in 2026-11-14: repository tail overlay + view fast path. See the new
  section above. The remaining linear term is content copying in the reply.

### Remaining costs and next experiments (updated)

- Cumulative message bodies and per-row presentation still grow with reply
  length (`with_continuation` / `with_external_results` clone content per
  token, accumulator `result()` clones parts). An incremental part-update
  interface could reduce those copies, but must preserve adapter/snapshot
  contracts. This is the next target after the 2026-11-14 overlay work.
- Copying a retained immutable snapshot necessarily costs more than patching a
  uniquely held one. Run-start/end transitions and structural edits still scan.
- The scheduler budget counts input slices, not bytes or elapsed time. One
  arbitrarily large caller-provided slice can still take a long synchronous
  decode. Measure byte-bounded feeding separately before changing that API.
- Large user-message display is paged; the composer and edit surface are not
  newly virtualized. Retaining or editing a huge source still has its own cost.
- GPU visual fidelity and whole-process application CPU require their own
  measurements. The results above do not replace the React comparison or prove
  a universal speed ratio across hardware and workloads.
- The offscreen task pump is bounded (2026-11-11) and covered by regression
  tests. Real windows use the Slint executor. Stream fairness tests do not
  establish this helper's scheduling behavior.
- `scripts/ab-bench.sh` builds both revisions in one target directory and can
  link a mixed binary; see the 2026-11-11 measurement hazards.

## Earlier accepted and rejected work

`docs/STATUS.md` records earlier improvements: incremental Markdown, a bounded
worker pool, O(1) append at the selected head, structural-version caching,
in-place Slint rows and chunked reasoning. Keep those mechanisms.

An earlier experiment with one Slint list row per Markdown block was reverted:
rendering stayed within noise while application work increased because blocks
were re-compared on each update. Do not repeat that approach without a different
change-propagation model and an interleaved end-to-end measurement.
