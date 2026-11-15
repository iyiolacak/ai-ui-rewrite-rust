#!/usr/bin/env python3
"""Build the browser fetch client fixture and run it in headless Chromium.

Needs wasm-bindgen 0.2.129 (WASM_BINDGEN), Playwright (AI_UI_PLAYWRIGHT_MODULE) and
optionally a browser binary (AI_UI_BROWSER_EXECUTABLE). No window opens.
"""
import os
from pathlib import Path
import subprocess
import tempfile

here = Path(__file__).resolve().parent
env = dict(os.environ)
bindgen = env.get("WASM_BINDGEN", "wasm-bindgen")
if subprocess.check_output([bindgen, "--version"], text=True).strip() != "wasm-bindgen 0.2.129":
    raise SystemExit("Expected wasm-bindgen 0.2.129")
with tempfile.TemporaryDirectory(prefix="ai-ui-http-browser-") as tmp:
    target = Path(env.get("CARGO_TARGET_DIR", Path(tmp) / "target")).resolve()
    env["CARGO_TARGET_DIR"] = str(target)
    subprocess.run(["cargo", "build", "--offline", "--locked", "--target", "wasm32-unknown-unknown", "--manifest-path", str(here / "Cargo.toml")], env=env, check=True)
    subprocess.run([bindgen, "--target", "web", "--out-dir", tmp, "--out-name", "test", str(target / "wasm32-unknown-unknown/debug/ai_ui_http_browser_test.wasm")], check=True)
    subprocess.run(["node", str(here / "smoke.mjs"), tmp], env=env, check=True, timeout=60)
