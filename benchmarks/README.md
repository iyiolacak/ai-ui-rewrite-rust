# Benchmarks

Reproducibility harnesses behind the published numbers. The results and how to read them are in
[docs/RESULTS.md](../docs/RESULTS.md) (checked against `docs/results.json` by `python3 scripts/results.py check`).

| Directory | What |
|---|---|
| `react-baseline` | Upstream assistant-ui (React) workload for CPU and memory comparison |
| `ai-sdk-stream` | AI SDK oracle and differential test for `ai-ui-sdk-stream` |
| `upstream-look` | Screenshots of upstream's thread for visual comparison |
| `results` | Scripts that produced `docs/results.json` |
| `performance-2026-11-10` | Raw Rust measurements cited in [docs/PERFORMANCE.md](../docs/PERFORMANCE.md) |
| `performance-2026-11-11` | Host row-change feed: A/B samples, RSS by thread size, harness ([README](performance-2026-11-11/README.md)) |
