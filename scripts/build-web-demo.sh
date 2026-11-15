#!/usr/bin/env bash
# Build the thread example for the browser and package it.
#   Output: dist-web/ai-ui-web-demo.tar.gz (JS glue + wasm + pixel-snap.js) and dist-web/ai-ui-web-demo.sha256
#   WASM_BINDGEN=/path/to/wasm-bindgen overrides the binary; its version must equal the one in Cargo.lock.
set -euo pipefail
cd "$(dirname "$0")/.."

want=$(awk '/^name = "wasm-bindgen"$/{getline; gsub(/[^0-9.]/,"",$3); print $3; exit}' Cargo.lock)
bindgen=${WASM_BINDGEN:-$(command -v wasm-bindgen || true)}
[ -x "${bindgen:-}" ] || { echo "wasm-bindgen $want not found; set WASM_BINDGEN=/path/to/wasm-bindgen" >&2; exit 1; }
have=$("$bindgen" --version | awk '{print $2}')
[ "$have" = "$want" ] || { echo "wasm-bindgen is $have but Cargo.lock needs $want" >&2; exit 1; }

cargo build --locked --release --target wasm32-unknown-unknown -p ai-ui-example-thread --features gpu-window
target=${CARGO_TARGET_DIR:-target}/wasm32-unknown-unknown/release/ai-ui-example-thread.wasm

out=dist-web
stage=$out/stage
rm -rf "$stage" && mkdir -p "$stage"
"$bindgen" --target web --no-typescript --out-name ai_ui_example_thread --out-dir "$stage" "$target"
cp examples/thread/web/pixel-snap.js "$stage/"
tar -C "$stage" -czf $out/ai-ui-web-demo.tar.gz ai_ui_example_thread.js ai_ui_example_thread_bg.wasm pixel-snap.js
(cd $out && sha256sum ai-ui-web-demo.tar.gz > ai-ui-web-demo.sha256)
ls -l "$stage"/*.wasm "$stage"/*.js $out/ai-ui-web-demo.tar.gz
rm -rf "$stage"
