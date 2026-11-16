# Port status

Reference: assistant-ui `eb65bd4` (MIT). "Ported" means upstream's behaviour,
checked by tests derived from upstream's; gaps are listed, not implied.

## Ported

| Upstream | Crate | Notes |
|---|---|---|
| `@assistant-ui/core` types, `MessageRepository` | `ai-ui-core` | Upstream JSON shape; unknown fields round-trip. |
| `ChatModelAdapter`, local thread runtime | `ai-ui-core` | Append, edit, regenerate, branches, cancel, tool-result continuation, `maxSteps`, human tools, feedback, export/import. |
| Thread and edit composers | `ai-ui-core` | Send, cancel, edit branching. No attachment adapter or dictation yet. |
| `assistant-stream`: chunks, accumulator, state operations, partial JSON, `assistant-transport` SSE | `ai-ui-stream` | Timing metadata not computed. |
| AI SDK UI message stream and data stream decoders | `ai-ui-stream` | Both protocols, including assistant-ui's `aui-*` data-stream extensions. |
| `@assistant-ui/react-data-stream` | `ai-ui-data-stream` | Request conversion (`toModelMessages`), protocol detection, error statuses (without the response text). |
| `useSmooth` smooth streaming | `ai-ui-core` (`SmoothText`) | Same pacing as upstream; the example reveals text per frame, including the working indicator, stop button and hidden actions while generating. `--screenshot PATH --mid-stream MS` captures a partial reply offscreen. |
| Frontend tools and approvals | `ai-ui-core` (`Tool`, `ToolRegistry`) | Tools with `execute` run in the app after each model step and their results feed the next step. `requires_approval` pauses the run with a pending approval; `respond_to_tool_approval` resumes it (approve runs the tool, deny answers the model with an error). Cancelling aborts a running tool. The example has `get_weather` (auto) and `send_email` (asks first); try `--ask "Send an email to the team" [--decide allow|deny]`. Tools run sequentially; per-tool presentation through a presenter, no custom component UI. |
| HTTP | `ai-ui-http` | Native (ureq + rustls) client tested over loopback sockets; browser `fetch` client is exercised in headless Chromium (`crates/ai-ui-http/tests/browser/run.py`: UI message stream and data stream over a slow loopback server, an HTTP 500, and cancelling aborts the request; Chromium only, no live provider). |
| `ThreadRuntime`, `useExternalStoreRuntime` | `ai-ui-core` | One runtime trait for local, external-store and thread-list threads. External store: app-owned messages, edit/reload/cancel/tool/feedback callbacks, `Arc` identity kept for unchanged messages. |
| Thread list, `useRemoteThreadListRuntime` | `ai-ui-core` (`ThreadList`) | Switch, new, rename, archive, unarchive, delete, generated titles, load; optimistic with rollback. Host supplies the `Spawner`. |
| Scoped state (`MessageState`, `ThreadState`, `ComposerState`) | `ai-ui-state` | `ThreadView`: UI-agnostic projection and actions over any `ThreadRuntime`; caller-supplied clock for smooth streaming. |
| `@assistant-ui/react` thread look | `ai-ui-slint` | Thread, messages, composer, suggestions, reasoning and tool disclosure, branch picker, copy/edit/regenerate/feedback actions, Markdown. |
| `@assistant-ui/markdown` | `ai-ui-markdown`, `ai-ui-markdown-slint` | Parsed off the UI thread; formatting stays visible while newer text is pending. Streaming text is split into settled blocks (formatted once) and a live tail. |

The `examples/thread` app runs the thread with a built-in streaming demo model
or any AI SDK endpoint (`--api URL`). `--screenshot PATH [--conversation]`
renders offscreen.

## Visual fidelity

Offscreen screenshots (`--screenshot`) use Slint's software renderer, which
draws styled text as plain characters. In those images list bullets, bold and
inline code styling are therefore missing by construction; this says nothing
about the GPU renderers the real window uses. Fidelity of Markdown styling in
the real window is **unverified** until it is captured with a GPU renderer
(for example the browser build in headless Chromium, or a native window).

