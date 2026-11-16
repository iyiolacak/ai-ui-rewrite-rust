# Benchmark runs dataset (`docs/runs.json`)

One structured file with every raw benchmark run of 2026-11-11 (Rust `--bench-render` against upstream React's default thread, which does not virtualise the list, in headless Chromium),
the web-demo measurements and the test runs. The landing page draws its box and strip plots, per-pair points and run comparisons from it.
`docs/results.json` / `docs/RESULTS.md` remain the published, ranked result tables; this file keeps every per-pair value.

## Regenerate and check

```sh
python3 benchmarks/results/make-runs-json.py          # parse the raw files, write docs/runs.json
python3 benchmarks/results/make-runs-json.py --check  # exit 1 if docs/runs.json is out of date
```

Stdlib only. Every raw file is in the repository: runs 1 to 4 in `benchmarks/results/like-for-like/raw`, `like-for-like-run2`,
`like-for-like-run3` and `like-for-like-run4`, the web measurements and test logs in `benchmarks/results/web-and-tests` (home-directory
paths in the test logs replaced by `~`). Run 4 is included only when its folder is complete (9 `rust-pairN.out`, 9 `react-pairN.json`,
`power-after.txt` or `power-final.txt`, `load.txt`).

The landing page's numbers are derived from this file by `site/scripts/fill-bench.py` (`--check` fails when they drift): for each
measure it shows the median of the run least favourable to Rust and the range of the four run medians.

Two kinds of range appear in the docs. Here, and on the landing page, a ratio range is the minimum and maximum of ratios (per run
median, or per pair). `docs/RESULTS.md` instead gives each side's min-to-max values and divides the extremes (slowest React by
fastest Rust and the reverse), which is wider.

## Schema (version 1)

Top level: `{schema, generated_from, generated_by, runs, web, tests, earlier_rows, notes}`.

`runs[]` (each a set of interleaved Rust/React pairs):

| field | meaning |
|---|---|
| `id`, `label`, `date`, `pairs` | `run1`, `run2`, `run3`, `run4`, plus `run1-200x40` and `run1-10000` (other workloads of run 1) |
| `workload` | `{messages, prompt_kb, window_s, ...}` |
| `power` | `{tuned_profile, governor, epp}`; the string `"not recorded"` when unknown. Run 3/4 also carry `power_before_raw`, `power_after_raw` (the captured lines) |
| `ac` | `"plugged in"` or `"unknown"` |
| `load_before_each_pair`, `load_after_each_pair` | 1-minute load average, one per pair, or `"not recorded"` |
| `mean_cpu_mhz_per_pair` | mean CPU frequency (MHz, all cores, sampled every 5 s) during each pair, or `"not recorded"` |
| `conditions`, `sources` | one-sentence conditions; raw-file locations |
| `measures` | see below |

`measures.<name>`: `open_like_for_like` (ms), `open_first_sync_rust_only` (ms, not comparable with React), `cpu_60s` (ms),
`stall` (ms), `memory_end_rss` (MiB), `memory_end_pss` (MiB; React PSS against Rust RSS), `rust_iterations_over_4ms`,
`react_frames_over_33ms`, `react_frame_p95` (ms). Each has `unit`, `better` (`lower`), a `note`, and where present:
`rust` / `react` (arrays, one value per pair, pair order), `*_median`, `*_min`, `*_max`, `ratio_rule`,
`ratio_per_pair_floor` (array), `ratio_floor_median` (ratio of the two medians, the headline form), `ratio_floor_least_favourable`
and `ratio_floor_most_favourable` (min and max of the per-pair ratios) and `ratio_floor_median_of_pair_ratios`.
Ratios are computed by the generator from the stored values, floored to one decimal (never rounded up). For a measure where higher
is better the rule is rust/react; the current measures are all lower-is-better.

`web`: `wasm_bytes` / `wasm_mib` (raw, gzip, brotli), and `measurements.job1` / `job4_fps_fixed`, each with `load_1min_before`,
`history_messages` (10,000) and `scenarios.empty` / `scenarios.history`: per-run `first_paint_ms`, `wasm_memory_mib`, `fps`,
`console_errors`, and medians. job1 recorded fps 0 for the empty thread (harness did not drive frames); job4 is the fixed fps harness.
`tests`: three sequential `cargo test --workspace --offline --release` runs, passed/failed/ignored summed per run (334 / 0 / 7 each).
`earlier_rows`: the 2,000-message long-fixture rows and GPU rows from `docs/results.json`, copied as given, with their own conditions.

