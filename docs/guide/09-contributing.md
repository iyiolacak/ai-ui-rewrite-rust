---
title: Contributing and testing
description: Repository layout, prerequisites, the gate, tests, the benchmark method, and the conventions for code, docs and licences.
order: 9
section: Reference
---

# Contributing and testing

**What this page gives you.** This page covers the repository layout, prerequisites, the gate (`scripts/gate.sh`), the tests, the benchmark and A/B method, how to add a crate, and the documentation and licence conventions.

Read [CONTRIBUTING.md](../../CONTRIBUTING.md) and [AGENTS.md](../../AGENTS.md) first. This page expands them.
Behaviour follows upstream assistant-ui, and gaps are listed in [STATUS](../STATUS.md) rather than implied.

## Repository layout

| Path | Contents |
|---|---|
| `crates/` | Ten crates: `ai-ui-core`, `-stream`, `-sdk-stream`, `-data-stream`, `-http`, `-state`, `-markdown`, `-clipboard`, and the Slint crates `ai-ui-markdown-slint` and `ai-ui-slint` |
| `examples/thread` | The demo app and the host of the Slint components; also the web demo and the `--bench` harness |
| `benchmarks/` | `react-baseline`, `ai-sdk-stream` (AI SDK oracle), `upstream-look`, `results` (scripts behind `docs/results.json`), `performance-2026-11-10` (raw samples) |
| `docs/` | Reference documents, the guide in `docs/guide/`, and `results.json` |
| `scripts/` | `gate.sh`, `ab-bench.sh`, `build-web-demo.sh`, `results.py`, `check-guide.py` |
| `patches/` | The carried femtovg patch, described in `patches/README.md` |

Ownership by crate is in AGENTS.md: the core stays free of UI, I/O, ambient clocks and randomness; `ai-ui-sdk-stream`
is a separate strict reducer that you should not casually merge with the compatibility-oriented decoder in
`ai-ui-stream`; application-specific caching and paging belong in the host, not the components.

## Prerequisites

- **Rust.** The workspace `rust-version` is 1.85. A few crates need more, because their locked dependencies do:
  1.86 for `ai-ui-clipboard`, 1.88 for `ai-ui-markdown`, 1.92 for `ai-ui-slint` and `ai-ui-markdown-slint`. Do not
  raise a version to make a convenience API compile.
- **fontconfig headers** for the Slint crates: `libfontconfig1-dev` on Debian and Ubuntu. If `pkg-config` cannot find
  them, point `PKG_CONFIG_PATH` at the directory holding `fontconfig.pc`. Without the headers, build only the
  toolkit-independent crates: `-p ai-ui-core -p ai-ui-stream -p ai-ui-data-stream -p ai-ui-http -p ai-ui-state
  -p ai-ui-markdown`.
- **wasm.** `rustup target add wasm32-unknown-unknown`. Browser tests and the web demo need `wasm-bindgen` 0.2.129,
  the version in `Cargo.lock`; set `WASM_BINDGEN=/path/to/wasm-bindgen` if it is not on `PATH`.
- **Node** for the AI SDK interop check (`benchmarks/ai-sdk-stream`) and the React baseline.
- **Chromium** for the browser tests: set `AI_UI_PLAYWRIGHT_MODULE` and, optionally, `AI_UI_BROWSER_EXECUTABLE`.

## The gate

`scripts/gate.sh` runs the same checks CI runs, in this order:

1. `cargo fmt --all --check`
2. `cargo clippy --locked ... --all-targets -- -D warnings`
3. `cargo doc --locked --no-deps` on the runtime crates, with `RUSTDOCFLAGS="-D warnings"`
4. The AI SDK interop check: `npm ci --ignore-scripts` and `npm run verify` in `benchmarks/ai-sdk-stream`, then
   `AI_UI_REQUIRE_INTEROP=1` so a skipped interop test fails
5. `cargo build --locked --target wasm32-unknown-unknown` for the non-Slint crates
6. `cargo test --locked`

`scripts/gate.sh --quick` limits the scope to the runtime crates and skips the interop and wasm steps; it needs no
fontconfig headers or Node. Run the full gate before you propose a change that touches the Slint crates, wasm or
a wire protocol.

## Tests

- **Ported behaviour.** Derive tests from upstream's for each module you port, and add a test for every behaviour
  change.
- **Headless Slint window tests.** The Slint crates test components in offscreen windows
  (`crates/ai-ui-slint/tests/`, for example `action_bar.rs`, `composer_keys.rs`, `paged_text.rs`). They need no
  display. Render behaviour changes in an offscreen test and look at the image. The software renderer draws styled
  text as plain characters, so it does not establish Markdown styling fidelity; check that in a real window with
  `--features gpu-window`.
