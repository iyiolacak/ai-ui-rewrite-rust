#!/usr/bin/env bash
R=$HOME/.cache/ai-ui-rewrite-rust/results/haiku-tests
RB=$HOME/.cache/ai-ui-rewrite-rust/worktrees/haiku-tests/benchmarks/react-baseline
BIN=$HOME/.cache/ai-ui-rewrite-rust/targets/haiku-tests/release/ai-ui-example-thread
export PLAYWRIGHT=$HOME/.cache/ai-ui-rewrite-rust/tools/pw/node_modules/playwright
export CHROME=$HOME/.cache/ms-playwright/chromium-1248/chrome-linux64/chrome
cd "$RB" || exit 1
for i in $(seq 1 9); do
  echo "load before pair $i: $(uptime)" >> $R/job2-load.txt
  "$BIN" --bench 1000 --bench-reply-kb 9 --bench-render > $R/job2-rust-pair$i.out 2> $R/job2-rust-pair$i.err.log
  echo "rust exit $?" >> $R/job2-status.txt
  node bench.mjs 1000 9 60 > $R/job2-react-pair$i.json 2> $R/job2-react-pair$i.err.log
  echo "react exit $?" >> $R/job2-status.txt
  echo "load after pair $i: $(uptime)" >> $R/job2-load.txt
done
echo ALLDONE >> $R/job2-status.txt
