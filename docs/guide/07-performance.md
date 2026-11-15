---
title: Performance
description: Why ai-ui is fast, the measured results against upstream assistant-ui, and how to reproduce every number.
order: 7
section: Reference
---

# Performance

**What this page gives you.** This page explains where the cost goes, with file names; gives the measured results against upstream assistant-ui with their conditions; lists what was not measured and what was corrected; and gives the commands to reproduce every figure.

This page has two parts. The first explains the design decisions behind the speed, with the files that hold them.
The second gives the measurements, how to read them, what they do not cover and how to reproduce them. Every figure
comes from [RESULTS](../RESULTS.md) (generated from [`results.json`](../results.json)). The engineering record,
including rejected experiments, is [PERFORMANCE](../PERFORMANCE.md).

## Why it is fast

A streaming chat does the same small thing many times: a few bytes arrive, one message grows, one row on screen
changes. A slow chat redoes work proportional to the whole thread for each of those events. Most of the design is
about making the cost of one event independent of everything else in the thread.

### One streamed update costs the same at any thread length

The repository behind every runtime (`crates/ai-ui-core/src/repository.rs`) keeps a bounded change journal: the last
64 content revisions, stored as the indices of the selected-path messages that changed, not as copies of message
bodies. Each `ThreadView` (`crates/ai-ui-state/src/view.rs`) owns its own cursor, so one consumer never consumes
another's changes. When the cursor is current and the thread's shape is unchanged, the view projects only the rows
that changed. If the cursor is missing or expired, or the structure changed (an edit, a branch switch, a new
message), the view falls back to a full projection. Correctness never depends on the journal.

The second half is `ThreadRuntime::structure_version`. While the tree's shape is unchanged, the view reuses branch
positions instead of looking each message up again, and patches its state and the repository's selected path in place
when nobody else holds a reference (`Arc::get_mut`). Retained snapshots stay immutable; copying them costs more, and
the code never mutates through a shared pointer.

The effect, from the 2026-11-10 record in PERFORMANCE: with smooth streaming on, one streamed update through runtime
and projection went from 8.5 to 1.7 microseconds at 1,000 existing messages and from 38.4 to 4.7 microseconds at
5,000 (median of five interleaved pairs). With 10 messages it did not change. That is a runtime-plus-projection
workload, not frame latency.

### Row sync touches only changed rows

The Slint side does not rebuild the list when a reply grows. The example host compares row identities (messages are
`Arc`, so identity is a pointer comparison) and updates changed rows and their part models in place. Streaming text
goes into one in-place row model per document. Animators for smooth streaming exist only for parts that are
streaming. The example's `App::sync` still compares identities across the selected thread; PERFORMANCE lists that as a
remaining cost.

### Markdown runs off the UI thread

`ai-ui-markdown` formats on a pool of worker threads (`crates/ai-ui-markdown/src/pool.rs`): one per core minus one for
the UI, between 1 and 8. Several coordinators sit behind one API, and a key always maps to the same shard. Each
shard keeps one parse in flight and only the newest source for a key. An older completion is labelled a preview, so
the UI keeps showing formatted text while newer text is pending instead of flashing plain text. The Slint cache
(`crates/ai-ui-markdown-slint/src/cache.rs`) is told which parts are visible, in priority order, so the parts the
person can see are submitted first. The browser build has no threads and runs a wasm-only inline backend instead.

### Settled blocks are formatted once, plus a live tail

Streaming Markdown is not re-parsed from the start on every delta. `StreamSplit` (`split.rs`) cuts the text into
settled blocks and a live tail; `StreamingCache` formats each settled block once and re-formats only the tail on each
frame. The result is tested to be equivalent to formatting the whole document, over every prefix of five documents.

### Long reasoning text is chunked

Reasoning can run to tens of kilobytes. The example (`examples/thread/src/chunks.rs`) shows it as chunks that are
appended once and never re-wrapped, with each chunk's height held in a property so the layout reads a cached value.
Layout and paint per frame at the end of a 40 KB reasoning stream went from 5.7 ms to 2.3 ms, and stayed flat over
the stream. Collapsed reasoning is not converted at all.

### Cheap decoding and fair scheduling

Complete SSE lines borrow the incoming bytes. Fragmented lines reuse a scratch buffer. The assistant-transport decoder
moves its JSON tree into typed deserialization instead of cloning it. The stream's 64-read budget now yields even for
heartbeat-only reads, so a fast stream cannot starve input and painting. Cumulative snapshots are coalesced; protocol
events never are. On the recorded workload (10,000 deltas, 240,000 bytes), the UI-message decoder plus accumulator
took 4.7 ms instead of 8.0 ms with 4 KB reads.

