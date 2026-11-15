#!/usr/bin/env bash
# The same checks CI runs, for a local machine. Usage: scripts/gate.sh [--quick]
#   --quick  skip the Slint crates, the interop test and the wasm builds (needs no fontconfig headers or Node)
set -euo pipefail
cd "$(dirname "$0")/.."

RUNTIME="-p ai-ui-sdk-stream -p ai-ui-core -p ai-ui-stream -p ai-ui-data-stream -p ai-ui-http -p ai-ui-state -p ai-ui-markdown -p ai-ui-clipboard"
WASM="-p ai-ui-sdk-stream -p ai-ui-core -p ai-ui-state -p ai-ui-stream -p ai-ui-data-stream -p ai-ui-http -p ai-ui-markdown -p ai-ui-clipboard"
SCOPE="--workspace"
[ "${1:-}" = "--quick" ] && SCOPE="$RUNTIME"

echo "== fmt";    cargo fmt --all --check
echo "== clippy"; cargo clippy --locked $SCOPE --all-targets -- -D warnings
echo "== docs";   RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps $RUNTIME
if [ "${1:-}" != "--quick" ]; then
  echo "== interop"
  (cd benchmarks/ai-sdk-stream && npm ci --ignore-scripts >/dev/null && npm run --silent verify)
  export AI_UI_REQUIRE_INTEROP=1
  echo "== wasm32"; cargo build --locked --target wasm32-unknown-unknown $WASM
fi
echo "== tests";  cargo test --locked $SCOPE
echo "gate passed"
