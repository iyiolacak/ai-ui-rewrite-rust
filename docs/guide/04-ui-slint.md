---
title: Building a UI with Slint
description: The Slint thread UI, the Markdown stack behind it, how the example binds them to a runtime, settings, scaling, keyboard and focus, renderers, and how to embed the thread in your own Slint app.
order: 4
section: Build
---

# Building a UI with Slint

**What this page gives you.** This page covers the Slint thread UI: the components in `ai-ui-slint`, the Markdown stack, how `examples/thread` binds them to a runtime, settings, scaling, keyboard and focus, renderers, and embedding the thread in your own Slint app.

The runtime crates know nothing about windows. The Slint UI is two layers on top of them:

- `crates/ai-ui-slint` is a Slint source library: the components, fonts, icons and the `AssistantTheme` global.
- `examples/thread` is a complete application that binds those components to a runtime through `ai-ui-state`. It is both the demo and the reference for how to write the binding.

The components are presentation only. They draw what the host gives them and report what the user did. The host owns the conversation, the clipboard, disclosure state and every effect. That split is what lets the same components sit on the local runtime, an external store, or a thread list.

## What is in `ai-ui-slint`

The Rust side of the crate is one function, `ai_ui_slint::ui_dir()`, which returns the folder holding `lib.slint`. It has no dependencies, no renderer and no runtime code. Everything else is `.slint` source under `crates/ai-ui-slint/ui/`. A consumer imports from `lib.slint` only; files under `components/` are internal and can change.

The main components, all exported from `lib.slint`:

| Component | What it does |
|---|---|
| `AssistantThread` | Scrolling list of `ChatRow` messages. Tracks whether the reader follows the bottom, exposes `scroll-to-bottom()`, forwards every message action. |
| `AssistantMessage` (internal) | One row: user or assistant bubble, parts, status line, the action bar (copy, edit, regenerate, feedback) and the branch picker. |
| `AssistantComposer` | The input. Enter sends, Shift+Enter adds a line, Escape stops a running reply. Optional attachments row. |
| `AssistantEditComposer` | Inline editor shown inside a message while it is being edited. |
| `AssistantReasoningPreview` | Reasoning text in a bounded, collapsible viewport. |
| `AssistantToolFallback` (internal) | The default disclosure for a tool call: arguments, optional detail and result as plain text, with optional copy buttons. |
| `AssistantApproval` | The prompt for a tool that needs approval, with the choices the runtime offers. |
| `AssistantBranchPicker` | Previous/next control with "n / m". Disabled while a run is active. |
| `AssistantMarkdown` | Renders Markdown blocks: paragraphs, headings, code with a copy button, tables, rules. |
| `AssistantSuggestions` | Up to four suggested prompts on an empty thread. |
| `AssistantThreadList` | The sidebar: New Thread row, search field, one row per thread with a running indicator, a menu (Rename, Archive, Delete) and inline rename. |
| `AssistantThreadSelector` | A compact dropdown over a string model, with keyboard navigation and typeahead. |
| `AssistantSettingsMenu` | The Settings row and popover: text size, reading width, theme. |
| `AssistantAttachmentList`, `AssistantAttachmentChip`, `AssistantSourceCard` | Attachment and source presentation. |
| `AssistantButton`, `AssistantIconButton`, `AssistantCopyButton`, `AssistantFeedback`, `AssistantScrollbar`, `AssistantTooltipContent` | Building blocks. |
| `AssistantPagedText`, `AssistantChunkedText` | Long text shown as bounded pages or measured chunks, so one huge message does not stall layout. |

The shared records live in `models.slint`: `ChatRow`, `ChatPart`, `ChatPartKind`, `MessageRole`, `ApprovalRequest`, `ApprovalChoice` and others. A row carries its parts, a status string and tone, a branch token with `branch-number` and `branch-count`, and flags such as `can-edit`, `can-regenerate`, `can-copy` and `copied`. Fonts are Inter (regular, medium, semibold) and IBM Plex Mono for code, registered by `theme.slint`, with their licences next to them.

## The Markdown stack

Markdown is the expensive part of a chat UI, so it is kept off the UI thread and out of the per-frame path.

1. **`ai-ui-markdown`** parses and projects. `parse` builds a bounded `Document`; `presentation::project` produces sanitized render blocks. Raw HTML, images and unsupported URLs become text, never executable content. It does no I/O and has hard limits (`MAX_SOURCE_BYTES`, `MAX_EVENTS`, `MAX_NESTING`, `MAX_OUTPUT_BYTES`) that fail with an error instead of truncating.
2. **Settled blocks and a live tail.** While a reply streams only its end can change. `split::StreamSplit` keeps the blocks before the last two as settled, so each is formatted once, and re-parses only the live tail on each update. Two blocks stay live because a half-typed line can be absorbed by the block before it a moment later. If the source contains a link reference or footnote definition, which can change earlier blocks, the splitter stops settling and treats the whole source as the tail. Finished text is always formatted in one piece, which is exact for every document.
3. **Worker pool.** On native targets `pool::CoordinatorPool` runs several `Coordinator`s, one worker thread each, sharded by key. A key always goes to the same shard, so only the newest source for it is kept while a parse runs, and older results are labelled as previews. The default is one worker per core, leaving one for the UI thread, between 1 and 8. Setting `AI_UI_MARKDOWN_WORKERS=1` in the example restores a single worker for comparisons.
4. **`ai-ui-markdown-slint`** is the UI-side adapter. Its `Factory` trait maps portable blocks into the record types your own `.slint` generates (Slint generates distinct types per application). `cache::Cache` and `streaming::StreamingCache` hold the formatted blocks on the UI thread. You call `observe_one(key, text, streaming)` when a text changes, `poll()` on a timer, `take_changed()` to learn which texts finished, and `view(key, source, streaming)` to get the blocks. While a newer source is still being formatted the previous formatting stays on screen, so streaming text never flashes back to plain text.

