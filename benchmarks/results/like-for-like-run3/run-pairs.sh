#!/usr/bin/env bash
R=$HOME/.cache/ai-ui-rewrite-rust/results/perf-profile
RB=$HOME/.cache/ai-ui-rewrite-rust/worktrees/haiku-tests/benchmarks/react-baseline
BIN=$HOME/.cache/ai-ui-rewrite-rust/targets/haiku-tests/release/ai-ui-example-thread
export PLAYWRIGHT=$HOME/.cache/ai-ui-rewrite-rust/tools/pw/node_modules/playwright
export CHROME=$HOME/.cache/ms-playwright/chromium-1248/chrome-linux64/chrome
export TMPDIR=$HOME/.cache/ai-ui-rewrite-rust/tmp
cd "$RB" || exit 1
for i in $(seq 1 9); do
  echo "load before pair $i: $(uptime)" >> $R/load.txt
  echo "pair $i start $(date +%T)" >> $R/markers.txt
  "$BIN" --bench 1000 --bench-reply-kb 9 --bench-render > $R/rust-pair$i.out 2> $R/rust-pair$i.err.log
  echo "rust exit $?" >> $R/status.txt
  node bench.mjs 1000 9 60 > $R/react-pair$i.json 2> $R/react-pair$i.err.log
  echo "react exit $?" >> $R/status.txt
  echo "pair $i end $(date +%T)" >> $R/markers.txt
  echo "load after pair $i: $(uptime)" >> $R/load.txt
done
echo ALLDONE >> $R/status.txt
