#!/usr/bin/env bash
# Interleaved A/B of two prebuilt example binaries, with CPU and peak RSS.
# Usage: ab-run.sh DIR PAIRS   (DIR holds binaries a and b; writes DIR/bench.log)
set -euo pipefail
dir=$1; pairs=${2:-5}
cd "$dir"
log=bench.log
uptime > "$log"
scenarios=(
  "--bench 1000 --bench-reply-kb 9 --bench-render"
  "--bench 1000 --bench-reply-kb 9"
  "--bench 200 --bench-reply-kb 40 --bench-render"
)
for s in "${scenarios[@]}"; do
  # One warmup per side, discarded.
  # shellcheck disable=SC2086
  ./a $s >/dev/null 2>&1; ./b $s >/dev/null 2>&1
  for i in $(seq 1 "$pairs"); do
    if (( i % 2 )); then order=(a b); else order=(b a); fi
    for side in "${order[@]}"; do
      echo "=== $side | $s | run $i" >> "$log"
      # shellcheck disable=SC2086
      /usr/bin/time -f 'TIME user=%U sys=%S wall=%e maxrss_kb=%M' ./$side $s >> "$log" 2>&1
    done
  done
done
uptime >> "$log"
