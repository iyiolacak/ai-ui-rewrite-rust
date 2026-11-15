#!/usr/bin/env bash
# Interleaved Rust/React pairs for docs/RESULTS.md. Raw output goes to OUT/, one
# file per run, with the machine load before each run.
#
#   run-pairs.sh MODE HISTORY KB PAIRS OUT [FIXTURE_JSON]
#
# MODE  stream   open + 60 s reply (benchmarks/react-baseline/bench.mjs vs --bench-render)
#       scroll   600 notches of NOTCH_PX (default 1500) through the opened thread, no streaming
#       typing   100 keys, one per 150 ms, starting 5 s into a streaming reply
#       memory   300 s reply, resident memory every 5 s
# Environment: RUST_BIN (an example build with benchmarks/results/rust-bench-extras.patch
# for scroll, typing and memory; any build for stream), REACT_BASELINE (the react-baseline
# directory of a build that accepts ?data= for FIXTURE_JSON), PLAYWRIGHT, CHROME.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
mode=$1 history=$2 kb=$3 pairs=$4 out=$5 fixture=${6:-}
mkdir -p "$out"
rust_args=(--bench "$history" --bench-reply-kb "$kb" --bench-render)
[ -n "$fixture" ] && rust_args+=(--history-file "$fixture")
export REACT_DIST=${REACT_DIST:-$REACT_BASELINE/dist}
for i in $(seq 1 "$pairs"); do
  for side in rust react; do
    f="$out/$mode-$history-$kb-pair$i-$side.txt"
    { echo "# load before: $(cat /proc/loadavg)"; date -u +"# start %FT%TZ"; } > "$f"
    case "$mode/$side" in
      stream/rust) "$RUST_BIN" "${rust_args[@]}" >> "$f" 2>&1 ;;
      stream/react) node "$REACT_BASELINE/bench.mjs" "$history" "$kb" 60 $fixture >> "$f" 2>&1 ;;
      scroll/rust) AI_UI_BENCH_SCROLL=600 AI_UI_BENCH_SCROLL_ONLY=1 AI_UI_BENCH_SCROLL_PX=${NOTCH_PX:-1500} \
        "$RUST_BIN" "${rust_args[@]}" >> "$f" 2>&1 ;;
      scroll/react) NOTCH_PX=${NOTCH_PX:-1500} node "$here/react-extra.mjs" scroll "$history" "$kb" $fixture >> "$f" 2>&1 ;;
      typing/rust) AI_UI_BENCH_TYPE=1 AI_UI_BENCH_WINDOW_S=${WINDOW_S:-40} \
        "$RUST_BIN" "${rust_args[@]}" >> "$f" 2>&1 ;;
      typing/react) node "$here/react-extra.mjs" typing "$history" "$kb" $fixture >> "$f" 2>&1 ;;
      memory/rust) AI_UI_BENCH_SAMPLES=1 AI_UI_BENCH_WINDOW_S=300 "$RUST_BIN" "${rust_args[@]}" >> "$f" 2>&1 ;;
      memory/react) node "$here/react-extra.mjs" memory "$history" "$kb" $fixture >> "$f" 2>&1 ;;
    esac
    echo "# exit $?" >> "$f"
    echo "# load after: $(cat /proc/loadavg)" >> "$f"
  done
done