Markdown blocks carry no font sizes, which is why a text size change does not touch this stack (see Scaling below). In the browser there are no worker threads, so the same projection runs inline on the UI thread under a 4 ms budget per poll. See [Running on the web](05-web.md).

## How the example wires it

`examples/thread/src/app.rs` is the binding. The shape is short:

- A `ThreadView` from `ai-ui-state` turns any `ThreadRuntime` into `ThreadState`: an `Arc<MessageState>` per message plus run status. The example subscribes to the view and calls `App::sync` on every notification.
- The runtime keeps a bounded change journal (`MessageChanges`). The view reads it with its own cursor, so a notification costs work in proportion to what changed. If a view falls behind the journal it falls back to one full read.
- `sync` compares each message's `Arc` pointer with the pointer the row last showed. Only rows whose pointer changed are rebuilt. A rebuilt row reuses its parts model: `update_row` writes only the parts that differ into the existing `VecModel`, so Slint re-lays out those parts and not the whole message. Rows that no longer match the thread are removed, and a structural change also drops Markdown work for messages outside the newest window.
- The Markdown cache is keyed by message id and part index. `sync` declares changed texts newest first, so the visible end of the thread formats first, and a 1 ms timer polls for finished work and refreshes only the rows whose text completed.
- Window callbacks (`send`, `stop`, `switch-branch`, `regenerate`, `begin-edit`, `save-edit`, `toggle-part`, `resolve-approval`, `submit-feedback`, thread rename/archive/delete/search) forward to `ThreadView` and `ThreadList`. Expanded or collapsed state is owned by the host, so it survives Slint recycling rows.

## Settings

The sidebar footer has a Settings row. It opens a popover with three groups:

| Group | Options |
|---|---|
| Text size | Small (0.9), Default (1.0), Large (1.15), Extra large (1.3) |
| Reading width | Compact, Default, Wide, Extra wide, Full |
| Theme | System, Light, Dark |

Reading width sets the maximum width of the message column at text scale 1.0: Compact 576 px, Default 704, Wide 880, Extra wide 1100, and Full, which uses the whole main area with 40 px side padding. The pixel width is multiplied by the text scale so the line length in characters stays the same, and the layout clamps it to the available area. The theme defaults to dark. System follows the operating system.

In the popover, Up and Down (and Home and End) move a cursor, Enter or Space chooses, and Escape closes it and returns focus to the Settings row. The popover scrolls when the window is shorter than its content.

**Keyboard shortcuts.** Ctrl+= makes text larger by one step, Ctrl+- smaller, and Ctrl+0 resets to Default. Cmd works the same on macOS. They are caught by a `FocusScope` around the window, so they work from the composer. The plain `+` and `_` keys count too, so the shortcut works without holding Shift on layouts where `=` needs it. The steps stop at the ends.

**Saved file.** Native runs save settings to `$XDG_CONFIG_HOME/ai-ui-example-thread/settings.json`, falling back to `~/.config/...` and then `%APPDATA%`. The file is small JSON:

```json
{
  "text_scale": 1.0,
  "reading_width": "default",
  "theme": "dark"
}
```

`reading_width` is one of `compact`, `default`, `wide`, `xwide`, `full`. Reading and writing never fail the app: a missing or malformed file means defaults, bad fields are ignored, numbers are clamped (0.75 to 2.0), and a failed write is ignored. Writes go through a temporary file that is renamed over the target. Screenshot and benchmark runs neither read nor write the file.

**Command-line flags** override the saved values for one run and do not save until you change something in the UI:

```sh
cargo run -p ai-ui-example-thread -- --theme light --text-scale 1.15 --reading-width wide
cargo run -p ai-ui-example-thread -- --light          # same as --theme light
cargo run -p ai-ui-example-thread -- --api https://example.com/api/chat
```

`--open-settings` opens the popover, which is useful with `--screenshot`. In the browser, the same options come from the page; see the web guide.

## Scaling

Text size is one property: `AssistantTheme.text-scale`, a global in `theme.slint`. Every font size in message text, Markdown, code, reasoning, tool calls, approvals, source and attachment cards, buttons, the branch picker, the action row and the composer multiplies by it. Icons in scaled rows scale with the text. `AssistantTheme.hit-scale` is `max(1, text-scale)`: hit targets grow with the text but never shrink below their normal size, which is 28 to 32 px.