### A femtovg pipeline-cache patch

In a real GPU window, femtovg 0.27 dropped every cached render pipeline that a flush did not use, so a streaming
chat recompiled pipelines every few frames (about 70% of the main thread). The repository carries upstream's fix as
a patch in `patches/femtovg` (see `patches/README.md`). Process CPU in a 30 s run fell from 23.1-25.5 s to
11.8-14.1 s, and the median frame rate rose from 105-108 to 141-142 fps, the display's limit. An app that embeds the
Slint crates adds the same `[patch.crates-io]` entry until Slint ships femtovg 0.28.

### The list and large messages

The thread is a Slint `ListView`, so layout and paint cost follows the rows on screen. PERFORMANCE records that
changing text scale or reading width with 20,000 messages costs a few milliseconds of layout, and that the cost does
not grow with thread length because only visible rows relayout. STATUS still lists row instantiation across the whole
thread as a known remaining cost, so this guide does not claim a fully virtualised model.

A user message over 64 KiB or with at least 1,000 newlines is shown one verbatim page at a time (at most 16 KiB and
400 newlines; `examples/thread/src/paging.rs`). The runtime still holds the whole message, and copy, edit and model
requests use the full source. In a same-window layout microbenchmark with a 32,742-byte message, first layout and
paint took 9.589 ms raw against 5.554 ms paged. That is a layout figure, not a whole-application one.

### Memory

The Rust side keeps parsed text once and instantiates Slint rows. Resident memory in the like-for-like runs was
30 MiB at 1,000 messages and 64 MiB at 10,000 (table below). In the larger Rust-only runs, RESULTS records 166 MiB at
5,000 long structured messages and 552 MiB at 20,000 (117.9 MB of text, one run). Memory grows with the thread, as it must,
but slowly compared with a DOM.

One experiment was tried and reverted: one list row per Markdown block. It stayed within noise in rendering and
increased application work, because every update re-compared the blocks. The measurement is in PERFORMANCE.

## The measurements

Workload: upstream's thread (`@assistant-ui/react` 0.15.33 with plain CSS, in headless Chromium) against
`ai-ui-example-thread --bench N --bench-reply-kb K --bench-render` (offscreen, software raster, at most 60 frames a
second). The same history, the same demo reply (reasoning, then the answer, one word every 18 ms) and the same 60 s
window on both sides. Each cell is the median of 5 interleaved Rust and React pairs, release build, run on
2026-11-11, machine load about 4-6. Ranges are the minimum to maximum over the five runs. Ratios are the median
React value divided by the median Rust value, floored to one decimal.

### 1,000 messages, 9 KB prompt

| Measure | Rust | React | Ratio | Range of ratio | RESULTS row |
|---|---|---|---|---|---|
| Open the thread, like for like | 105.3 ms (100.8-150.9) | 2940 ms (2794-2987) | 27.9x | 18.5x-29.6x | 15 |
| CPU while streaming, 60 s | 17160 ms (13700-17650) | 73260 ms (73050-73540) | 4.2x | 4.1x-5.3x | 18 |
| Memory at the end, RSS | 30.1 MiB (29.7-30.4) | 1481.5 MiB (1474.5-1526.6) | 49.2x | 48.5x-51.4x | 11 |
| Memory at the end, Rust RSS vs React PSS | 30.1 MiB | 850.3 MiB (841.8-895.4) | 28.2x | 27.6x-30.1x | 14 |
| Longest iteration or frame while streaming | 34 ms (17.5-35.4) | 916.6 ms (866.6-1183.3) | 26.9x | 24.4x-67.6x | 7 |

### 200 messages, 40 KB prompt

| Measure | Rust | React | Ratio | Range of ratio | RESULTS row |
|---|---|---|---|---|---|
| Open the thread, like for like | 33.2 ms (31.8-46.6) | 1104 ms (1029-1197) | 33.2x | 22x-37.6x | 13 |
| CPU while streaming, 60 s | 17020 ms (13660-19450) | 51160 ms (41900-57120) | 3x | 2.1x-4.1x | 19 |
| Memory at the end, RSS | 24.6 MiB (24.5-24.8) | 1212.7 MiB (1210-1268) | 49.2x | 48.7x-51.7x | 10 |
| Memory at the end, Rust RSS vs React PSS | 24.6 MiB | 584 MiB (582.4-640.9) | 23.7x | 23.4x-26.1x | 16 |
| Longest stall | 109.9 ms (36.2-136.2) | 2399.9 ms (2183.2-2649.9) | 21.8x | 16x-73.2x | 5 |

### 10,000 messages, 9 KB prompt

