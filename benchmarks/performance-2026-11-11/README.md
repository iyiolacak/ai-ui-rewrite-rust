# 2026-11-11: projected row changes reach the host

Raw measurements cited in [docs/PERFORMANCE.md](../../docs/PERFORMANCE.md#2026-11-11-projected-changes-reach-the-host).

- Base: `173765d` (`main`). Candidate: `437fd45` (`perf/host-row-changes-and-cancellation`).
- rustc 1.95.0, release profile (`lto = true`, `codegen-units = 1`), Slint software renderer.
- Intel i7-10750H, 12 threads, 16 GB, Fedora 43 (Linux 7.2). The machine was **not quiet**:
  load average 4–8 during the A/B session, and run times drift by up to 35% across a session.
  Read the per-pair ratios, not the medians.

| File | What |
|---|---|
| `host-reconciliation.txt` | `perf.rs` microbenchmark: full host scan vs changed-row feed, candidate state crate in both modes, 9 interleaved pairs |
| `ab-bench.log` | Interleaved A/B, 3 scenarios × (1 discarded warmup + 5 pairs, alternating order), with `/usr/bin/time` |
| `ab-summary.txt` | `ab-parse.py` output for `ab-bench.log` |
| `rss-by-size.txt` | RSS after opening and at the end of a short run, 200/1000/4000 messages, 3 runs per side |
| `ab-run.sh`, `ab-parse.py` | The harness and summarizer used here |

## Reproduce

```sh
# Build each side in its own target directory (see the shared-target note in PERFORMANCE.md).
git worktree add --detach /tmp/ab-base 173765d
(cd /tmp/ab-base && CARGO_TARGET_DIR=$PWD/target/ab-base cargo build --release --locked -p ai-ui-example-thread)
CARGO_TARGET_DIR=$PWD/target/ab-new cargo build --release --locked -p ai-ui-example-thread
mkdir -p /tmp/ab
# Copy both binaries at the same time: see the RSS note in PERFORMANCE.md.
cp /tmp/ab-base/target/ab-base/release/ai-ui-example-thread /tmp/ab/a
cp target/ab-new/release/ai-ui-example-thread /tmp/ab/b
bash benchmarks/performance-2026-11-11/ab-run.sh /tmp/ab 5
python3 -I benchmarks/performance-2026-11-11/ab-parse.py /tmp/ab/bench.log

cargo test --locked --release -p ai-ui-state --test perf \
  host_reconciliation_cost_per_streamed_update -- --ignored --nocapture
```
