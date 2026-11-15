# Assistant Slint components

> **Known issue: extra CPU with Slint's FemtoVG-WGPU renderer.** Slint 1.18 uses femtovg 0.27, which drops its cached
> GPU pipelines whenever a frame does not use them, so a streaming chat (whose frames differ) recompiles pipelines and
> shaders every few frames. In our real-window measurement a 30 s stream into a 1000-message thread took 16.4-23.6 s of
> process CPU without the fix and 9.6-10.2 s with it. The fix is upstream (femtovg #366, released in 0.28), but no Slint
> release uses it yet. Until one does, add this to your application's `Cargo.toml`:
>
> ```toml
> [patch.crates-io]
> femtovg = { git = "https://github.com/iyiolacak/ai-ui-rewrite-rust" }
> ```
>
> It points Cargo at femtovg 0.27.0 with #366 backported by hand (`patches/femtovg`, MIT OR Apache-2.0, one changed
> file). Slint's software and Skia renderers are not affected. When femtovg publishes a 0.27.x with the fix, or a Slint
> release moves to femtovg 0.28, remove the patch and run `cargo update`.

This is a Slint 1.18.1 source component library, separate from the
portable Rust conversation state. The
[thread example](https://github.com/iyiolacak/ai-ui-rewrite-rust/tree/main/examples/thread) shows how to import and
bind it. It does no
networking, persistence, authorization or tool execution. An in-process module
boundary does not isolate crashes or resource exhaustion.

Consumers import public types and components from `ui/lib.slint`; internal files
can change without becoming consumer imports. The application owns integration
and must explicitly permit this source dependency in its architecture boundary.

## Cargo consumer setup

The dependency-free `ai-ui-slint` Rust crate provides the source location
at **build time**; it does not create a window or add a Slint runtime dependency.
The package is currently private (`publish = false`), so consume it by path or
an explicitly managed Git dependency until distribution and licensing are decided.
Use the matching Slint and `slint-build` version, currently 1.18.1:

```toml
[dependencies]
slint = { version = "=1.18.1", features = ["compat-1-18"] }

[build-dependencies]
ai-ui-slint = { path = "../path/to/crates/assistant-slint" }
slint-build = "=1.18.1"
```

```rust,ignore
// build.rs
let libraries = std::collections::HashMap::from([(
    "assistant".to_string(), ai_ui_slint::ui_dir(),
)]);
let config = slint_build::CompilerConfiguration::new().with_library_paths(libraries);
slint_build::compile_with_config("ui/window.slint", config).unwrap();
```

```slint
import "@assistant/assets/Inter-Regular.ttf";
import "@assistant/assets/Inter-Medium.ttf";
import { AssistantThread, AssistantComposer } from "@assistant/lib.slint";
```

`ui_dir()` is for the build script only; do not keep the source path in a
runtime configuration. The package includes component sources, fonts, icons and
their attribution files. Consumers choose the backend and renderer. The
[thread example](https://github.com/iyiolacak/ai-ui-rewrite-rust/tree/main/examples/thread) compiles those
imports and binds them to a runtime; it explicitly rejects
other content and is not a full projection or a production integration. The
host must publish scoped state and handle every required action and effect.

The host projects accepted conversation state into a stable Slint model of
`ChatRow` records containing ordered `ChatPart` models, and supplies the
currently selected scope's draft and run state. Parts represent text, reasoning,
tool activity, attachment metadata and source, remote-file and named-data
summaries. The host owns the expanded state, keyed
by scope, thread, message and part, and handles `toggle-part(message-id, part-id)`;
this preserves it across recycled rows and model updates. The host changes
`conversation-revision` when changing conversation or scope. `AssistantThread`
tracks whether the reader follows the bottom and offers `scroll-to-bottom()`.
The host also supplies a `MessageStatusTone` for each row; `error` uses the
theme's error color and `neutral` uses the muted color. The component never
infers failure from the displayed status text.
The source card is read-only by default. To offer its optional action, the
host enables `AssistantThread.can-open-source` and the individual
`ChatPart.source-action-enabled`. `source-requested` carries the captured
conversation revision, message ID, branch token and part ID. The host must
revalidate that scope, resolve the full source and decide what action to take;
the component neither opens a URL nor treats preview text as authority. No
existing product host is wired to this action yet.
`AssistantComposer` binds `draft`,
calls `submit(text) -> bool` and clears the input only on acceptance. Enter submits;
Shift+Enter inserts a newline. The host binds a `RunReference` for the displayed
run and handles `cancel(run-reference)` using its captured scope and run identity;
it also handles, if enabled,
`add-attachment()` outside UI code. `can-attach` defaults to false. A host must
replace or clear scoped draft and rows when its workspace or thread changes.

The edit composer can optionally expose copy for a nonempty draft. The thread's
`can-copy-edit-draft` defaults to false; when enabled, `copy-edit-draft` sends
only the scoped edit token. The host must revalidate the active session, resolve
the authoritative draft and handle clipboard completion. The component does
not access the clipboard. Its action groups wrap at narrow widths.

`AssistantButton` has independent size and style variants and forwards actions
to its host. `AssistantDisclosureButton` is a controlled, reusable disclosure
trigger. `AssistantBranchPicker` is controlled by the host; the thread forwards
its previous/next actions with an opaque host snapshot token and disables them
while running. These components use the assistant-ui theme and draw design ideas from
[shadcn's button](https://github.com/shadcn-ui/ui/blob/8a3536678a528cc012b9e62484290e79d0be410f/apps/v4/registry/new-york-v4/ui/button.tsx)
and [collapsible](https://ui.shadcn.com/docs/components/collapsible). They do not
introduce React, Radix or a JavaScript runtime.

`AssistantSuggestions` displays up to four host-selected, controlled prompts.
It sends the host's opaque scope and suggestion ID captured at gesture start;
the host owns the full prompt and must revalidate both before populating or
sending a draft. The offline demo populates only an empty draft and does not
send automatically. Dynamic generation, descriptions, queueing and production
host integration remain open.

The controlled tool fallback displays host-formatted arguments, optional detail,
and a result as inert plain text, in separate panels inspired by the pinned
[assistant-ui ToolFallback](https://github.com/assistant-ui/assistant-ui/blob/5d55d7eb66e41e859baf2994c141693cef28f466/packages/ui/src/components/react/assistant-ui/elements/tool-fallback.aui.tsx).
Set `tool-structured` and supply `tool-arguments`, `tool-detail`,
`tool-has-result`, `tool-result-label` and `tool-result` on a tool `ChatPart`.
`tool-has-result` distinguishes an empty result from a pending one. Hosts that
do not supply structured tool data may continue to use `body` as before. The
host owns disclosure, tool execution, approval decisions and authorization;
the fallback neither parses nor executes tool contents.

Tool arguments and results may expose separate opt-in copy requests. The host
must enable `can-copy-tool` and the relevant per-part capability, then
revalidate the emitted revision, message, branch, part ID and typed target
before resolving the complete value and invoking a clipboard adapter. The
component passes no preview text and performs no clipboard I/O. Pending results
have no copy action; explicitly copyable empty completed values are supported.

`AssistantAttachmentChip` is a presentation-only, controlled attachment row.
Its host supplies a name, optional detail, `ready`, `uploading` or `failed`
state, and optional progress fraction; progress is clamped for drawing. The
optional remove action emits a callback without mutating host state. A host
must route it to the captured attachment and scope, decide whether cancellation
is permitted during upload, and publish the resulting state. Disabling the
action prevents activation; the default is read-only. Uploads, thumbnail
decoding, file picking and previews are not implemented by this component.
The existing message attachment part reuses the read-only variant.

Retry is also opt-in: the host marks an individual `failed` attachment
`retryable`, enables the host retry capability, and supplies a nonempty draft
scope for the normal composer. Its callback carries the scope and attachment
ID. The edit composer additionally requires managed edit attachments and
forwards the edit-session token with the ID. Retry and removal permissions are
independent. The host must revalidate a definite failure before any retry;
unknown outcomes must not be marked retryable. No existing product host has
wired this action to an upload executor.
`AssistantSourceCard` displays a compact title and optional detail, with a
host-controlled, opt-in open action. Its text can be elided; the host retains
the complete original metadata and decides whether and how to open the source.
Through `AssistantThread`, the request carries the revision, message, branch
and part captured at gesture start. The host must revalidate that scope before
resolving or opening the source. The legacy direct `AssistantPart` callback is
unscoped; direct consumers must manage their own identity and validation.
The card does not interpret URLs or render remote file or structured-data bytes.
The fixture bounds each untrusted metadata or error preview to 512 UTF-8 bytes
at a character boundary and indicates truncation; this is a fixture policy, not
a limit on the host-owned original data.
`AssistantAttachmentList` renders a wrapped, vertically scrollable list of host-owned
`AssistantDraftAttachment` records. Its remove callback returns the opaque ID;
the viewport caps at three rows and leaves later attachments reachable by scrolling.
IDs must be unique in the displayed scope and revalidated before an effect.
`AssistantComposer` forwards that model and the add/remove callbacks; its
`attachments-ready` flag defaults to false for a nonempty model, so the host
must explicitly allow submission when all attachments are ready. This applies
to Enter as well as the send button. The host must still validate on submission and
clear attachments only after accepting the complete draft. The demo supplies
labelled synthetic references; it does not select files or upload them.

`AssistantThreadList` is the upstream-style sidebar: a New Thread row, a search
field that the host filters, and per-thread rows with a running indicator, an
ellipsis menu (Rename, Archive, Delete) and inline rename. The host owns every
operation and receives row indices. Its `search.svg`, `ellipsis.svg`,
`archive.svg` and `trash.svg` are Lucide 0.468.0 icons (`trash.svg` is Lucide's
`trash-2`), SHA-256 `9322d964b61cf729b3a81210568d5a97eaf4a1f3239f25a056fda3013bf35611`,
`4f495cc72013ffdfec677f03b33a150f7b4dd741979283fd6853a09024bca112`,
`78c732e34e232db63af06a2330b4448045cfdf49b48b59f2d31922df54c7bb17` and
`e63698acf757d3043fcb4a9ea6ffceff9dd84708707eafc879c86b34a6ff2786`.

`AssistantThreadSelector` displays a host-owned string model and selected index;
it emits an index for the host to revalidate and commit. The host must advance
`model-revision` before every model replacement, reorder or content mutation,
including mutations to a shared model, and must not reuse a revision during a
gesture. Revision changes close the popup, so selection requires a fresh gesture.
A pending keyboard
cursor does not change selection until activation. The popup materializes at
most six rows; accumulated small wheel deltas, arrow keys, Page Up/Down,
Home/End and the overflow buttons make other rows reachable. Selection and
Escape return keyboard focus to the trigger; an empty or disabled selector
cannot open. Its outlined trigger, 6px menu inset, row highlight and check indicator
take cues from [shadcn's Select](https://github.com/shadcn-ui/ui/blob/main/apps/v4/registry/new-york-v4/ui/select.tsx),
using assistant-ui theme tokens. Optional typeahead calls the host-owned
`find-prefix(query, start)` lookup and moves only the pending cursor; the host
still validates and commits activation. The offline demo uses a wrapping,
case-insensitive linear search. Space activates the pending selection, and
shortcut modifiers do not enter the query. It is a compact fixture selector,
not a port of Radix Select or assistant-ui's full thread-list primitive:
groups, automatic popup motion and per-option disabled states are not provided.
Its `chevron-up.svg` is from the pinned Lucide revision (ISC, `ui/assets/Lucide-LICENSE.txt`; SHA-256 `d09f13fcbef3c493ead7e4663b5ee91804fec38d5edb5e7cc3acfbd547641cf4`).

`AssistantEditorSurface` is an experimental presentation/input boundary for a
host-rendered editor frame. Its edit and clipboard callbacks use typed operation
enums; `edit` carries inserted text or an empty string and a selection-extension
flag, and `place-caret` carries local logical-pixel coordinates. The host owns
the complete draft, selection, IME, clipboard, submission and all revision and
session fencing. It supplies `frame-width` and `frame-height` in logical pixels
alongside the image and must fence frames to the current draft and geometry.
The surface hides a frame and suppresses pointer placement when those dimensions
do not match its size; the host must request a new frame on `geometry-changed`.
Optional pointer-selection callbacks forward press, move, release and
cancellation; the host owns the anchor, revision fences and pending-frame
handling. The scroll callback carries a logical-pixel delta for the host to
apply. The isolated native editor example uses these at scale 1. The surface
does not replace either existing composer or establish physical pointer, IME,
non-unit scaling or device-performance behavior.

`AssistantInlineEditor` is a single-line, host-controlled draft with explicit
save and cancel callbacks. Enter and Escape are shortcuts for those actions;
the host validates the captured target, reports invalid input and decides when
to close it. The fixture uses it to rename a local thread, trims surrounding
whitespace, rejects empty names and names over 256 UTF-8 bytes, and retains the
draft after rejection. Renames are in-memory fixture state; this does not add
remote synchronization or durable persistence.

`AssistantApproval` presents an explicit binary request; its state, available
actions and result are host-owned. The part callback returns the complete
`ApprovalRequest` and `ApprovalChoice`; the host must validate the captured
workspace, decimal scope generation, thread, run, message, part and approval ID
against the current request before dispatching the effect. It must keep a
submitted decision pending until a matching receipt, and must not retry an
uncertain delivery. The offline fixture demonstrates these transitions without
executing a tool. Enforcing a denial at a real tool executor remains the host's
responsibility. The core records the receipt; it does not execute or prohibit
tool execution.

`AssistantMarkdown` accepts typed, already sanitized presentation blocks; it
does not parse untrusted source, open links or use the clipboard. Its link and
copy callbacks are host capabilities, with copying disabled by default. The
`AssistantIconButton` also accepts an optional hover tooltip; its presentation
is shared through `AssistantTooltipContent`. The copy button opts into it.
Delayed hover, clipping escape and click-through were exercised in a software
window. Placement across backends and source visual equivalence remain unverified.

The code header uses the reusable `AssistantCopyButton`; the host sets a block's
`copied` flag only after a confirmed clipboard write and clears it when its
indication expires. A nonempty `copy-token` selects the scoped
`copy-code-block` callback; the host must use non-reused tokens and validate
the captured branch, part, block and current content before dispatch and on
completion. Empty tokens keep the existing `copy-code` callback. An unavailable
or failed clipboard write must not set `copied`. The demo uses a bounded
three-second indicator that survives same-token row reprojection and cache
eviction. It is session state, not durable history. A scope change suppresses
stale feedback but cannot retract a clipboard write already dispatched to the
operating system or browser.
The header's icon geometry follows the pinned upstream; the source icon
transitions and exact tooltip placement remain open. The offline fixture validates
HTTP(S) destinations again and declines to open them.
Slint's current styled-text links are pointer-accessible but lack a verified
keyboard/assistive-technology path. Code blocks retain long lines in a
horizontally scrollable viewport with a controlled scrollbar. Syntax highlighting,
rich image display and fully responsive tables remain open.

The fixture supports inline text editing and changes to a separate attachment
draft. `edit-branch-token` identifies the row; a fresh host-issued
`edit-token` identifies the edit session and accompanies save, cancel and
add/remove callbacks. An empty `edit-branch-token` preserves the older behavior
where `edit-token` identifies the branch. Hosts opt in to a controlled selected
attachment model with `edit-attachments-enabled`; otherwise retained attachments
are read-only. `can-add-edit-attachment` exposes the Add control, whose callback
passes the edit-session identity to the host; `edit-attachments-ready` must be
false while any selected attachment is not ready. The host owns picking,
upload, cancellation and cleanup, and must revalidate the session, branch and
every selected reference on save. The fixture adds synthetic already-uploaded
references only; it does not upload. Rich approval options remain unimplemented.
Earlier visible responses can be
regenerated into separate branches. The fixture opens streaming reasoning by default and retains explicit per-part user choices;
tools still start collapsed. The reusable bounded reasoning preview follows the
bottom while streaming, preserves reader scrolling, and uses the shared theme's
background for its edge fades. Hosts should render it on that same background.
Upstream grouping and disclosure motion remain open.
The demo is not a general projection of all core state.

The message action row uses `AssistantIconButton` for copying when the host
marks a message copyable. `AssistantThread` sends the host's opaque message
action token through `copy-message`; it never reads the clipboard itself. The
offline fixture validates the current branch snapshot, copies nonempty text
parts in order separated by newlines, and excludes reasoning, tools, source
metadata and attachments. It caps copied text at 1 MiB and reports the platform
write result in the fixture status; the code and message actions share a
revision so old completions cannot replace newer status. This is a partial
counterpart of [assistant-ui's copy action](https://github.com/assistant-ui/assistant-ui/blob/5d55d7eb66e41e859baf2994c141693cef28f466/packages/react/src/primitives/actionBar/ActionBarCopy.tsx):
its confirmed per-message copied state is shown for three seconds in the fixture,
with at most 64 replaceable timers. At the cap the oldest indication may expire
early. Copying an edit draft is not yet implemented. The copy icon is Lucide revision `2c2ee80687cbd86030d10f2315daf509b12829c3`
(SHA-256 `ea80e566c7a12628a447cb53179b19aea4f60b9143e2becd0d0f20bf260e5718`),
covered by the adjacent Lucide license notice.

`AssistantFeedback` provides controlled positive and negative message actions.
Its `feedback` value and `enabled` state come from the host; selecting an action
emits a choice without changing the displayed state. `AssistantThread` forwards
the opaque branch token and choice so the host can validate and submit it. The
offline fixture accepts completed assistant responses, rejects stale branch
tokens, and records feedback per response only in memory. It performs no network
submission and does not persist feedback. This is a partial counterpart of the
[upstream feedback actions](https://github.com/assistant-ui/assistant-ui/blob/5d55d7eb66e41e859baf2994c141693cef28f466/packages/core/src/react/primitive-hooks/useActionBarFeedback.ts);
the upstream optional comment and adapter integration are not provided. The
controls use the existing theme and shadcn's selected-toggle treatment. The
thumb icons are Lucide 0.468.0, SHA-256 `f66a57d3b8491f866e7da9375c04b9b7627a4ed898fbfe1a0a331dc12b16fbd8`
and `56b6c59c3662efe31bde46b3fc186aaf252fc9f705287a3af81eaada660b5e74`.
`AssistantTheme` exposes dark and
light token variants, with dark selected by default. `theme.slint` registers
the bundled fonts (Inter regular, medium and semibold; IBM Plex Mono), so the
consuming root needs no font imports; provenance and licences of the bundled assets are in
[THIRD_PARTY.md](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/THIRD_PARTY.md).

`AssistantScrollbar` is a controlled vertical scrollbar. Its host supplies the
content height, visible height and offset, owns clamping the offset when the
extent changes, and handles its `scrolled` callback for user wheel, track and
thumb input. The thread uses it in place of Slint's stock scrollbar so all
three interactions update its follow-latest state. Touch momentum and
physical-device responsiveness are not yet qualified.

`AssistantHorizontalScrollbar` has the equivalent controlled width and offset
API. It handles horizontal wheel, track and thumb input, and rejects vertical
wheel events so an enclosing thread can handle them. The Markdown code viewport
uses it for long lines; its offset and scroll bounds remain owned by the viewport.
The focused software-window interaction check is run with
`python3 crates/assistant-slint/tests/run-markdown-scroll.py` from the workspace
root, using the demo's pinned Slint dependency graph. Physical touch and device
performance remain unverified.

`AssistantPagedText` is a controlled building block for hosts that explicitly
choose page-based reading. It receives only the current page, a one-based page
number and count, and emits previous/next requests; the host owns the complete
source, scope and boundary validation. It does not establish safe geometry by
itself. The component has a source check and an inspected static preview; it is
not connected to the conversation consumers or interaction-qualified.
