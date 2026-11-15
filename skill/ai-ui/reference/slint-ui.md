# Embedding the Slint thread

Source: [Building a UI with Slint](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/docs/guide/04-ui-slint.md) and `examples/thread`. Read `examples/thread/src/app.rs` before writing the host: it is the reference binding.

## Before you start

- Rust 1.92 or newer (`ai-ui-slint`, `ai-ui-markdown-slint`).
- Linux: install the fontconfig development package (`libfontconfig1-dev` on Debian and Ubuntu, `fontconfig-devel` on Fedora). If it is in a non-standard prefix, set `PKG_CONFIG_PATH`.
- **Ask the user which Slint licence applies.** Slint is GPL-3.0, or the Slint Royalty-Free licence (requires an attribution badge), or a commercial licence. The ai-ui code is MIT. Do not decide for them. See [THIRD_PARTY.md](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/THIRD_PARTY.md) and [slint.dev](https://slint.dev).
- If the project already uses a different Slint version, tell the user: these crates pin `=1.18.1`.

## Cargo.toml

Use the same Slint version as the crates, `=1.18.1`.

```toml
[dependencies]
slint = { version = "=1.18.1", features = ["compat-1-18"] }
ai-ui-core = "0.1"
ai-ui-state = "0.1"
ai-ui-markdown-slint = "0.1"

[build-dependencies]
ai-ui-slint = "0.1"
slint-build = "=1.18.1"

# Until a Slint release uses femtovg 0.28: without it a streaming chat uses about twice the CPU with the
# FemtoVG-WGPU renderer (see patches/README.md).
[patch.crates-io]
femtovg = { git = "https://github.com/iyiolacak/ai-ui-rewrite-rust" }
```

In a workspace, put `[patch.crates-io]` in the workspace root `Cargo.toml`. Remove it when Slint depends on femtovg 0.28 or later ([patches/README.md](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/patches/README.md)). Add `ai-ui-data-stream` and `ai-ui-http` for a backend ([backend.md](backend.md)).

## build.rs

```rust,ignore
use std::collections::HashMap;

fn main() {
    let libraries = HashMap::from([("assistant".to_string(), ai_ui_slint::ui_dir())]);
    let config = slint_build::CompilerConfiguration::new().with_library_paths(libraries);
    slint_build::compile_with_config("ui/app.slint", config).unwrap();
}
```

`ai_ui_slint::ui_dir()` returns the folder holding `lib.slint`. Import only from `lib.slint`; files under `components/` are internal.

## ui/app.slint

Slint generates record types per application, so re-export the ones your Rust code needs.

```slint
import "@assistant/assets/Inter-Regular.ttf";
import "@assistant/assets/Inter-Medium.ttf";
import { AssistantTheme, AssistantThread, AssistantComposer, ChatRow } from "@assistant/lib.slint";
export { ChatRow, ChatPart, AssistantTheme, AssistantColorScheme } from "@assistant/lib.slint";

export component AppWindow inherits Window {
    default-font-family: "Inter";
    background: AssistantTheme.background;

    in property <[ChatRow]> messages;
    in property <bool> running;
    in property <int> thread-revision: 1;
    in-out property <string> draft;
    callback send(string) -> bool;
    callback stop();
    callback toggle-part(string, string);

    VerticalLayout {
        AssistantThread {
            vertical-stretch: 1;
            messages: root.messages;
            busy: root.running;
            conversation-revision: root.thread-revision;
            toggle-part(message-id, part-id) => { root.toggle-part(message-id, part-id); }
        }
        AssistantComposer {
            busy: root.running;
            draft <=> root.draft;
            submit(text) => { return root.send(text); }
        }
    }
}
```

The components are presentation only. The host owns the conversation, the clipboard, disclosure state and every effect.

## Rust side: wiring ThreadView to the UI

Follow `examples/thread/src/app.rs`. The shape:

1. Build a `ThreadView` over your runtime.
2. Keep an `Rc<VecModel<ChatRow>>`. Subscribe to the view; on each notification, read `view.state()` and compare each `Arc<MessageState>` pointer with the one the row last showed. Rebuild only rows whose pointer changed.
3. Give each Markdown text a key (message id and part index), feed changed texts to a `StreamingCache` (`observe_one`), poll it on a short timer (`poll`, then `take_changed`), and refresh only the rows whose text finished. The compiling version is `examples/thread/src/markdown.rs`.
4. Map window callbacks (`send`, `stop`, `switch-branch`, `regenerate`, `begin-edit`, `save-edit`, `toggle-part`, `resolve-approval`, `submit-feedback`, thread rename, archive, delete) to `ThreadView` and `ThreadList` actions.
5. Drive the smooth-text clock: call `view.tick(now_ms)` once per frame while `view.needs_frame()` is true.
6. Spawn every `RunFuture` with `slint::spawn_local`.

Theme and text size, set from Rust:

```rust,ignore
use slint::ComponentHandle;

let theme = ui.global::<AssistantTheme>();
theme.set_text_scale(1.15);
theme.set_follow_system(false);
theme.set_scheme(AssistantColorScheme::Light);
```

## Rules for the host

- Change `conversation-revision` whenever the selected conversation changes, even if the model is emptied and refilled in between. Reset per-conversation caches then too.
- Every action with an effect (copy, open a source, retry an upload) is off by default. Turn on `can-copy-message`, `can-copy-code`, `can-open-source` and the like only when the host handles them, and revalidate the identifiers a callback carries before acting.
- Keep expanded or collapsed state in the host so it survives Slint recycling rows.

## Renderers

- GPU window: `SLINT_BACKEND=winit-femtovg`, with the example built `--features gpu-window`. This is where the femtovg patch matters.
- Software renderer: used for tests, screenshots and benchmarks. It draws styled text as plain characters, so screenshots do not show bold or list styling. Markdown styling fidelity in a real GPU window is marked unverified in [STATUS](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/docs/STATUS.md#visual-fidelity).
- For software-rendered text at scaled sizes, `examples/thread/build.rs` sets `SLINT_FONT_SIZES` and embeds resources for the software renderer. Copy that logic only if your app uses the software renderer.

## Try the example first

```sh
git clone https://github.com/iyiolacak/ai-ui-rewrite-rust
cd ai-ui-rewrite-rust
cargo run -p ai-ui-example-thread --release
```

Add `-- --api URL` for an AI SDK endpoint. Flags and scenarios: [Getting started](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/docs/guide/01-getting-started.md).
