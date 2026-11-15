# Assistant Markdown

This crate parses untrusted CommonMark/GFM snapshots into typed data and a
sanitized presentation. It does not fetch images, open links, execute HTML or
access the clipboard. Hosts decide how to display and activate content.

`presentation::project` processes a complete snapshot. Appending Markdown can
change earlier content, for example when a later link reference definition
resolves an earlier reference, so the crate does not assume a stable prefix.
Parser and presentation limits produce errors rather than partial results.

For Slint 1.18, safe link labels preserve inline code. A standalone link with
one strong, emphasis or strikethrough run can retain that run by serializing
clickable link fragments inside the formatting. The fragments all use the same
validated destination. A destination over 512 bytes, nested or multiple styled
runs, and styled links embedded in surrounding text keep their label flat. This
avoids Slint 1.18's link-range error when a nested style closes inside a link,
and bounds extra URL copies to at most two. Unsafe destinations stay inert.
The [parser compatibility fixture](test-support/slint-link/run.py) checks the
actual Slint 1.18.1 parser; it is not a physical pointer or device test.

`split::StreamSplit` divides a growing document into settled blocks and a live tail, so a host formats settled blocks once and only the tail on each update; the result is equivalent to formatting the whole document (tested over every prefix of several documents).

`scheduling::Scheduler` helps a host avoid projecting superseded snapshots. It
holds bounded queued input, dispatches one complete snapshot at a time, and
rejects results invalidated by a newer snapshot, cancellation or scope reset.
The key must identify the exact message part in the host's session. A rejected
submission also invalidates older work for that key; the host must display a
failure or fallback rather than silently retain stale output. Hosts should avoid
resubmitting an unchanged snapshot on every view refresh.

Submitting on every chunk can starve publication if chunks keep arriving
faster than projection finishes. For incremental display, a host can submit
one snapshot and, while it runs, retain only the latest subsequent source and
its host-owned sequence number in a bounded slot. On completion, it compares
the completed source's sequence with the current authoritative sequence: if
they differ, it may show the older result only as an intermediate preview with
an explicit updating state. It then submits the retained latest source. Repeat
at completion cadence, with a host-selected minimum display interval if
needed. A sequence must never go backwards, and an error or oversize latest
source must be displayed as a failure rather than described as current. The
host must independently bound the retained source and clear it on scope change;
the scheduler's queue limits do not cover that slot. This policy trades visual
freshness for progress and does not make the preview equal to the latest text.

`coordinator::Coordinator` provides this completion-cadence policy on native
hosts. Construct it with an explicit maximum key count and total latest-source
bytes. `submit(key, source)` retains one current snapshot per key and returns a
monotonically allocated `SourceRevision`; identical input keeps its revision.
Call `poll()` from the owner event loop to dispatch and receive results without
blocking. A `Completed` event whose `is_current` is false is an older preview,
not the current text. Even when true, the host must match its own current scope,
part identity, source revision and visibility before displaying it. A rejected
submission invalidates old work for that key; show literal source or an explicit
failure. Call `cancel` for keys no longer displayed and `clear` before changing
scope. A running old-scope parse is allowed to finish before new work dispatches,
so reset never spawns overlapping workers. The coordinator never stores results
for the host; the host must bound its own presentation cache. A terminal
`ExecutorFailed` event clears tracked sources and prevents further submissions;
show fallback and explicitly replace the coordinator if the host can safely
account for the old worker. Dropping a coordinator does not join its worker, so
a still-running pure parse and its input can outlive the owner. Key allocations,
one in-flight input, one bounded parse result, Slint conversion and host caches
are outside the latest-source budget. No automatic restart or timeout is supplied.

On native targets, `worker::Worker` can execute the owned job on a single
background thread. Its completion callback runs on that thread, so a Slint
host must marshal the result to the event loop, call `Scheduler::complete`,
verify the current scope and message are still present, publish only `Ready`,
and then dispatch the next queued job. If delivery to the executor fails, return
the job with `return_undispatched` and surface the failure. A host must detect
unexpected worker termination and must not wait forever for a lost completion.
Keep the dispatched `Job::ticket()` on the owner thread. On confirmed worker
failure or deliberate abandonment, call `Scheduler::abandon` with that ticket;
late completions are rejected. Stop using and replace the failed executor before
dispatching the next job, unless the host knows that it has finished. A host
must decide when a missing completion warrants abandonment and report failure;
this module does not provide a watchdog. Clear the scheduler at sign-out and
before changing scope; if waiting for an old job is unacceptable, abandon its
ticket and retire its executor before dispatching new-scope work. Dropping the worker
does not block: a dispatched source may remain in memory until its pure
projection finishes. A replacement can therefore overlap with a retired busy
thread: the host must bound retired workers and their memory. Queue limits do
not include keys, in-flight projection,
Slint conversion, layout, or other application allocations.

The offline demo integrates the scheduler with the native worker and a separate
WebAssembly worker bundle. The browser adapter keeps scheduler identity and the
job on the UI thread, sends a single bounded source snapshot, and attaches the
returned presentation to that same job before checking its revision. A failed,
malformed or oversized response is shown as literal source with a warning. The
worker is a trusted same-origin asset; hosts must validate responses from any
other executor. See the demo README for its build and hosting requirements.
This integration is not connected to the product. Browser JSON decoding, Slint
conversion and layout still consume time on the UI thread, and no device or
end-to-end performance claim follows from these APIs.
