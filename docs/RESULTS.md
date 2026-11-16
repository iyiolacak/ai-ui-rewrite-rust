# Rust port against upstream React: measured results

Every number here is measured, with its conditions. **Scope:** "React" means upstream's default thread
(`@assistant-ui/react` 0.15.33), which renders every message into the DOM and does not virtualise the list. A virtualised
React thread has not been measured; it would reduce DOM size, open time and memory on long threads, so the large-thread
multipliers below are not a statement about virtualised React. Ratios are computed by `scripts/results.py` from the values in
`docs/results.json` and floored to one decimal (never rounded up); `python3 scripts/results.py check` fails if this
page and the JSON disagree. The tables are ranked by the top of each row's range. The top of a range pairs the best
Rust figure with the worst React figure of the same workload, so each row also shows the ratio of the two stated values and
the lowest ratio. Read the conditions column with the number.

Every per-pair raw value, the power and load conditions of each run and the web and test results are in `docs/runs.json`; see [RUNS.md](RUNS.md).

Status of this page (2026-11-11): the 1000-message, 200-message and 10,000-message comparisons were re-run with raw files
kept (5 interleaved pairs each, `benchmarks/results/like-for-like/raw`). **Every "open the thread" row compares like with like**:
Rust until the last message's Markdown is formatted and painted at least once, React until all messages are in the DOM with
their Markdown rendered. An earlier version of this page headlined Rust's open time *before* Markdown was formatted (0.12 s,
"24x") against React's open time *with* Markdown rendered; that comparison is withdrawn. Measured like for like, opening a
1000-message thread is 105 ms (Rust) vs 2.94 s (React), 27.9x (range 18.5x to 29.6x). The 2,000- and 5,000-message rows below
come from the 2026-11-10 session; their Rust open figures were corrected to include Markdown formatting.

## Second run (2026-11-11, lighter load)

A second, independent run of the 1000-message, 9 KB prompt comparison: 9 interleaved pairs, release build, machine load about
3-5 (run 1: 4-6). Raw files, the load before and after every pair and the commands are in
`benchmarks/results/like-for-like-run2/` (README there). Its rows are marked "run 2" in the tables below and sit next to
run 1's rows; run 1 is not replaced. Same definitions as run 1; ratios are computed by `scripts/results.py` from the stored values.

| Measure (1000 messages, 9 KB prompt) | Run 1 (5 pairs, load 4-6): Rust vs React, ratio | Run 2 (9 pairs, load 3-5): Rust vs React, ratio | Pairing range of ratio, run 1 | Pairing range of ratio, run 2 |
|---|---|---|---|---|
| Open, like for like | 105.3 ms vs 2940 ms, 27.9x | 97.5 ms vs 2472 ms, 25.3x | 18.5x to 29.6x | 8.7x to 33.5x |
| Longest stall | 34 ms vs 916.6 ms, 26.9x | 16.68 ms vs 550 ms, 32.9x | 24.4x to 67.6x | 13.6x to 83.8x |
| Memory, Rust RSS vs React PSS | 30.1 vs 850.3 MiB, 28.2x | 30.1 vs 888 MiB, 29.5x | 27.6x to 30.1x | 26.2x to 35.7x |
| Memory, RSS | 30.1 vs 1481.5 MiB, 49.2x | 30.1 vs 1531.5 MiB, 50.8x | 48.5x to 51.4x | 47.2x to 57.3x |
| CPU over 60 s | 17160 ms vs 73260 ms, 4.2x | 9690 ms vs 69360 ms, 7.1x | 4.1x to 5.3x | 4.5x to 9.6x |

Run 1 and run 2 differ because of machine load. Rust's median CPU over the 60 s window was 17.2 s in run 1 and 9.7 s in run 2,
so the CPU ratio moved from 4.2x to 7.1x. CPU is load-sensitive (Rust includes software rasterisation, which competes for
cores) and is therefore not a headline figure. Open, stall and memory agreed: each run-2 ratio (25.3x, 32.9x, 29.5x, 50.8x)
lies inside run 1's pairing range for the same measure, whereas the CPU ratio (7.1x) lies outside run 1's (4.1x to 5.3x).