Fonts follow upstream's default thread: Inter (regular, medium and semibold, which bold text uses; subset to Latin
and common punctuation, other scripts fall back to system fonts) and IBM Plex Mono for code, both registered by
`theme.slint`. Slint resolves `font-family` as a literal name, so code names the bundled mono family explicitly.

## Not yet ported

- Runtimes: assistant-transport (server-owned state) as a runtime, a concrete HTTP remote thread-list adapter, LangGraph and other framework adapters.
- Custom per-tool components (generative UI), attachment, feedback, speech, dictation, suggestion and history adapters, message queue.
- Composer attachments with an adapter, quotes.

## Performance

Measured results against upstream: [`RESULTS.md`](RESULTS.md).

### Latest Rust improvements (2026-11-10)

[`PERFORMANCE.md`](PERFORMANCE.md) records the change-journal projection,
stream decoder allocation reductions, cooperative scheduling, and bounded
large-user-message display. On the recorded same-machine release workload,
runtime plus state projection with smoothing improved from 8.5 to 1.7 us/update
at 1,000 messages and from 38.4 to 4.7 us/update at 5,000 messages. UI decoding
plus accumulation of 10,000 deltas with 4 KB reads improved from 7.975 to
4.725 ms. Raw samples and reproduction commands are linked there.

The older full-stream harness used `Vec::remove(0)` for simulated network reads;
the new comparison corrects it on both revisions. Treat the historical numbers
below as records of their original harnesses, not directly comparable results.

### Against upstream React (default thread, not virtualised)

`benchmarks/react-baseline` runs upstream assistant-ui (the published `@assistant-ui/react` 0.15.33 and
`-react-markdown` 0.14.25 of our reference commit) on the same workload as `--bench-render`: the same history, the same
demo reply (reasoning, then the answer, one word every 18 ms) and the same 60 s window, in headless Chromium.
The React side is upstream's default thread, which renders every message into the DOM and does not virtualise the
list. Upstream documents a virtualised thread for long conversations; it has not been measured here. It would reduce
DOM size, open time and memory on long threads, so none of the ratios below, least of all the 10,000-message ones,
is a statement about virtualised React. The React page also runs on `useLocalRuntime`, which recomputes each message's
state twice per streamed update; upstream's templates use the AI SDK runtime (an external store), which does it once,
so the CPU and stall ratios likely overstate that part. A corrected rerun is in progress. `compare.sh` interleaves the two. Rust draws at most 60 frames a second, like a display. CPU is the whole process tree
(every thread and process); memory is resident memory (React: every Chromium process, RSS and PSS). The current numbers
are the 2026-11-11 re-run below (median of 5 interleaved pairs, raw files in
`benchmarks/results/like-for-like/raw`); the 2026-11-10 table after it is kept as history.

**Current, like for like (2026-11-11).** "Open" is the same end state on both sides: all messages shown with their Markdown
formatted. Rust: from the start of the import until the Markdown worker is idle and a frame has been painted (`open (formatted and
painted, like for like)` in `--bench`); React: navigation until every message and its Markdown table is in the DOM, plus two
frames (`open_formatted_ms`). Rust's first sync alone (rows on screen, Markdown not yet formatted) is a different, earlier
event and is not compared with React. Machine load about 4-6; ranges are min to max of the 5 runs.

| Workload | Measure | Rust (`ai-ui-slint`) | React (upstream default, not virtualised) | Ratio |
|---|---|---|---|---|
| 1000 messages, 9 KB prompt | open, like for like | 105 ms (101-151); first sync alone 12.9 ms | 2.94 s (2.79-2.99) | 27.9x (18.5x-29.6x) |
| | CPU while streaming (60 s) | 17.2 s (13.7-17.7) | 73.3 s (73.1-73.5) | 4.2x |
| | memory at the end | 30 MiB | 1481 MiB RSS, 850 MiB PSS (9 processes) | 49x RSS, 28x PSS |
| | longest stall | 34 ms (17-35) | 917 ms (867-1183) | 26.9x |
| 200 messages, 40 KB prompt | open, like for like | 33 ms (32-47); first sync alone 2.1 ms | 1.10 s (1.03-1.20) | 33.2x |
| | CPU while streaming (60 s) | 17.0 s (13.7-19.5) | 51.2 s (41.9-57.1) | 3.0x |
| | memory at the end | 25 MiB | 1213 MiB RSS, 584 MiB PSS | 49x RSS, 23x PSS |
| | longest stall | 110 ms (36-136) | 2.40 s (2.18-2.65) | 21.8x |
| 10,000 messages, 9 KB prompt | open, like for like | 436 ms (383-472); first sync alone 148 ms | 26.1 s (23.6-27.2) | 59.8x |
| | CPU while streaming (60 s) | 22.8 s (20.9-23.2) | 83.5 s (76.1-84.7) | 3.6x |
| | memory at the end | 64 MiB | 3718 MiB RSS, 3089 MiB PSS | 58x RSS, 48x PSS |
| | longest stall | 41 ms (28-46) | 11.6 s (9.8-12.9) | 282x |

