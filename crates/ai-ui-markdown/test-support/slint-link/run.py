#!/usr/bin/env python3
"""Check projected markup against Slint's pinned Markdown parser."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

here = Path(__file__).resolve().parent
repo = here.parents[3]
with tempfile.TemporaryDirectory(prefix="ai-ui-slint-link-") as temporary:
    root = Path(temporary)
    shutil.copyfile(repo / "Cargo.lock", root / "Cargo.lock")
    (root / "Cargo.toml").write_text(f'''[package]
name = "ai-ui-slint-link-compat-test"
version = "0.0.0"
edition = "2021"
[workspace]
[[test]]
name = "compat"
path = {json.dumps(str(here / "compat.rs"))}
[dependencies]
ai-ui-markdown = {{ path = {json.dumps(str(repo / "crates/ai-ui-markdown"))} }}
i-slint-common = {{ version = "=1.18.1", features = ["markdown"] }}
pulldown-cmark = {{ version = "=0.13.6", default-features = false }}
''')
    env = dict(os.environ)
    env.setdefault("CARGO_BUILD_JOBS", "1")
    rustc = subprocess.check_output(["rustc", "-vV"], cwd=repo, text=True, env=env)
    host = next(line.removeprefix("host: ") for line in rustc.splitlines() if line.startswith("host: "))
    manifest = str(root / "Cargo.toml")
    subprocess.run(["cargo", "metadata", "--manifest-path", manifest, "--offline", "--format-version", "1", "--filter-platform", host], cwd=repo, env=env, check=True, stdout=subprocess.DEVNULL)
    subprocess.run(["cargo", "test", "--manifest-path", manifest, "--offline", "--locked", "--test", "compat"], cwd=repo, env=env, check=True)
