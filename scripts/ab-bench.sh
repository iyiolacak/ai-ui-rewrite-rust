#!/usr/bin/env bash
# Decide whether a change is worth keeping: build two git revisions of the
# example and run the same bench scenarios interleaved (A, B, A, B, ...), so a
# busy machine affects both sides alike.
#
#   scripts/ab-bench.sh BASE_REV NEW_REV [PAIRS] > ab.txt
#   (NEW_REV may be "WORKTREE" for the uncommitted working tree.)
#
# Scenarios (override with AB_SCENARIOS, one per line):
#   1000 messages, 9 KB reply, with layout and software paint
#   1000 messages, 9 KB reply, app work only
#   200 messages, 40 KB reply, with layout and software paint
# Compare medians of p95, total and "iterations over 4 ms"; keep a change only
# when it wins without a regression elsewhere. The 2026-11-10 flat-row
# experiment (docs/STATUS.md#performance) was judged this way.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
base=${1:?base revision}
new=${2:?new revision or WORKTREE}
pairs=${3:-5}
scenarios=${AB_SCENARIOS:-"--bench 1000 --bench-reply-kb 9 --bench-render
--bench 1000 --bench-reply-kb 9
--bench 200 --bench-reply-kb 40 --bench-render"}
work=$(mktemp -d "${TMPDIR:-/tmp}/ai-ui-ab.XXXXXX")
trap 'git -C "$root" worktree remove --force "$work/tree" >/dev/null 2>&1 || true' EXIT

build() { # revision -> binary path
  local rev=$1 out=$2 src
  if [ "$rev" = WORKTREE ]; then
    src=$root
  else
    git -C "$root" worktree remove --force "$work/tree" >/dev/null 2>&1 || true
    git -C "$root" worktree add -q --detach "$work/tree" "$rev"
    src=$work/tree
  fi
  # ai-ui-slint bakes its source path into the build dependency (ui_dir), and a
  # shared target dir would otherwise reuse the other tree's copy.
  touch "$src/crates/ai-ui-slint/src/lib.rs" "$src/examples/thread/build.rs"
  (cd "$src" && CARGO_TARGET_DIR="$root/target" cargo build --release --offline -q -p ai-ui-example-thread)
  command cp -f "$root/target/release/ai-ui-example-thread" "$out"
}
build "$base" "$work/a"
build "$new" "$work/b"
# The next ordinary build must not reuse paths into the removed worktree.
touch "$root/crates/ai-ui-slint/src/lib.rs" "$root/examples/thread/build.rs"

while IFS= read -r scenario; do
  [ -n "$scenario" ] || continue
  for i in $(seq 1 "$pairs"); do
    for side in a b; do
      echo "=== $side ($([ $side = a ] && echo "$base" || echo "$new")) $scenario run $i"
      # shellcheck disable=SC2086
      "$work/$side" $scenario
    done
  done
done <<< "$scenarios"
