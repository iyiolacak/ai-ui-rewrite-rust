#!/usr/bin/env bash
# Interleaved Rust vs React runs of the same workload; raw output on stdout.
#
#   benchmarks/react-baseline/compare.sh HISTORY REPLY_KB PAIRS [RUST_BIN]
#
# Rust: `ai-ui-example-thread --bench HISTORY --bench-reply-kb REPLY_KB --bench-render`
# (offscreen; app work + layout + software paint on one thread, 60 s window).
# React: bench.mjs in headless Chromium (main-thread time; raster runs on other
# threads and is not counted, which favours React).
# Needs: a release build of the example, `npm ci --ignore-scripts && npm run build`
# here, PLAYWRIGHT (path to a playwright package) and, if Playwright's browser
# is not installed, CHROME (a chrome-headless-shell binary).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
history=${1:-1000}
kb=${2:-9}
pairs=${3:-5}
bin=${4:-$root/target/release/ai-ui-example-thread}
for i in $(seq 1 "$pairs"); do
  echo "=== rust history=$history kb=$kb run $i"
  "$bin" --bench "$history" --bench-reply-kb "$kb" --bench-render
  echo "=== react history=$history kb=$kb run $i"
  node "$here/bench.mjs" "$history" "$kb" 60
done
