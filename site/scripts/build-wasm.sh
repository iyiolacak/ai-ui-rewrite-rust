#!/usr/bin/env bash
# Build the example for the browser into site/wasm (git-ignored).
# Needs: rustup target wasm32-unknown-unknown, wasm-bindgen CLI 0.2.129 (WASM_BINDGEN to point at it).
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
bindgen="${WASM_BINDGEN:-wasm-bindgen}"
[ "$("$bindgen" --version)" = "wasm-bindgen 0.2.129" ] || { echo "need wasm-bindgen 0.2.129" >&2; exit 1; }
cd "$root"
# The binary is published, and panic messages embed source paths: map the checkout and the cargo home to neutral
# prefixes so no home-directory path (user name, folder names) ends up in the .wasm.
# The target directory too: generated sources (Slint's app.rs in OUT_DIR) live there.
cargo_home="${CARGO_HOME:-$HOME/.cargo}"
target="${CARGO_TARGET_DIR:-$root/target}"
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$cargo_home=/cargo --remap-path-prefix=$target=/target --remap-path-prefix=$root=/build"
cargo build --release --offline --locked --target wasm32-unknown-unknown -p ai-ui-example-thread --features gpu-window
mkdir -p site/public/wasm
"$bindgen" --target web --no-typescript --out-dir site/public/wasm --out-name ai_ui_example_thread \
  "$target/wasm32-unknown-unknown/release/ai-ui-example-thread.wasm"
cp examples/thread/web/pixel-snap.js site/public/wasm/pixel-snap.js
if grep -aq "$HOME" site/public/wasm/*; then echo "the build still contains $HOME paths" >&2; exit 1; fi
ls -l site/public/wasm