Correction. The earlier table headlined "open the thread 0.12 s" for Rust (24x against React's 2.9 s). That Rust figure was the
time before Markdown was formatted, while React's 2.9 s ends with Markdown rendered, so it was not like for like; the same table
said "Markdown formatted by 0.53 s" (5.4x). Since then pooled Markdown workers cut formatting from 0.52 s to 0.09-0.11 s, and the
measured like-for-like ratio against the default unvirtualised thread is 27.9x in this run and 16.6x to 27.9x across the four runs of 2026-11-11 ([RUNS.md](RUNS.md)). At 10,000 messages React's page survived all 5 runs here (the 2026-11-10 failure was not
reproduced). Ratios are floored by `scripts/results.py` (see [RESULTS.md](RESULTS.md)).

History: 2026-11-10, median of 3 interleaved pairs on a shared machine (load 10-21), raw files not kept; superseded by the table
above:

| Workload | Measure | Rust (`ai-ui-slint`) | React (upstream default, not virtualised) |
|---|---|---|---|
| 1000 messages, 9 KB prompt | CPU while streaming (60 s) | 18.8 s | 68.1 s (main thread 59.8 s, saturated) |
| | open the thread | 0.12 s before Markdown was formatted (not comparable), formatted by 0.53 s | 2.9 s |
| | memory at the end | 28 MiB | 950 MiB RSS, 702 MiB PSS (JS heap 376 MiB) |
| | slow frames | longest iteration 52 ms | 208 frames over 33 ms, longest 867 ms |
| 200 messages, 40 KB prompt | CPU while streaming (60 s) | 16.6-16.9 s | 38.1 s |
| | memory at the end | 23 MiB | 663 MiB RSS, 418 MiB PSS |
| | longest stall | 78-100 ms | 5.0 s |

The Rust figures include software rasterisation on the CPU (`--bench-render`); a GPU window moves that to the GPU,
while Chromium's figures include its own raster and compositor work. The React thread follows upstream's default
structure (no list virtualisation) but with plain CSS instead of Tailwind and Radix. That skips the kit's styling work,
but it also omits the kit's `content-visibility: auto` on message roots, which lets Chromium skip rendering off-screen
messages. The net effect against upstream's styled default has not been measured, so the CSS difference cannot be said
to favour React.

Two changes made on the way (2026-11-10): long reasoning text is shown as chunks that are appended once and never
re-wrapped (`examples/thread/src/chunks.rs`, `ChatPart.body-chunks`), with each chunk's height held in a property so
the layout reads a cached value; per-frame layout and paint at the end of a 40 KB reasoning stream went from 5.7 ms to
2.3 ms (flat over the stream). Collapsed reasoning no longer converts its text at all.

### Real window (2026-11-10)

FemtoVG-WGPU, Intel UHD, Wayland; 30 s streaming a ~9 KB reply into a 1000-message
thread; process CPU and peak RSS from `/usr/bin/time`, interleaved runs. femtovg 0.27 dropped every cached render
pipeline a flush did not use, so the chat recompiled pipelines and shaders every few frames (~70% of the main
thread). With upstream's fix (#366) carried as a patch (`patches/README.md`): 23.1-25.5 s -> 11.8-14.1 s CPU,
median 105-108 -> 141-142 fps (the display's limit), peak RSS 92 -> 95-96 MiB. An app embedding these crates
gets the same by adding the `[patch.crates-io]` entry to its own workspace, until Slint ships femtovg 0.28.

### Scale (2026-11-10)

At 10,000 messages upstream's page did not survive in headless Chromium on 2026-11-10 (the browser closed mid-run, and on a second
attempt at launch; on 2026-11-11 it survived 5 of 5 runs, see the table above), and the harness could not type a 512 KB prompt into its composer within 30 s. Rust, after the
changes below: 10,000 messages open in 75 ms (Markdown formatted by 0.11 s), stream at 14.9 s CPU per 60 s with p95
2.5 ms, 63 MiB.

