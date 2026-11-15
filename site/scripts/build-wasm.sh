#!/usr/bin/env bash
# Build the example for the browser into site/wasm (git-ignored).
# Needs: rustup target wasm32-unknown-unknown, wasm-bindgen CLI 0.2.129 (WASM_BINDGEN to point at it).
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
bindgen="${WASM_BINDGEN:-wasm-bindgen}"
[ "$("$bindgen" --version)" = "wasm-bindgen 0.2.129" ] || { echo "need wasm-bindgen 0.2.129" >&2; exit 1; }
cd "$root"
cargo build --release --offline --locked --target wasm32-unknown-unknown -p ai-ui-example-thread --features gpu-window
target="${CARGO_TARGET_DIR:-$root/target}"
mkdir -p site/public/wasm
"$bindgen" --target web --no-typescript --out-dir site/public/wasm --out-name ai_ui_example_thread \
  "$target/wasm32-unknown-unknown/release/ai-ui-example-thread.wasm"
cp examples/thread/web/pixel-snap.js site/public/wasm/pixel-snap.js
ls -l site/public/wasm