- **Browser tests.** `crates/ai-ui-clipboard/tests/browser` and `crates/ai-ui-http/tests/browser` run headless Chromium
  through `run.py` against a wasm build.
- **Interop.** `crates/ai-ui-http/tests/interop_ai_sdk.rs` runs against a real AI SDK 7 server over a mock model.
- **Debug builds are not for timing.** They include repository invariant walks.
- **Test status (2026-11-11, origin/main c2beb60).** Three consecutive full workspace runs (`cargo test --workspace --offline --release`): 334 passed, 0 failed, 7 ignored each time, no flaky tests observed. The ignored tests are 5 perf benchmarks, 1 example test and 1 doc test.

## Benchmark method

A change that claims to be faster must be measured, and the measurement must be reproducible:

- Use release builds, the same toolchain, workload and profile for both revisions.
- Warm up, then alternate the two builds (A, B, A, B...) with `scripts/ab-bench.sh BASE_REV NEW_REV [PAIRS]`. Do not
  compile or run other benchmarks at the same time.
- Report medians and ranges, keep the raw samples, and separate decoding, snapshots, projection, layout and paint, and
  whole-process CPU. A faster parser is not a frame measurement.
- The development machine is shared and busy (load 4-25 in recorded runs). Absolute numbers move; ratios from
  interleaved pairs are what count. Say so in the write-up.
- **Rule: results are published only if they were measured and can be reproduced.** Put the command and the raw files
  in the repository, record rejected experiments next to accepted ones in [PERFORMANCE](../PERFORMANCE.md), and add
  comparisons against upstream to `docs/results.json`. `python3 scripts/results.py render` rewrites the tables in
  RESULTS.md and floors every ratio to one decimal; `check` fails if the page and the JSON disagree.
- Do not add machine-specific compiler flags, `unsafe` code or dependencies without evidence that the bottleneck
  warrants them.

## Adding a crate or a feature

1. A new crate goes in `crates/<name>` (the workspace takes `crates/*`). Inherit `edition`, `license` and
   `rust-version` from the workspace, and give it a one-line `description`. Mark it `publish = false` if it depends on
   Slint.
2. Decide whether it must build for wasm. If so, add it to `RUNTIME` and `WASM` in `scripts/gate.sh` and to the CI
   package set, and keep it free of threads, clocks and file I/O.
3. If its dependencies need a newer Rust than 1.85, declare that in the crate's `rust-version`, and expect CI to check
   it with that exact toolchain.
4. Port the matching upstream tests. Record anything left out in STATUS.
5. Update the crate table in [Crate reference](06-crates.md), and [Coming from assistant-ui](08-from-assistant-ui.md)
   if it changes parity.
6. For a runtime feature that is hot, check the contracts in AGENTS.md: the bounded change journal, in-place updates
   only when a snapshot is uniquely held, and the 64-read yield in the stream scheduler.

## Documentation conventions

Every guide page is plain Markdown with front matter: `title`, `description` (one sentence), `order` (the number in the
file name) and `section` (Start, Build or Reference). The rules in the [guide index](README.md) apply:

- Performance numbers come only from `docs/RESULTS.md` or `docs/results.json`, with their conditions. Never mix a
  Rust time before Markdown formatting with a React fully rendered time.
- Compare only with upstream assistant-ui. State limits. No hype words, no emoji.
- Code must match the APIs in this repository. A snippet not compiled by tests is fenced as `rust,ignore`.
- Run `python3 scripts/check-guide.py` before you commit. It checks front matter, that the order matches the file
  name, and that every relative link resolves.

## Licensing

The project's own code is MIT (`LICENSE`). Portions are ported from assistant-ui (MIT, AgentbaseAI Inc.); keep the
notice in `NOTICE`, and mark new files that derive from upstream as ported from it. Bundled fonts, icons and
adapted code (Inter and IBM Plex Mono under the SIL Open Font License, Lucide icons under ISC, one file adapted from
the Vercel AI SDK under Apache-2.0) are listed in `THIRD_PARTY.md` with their licences beside the files. Add an entry
there when you bring in third-party material. Slint is GPL-3.0, royalty-free with an attribution badge, or commercial;
only the Slint crates and the example depend on it.

## CI policy

The gate is what counts, and it runs locally or on self-hosted machines. The project does not depend on paid, hosted
CI runners. `.github/workflows/ci.yml` mirrors the gate for whoever runs it, and PERFORMANCE records that a hosted
run on 2026-11-10 failed before any step started, which is why local results are what a change is judged on.
Report what you ran (the commands and the test counts) in your pull request.

Next: the [FAQ](10-faq.md), or [STATUS](../STATUS.md) for what is ported and what is not.