Changes, each judged by `scripts/ab-bench.sh` against the previous main (median of 3 interleaved pairs; 1000 messages
and a 9 KB prompt): open 61-91 ms -> 8-13 ms, Markdown formatted 0.52 s -> 0.09-0.11 s, app CPU while streaming 6.6 s ->
3.1 s (iterations over 4 ms 45 -> 1), with rendering 15.3 s -> 11.9 s; a 40 KB prompt unchanged (within noise); +0.8 MiB.

- Markdown is formatted on a pool of worker threads (cores - 1, at most 8), one coalescing coordinator per thread,
  sharded by key (`ai-ui-markdown::pool`). `AI_UI_MARKDOWN_WORKERS=1` restores one in the example.
- Appending under the head is O(1): it walked to the root, which made opening a long thread quadratic (10,000 messages
  took 10.6-45 s).
- `ThreadRuntime::structure_version`: while the tree's shape is unchanged, `ThreadView` reuses branch positions instead
  of a lookup per message per update; the view's state and the repository's path are updated in place when no one else
  holds them. Per streamed update at 5,000 messages: 1.8-2.1 ms -> 0.10-0.13 ms.

The previous large-user-message display limit is addressed by bounded pages in
the example: text over 64 KiB or with at least 1000 newlines renders one verbatim
page at a time, at most 16 KiB and 400 newlines. Copy/edit/model input retain the
full source. This avoids passing an enormous message height to the software
renderer. Draft and full-text editor layout remain separate limits; see
[`PERFORMANCE.md`](PERFORMANCE.md#large-message-presentation).

### Rust only

Measured on this machine (12 cores, shared and busy, so numbers are indicative; comparisons are interleaved runs).

**Engine** (`cargo test --release -p ai-ui-stream --test perf -- --ignored --nocapture`, and `-p ai-ui-state
--test perf`): 400 KB streamed through decode and fold in ~115 ms. One streamed update through runtime and state
projection costs 7 us at 10 messages, 18 us at 100, 195 us at 1000 and 1.4 ms at 5000 (linear in thread length; smooth
streaming no longer adds a quadratic term: it was 14 ms at 5000).

**Application work per frame** (`ai-ui-example-thread --bench N [--bench-reply-kb K]`, release build, offscreen: CPU
time of state projection, Slint model updates, Markdown polling and smooth streaming; rendering excluded):

| Scenario | Before | After |
|---|---|---|
| Stream a reply into a 1000-message thread, p95 per frame | 11.1 ms (204 frames over 4 ms) | ~1 ms (0 to 2 over 4 ms) |
| Same, total app time for the reply | 2020 ms | ~245 ms |
| Open a 1000-message thread (Markdown formatted) | 4.5 s | 0.5 s |
| Open a 100-message thread | 0.46 s | 0.05 s |
| Stream a 40 KB reply, p50 per frame | 1.33 ms | ~0.5 ms |

**Rendering in a real window** (GPU build, femtovg, Slint `SLINT_DEBUG_PERFORMANCE=refresh_lazy,console`): streaming a
~9 KB reply held 84-139 fps throughout after the in-place row updates; before, it fell from ~140 to 56 fps as the reply
grew (every update re-created all blocks). Single runs, indicative.

What changed: delta-based sync (only changed rows are observed and rebuilt), a Markdown cache with delta updates and
one in-place row model per streaming document, per-row parts models updated in place, a 1 ms Markdown worker poll
while work is pending, no per-message allocation in the state projection, smooth-streaming animators only for streaming
parts, and no copy of raw text for parts that have formatted Markdown.

**Against the imported AI SDK runtime** (same machine and load, nine runs each, 10,000 text deltas of 24 bytes; the
workload of `benchmarks/ai-sdk-stream`): `ai-ui-sdk-stream` incremental updates ~11.6 ms and a snapshot per event
~88 ms; `ai-ui-stream`'s UI path ~18 ms with results delivered once per wake-up (3 results from 4 KB reads) and
~50 ms with a read per event (157 results). The numbers measure different things: the AI SDK crate can update one part
without copying the message, ours hands the UI a cumulative message each time it wakes up. The PR's own report (JS
reference 102 ms, Rust snapshots 33 ms, incremental 5.5 ms) was taken on a quieter machine; use ratios, not absolutes.
Reproduce with `cargo test --release -p ai-ui-stream --test perf ui_stream_text -- --ignored --nocapture` and
`cargo bench -p ai-ui-sdk-stream --bench stream_replay`. Using the incremental updates in the state layer is the open
way to remove the remaining per-update work that grows with reply length.

