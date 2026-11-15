# 2026-11-14: view fast path, paired measurement

Raw samples cited in [docs/PERFORMANCE.md](../../docs/PERFORMANCE.md#2026-11-14-overlay-removed-view-fast-path-rejected).

- `projection-pairs.txt`: `perf.rs` `projection_cost_per_streamed_update`, release,
  7 interleaved rounds of A, B, C (rotating start order) after one discarded
  warm-up run of each. Lines read `<side> pair<N> smooth=<bool> <messages> messages: <us> us ...`.
  Load averages are recorded before, after each round and at the end.
- A = `a66e993^` (before the overlay commit). B = A + the view fast path
  (`message_at`/`len_selected` in `build_into`), overlay removed.
  C = B with `view.rs` restored to A's. rustc 1.95.0, Intel i7-10750H (12 threads),
  Fedora 43, Linux 7.3.
- Machine not idle: load average about 4.5-4.7 on 12 threads.
- Result: B is not distinguishable from A or C (see PERFORMANCE.md); the fast path was reverted.

## Reproduce

```sh
S=/tmp/ab; mkdir -p $S/bin
git worktree add --detach $S/A a66e993^
git worktree add --detach $S/B 44bed08          # overlay removed, fast path kept
git worktree add --detach $S/C 44bed08          # then restore view.rs from a66e993^ in C
git show a66e993^:crates/ai-ui-state/src/view.rs > $S/C/crates/ai-ui-state/src/view.rs
for r in A B C; do (cd $S/$r && CARGO_TARGET_DIR=$S/t-$r cargo test --release --locked -p ai-ui-state --test perf --no-run); done
# copy each perf-* executable to $S/bin/$r, run one discarded warm-up each, then rounds:
$S/bin/$r --ignored --nocapture projection_cost_per_streamed_update
```