Nothing on the Rust side re-runs when you change it. The host sets the property; Slint re-evaluates only the rows the `ListView` has instantiated. Markdown is not re-parsed because its blocks hold unsized styled text. The measurements are in [PERFORMANCE](../PERFORMANCE.md#settings-text-size-reading-width-theme), including the note that layout cost does not grow with thread length because only visible rows lay out.

In your own app, set the theme and scale the same way:

```rust,ignore
use slint::ComponentHandle;

let theme = ui.global::<AssistantTheme>();
theme.set_text_scale(1.15);
theme.set_follow_system(false);
theme.set_scheme(AssistantColorScheme::Light);
```

## Keyboard and focus

- **Composer.** Enter sends when no run is active; Shift+Enter inserts a newline. While a run is active Enter does nothing and inserts no newline. Escape stops the run.
- **Focus follows intent.** After switching threads, starting an edit, or pressing the scroll-to-bottom button, focus moves to the composer. A run that starts does not steal focus from the sidebar if the sidebar has it.
- **Focus rings are for the keyboard.** A click focuses a button without drawing a ring; Tab draws one, and a later click hides it again. Tests in `crates/ai-ui-slint/tests/focus_visible.rs` cover this.
- **Hit targets.** An icon button's pointer area reaches past its visual (a 32 px target around a 24 px icon), and actions that opt in ignore a fast double click.
- **Thread list.** Enter or Space activates a row or New Thread; inline rename takes Enter to commit and Escape to cancel.
- **Following the bottom.** `AssistantThread.following` turns off only when the user scrolls away from the end, and back on when they return within a few pixels. Streaming content grows under a reader who follows and stays put for one who does not.
- **Accessibility.** Controls carry accessible labels (for example "Message input", "Settings", "Copy message"), and the Settings row exposes a default action.

## Renderers

The example enables Slint's winit backend with two renderers: femtovg on wgpu for windows, and the software renderer for tests, screenshots and benchmarks.

- **GPU window.** `SLINT_BACKEND=winit-femtovg cargo run -p ai-ui-example-thread --features gpu-window`. The `gpu-window` feature makes `build.rs` embed fonts and images as files instead of pixels. GPU renderers draw any font size from the font file.
- **Software renderer.** The default build embeds resources for it. Offscreen `--screenshot out.png` and `--bench` use it. It draws glyphs pre-rendered at sizes it finds in the source, so `build.rs` sets `SLINT_FONT_SIZES` to the sizes the four text scales produce; without that a scaled size would snap to the nearest embedded one. This adds about 1.4 MB to the binary. Component tests in `ai-ui-slint` render offscreen with their own software platform.
- **Browser.** The web build uses the femtovg renderer on WebGL with winit on a canvas.

Software-renderer screenshots show layout, not GPU text or effects.

**The femtovg patch.** The workspace carries `patches/femtovg`, femtovg 0.27.0 with one upstream change backported by hand (femtovg #366, "Keep WGPU pipelines across flushes that don't use them"). Without it, a streaming chat whose frames differ in what they draw recompiled GPU pipelines every few frames. The patch applies through `[patch.crates-io]` in the workspace `Cargo.toml` and is meant to go away when Slint depends on femtovg 0.28 or later. An application outside this workspace must add the same entry; see [patches/README.md](../../patches/README.md) for the numbers and the removal condition.

## Embedding the thread in your own app

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

`build.rs` registers the library path (this is the example's build script, trimmed):

```rust,ignore
use std::collections::HashMap;

fn main() {
    let libraries = HashMap::from([("assistant".to_string(), ai_ui_slint::ui_dir())]);
    let config = slint_build::CompilerConfiguration::new().with_library_paths(libraries);
    slint_build::compile_with_config("ui/app.slint", config).unwrap();
}
```

Then import from `@assistant` in your `.slint`. Slint generates record types per application, so re-export the ones your Rust code needs:

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

On the Rust side, follow `examples/thread/src/app.rs`: build a `ThreadView` over your runtime, keep an `Rc<VecModel<ChatRow>>`, update it from a view subscription, and map the window callbacks to view actions. Give each Markdown text a key, feed changed texts to a `StreamingCache`, and poll it on a short timer.

Three rules from the components' own documentation matter when you write the host:

- Change `conversation-revision` whenever the selected conversation changes, even if the model is emptied and refilled in between.
- Every action that can have an effect (copy, open a source, retry an upload) is off by default. Turn on `can-copy-message`, `can-copy-code`, `can-open-source` and the like only when the host handles them, and revalidate the identifiers a callback carries before acting.
- Slint, which is not MIT, has its own licence terms: GPL-3.0, the Royalty-Free licence with an attribution badge, or a commercial licence. See [THIRD_PARTY.md](../../THIRD_PARTY.md).

Next: [Running on the web](05-web.md) for the WebAssembly build of this same example, and the [crate reference](06-crates.md) for the Rust side.