## Ranked multipliers (Rust vs React)

<!-- results:vs_react -->
| Rank | Workload | Measure | Rust | React | Ratio | Range | Source (commit), date | Conditions | What drives it |
|---|---|---|---|---|---|---|---|---|---|
| 1 | 10,000 messages, 9 KB prompt | longest stall | 28.3 to 46 ms (value 41.1) | 9833 to 12899.5 ms (value 11599.5) | 282.2x | 213.7x to 455.8x | benchmarks/results/like-for-like/raw (5 interleaved pairs, release build of 424a4fb plus the open reporting of this change; summary: benchmarks/results/like-for-like/summarize.py), 2026-11-11 | Median of 5 interleaved pairs (range = min to max over the 5 runs), machine load about 4-6, 60 s window, headless Chromium vs --bench-render. Rust = longest loop iteration (unpaced), React = longest frame interval (median frame 183 ms in run 1). | Main thread saturation in React. |
| 2 | 2,000 long structured messages (12.2 MB) | open the thread | 485.5 to 528.3 ms (value 527.1) | 42090 to 96584 ms (value 42090) | 79.8x | 79.6x to 198.9x | benchmarks/results/summary-run1.json (raw: results-run1/a-2000, 3 Rust runs, 2 of 3 React runs finished); hist build = 2cbedad + origin/demo/long-threads loader + benchmarks/results/rust-bench-extras.patch, 2026-11-10 | words.py seed 7 fixture, 2,000 messages, 12.2 MB of text, long structured replies, 9 KB prompt, 60 s window, headless Chromium vs --bench-render, 3 interleaved pairs, machine load 6-25. Rust = import incl. JSON parse + first sync + Markdown formatted (open_ms + formatted_after_s of each raw run: 485.5, 527.1, 528.3 ms; first sync alone was 375.5 to 417.1 ms and is not comparable); React = navigation until all messages are in the DOM (that old harness did not check Markdown, so this is if anything a lower bound for React). Third React run did not finish within the 120 s harness limit and is excluded from the range, so the range understates React. | React renders every message as DOM and parses all Markdown on the main thread; Rust formats on worker threads. |
| 3 | 5,000 long structured messages (29.7 MB) | open the thread | 807.3 to 1256.7 ms (value 1256.7) | 120000 ms (value 120000) | >= 95.4x | >= 95.4x to >= 148.6x | benchmarks/results/summary-run1.json (raw: results-run1/a-5000, 3/3 pairs), 2026-11-10 | Same fixture family, 3 interleaved pairs. React did NOT have all 5,000 messages in the DOM within the harness's 120 s limit in 3 of 3 runs, so 120 s is a floor for React, not a measurement; Rust = import + first sync + Markdown formatted, value its slowest of 3 runs (807 to 1257 ms; first sync alone was 697 to 1117 ms). The ratio is therefore a lower bound. | As above, growing with thread size. |
| 4 | 2,000 long structured messages | longest single iteration or frame while streaming | 35 to 74.8 ms (value 41.5) | 1633.2 to 4583.1 ms (value 1633.2) | 39.3x | 21.8x to 130.9x | benchmarks/results/summary-run1.json (raw: results-run1/a-2000, 3 Rust runs, 2 of 3 React runs finished); hist build = 2cbedad + origin/demo/long-threads loader + benchmarks/results/rust-bench-extras.patch, 2026-11-10 | words.py seed 7 fixture, 2,000 messages, 12.2 MB of text, long structured replies, 9 KB prompt, 60 s window, headless Chromium vs --bench-render, 3 interleaved pairs, machine load 6-25. Rust = longest iteration (app work + layout + software paint, unpaced); React = longest frame interval. Single-event stalls are noisy: all runs shown in the range. | React main thread saturated (61 s busy in a 60 s window). |
| 5 | 1000 messages, 9 KB prompt | longest iteration/frame while streaming, run 2 | 11.73 to 30.53 ms (value 16.68) | 416.6 to 983.3 ms (value 550) | 32.9x | 13.6x to 83.8x | benchmarks/results/like-for-like-run2 (9 interleaved pairs, release build, job2-run-pairs.sh; second independent run, summary job2-summary.txt), 2026-11-11 | RUN 2, a second independent run: median of 9 interleaved pairs (range = min to max over the 9 runs), machine load about 3-5 (lighter than run 1's 4-6), 60 s window, headless Chromium vs --bench-render, 1000 messages, 9 KB prompt. Rust = longest loop iteration (unpaced), React = longest animation-frame interval. | Main thread saturation in React. |
| 6 | 200 messages, 40 KB prompt | longest stall | 36.2 to 136.2 ms (value 109.9) | 2183.2 to 2649.9 ms (value 2399.9) | 21.8x | 16x to 73.2x | benchmarks/results/like-for-like/raw (5 interleaved pairs, release build of 424a4fb plus the open reporting of this change; summary: benchmarks/results/like-for-like/summarize.py), 2026-11-11 | Median of 5 interleaved pairs (range = min to max over the 5 runs), machine load about 4-6, 60 s window, headless Chromium vs --bench-render. Rust = longest loop iteration (unpaced), React = longest frame interval. Rust varied 36 to 136 ms between runs. | Long reasoning text: Rust shows it as cached chunks laid out once; React re-wraps and re-lays out. |
| 7 | 10,000 messages, 9 KB prompt | open the thread (like for like: Markdown formatted and painted) | 382.9 to 472.1 ms (value 435.5) | 23591 to 27196 ms (value 26081) | 59.8x | 49.9x to 71x | benchmarks/results/like-for-like/raw (5 interleaved pairs, release build of 424a4fb plus the open reporting of this change; summary: benchmarks/results/like-for-like/summarize.py), 2026-11-11 | Median of 5 interleaved pairs (range = min to max over the 5 runs), machine load about 4-6, 60 s window, headless Chromium vs --bench-render. Like for like: Rust = until the last message's Markdown is formatted and painted at least once (import + first sync + formatting on worker threads + a forced frame); React = navigation until all messages are in the DOM with their Markdown rendered (every reply's table present) plus two frames. Rust first sync alone: 147.8 ms (138.0 to 207.4), not comparable. | As above, growing with thread size. |
| 8 | 1000 messages, 9 KB prompt | longest iteration/frame while streaming | 17.5 to 35.4 ms (value 34) | 866.6 to 1183.3 ms (value 916.6) | 26.9x | 24.4x to 67.6x | benchmarks/results/like-for-like/raw (5 interleaved pairs, release build of 424a4fb plus the open reporting of this change; summary: benchmarks/results/like-for-like/summarize.py), 2026-11-11 | Median of 5 interleaved pairs (range = min to max over the 5 runs), machine load about 4-6, 60 s window, headless Chromium vs --bench-render. Rust = longest loop iteration (unpaced), React = longest animation-frame interval. React also had 105 frames over 33 ms in run 1. | Main thread saturation in React. |
| 9 | 10,000 messages, 9 KB prompt | memory at the end (RSS) | 63.3 to 64 MiB (value 63.6) | 3675.4 to 3926 MiB (value 3717.8) | 58.4x | 57.4x to 62x | benchmarks/results/like-for-like/raw (5 interleaved pairs, release build of 424a4fb plus the open reporting of this change; summary: benchmarks/results/like-for-like/summarize.py), 2026-11-11 | Median of 5 interleaved pairs (range = min to max over the 5 runs), machine load about 4-6, 60 s window, headless Chromium vs --bench-render. React RSS over all Chromium processes; PSS 3089 MiB (3051 to 3299). | As above. |
| 10 | 1000 messages, 9 KB prompt | memory at the end (RSS), run 2 | 29.8 to 30.2 MiB (value 30.1) | 1427.7 to 1708.5 MiB (value 1531.5) | 50.8x | 47.2x to 57.3x | benchmarks/results/like-for-like-run2 (9 interleaved pairs, release build, job2-run-pairs.sh; second independent run, summary job2-summary.txt), 2026-11-11 | RUN 2, a second independent run: median of 9 interleaved pairs (range = min to max over the 9 runs), machine load about 3-5 (lighter than run 1's 4-6), 60 s window, headless Chromium vs --bench-render, 1000 messages, 9 KB prompt. React RSS over all 9 Chromium processes. | DOM plus JS heap. |
| 11 | 10,000 messages, 9 KB prompt | memory at the end (Rust RSS vs React PSS) | 63.3 to 64 MiB (value 63.6) | 3051.1 to 3298.6 MiB (value 3088.9) | 48.5x | 47.6x to 52.1x | benchmarks/results/like-for-like/raw (5 interleaved pairs, release build of 424a4fb plus the open reporting of this change; summary: benchmarks/results/like-for-like/summarize.py), 2026-11-11 | Median of 5 interleaved pairs (range = min to max over the 5 runs), machine load about 4-6, 60 s window, headless Chromium vs --bench-render. Conservative PSS comparison. | As above. |
| 12 | 200 messages, 40 KB prompt | memory at the end (RSS) | 24.5 to 24.8 MiB (value 24.6) | 1210 to 1268 MiB (value 1212.7) | 49.2x | 48.7x to 51.7x | benchmarks/results/like-for-like/raw (5 interleaved pairs, release build of 424a4fb plus the open reporting of this change; summary: benchmarks/results/like-for-like/summarize.py), 2026-11-11 | Median of 5 interleaved pairs (range = min to max over the 5 runs), machine load about 4-6, 60 s window, headless Chromium vs --bench-render. React RSS over all Chromium processes. | As above. |
| 13 | 1000 messages, 9 KB prompt | memory at the end (RSS) | 29.7 to 30.4 MiB (value 30.1) | 1474.5 to 1526.6 MiB (value 1481.5) | 49.2x | 48.5x to 51.4x | benchmarks/results/like-for-like/raw (5 interleaved pairs, release build of 424a4fb plus the open reporting of this change; summary: benchmarks/results/like-for-like/summarize.py), 2026-11-11 | Median of 5 interleaved pairs (range = min to max over the 5 runs), machine load about 4-6, 60 s window, headless Chromium vs --bench-render. React RSS over all 9 Chromium processes. | DOM plus JS heap. |
| 14 | 2,000 long structured messages | resident memory at the end | 87.8 to 88.4 MiB (value 88.1) | 2946.3 to 3449.1 MiB (value 2946.3) | 33.4x | 33.3x to 39.2x | benchmarks/results/summary-run1.json (raw: results-run1/a-2000, 3 Rust runs, 2 of 3 React runs finished); hist build = 2cbedad + origin/demo/long-threads loader + benchmarks/results/rust-bench-extras.patch, 2026-11-10 | words.py seed 7 fixture, 2,000 messages, 12.2 MB of text, long structured replies, 9 KB prompt, 60 s window, headless Chromium vs --bench-render, 3 interleaved pairs, machine load 6-25. React = RSS summed over all Chromium processes. | Rust keeps parsed text once and instantiates Slint rows; React keeps DOM, JS heap (1.47 GiB in run 1) and Markdown tree. |
| 15 | 200 messages, 40 KB prompt | open the thread (like for like: Markdown formatted and painted) | 31.8 to 46.6 ms (value 33.2) | 1029 to 1197 ms (value 1104) | 33.2x | 22x to 37.6x | benchmarks/results/like-for-like/raw (5 interleaved pairs, release build of 424a4fb plus the open reporting of this change; summary: benchmarks/results/like-for-like/summarize.py), 2026-11-11 | Median of 5 interleaved pairs (range = min to max over the 5 runs), machine load about 4-6, 60 s window, headless Chromium vs --bench-render. Like for like: Rust = until the last message's Markdown is formatted and painted at least once (import + first sync + formatting on worker threads + a forced frame); React = navigation until all messages are in the DOM with their Markdown rendered (every reply's table present) plus two frames. Rust first sync alone: 2.1 ms (1.5 to 6.1), not comparable. | As above. |
| 16 | 1000 messages, 9 KB prompt | memory at the end (Rust RSS vs React PSS), run 2 | 29.8 to 30.2 MiB (value 30.1) | 793.2 to 1066.3 MiB (value 888) | 29.5x | 26.2x to 35.7x | benchmarks/results/like-for-like-run2 (9 interleaved pairs, release build, job2-run-pairs.sh; second independent run, summary job2-summary.txt), 2026-11-11 | RUN 2, a second independent run: median of 9 interleaved pairs (range = min to max over the 9 runs), machine load about 3-5 (lighter than run 1's 4-6), 60 s window, headless Chromium vs --bench-render, 1000 messages, 9 KB prompt. Conservative: React's PSS, which discounts shared pages. | As above. |
| 17 | 1000 messages, 9 KB prompt | open the thread (like for like: Markdown formatted and painted), run 2 | 85.7 to 213.2 ms (value 97.5) | 1870 to 2874 ms (value 2472) | 25.3x | 8.7x to 33.5x | benchmarks/results/like-for-like-run2 (9 interleaved pairs, release build, job2-run-pairs.sh; second independent run, summary job2-summary.txt), 2026-11-11 | RUN 2, a second independent run: median of 9 interleaved pairs (range = min to max over the 9 runs), machine load about 3-5 (lighter than run 1's 4-6), 60 s window, headless Chromium vs --bench-render, 1000 messages, 9 KB prompt. Like for like: Rust = until the last message's Markdown is formatted and painted at least once (import + first sync + formatting on worker threads + a forced frame); React = navigation until all messages are in the DOM with their Markdown rendered (every reply's table present) plus two frames. Same definitions as run 1. | As run 1. |
| 18 | 1000 messages, 9 KB prompt | memory at the end (Rust RSS vs React PSS) | 29.7 to 30.4 MiB (value 30.1) | 841.8 to 895.4 MiB (value 850.3) | 28.2x | 27.6x to 30.1x | benchmarks/results/like-for-like/raw (5 interleaved pairs, release build of 424a4fb plus the open reporting of this change; summary: benchmarks/results/like-for-like/summarize.py), 2026-11-11 | Median of 5 interleaved pairs (range = min to max over the 5 runs), machine load about 4-6, 60 s window, headless Chromium vs --bench-render. Conservative: React's PSS, which discounts shared pages. | As above. |
| 19 | 1000 messages, 9 KB prompt | open the thread (like for like: Markdown formatted and painted) | 100.8 to 150.9 ms (value 105.3) | 2794 to 2987 ms (value 2940) | 27.9x | 18.5x to 29.6x | benchmarks/results/like-for-like/raw (5 interleaved pairs, release build of 424a4fb plus the open reporting of this change; summary: benchmarks/results/like-for-like/summarize.py), 2026-11-11 | Median of 5 interleaved pairs (range = min to max over the 5 runs), machine load about 4-6, 60 s window, headless Chromium vs --bench-render. Like for like: Rust = until the last message's Markdown is formatted and painted at least once (import + first sync + formatting on worker threads + a forced frame); React = navigation until all messages are in the DOM with their Markdown rendered (every reply's table present) plus two frames. Rust first sync alone (rows visible, Markdown not yet formatted) was 12.9 ms (11.1 to 28.6) and is NOT comparable to React's figure. | Rust formats Markdown on worker threads and lays out only visible rows; React builds all DOM and parses all Markdown on the main thread. |
| 20 | 200 messages, 40 KB prompt | memory at the end (Rust RSS vs React PSS) | 24.5 to 24.8 MiB (value 24.6) | 582.4 to 640.9 MiB (value 584) | 23.7x | 23.4x to 26.1x | benchmarks/results/like-for-like/raw (5 interleaved pairs, release build of 424a4fb plus the open reporting of this change; summary: benchmarks/results/like-for-like/summarize.py), 2026-11-11 | Median of 5 interleaved pairs (range = min to max over the 5 runs), machine load about 4-6, 60 s window, headless Chromium vs --bench-render. Conservative PSS comparison. | As above. |
| 21 | 1000 messages, 9 KB prompt | process CPU while streaming 60 s, run 2 | 7570 to 14990 ms (value 9690) | 68190 to 73360 ms (value 69360) | 7.1x | 4.5x to 9.6x | benchmarks/results/like-for-like-run2 (9 interleaved pairs, release build, job2-run-pairs.sh; second independent run, summary job2-summary.txt), 2026-11-11 | RUN 2, a second independent run: median of 9 interleaved pairs (range = min to max over the 9 runs), machine load about 3-5 (lighter than run 1's 4-6), 60 s window, headless Chromium vs --bench-render, 1000 messages, 9 KB prompt. Rust includes software raster; React is the whole Chromium process tree. CPU is load-sensitive: Rust median 9.7 s here against 17.2 s in run 1 (load 3-5 against 4-6), so this row is not a headline figure. | React main thread. |
| 22 | 2,000 long structured messages | process CPU while streaming 60 s | 8780 to 9750 ms (value 9130) | 60260 to 65430 ms (value 60260) | 6.6x | 6.1x to 7.4x | benchmarks/results/summary-run1.json (raw: results-run1/a-2000, 3 Rust runs, 2 of 3 React runs finished); hist build = 2cbedad + origin/demo/long-threads loader + benchmarks/results/rust-bench-extras.patch, 2026-11-10 | words.py seed 7 fixture, 2,000 messages, 12.2 MB of text, long structured replies, 9 KB prompt, 60 s window, headless Chromium vs --bench-render, 3 interleaved pairs, machine load 6-25. Rust includes software raster; React plain CSS, whole Chromium process tree. | React re-renders and re-lays out the long thread on each streamed update. |
| 23 | 1000 messages, 9 KB prompt | process CPU while streaming 60 s | 13700 to 17650 ms (value 17160) | 73050 to 73540 ms (value 73260) | 4.2x | 4.1x to 5.3x | benchmarks/results/like-for-like/raw (5 interleaved pairs, release build of 424a4fb plus the open reporting of this change; summary: benchmarks/results/like-for-like/summarize.py), 2026-11-11 | Median of 5 interleaved pairs (range = min to max over the 5 runs), machine load about 4-6, 60 s window, headless Chromium vs --bench-render. Rust includes software raster; React is the whole Chromium process tree (main thread about 60 s, saturated). | React main thread. |
| 24 | 200 messages, 40 KB prompt | process CPU while streaming 60 s | 13660 to 19450 ms (value 17020) | 41900 to 57120 ms (value 51160) | 3x | 2.1x to 4.1x | benchmarks/results/like-for-like/raw (5 interleaved pairs, release build of 424a4fb plus the open reporting of this change; summary: benchmarks/results/like-for-like/summarize.py), 2026-11-11 | Median of 5 interleaved pairs (range = min to max over the 5 runs), machine load about 4-6, 60 s window, headless Chromium vs --bench-render. Rust includes software raster; Rust varied 13.7 to 19.5 s between runs. | Software raster is included on the Rust side. |
| 25 | 10,000 messages, 9 KB prompt | process CPU while streaming 60 s | 20930 to 23220 ms (value 22800) | 76060 to 84710 ms (value 83500) | 3.6x | 3.2x to 4x | benchmarks/results/like-for-like/raw (5 interleaved pairs, release build of 424a4fb plus the open reporting of this change; summary: benchmarks/results/like-for-like/summarize.py), 2026-11-11 | Median of 5 interleaved pairs (range = min to max over the 5 runs), machine load about 4-6, 60 s window, headless Chromium vs --bench-render. Rust includes software raster; React is the whole Chromium process tree. | React main thread. |
<!-- /results:vs_react -->

Notes on specific rows:

- Longest stall, 200 messages with a 40 KB prompt: Rust 36-136 ms (median 110) against React 2.2-2.6 s (median 2.4 s) is 21.8x
  by the medians, 16.0x with Rust's slowest run and React's fastest, and 73.2x at the other extreme. Both ends are real runs; the
  range is the honest statement. At 1000 messages it is 26.9x.
- Open rows are like for like (see the status paragraph). The withdrawn 24x compared Rust's first sync (rows visible, Markdown not
  yet formatted: 12.9 ms median now, 0.12 s when that figure was published) with React's finished page. The 5.4x recorded next to it
  on 2026-11-10 used Rust's then Markdown time (0.53 s); the pooled Markdown workers since reduced it, and the measured like-for-like
  ratio now is 27.9x at 1000 messages. React's open time with and without the Markdown check differs by only 30 to 40 ms
  (`open_ms` vs `open_formatted_ms` in the raw files): React renders Markdown during the same pass.
- Withdrawn and replaced by measurement on 2026-11-11: the 2026-11-10 values for 1000 and 200 messages (CPU 18.8 s vs 68.1 s,
  React memory 950 MiB, React stall 867 ms and 5.0 s, ...) came from 3 pairs whose raw files were not kept. The new 5-pair medians
  differ (for example React memory 1481 MiB RSS with 9 Chromium processes, React 200-message stall 2.4 s), so the old figures are
  not quoted any more.
- 10,000 messages: on 2026-11-10 upstream's page did not survive in headless Chromium. In these 5 pairs it did: it opened in 23.6 to
  27.2 s and finished the 60 s window with 3.7 GiB RSS and 11.6 s median longest frame. Both outcomes are real; the earlier failure
  was not reproduced.
- 2,000 long structured messages: the third React run did not finish opening within the harness's 120 s limit, so it is excluded
  from the React range; this understates React. At 5,000 messages React did not finish opening within 120 s in 3 of 3 runs.
  That row is a lower bound (the real React open time is above 120 s, unmeasured).
- Memory rows compare Rust RSS with React's RSS summed over every Chromium process; the PSS rows are the more conservative reading.

## Rust against its own earlier state (not React)

Columns read "Rust" = now, "React" = the earlier Rust build in this table only.

<!-- results:rust_only -->
| Rank | Workload | Measure | Rust | React | Ratio | Range | Source (commit), date | Conditions | What drives it |
|---|---|---|---|---|---|---|---|---|---|
| 1 | 1000 messages, ~9 KB reply, real GPU window (FemtoVG-WGPU, Intel UHD, Wayland, 30 s) | process CPU | 11.8 to 14.1 s (value 14.1) | 23.1 to 25.5 s (value 25.5) | 1.8x | 1.6x to 2.1x | docs/STATUS.md at 2cbedad (6add923 React comparison, 54c371f scale, 48bc8ec real window); raw runs were not kept, 2026-11-10 | Rust now (femtovg pipeline-cache patch) vs Rust before; NOT a React comparison. Interleaved runs. | femtovg recompiled pipelines every few frames. |
| 2 | same | median frame rate | 141 to 142 fps (value 141) | 105 to 108 fps (value 105) | 1.3x | 1.3x to 1.3x | docs/STATUS.md at 2cbedad (6add923 React comparison, 54c371f scale, 48bc8ec real window); raw runs were not kept, 2026-11-10 | Rust now vs Rust before; display limit 142 fps. | same |
<!-- /results:rust_only -->

## Rust-only results of the new runs (no React figure)

- 5,000 long structured messages (29.7 MB text): open 0.70-1.12 s, resident memory 166 MiB, process CPU 7.8-9.0 s per 60 s, p95 iteration 2.0-2.9 ms (3 runs, `results-run1/a-5000`).
- 20,000 long structured messages (117.9 MB text): open 4.1 s, 552 MiB, 13.8 s CPU, p95 3.6 ms (1 run, `results-run1/a-20000`). React's run was interrupted, so there is no ratio.
- 2,000 messages, scroll smoke test (Rust only, one run, 120 notches of 400 px, load low): per-frame work p50 3.1 ms, p95 8.6 ms, 2 of 120 frames over 16.7 ms. Not a comparison.
- 2,000 messages, typing while streaming smoke test (Rust only, one run, 20 s window): 100 of 100 keys painted, key to next paint p50 10.1 ms, p95 16.5 ms, max 20.0 ms (paced to a 60 Hz refresh, so it includes waiting for the next frame). Not a comparison.
- React smoke test of scrolling, 2,000 messages, one run under a concurrent build (not clean): 600 notches of 1500 px took 46 s; frame interval p95 150 ms. Indicative only, not in the table.

## Methods

- Workload: the same history, the same demo reply (reasoning, then answer, one word per 18 ms) and the same 60 s window in both apps
  (`benchmarks/react-baseline`, `docs/STATUS.md#against-upstream-react`). React is the published `@assistant-ui/react` 0.15.33 with
  plain CSS in headless Chromium (chrome-headless-shell 1248); Rust is `ai-ui-example-thread --bench N --bench-reply-kb K --bench-render`
  (offscreen, software raster, at most 60 frames a second).
- Interleaved pairs (Rust, React, Rust, React, ...) so machine load hits both alike; the load average before each run is in the raw files.
  Machine: 12 cores, shared and busy (load 4-25 during these runs), so absolutes move and only ratios count. Medians are used for published
  rows; for the new rows all runs are listed as ranges.
- New fixtures: `benchmarks/mock-data/words.py --messages N --seed 7` from `origin/demo/long-threads` (long structured replies with headings,
  lists, tables, code). The Rust side loads them with `--history-file`, the React page with `?data=`; both are on that branch and were built
  in a throwaway worktree (main 2cbedad merged with the branch, conflicts resolved) plus `benchmarks/results/rust-bench-extras.patch`
  (scroll, typing and memory-sample phases and a longer open timeout). Nothing was merged into this branch.
- Reproduce: `benchmarks/results/run-pairs.sh MODE HISTORY KB PAIRS OUT FIXTURE` with `RUST_BIN`, `REACT_BASELINE`, `PLAYWRIGHT`, `CHROME`
  set (see the script header); `python3 benchmarks/results/summarize.py DIR` summarises raw files;
  `python3 benchmarks/results/make-results-json.py` rewrites `docs/results.json`; `python3 scripts/results.py render|check`.
- Raw runs: the like-for-like rows (1000, 200, 10,000 messages) are in `benchmarks/results/like-for-like/raw` with
  `summarize.py` there (rows via `benchmarks/results/run-pairs.sh stream H KB 5 OUT`, release build). The 2,000/5,000/20,000-message rows:
  `~/.cache/ai-ui-rewrite-rust/results/results-run1/` (outside git; summarised in `benchmarks/results/summary-run1.json`).
- What "open" measures: Rust `--bench` prints `open (formatted and painted, like for like)` = from the start of the import to a forced frame
  after the Markdown worker is idle (`history import+first sync` is only the first part). React `bench.mjs` reports `open_ms` (all messages
  in the DOM, plus two frames) and `open_formatted_ms` (every assistant message's table rendered, plus two frames); the tables use the latter.

## Caveats

- Single-event stalls are noisy: Rust's longest iteration at 2,000 messages ranged 35-75 ms, React's 1.6-4.6 s. We report every run, not the best.
- The Rust figures include software rasterisation on the CPU; React's include Chromium's own raster and compositing, with plain CSS (less work than upstream's Tailwind/Radix thread).
  React's main-thread busy time does not count raster threads, which favours React; whole-process CPU is used where stated.
- Rust's "longest stall" is the longest loop iteration (unpaced work); React's is the longest animation-frame interval. Different clocks, same workload.
- The 2,000-message React runs varied a lot (open 42 s and 97 s, one run over 120 s) under a loaded machine; Rust varied little.
- The 5,000 and 20,000 ratios are lower bounds or absent for the reasons above. No case was found where React wins or ties on these workloads.

## Not measured yet (interrupted, or not feasible)

- Interrupted before they ran: React at 20,000 messages (the run was started and stopped); fast-scroll frame time at 2,000/5,000/20,000
  messages for both apps (harnesses written and smoke-tested, no clean pairs); composer typing latency with 5,000 messages (harness written and smoke-tested for 2,000 on Rust only);
  memory growth over a 5-minute stream (RSS every 5 s at 5,000 messages; harness written, not run); React at 5,000 messages with a longer open timeout.
- Not feasible here: React in a real GPU window (no popup windows allowed), so no GPU Rust-vs-React ratio; the GPU rows above are Rust against Rust.