## The runs

| id | pairs | workload | power profile / governor | AC | load before pairs | notes |
|---|---|---|---|---|---|---|
| run1 | 5 | 1000 msgs, 9 KB | not recorded (probably power saver) | unknown | 4.6 - 4.9 | the first like-for-like run |
| run1-200x40 | 5 | 200 msgs, 40 KB | not recorded | unknown | 3.7 - 5.2 | same session |
| run1-10000 | 5 | 10,000 msgs, 9 KB | not recorded | unknown | 4.6 - 6.0 | same session; not like for like: Rust formatted Markdown only for the newest 1,200 messages |
| run2 | 9 | 1000 msgs, 9 KB | not recorded | unknown | 2.7 - 4.8 | independent repeat |
| run3 | 9 | 1000 msgs, 9 KB | tuned `throughput-performance`, governor `performance` | plugged in | 0.6 - 2.9 | frequency fell from about 4.2 GHz (pairs 1-4) to about 2.5-2.9 GHz (pairs 5-9), see `mean_cpu_mhz_per_pair` |
| run4 | 9 | 1000 msgs, 9 KB | tuned `balanced`, governor `powersave`, EPP `balance_performance` | plugged in | 1.8 - 13.6 | confounded: an unrelated `rustc` process and other activity raised the load during pairs 2, 3, 6, 7, 8; mean CPU frequency 3.2 - 4.0 GHz per pair |

Headline open-time ratio against the default unvirtualised React thread (React median / Rust median, floored): run 1 27.9x, run 2 25.3x, run 3 16.6x, run 4 20.1x
(see `ratio_floor_median`; the per-pair range is in the least/most favourable fields).

## The honest reading

- Every React figure here is upstream's default thread, which renders every message into the DOM and does not virtualise the list.
  Upstream documents a virtualised thread for long conversations; it has not been measured yet and would narrow the gap on long threads.
  The React page runs on `useLocalRuntime`, which recomputes each message's state twice per streamed update; upstream's templates use
  the AI SDK runtime (an external store), which does it once, so the CPU and stall ratios likely overstate that part. A corrected rerun is in progress.
  The React page's plain CSS also omits the default kit's `content-visibility: auto` on message roots, which lets Chromium skip
  rendering off-screen messages, so the CSS difference cannot be said to favour React; its net effect is not measured.
- At 10,000 messages (`run1-10000`) the Rust app formatted Markdown only for the newest 1,200 messages (600 of 5,000 replies;
  older ones showed plain text), while React formatted all of them, so the 10,000-message rows are not like for like. The
  2,000-message `earlier_rows` (2026-11-10) had the same limit. A fix and a rerun are in progress. The 1000- and 200-message
  runs are unaffected: every reply there was formatted.
- The open-time ratio differs between runs because of power profile and machine load. React's open time is more CPU-speed sensitive
  than Rust's (React 2.9 s, 2.5 s, 1.4 s, 1.7 s; Rust 105 ms, 98 ms, 83 ms, 83 ms in runs 1 to 4), so the faster the CPU runs, the smaller the ratio.
  Rust is faster in every pair of every run; the size of the gap is not one number. Show the range, not one figure.
- CPU totals are load-sensitive (Rust 17.2 s in run 1, 4.3 s in run 3, 6.9 s in run 4) and are not a headline figure.
- Run 4 (Balanced) cannot separate the power profile from load: its load varied from 1.8 to 13.6 because of activity outside the benchmark. Rust's open time is the same in runs 3 and 4 (83.4 and 83.2 ms); React's differs (1385 and 1674 ms), and one React pair in run 4 took 4571 ms, which drives the top of its range (54.9x for that pair).
- Stalls are single-event maxima and noisy; memory is the most stable measure across runs.
- Within a run, pairs are interleaved (Rust, React, Rust, ...) so load hits both alike; pair order is the file order.
- Runs 1 and 2 did not record the power profile, and whether the machine was plugged in is unknown.
