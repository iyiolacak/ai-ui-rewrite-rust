# ai-ui-state

UI-agnostic state for assistant-ui, after upstream's scoped state layer: what a UI renders (`ThreadState`, `MessageState`, parts, composer) and the actions it calls, over any `ThreadRuntime`.

A UI never reads a runtime directly. It renders `ThreadState` from a `ThreadView` and calls the view's actions (send, stop, reload, branch, copy, feedback, tool approvals). The view caches rows by `Arc` identity and takes a caller-supplied clock for smooth streaming (`tick(now_ms)`, `needs_frame`). See the crate docs for a runnable example.

For incremental model updates, call `view.state_changes_since(cursor.as_ref())`.
The returned `ViewChanges` contains the current snapshot, a new `ViewCursor`, and
`indices`: `Some(indices)` identifies rows to reconcile, while `None` requires a
full reconciliation. Always update the thread-level fields as well. Keep one
cursor per consumer and save the new cursor after applying the snapshot; other
consumers and ordinary `state()` reads do not consume your changes.

The journal holds 64 projected revisions, containing only indices. Foreign or
expired cursors, rebinds and structural/global changes fall back to a full read.
Drop the returned parent snapshot after reconciliation to preserve in-place
projection on the next update. Retaining it is supported and keeps it immutable,
but requires the full projection path. To track which rows a consumer shows,
hold `Weak<MessageState>` rather than row `Arc`s: strong references keep the old
rows alive while the view rebuilds them on a full reconciliation.

Part of [assistant-ui for Rust](../../README.md), a port of [assistant-ui](https://github.com/assistant-ui/assistant-ui) (MIT). Not affiliated with the assistant-ui project. Status and gaps: [docs/STATUS.md](../../docs/STATUS.md).
