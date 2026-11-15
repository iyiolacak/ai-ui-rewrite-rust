# Like-for-like run 2 (2026-11-11)

A second, independent run of the 1000-message, 9 KB prompt comparison in `../like-for-like/` (run 1). Run 1 is not replaced.

- 9 interleaved pairs (Rust, then React, nine times), release build, 60 s window, headless Chromium vs `--bench-render`.
- Commands: `job2-run-pairs.sh` (per pair: `ai-ui-example-thread --bench 1000 --bench-reply-kb 9 --bench-render`, then
  `node bench.mjs 1000 9 60` in `benchmarks/react-baseline`). Per-pair outputs: `job2-rust-pairN.out`, `job2-react-pairN.json`.
  `job2-summary.txt` has medians and ranges; `job2-load.txt` has `uptime` before and after each pair.
- Machine load average (1 min) before each pair 1 to 9: 3.19, 2.71, 2.70, 2.65, 4.02, 4.09, 4.76, 2.83, 2.66; after pair 9: 2.41.
  This is lighter than run 1 (about 4-6), which is why Rust's CPU is lower here (median 9.7 s against 17.2 s) and the CPU ratio
  higher (7.1x against 4.2x). CPU is load-sensitive and not a headline figure; open, stall and memory agreed within the ranges.
- Rows: `python3 benchmarks/results/make-results-json.py` parses these files into `docs/results.json` (ids `run2-*`);
  `python3 scripts/results.py render|check` produces and verifies the tables in `docs/RESULTS.md`.
