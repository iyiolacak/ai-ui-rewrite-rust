# Markdown to Slint adapter

This adapter converts a bounded `ai-ui-markdown` presentation into a
host application's generated Slint Markdown block and cell types. The app
implements `Factory` with two constructors, mapping `BlockKind` and `Alignment`
to its generated enums and `Block::cells` to a Slint model. Each Slint build
produces distinct record types, so the factory is an explicit compile-time
boundary rather than a dependency on a particular window.

`convert` validates presentation node and byte limits before creating records;
this is not a safety check for arbitrary forged presentations. The supplied
presentation must come from the trusted portable projector or an equivalently
trusted executor. It preserves the portable representation's depth, quote depth, headings, code,
cell alignment, and order. Inline text uses Slint's Markdown parser and falls
back to the supplied plain text with an explicit warning if that parser rejects
it. `literal` preserves source text and a caller-supplied warning when projection
is unavailable or the displayed source is only a truncated suffix.

Run this conversion on the Slint thread. It does no network, filesystem,
clipboard, image loading or link-opening work; the host controls any actions,
including code-copy tokens and link callbacks. The host must verify scope and
source revision before displaying an asynchronously produced presentation, and
must bound how much conversion and retained output it accepts. This crate does
not promise frame-time or allocator-memory limits and is not a sandbox.

`cache` provides reusable UI-thread ownership for visible source snapshots. A
host supplies a nonblocking `Backend` that owns revision fencing and executor
lifecycle, and observes unique `(key, source)` pairs in priority order. The
cache holds bounded source slots and a separately budgeted estimate of rendered
presentation strings and nodes. `view` only returns blocks for the exact source,
and reports clipped suffixes as literal text. A capacity-rejected presentation
is retried only when other cached content releases capacity. `has_pending` lets
a host stop polling once backend work and undelivered failures are exhausted.
Call `clear` before changing scope, and retain the host's own authorization and
part-visibility checks. The native `NativeBackend` uses the separate
single-worker Markdown coordinator; a browser host supplies its own adapter.
