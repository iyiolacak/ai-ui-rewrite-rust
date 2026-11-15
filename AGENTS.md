# Guide for coding agents

Read `CONTRIBUTING.md`, `docs/STATUS.md`, and `docs/PERFORMANCE.md` before changing
runtime or rendering behavior. This repository is a Rust port of assistant-ui;
protocol and lifecycle parity are part of correctness. The upstream reference
and license attribution are recorded in `NOTICE`.

Before changing streaming text reveal, motion, or renderer-side GPU effects,
read `docs/STREAMING_MOTION.md`. It records verified shader mathematics and
an unimplemented proposal, with constraints on Unicode, accessibility,
cancellation, and frame-time measurement.

## Architecture and ownership

- `ai-ui-core`: message types, branching repository, executor-agnostic local and
  external-store runtimes, composers, tools, thread lists, smooth text.
- `ai-ui-stream`: assistant-transport, legacy data stream and UI message stream
  decoding, accumulation, cumulative runtime results.
- `ai-ui-sdk-stream`: a separate strict AI SDK 7 reducer checked against the
  published JavaScript package. Do not casually merge its semantics with the
  compatibility-oriented decoder in `ai-ui-stream`.
- `ai-ui-data-stream` and `ai-ui-http`: model request conversion and HTTP I/O.
- `ai-ui-state`: toolkit-independent projection (`ThreadView`) and scoped actions.
- `ai-ui-markdown` and `ai-ui-markdown-slint`: incremental splitting, bounded
  scheduling, worker pool, formatting and Slint model caches.
- `ai-ui-slint`: controlled components; `examples/thread` is their host and demo.
  Application-specific caching/paging policy belongs in the host.

Keep the core free of UI, I/O, ambient clocks and randomness. The UI/runtime
uses `Rc` and local futures deliberately; immutable message snapshots use `Arc`.
Do not add locks or a Tokio dependency merely to make a synchronous hot path
look concurrent. The Markdown worker pool is the existing CPU-parallel boundary.

## Performance contracts

### Message snapshots and change cursors

`MessageRepository::message_changes_since` keeps at most 64 content revisions,
containing indices rather than old message bodies. Each `ThreadView` owns its
cursor. A read never consumes another view's changes. An absent cursor, an
expired cursor or a structural change returns `indices: None` and requires a
complete read. `Some([])` means there are no content changes.

- Only record selected-path updates against a materialized path. When membership
  is unknown, invalidate the journal. Off-branch content updates do not dirty the
  selected rows; branch selection itself invalidates the shape and journal.
- Every structural mutation must call the repository's invalidation path.
  Reset cursors on runtime rebind, even if the new runtime has the same IDs and
  version numbers. Counter comparisons use wrapping distances.
- A custom runtime may opt in through the defaulted `ThreadRuntime` method.
  Its indices must cover every changed selected message and be valid for its
  current `messages()` snapshot. Returning `None` preserves the full-scan path.
- Running state, capabilities, tool presenters and UI-local stale rows affect
  projection independently of content revisions. Preserve those invalidations.
- `Arc::get_mut` allows in-place updates only when a snapshot has no other
  owners. Retained snapshots must remain immutable. Do not use raw-pointer
  mutation to avoid the necessary copy/full projection.

`ThreadView::state_changes_since` has a separate, view-scoped output cursor for
each UI consumer. Plain `state()`/`needs_frame()` reads must not consume those
changes. Its journal also retains at most 64 revisions; structural/global/full
projection is a barrier. Hosts reconcile all rows on `indices: None`, and update
thread-level fields even for `Some([])`. Keep host-only invalidations (Markdown
completion, page changes) separate from runtime changes. Track row identity
with `Weak<MessageState>` (it reserves the address without keeping the row's
contents); strong row `Arc`s keep a second copy of every row alive across full
reconciliations. Retaining the parent `Arc<ThreadState>` forces the
immutable-snapshot fallback. Reset host cursors and presentation caches when
switching conversations.

### Streaming and cooperative scheduling

The 64-input-slice budget in `results.rs` must yield even when input consists of
heartbeats, ignored events or incomplete frames. Returning a ready result is
not itself a yield: an eager `.next().await` consumer can poll again immediately.
After a full published batch, the following poll returns `Pending` and wakes
the task. This protects input, paint and cancellation scheduling.

Coalesce cumulative snapshots, never semantic protocol events. Borrow complete
SSE lines and reuse scratch buffers for fragmented lines; fragments can split
UTF-8 and CRLF. Keep byte limits, `[DONE]`, error ordering and the separate
decoders' EOF behavior intact. For UI text deltas, presence of `textDelta` takes
precedence over `delta`, including null/non-string values. Preserve explicit-ID
and active-part routing, reasoning, and provider metadata.

### Large user messages

`examples/thread/src/paging.rs` exposes verbatim UTF-8 pages for user text over
64 KiB or with at least 1000 newlines. Pages are bounded to 16 KiB and 400 newlines;
boundaries prefer existing whitespace and keep CRLF together. Full source stays
in the runtime for model input, copy and edit. Ordinary messages remain inline.

The page cache retains source `Arc` identity and byte ranges, not another full
text copy. Replacing a message releases all old part entries. Navigation validates
conversation revision, message/part, source identity, page token, displayed page
and direction. Do not accept callbacks from stale rows or use displayed text as
the authoritative source. Thread switching clears presentation caches.

## Verification and measurement

Use the locked dependencies and each crate's declared MSRV. Core/state/stream
and transport crates support Rust 1.85; UI dependencies have a higher minimum.
See the CI matrix rather than raising versions to make a convenience API compile.

```sh
bash scripts/gate.sh --quick
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

The full gate also checks AI SDK interop and wasm. Slint needs fontconfig
development headers. Render behavior changes in an offscreen test and inspect
the resulting image. Software-renderer screenshots do not establish GPU Markdown
styling fidelity; `docs/STATUS.md` explains that limitation.

Measure release builds, using the same toolchain, workload and profile for both
revisions. Debug builds include repository invariant walks and are unsuitable
for performance claims. Warm up, run interleaved pairs on a quiet machine, and
keep raw samples. Separate decoding, snapshots, state projection, layout/paint,
and whole-process CPU; a faster parser is not an end-to-end frame measurement.

The stream harness previously used `Vec::remove(0)` to simulate reads, adding
quadratic test overhead. Use `IntoIter` or `VecDeque`; port the corrected harness
to both revisions before comparing. Historical results made with the old
harness are not production speedup evidence.

Store accepted measurements, rejected approaches, remaining limits and exact
reproduction commands in `docs/PERFORMANCE.md` and its linked benchmark records.
Do not add machine-specific compiler flags, unsafe code or extra dependencies
without evidence that the real bottleneck warrants them.
