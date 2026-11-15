---
title: Getting started
description: Install ai-ui from git, run a first chat runtime in a few lines, stream a reply into the state layer and run the example app.
order: 1
section: Start
---

# Getting started

**What this page gives you.** This page takes you from an empty project to a running chat runtime: install from git, a first runtime in 20 lines, a streamed reply read through the state layer, the example app and a web build. Each snippet is built by the repository's tests or examples.

ai-ui is a Rust port of [assistant-ui](https://github.com/assistant-ui/assistant-ui). It gives native and
WebAssembly Rust apps the same building blocks that assistant-ui gives React apps: a message model with branching,
a chat runtime that drives a model adapter, tool calls with approvals, the AI SDK and assistant-stream wire
protocols, and a Slint thread UI. It follows upstream's architecture module for module, so a backend that already
serves assistant-ui clients keeps working. The pinned upstream commit and the list of what is and is not ported are in
[STATUS](../STATUS.md).

It is early and not on crates.io yet. It is a port, not a product of the assistant-ui or Vercel AI SDK projects.

The point of the port is cost. On the same workload, a Rust thread uses much less CPU and memory than upstream's React
thread. One example, with its conditions: opening a 1000-message thread (9 KB prompt) until every message's Markdown is
formatted and painted took 105 ms for Rust and 2.94 s for React, median of 5 interleaved pairs, release build, software
renderer on the Rust side, headless Chromium for React, measured on 2026-11-11. Other workloads, ranges and caveats are
in [RESULTS](../RESULTS.md); [Performance](07-performance.md) explains where the difference comes from.

## What you get

Three layers, each usable without the next:

| Layer | Crates | What it does |
|---|---|---|
| Runtime | `ai-ui-core`, `ai-ui-stream`, `ai-ui-data-stream`, `ai-ui-http` | Messages, branches, the run loop, tools, wire decoders, HTTP clients. No UI dependency. |
| State | `ai-ui-state` | `ThreadView`: what a UI renders (rows, parts, branch numbers, smooth text) and the actions it calls. UI-agnostic. |
| UI | `ai-ui-slint`, `ai-ui-markdown-slint`, `examples/thread` | A Slint thread, composer and Markdown renderer, plus the demo app. |

The first two layers compile for `wasm32-unknown-unknown`. Every crate is described in the [crate reference](06-crates.md).

## Install

Nothing is published yet, so depend on the repository. Cargo finds each crate by name inside it:

```toml
[dependencies]
ai-ui-core = { git = "https://github.com/iyiolacak/ai-ui-rewrite-rust" }
ai-ui-state = { git = "https://github.com/iyiolacak/ai-ui-rewrite-rust" }
# Only if you talk to an AI SDK backend:
ai-ui-data-stream = { git = "https://github.com/iyiolacak/ai-ui-rewrite-rust" }
ai-ui-http = { git = "https://github.com/iyiolacak/ai-ui-rewrite-rust" }
# The examples on this page drive futures with a small executor:
futures-executor = "0.3"
```

Pin a revision with `rev = "..."` once you depend on it for real. The non-UI crates need Rust 1.85 or newer
(`ai-ui-markdown` needs 1.88, the Slint crates 1.92). The runtime has no executor of its own, so you choose one:
`slint::spawn_local`, a Tokio `LocalSet`, `wasm-bindgen-futures`, or `futures_executor::block_on` in tests and scripts.

## A first runtime

A runtime holds one thread. Its model is a `ChatModelAdapter`: any closure from the run options to a stream of results
works. This one answers every request with a canned text.

```rust
use ai_ui_core::{
    adapter, AppendMessage, ChatModelRunResult, LocalRuntimeOptions, LocalThreadRuntime,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The adapter is the model. This one answers every step with a canned text.
    let runtime = LocalThreadRuntime::new(
        |_options| adapter::from_results([Ok(ChatModelRunResult::text("Hello!"))]),
        LocalRuntimeOptions::default(),
    );

    // `append` changes the thread now: the user message and an empty, running
    // assistant message exist before the run has been polled.
    let run = runtime.append(AppendMessage::user_text(None, "Hi"))?;
    println!("before the run: {} messages", runtime.messages().len());

    // The returned future does the model work. Any executor can drive it.
    futures_executor::block_on(run)?;

    for message in runtime.messages().iter() {
        println!("{:?}: {}", message.role(), message.text());
    }
    Ok(())
}
```

Running it prints:

```text
before the run: 2 messages
User: Hi
Assistant: Hello!
```

Two things to notice. First, `append` does its synchronous work immediately: the user message and a running, still
empty assistant message are in the thread before anything is awaited, which is what lets a UI show them in the same
frame. Second, the future it returns is the model run. Nothing happens until you spawn it. The same code is built as
[`quickstart.rs`](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/crates/ai-ui-core/examples/quickstart.rs):
`cargo run --example quickstart -p ai-ui-core`.

## Stream a reply and read the state

A real model streams. Each item an adapter yields is the whole reply so far, not a delta (the
[concepts page](02-concepts.md#chatmodeladapter-and-run-results) explains why). A UI does not read the runtime directly:
it holds a `ThreadView` from `ai-ui-state`, reads `view.state()` when told something changed, and calls the view's
actions (`send`, `stop`, `reload`, `switch_branch`, ...).

```rust
use ai_ui_core::{
    adapter, AssistantMessagePart, ChatModelRunResult, LocalRuntimeOptions, LocalThreadRuntime,
    PartStatus, RunError, TextPart,
};
use ai_ui_state::{PartKind, ThreadView, ViewOptions};
use std::cell::Cell;
use std::rc::Rc;

/// One cumulative update: the whole reply so far, and whether the part is still running.
fn step(text: &str, status: PartStatus) -> Result<ChatModelRunResult, RunError> {
    Ok(ChatModelRunResult::content(vec![
        AssistantMessagePart::Text(TextPart {
            status: Some(status),
            ..TextPart::new(text)
        }),
    ]))
}

fn reply(view: &ThreadView) -> String {
    match &view.state().messages[1].parts[0].kind {
        PartKind::Text { text, .. } => format!("{text:?}"),
        _ => String::new(),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let runtime = LocalThreadRuntime::new(
        |_options| {
            adapter::from_results([
                step("Rust", PartStatus::Running),
                step("Rust is a", PartStatus::Running),
                step("Rust is a systems language.", PartStatus::Complete),
            ])
        },
        LocalRuntimeOptions::default(),
    );
    let view = ThreadView::new(Rc::new(runtime), ViewOptions::default());

    // A UI reads `state()` whenever it is told something changed. That is also
    // how the view notices a part that is still streaming and starts to pace it.
    let changes = Rc::new(Cell::new(0u32));
    let _subscription = view.subscribe({
        let (view, changes) = (view.clone(), changes.clone());
        move || {
            changes.set(changes.get() + 1);
            let _ = view.state();
        }
    });

    let run = view
        .send("What is Rust?")?
        .expect("the composer was not empty");
    futures_executor::block_on(run)?;
    println!("run finished, shown so far: {}", reply(&view));

    // Once per frame the UI hands the view a clock reading.
    let mut now_ms = 0.0;
    while view.needs_frame() {
        now_ms += 16.0;
        view.tick(now_ms);
        if now_ms == 64.0 {
            println!("after 64 ms: {}", reply(&view));
        }
    }
    println!("after {now_ms} ms: {}", reply(&view));
    println!(
        "running: {}, notifications: {}",
        view.state().is_running,
        changes.get()
    );
    Ok(())
}
```

Its output:

```text
run finished, shown so far: ""
after 64 ms: "Rust is a"
after 160 ms: "Rust is a systems language."
running: false, notifications: 16
```

The model finished at once, but the view reveals the text at a steady pace, like upstream's `useSmooth`. A frame loop
calls `view.tick(now_ms)` while `view.needs_frame()` is true; here the loop is simulated with a 16 ms step. The notification
count varies with the library version and is only printed to show that the subscription fired. In a real app the
callback asks the UI toolkit for a redraw instead of reading state itself.

The text part is marked `PartStatus::Running` in the first two results. That is how the view knows a part is streaming
and paces it. The wire decoders set it for you; a hand-written adapter has to set it to get smooth text.

## Run the example app

`examples/thread` is assistant-ui's default thread in Slint: sidebar with threads, messages, reasoning and tool
disclosure, branch picker, copy, edit and regenerate, a composer and a settings panel. It needs the system fontconfig
development package (see [troubleshooting](#troubleshooting)) and Rust 1.92.

```sh
git clone https://github.com/iyiolacak/ai-ui-rewrite-rust
cd ai-ui-rewrite-rust
cargo run -p ai-ui-example-thread --release
```

With no flags it runs a built-in demo model that streams reasoning and then a Markdown answer. A prompt that contains
"weather" makes it call the `get_weather` tool, which runs without asking. A prompt that contains "email" calls
`send_email`, which stops and asks you to approve first. Flags go after `--` when you use `cargo run`; they are the
real ones from `examples/thread/src/main.rs`:

| Flag | Effect |
|---|---|
| `--api URL` | Talk to an AI SDK endpoint instead of the demo model (native build only). |
| `--header NAME:VALUE` | Add a request header with `--api`, for example an authorization header. |
| `--theme system\|light\|dark`, `--light` | Choose the theme for this run. |
| `--text-scale F` | Text size factor, clamped to 0.75 to 2.0. |
| `--reading-width compact\|default\|wide\|xwide\|full` | Width of the message column. |
| `--open-settings` | Start with the settings panel open. |
| `--history N` | Start with N messages already in the thread. |
| `--long-reply-kb K` | Ask the demo model for a reply of about K kilobytes of Markdown. |
| `--token-delay-ms MS` | Pause between demo tokens (default 18). |
| `--ask TEXT` | Run a prompt immediately; repeatable. |
| `--decide allow\|deny` | Answer a pending approval automatically. |
| `--new-thread` | Open a new thread first. |
| `--size WxH` | Window or screenshot size (default 900x760). |

Settings changed in the sidebar are saved in `$XDG_CONFIG_HOME/ai-ui-example-thread/settings.json`; flags override them
for one run. A scenario that exercises the long-thread paths:

```sh
cargo run -p ai-ui-example-thread --release -- --history 1000 --long-reply-kb 9 --token-delay-ms 4
```

To try the approval flow without clicking: `-- --ask "Send an email to the team" --decide allow`.

For a GPU renderer, build with `--features gpu-window` and set `SLINT_BACKEND=winit-femtovg`. The default build includes
Slint's software renderer, which also powers the headless flags: `--screenshot out.png` renders offscreen and exits (with
`--conversation`, `--expand-all`, `--mid-stream MS`). The software renderer draws styled text as plain characters, so
screenshots do not show bold or list styling; see the visual fidelity note in [STATUS](../STATUS.md#visual-fidelity).
`--bench N` and its relatives are the benchmark harness, covered in [Performance](07-performance.md).

To point it at your own backend: `cargo run -p ai-ui-example-thread --release -- --api http://localhost:3000/api/chat`.
What that request and response look like is in [Streaming and backends](03-streaming-and-backends.md).

## In the browser

The same example builds for WebAssembly and runs the demo model in a canvas. `scripts/build-web-demo.sh` produces the
JavaScript glue and the `.wasm` file. Flags become query parameters there (`?history=1000&light`). The build steps, what
works in the browser and its limits are in [Running on the web](05-web.md) and [WEB](../WEB.md).

## Troubleshooting

**`fontconfig` not found when building a Slint crate.** The Slint crates link the system fontconfig. Install the
development package: `libfontconfig1-dev` on Debian and Ubuntu, `fontconfig-devel` on Fedora. If it lives in a
non-standard prefix, point pkg-config at it before building:

```sh
export PKG_CONFIG_PATH=/path/to/prefix/lib/pkgconfig
```

The runtime and state crates do not need fontconfig, so `ai-ui-core`, `ai-ui-state`, `ai-ui-stream`, `ai-ui-data-stream`
and `ai-ui-http` build without it.

**`wasm-bindgen` version mismatch.** `scripts/build-web-demo.sh` stops with "wasm-bindgen is X but Cargo.lock needs Y"
when the command-line tool differs from the library locked in `Cargo.lock` (0.2.129 at the time of writing). Install that
exact version of the CLI, or set `WASM_BINDGEN=/path/to/wasm-bindgen`. Add the target once with
`rustup target add wasm32-unknown-unknown`.

**Old toolchain.** The Slint crates and the example need Rust 1.92; `rustup update` fixes most "requires rustc" errors.

**Your own Slint app drops frames during long streams.** Slint 1.18 depends on a femtovg release that recompiles GPU
pipelines too often for a streaming chat. This workspace carries a patch; an app that uses the Slint crates adds the same
`[patch.crates-io]` entry. See [patches/README.md](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/patches/README.md).

**Slint licence.** The code here is MIT, but Slint is available under GPL-3.0, a royalty-free licence that requires an
attribution badge, or a commercial licence. An app using the Slint crates must comply with the one it picks.

## Where next

- [Concepts](02-concepts.md): the message model, branches, runtimes, tools, smooth streaming and the state layer.
- [Streaming and backends](03-streaming-and-backends.md): wire protocols and connecting a real backend.
- [Building a UI with Slint](04-ui-slint.md): the thread UI in detail.