| Measure | Rust | React | Ratio | Range of ratio | RESULTS row |
|---|---|---|---|---|---|
| Open the thread, like for like | 435.5 ms (382.9-472.1) | 26081 ms (23591-27196) | 59.8x | 49.9x-71x | 6 |
| CPU while streaming, 60 s | 22800 ms (20930-23220) | 83500 ms (76060-84710) | 3.6x | 3.2x-4x | 20 |
| Memory at the end, RSS | 63.6 MiB (63.3-64) | 3717.8 MiB (3675.4-3926) | 58.4x | 57.4x-62x | 8 |
| Memory at the end, Rust RSS vs React PSS | 63.6 MiB | 3088.9 MiB (3051.1-3298.6) | 48.5x | 47.6x-52.1x | 9 |
| Longest stall | 41.1 ms (28.3-46) | 11599.5 ms (9833-12899.5) | 282.2x | 213.7x-455.8x | 1 |

Source for all rows: `benchmarks/results/like-for-like/raw`, a release build of 424a4fb plus the open reporting of
that change. The summary script is `benchmarks/results/like-for-like/summarize.py`.

### Second run

A second, independent run of the 1,000-message comparison (2026-11-11, 9 interleaved pairs, release build, machine load
about 3-5 against 4-6 for the first run). Raw files and commands: `benchmarks/results/like-for-like-run2/`. The first run's
table above is unchanged; the full rows are in RESULTS (rows marked "run 2").

| Measure | Rust | React | Ratio | Range of ratio |
|---|---|---|---|---|
| Open the thread, like for like | 97.5 ms (85.7-213.2) | 2472 ms (1870-2874) | 25.3x | 8.7x-33.5x |
| Longest iteration or frame | 16.68 ms (11.73-30.53) | 550 ms (416.6-983.3) | 32.9x | 13.6x-83.8x |
| Memory at the end, Rust RSS vs React PSS | 30.1 MiB (29.8-30.2) | 888 MiB (793.2-1066.3) | 29.5x | 26.2x-35.7x |
| Memory at the end, RSS | 30.1 MiB (29.8-30.2) | 1531.5 MiB (1427.7-1708.5) | 50.8x | 47.2x-57.3x |
| CPU while streaming, 60 s | 9690 ms (7570-14990) | 69360 ms (68190-73360) | 7.1x | 4.5x-9.6x |

The runs differ because of machine load: Rust's CPU was 17.2 s in run 1 and 9.7 s in run 2 (ratio 4.2x against 7.1x). CPU is
load-sensitive and is not a headline figure. Open, stall and memory agreed within the ranges.

Larger fixtures were measured earlier (2026-11-10, 3 interleaved pairs, load 6-25, long structured replies):

| Workload | Measure | Rust | React | Ratio | RESULTS row |
|---|---|---|---|---|---|
| 2,000 messages (12.2 MB) | open the thread | 527.1 ms (485.5-528.3) | 42090 ms (42090-96584) | 79.8x | 2 |
| 2,000 messages | longest iteration or frame | 41.5 ms (35-74.8) | 1633.2 ms (1633.2-4583.1) | 39.3x | 4 |
| 2,000 messages | resident memory | 88.1 MiB | 2946.3 MiB (2946.3-3449.1) | 33.4x | 12 |
| 2,000 messages | process CPU, 60 s | 9130 ms | 60260 ms | 6.6x | 17 |
| 5,000 messages (29.7 MB) | open the thread | 1256.7 ms (807.3-1256.7) | at least 120000 ms | at least 95.4x | 3 |

Two cautions apply to these. For 2,000 messages, one of three React runs did not finish opening within the harness's
120 s limit and is excluded, which understates React. For 5,000 messages React did not finish in 3 of 3 runs, so
120 s is a floor, not a measurement, and the ratio is a lower bound. Those older runs also used a harness that
measured React's open without checking the Markdown, which if anything understates React.

Rust against its own earlier state (not a React comparison): in a real GPU window, 1,000 messages, 30 s, process CPU
fell from 23.1-25.5 s to 11.8-14.1 s (1.8x, RESULTS rust-only row 1) and the median frame rate rose from 105 to 141 fps
(1.3x, row 2) after the femtovg patch.

## How to read these numbers