**With layout and paint** (`--bench N --bench-reply-kb K --bench-render`: also lays out and paints 900x760 with the
software renderer, at most 60 times a second; prints resident memory, process CPU and layout + paint per drawn frame
in 10 s buckets). Before the 60 Hz cap it drew every changed iteration (~280 a second): 1000 messages and a 9 KB reply: p50 ~0.8 ms, p95 ~6.4 ms,
about 10% of iterations over 4 ms, so painting, not app work, dominates a frame. The offscreen process stays at
20-28 MiB, so most of the ~100 MB measured in the GPU window belongs to the renderer and windowing, not thread data.
At the demo model's pace the reply does not finish within the bench's 60 s window; totals compare equal windows.
Tried and reverted (2026-11-10, five interleaved pairs per scenario): one list row per Markdown block instead of one
per message. That changed row granularity inside the same Slint `ListView`; it was not thread virtualisation and says
nothing about upstream's virtualised thread. With rendering it was within noise (p95 6.4 vs 7.0 ms at 1000
messages, 7.1 vs 6.5 ms for a 40 KB reply); app work alone got worse (total 6.8 to 9.1 s, frames over 4 ms 66 to
182) because every update re-compared the reply's blocks.

Since 2026-11-11 ([`PERFORMANCE.md`](PERFORMANCE.md)) projected row changes reach
the host, which no longer compares every row per update, and Markdown completions
are indexed by message ID. On a busy machine, app work for a 1,000-message thread
fell to 0.72× of base (0.95× with software paint); RSS is within 0.2 KB per
message of base. Full projection remains
necessary for structural/global changes, retained snapshots or runtimes without
a usable change journal. Per-update work still grows with the length of the
streaming reply; this change does not add thread virtualisation. The earlier
repository journal removes the historical-row scan from the local runtime's
steady-state `ThreadView` content updates.

## Minimum supported Rust versions

Each crate declares the oldest toolchain its locked dependencies build on, and CI checks every one with that exact
toolchain: **1.85** for `ai-ui-core`, `-stream`, `-state`, `-data-stream` and `-http`; **1.86** for
`ai-ui-clipboard` (a Wayland dependency); **1.88** for `ai-ui-markdown` (`url`'s ICU dependencies);
**1.92** for the Slint crates (Slint 1.18 itself). Raising a dependency can raise these; the CI job fails when a
declared version stops being true.

## Interop with a real backend

`crates/ai-ui-http/tests/interop_ai_sdk.rs` starts `benchmarks/ai-sdk-stream/interop-server.mjs`, a real AI SDK 7
`streamText` server over a mock model (no provider, no network), and runs the native client, the data-stream adapter and the
local runtime against it: reasoning, a streamed tool call executed by the server, a second step with text, and a follow-up
turn that replays the tool call and result. The SDK accepts our request body (it is passed straight to `streamText` as
`messages`) and the folded message is exactly what the server produced. It skips without Node or the harness packages
(`npm ci --ignore-scripts` in `benchmarks/ai-sdk-stream`); CI sets `AI_UI_REQUIRE_INTEROP=1` so a skip is a failure.
Covered: the AI SDK UI message stream. Not covered: other backends, authentication, retries.

