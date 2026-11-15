# Contributing

Coding agents should read [`AGENTS.md`](AGENTS.md). Performance changes should
preserve the contracts and record reproducible evidence described in
[`docs/PERFORMANCE.md`](docs/PERFORMANCE.md).

`scripts/gate.sh` runs everything CI runs (`--quick` skips the Slint crates, the interop test and wasm, which need
fontconfig headers and Node). By hand, from the repository root:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

- The Slint crates need the system `fontconfig` development package (`libfontconfig1-dev` on Debian/Ubuntu).
  Without it, build the toolkit-independent crates only: `-p ai-ui-core -p ai-ui-stream
  -p ai-ui-data-stream -p ai-ui-http -p ai-ui-state -p ai-ui-markdown`.
- Behaviour follows upstream assistant-ui (reference commit in [NOTICE](NOTICE) and `docs/STATUS.md`). Tests for
  ported behaviour should be derived from upstream's, and gaps are listed in `docs/STATUS.md` rather than implied.
- `wasm32-unknown-unknown` must keep building for `core`, `state`, `stream`, `data-stream`, `http` and `markdown`.
- Offscreen Slint screenshots (`examples/thread --screenshot`) use the software renderer, which draws styled text
  as raw characters. Check Markdown styling in a real window with `--features gpu-window` (see the example docs).
- Keep changes focused and add a test for behaviour changes.
- Browser checks (`crates/ai-ui-clipboard/tests/browser`, `crates/ai-ui-http/tests/browser`) run headless Chromium through `run.py`; set `WASM_BINDGEN` (0.2.129), `AI_UI_PLAYWRIGHT_MODULE` and optionally `AI_UI_BROWSER_EXECUTABLE`.