**What "open" means.** On the Rust side it is the time from the start of the history import until the last message's
Markdown is formatted and painted at least once: import, first sync, formatting on the worker threads, and a forced
frame. On the React side it is navigation until all messages are in the DOM with their Markdown rendered (every
reply's table present), plus two frames. Rust's first sync alone, when rows are on screen but Markdown is not yet
formatted, was 12.9 ms at 1,000 messages. It is a different, earlier event and is not compared with React.

**RSS and PSS.** RSS counts every resident page of a process, so summing it over Chromium's nine processes counts
shared pages several times. PSS divides shared pages among the processes that map them. The PSS ratio is therefore the
conservative memory comparison (28.2x against 49.2x at 1,000 messages). Rust runs as one process, where RSS is
the right figure.

**Medians and ranges.** Each value is the median of 5 interleaved runs. A range is the minimum to maximum of the
runs. The extremes of a ratio pair the best Rust run with the worst React run, so a wide ratio range is a statement
about noise, not a claim. The "longest stall" rows are single events and are the noisiest.

**A shared, busy machine.** The machine has 12 cores, is shared, and ran at load 4-6 for the like-for-like runs. Runs
are interleaved (Rust, React, Rust, React...) so load hits both sides alike. Absolute times move between days; only
the ratios and ranges are meaningful.

**Different clocks, same workload.** Rust's "longest stall" is the longest loop iteration, unpaced. React's is the
longest animation-frame interval. Rust's figures include software rasterisation on the CPU. React's CPU is the whole
Chromium process tree, including its own raster and compositing. React's main-thread busy time does not count raster
threads, which favours React. The React thread uses plain CSS and does somewhat less than upstream's default
Tailwind and Radix thread, which also favours React.

**Where React won.** No case was found on these workloads where React wins or ties.

## What we did not measure

- Any product other than upstream assistant-ui. This guide makes no comparison with others.
- React in a real GPU window. The harness cannot open one, so there is no GPU Rust-against-React ratio; the GPU rows
  are Rust against its earlier self.
- Long fixtures re-run with raw files kept: the 2,000- and 5,000-message rows come from 3 pairs on a loaded machine
  and have not been repeated at 5 pairs. React at 20,000 messages was started and interrupted.
- Fast-scroll frame time and composer typing latency with React. They were smoke-tested on Rust only and are not in
  the comparison tables.
- Memory growth over a five-minute stream.
- Retina or other high-DPI displays, and hardware other than the one machine above.
- Network and model latency. The demo reply is local; nothing here says how quickly a backend answers.

## Correction history

An earlier README and RESULTS headlined opening a 1,000-message thread at 0.12 s for Rust, against 2.9 s for React
(24x). The Rust figure was the time before Markdown had been formatted; the React figure ended with Markdown
rendered, so the comparison was not like for like. It was corrected on 2026-11-11. The measured like-for-like
ratio at 1,000 messages is 27.9x (18.5x to 29.6x), reflecting formatting that had since been cut from 0.52 s to
0.09-0.11 s by the worker pool. The 2026-11-10 values for 1,000 and 200 messages were also withdrawn, because their
raw files were not kept; the table above replaces them.

## Reproduce it yourself

The Rust side needs a release build; the React side needs Node and a headless Chromium. From the repository root:

```sh
cargo build --release -p ai-ui-example-thread
cd benchmarks/react-baseline
npm ci --ignore-scripts && npm run build
export PLAYWRIGHT=/path/to/node_modules/playwright   # any Playwright package
export CHROME=/path/to/chrome-headless-shell          # if Playwright's browser is not installed
node bench.mjs 1000 9 60        # one React run: 1000 messages, 9 KB prompt, 60 s; JSON on stdout
./compare.sh 1000 9 5           # five interleaved Rust and React pairs; raw output on stdout
```

The Rust half of one pair is `target/release/ai-ui-example-thread --bench 1000 --bench-reply-kb 9 --bench-render`.
To keep one file per run with the load average, use `benchmarks/results/run-pairs.sh stream 1000 9 5 OUT` with
`RUST_BIN`, `REACT_BASELINE`, `PLAYWRIGHT` and `CHROME` set (the script header lists them), then summarise with
`python3 benchmarks/results/like-for-like/summarize.py`. `python3 scripts/results.py check` verifies that RESULTS.md
and `results.json` agree.

To judge a Rust change against another revision, build both and alternate them:

```sh
scripts/ab-bench.sh BASE_REV NEW_REV 5 > ab.txt     # NEW_REV may be WORKTREE for uncommitted changes
```

It runs three scenarios (1,000 messages with render, 1,000 without, 200 messages with a 40 KB reply with render)
interleaved. Compare medians of p95, total and iterations over 4 ms, and keep the change only if nothing regresses.
The Rust-only decode and projection benchmarks are the ignored tests listed in PERFORMANCE, for example
`cargo test --locked --release -p ai-ui-state --test perf projection_cost_per_streamed_update -- --ignored --nocapture`.
Do not run them while compiling or while another benchmark is running.

Next: [Coming from assistant-ui](08-from-assistant-ui.md) for the parity table, or [Contributing and testing](09-contributing.md) for the gate and the A/B method.
