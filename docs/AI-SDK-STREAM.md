# AI SDK stream runtime (`ai-ui-sdk-stream`)

## Scope

`crates/ai-ui-sdk-stream` folds the **AI SDK 7.0.166 UI-message stream** into messages the way the AI SDK's own
reducer does: strict lifecycle errors, exact data, tool, approval and step semantics, and incremental updates instead of
a full snapshot per event. It is the *AI SDK profile*. `ai-ui-stream` decodes the same wire into assistant-ui's
message model (a lenient profile that follows assistant-ui's own decoder where it is lossy, see `PROTOCOL.md`).

It does not implement the legacy v4 data stream, assistant-transport SSE or the `aui-*` extensions (those are in
`ai-ui-stream`), HTTP, request bodies, tool execution, persistence, Markdown or any UI. The response header
`x-vercel-ai-ui-message-stream: v1` identifies the wire format, not an SDK major version; v5/v6 reducers differ in
lifecycle behaviour, so this crate pins the v7 reference instead of guessing.

| Responsibility | Owner |
|---|---|
| Network reads, headers, credentials, I/O cancellation | the host (`ai-ui-http` for a ready-made client) |
| SSE framing, schema validation, indexed message folding, run tickets | this crate |
| Current tool argument text and explicit partial-value materialisation | this crate |
| Paint scheduling, Markdown, interaction | a UI crate |
| Backend tools, agent execution, provider calls | the backend; never inferred or executed by the decoder |

The only dependency is `serde_json`; there is no renderer, platform, networking or async-runtime dependency.

## Incremental API

One `Runtime` owns one active assistant message. Ordered parts have indexed
lookups for active text, reasoning, tools, approvals and persistent data. The
ordinary text-delta path appends new bytes to the addressed part; it does not
clone the message or scan conversation history.

`push(ticket, bytes, callback)` accepts arbitrary network read boundaries. Each
complete JSON event is validated and applied before the callback receives:

- ordered `Update` values, including transient data and control events;
- a borrowed `MessageView`;
- whether the pinned upstream read helper would publish a snapshot here.

The renderer can collect changed part handles and schedule one visual update.
It must still consume all transient data, errors, approvals and lifecycle events
in order. There is no unbounded internal notification queue.

`MessageView::snapshot()` explicitly creates an owned cumulative UI message.
`part_snapshot(handle)` materializes only one part. `part(handle)` borrows its
stored structural fields, and `raw_tool_input(handle)` borrows streaming
argument text without copying its accumulated prefix. Tool partial JSON repair
runs on an explicit snapshot request, not on every incoming argument delta.
Consequently snapshot-heavy consumers still pay for repeated preview parsing;
this is not yet a resumable incremental JSON parser.

Part handles include a run ticket and structural generation. Step resets and
changed-message resets invalidate queued handles. Restart validates the new
initial message before replacing the old run. Work carrying an old ticket is
ignored before parsing, budgeting or publication, so a stale asynchronous completion cannot
change the message. Resource bounds limit what a hostile or broken stream can allocate. These are
runtime-level guarantees, not UI-level qualification.

## Protocol behavior and explicit policy

- SSE handles LF, CRLF and bare CR, byte-fragmented UTF-8, an initial BOM,
  comments, ignored `event:`/`id:` fields and multi-line `data:`.
- `[DONE]` is observed and ignored by the pinned upstream reader. It does not
  prevent later events. `end(ticket)` is the transport EOF operation.
- EOF without `[DONE]` is accepted; an event lacking its final blank line is
  discarded. EOF/finish/abort do not invent a text-end event.
- Duplicate starts and lifecycle errors follow the pinned reducer; deltas are
  never deduplicated by content.
- Explicit `error` chunks are delivered and continue by default. A host can
  select `ServerErrorPolicy::Stop`; malformed JSON, invalid known chunks,
  invalid lifecycle transitions and limit failures always stop that run.
- Unknown types are rejected by default. `UnknownPolicy::Preserve` forwards an
  opaque notification without pretending it is a known message part. This
  policy is explicitly different from strict upstream validation.
- Tool outputs may be JSON null or primitives. Preliminary outputs, approvals,
  source parts, files, custom/steering parts, step resets, citations and metadata
  follow the source profile. Missing arbitrary-JSON fields and explicit null
  are not interchangeable.

Defaults bound assembled data/current physical-line bytes to 2 MiB, accepted
initial-message-plus-wire bytes to 64 MiB per run, and retained parts to 16,384.
All three are configurable application policy; the wire specification has no
such cap. The byte budget is conservative accepted-input accounting, not a
precise heap/RSS promise. An over-limit network read is rejected before applying
any event from that read. Earlier reads remain committed. A framing/schema
failure within an accepted read preserves earlier complete events from it.

### Supported JSON boundary

This increment uses `serde_json::Value`, not JavaScript values. The following
known boundaries remain explicit compatibility work, not silently passing cases:

| Input | Current Rust behavior / difference from the pinned JavaScript reader |
|---|---|
| Escaped lone UTF-16 surrogate (for example `"\\ud800"`) | Rejects the event as invalid JSON; JavaScript can retain such a string. Valid paired surrogates and ordinary UTF-8 are supported. |
| Numeric overflow (for example `1e400`) | Rejects the event; JavaScript parses infinity, which the oracle's JSON normalization renders as null. |
| Integers beyond JavaScript's safe range | Rust may retain an exact 64-bit integer where JavaScript rounds it. Do not compare after coercing Rust output through JavaScript numbers to claim parity. |
| Very deep JSON | Uses the parser's default 128-level recursion guard, including outer containers. No JavaScript-equivalent arbitrary-depth acceptance claim. |
| Non-BMP primitive string metadata / incomplete escaped surrogate previews | Rust scalar strings do not reproduce JavaScript UTF-16 property enumeration and all intermediate partial-value materializations. Raw tool input is retained. |

Prototype-sensitive `__proto__` and `constructor.prototype` keys anywhere in wire
JSON are rejected before schema dispatch, matching provider-utils' secure parser.
This applies to extra/unknown fields and tool-argument preview parsing as well;
initial in-memory message state is not parsed through that wire-only guard.
Ordinary finite JSON with Unicode scalar strings is the intended first profile.
The limits above prevent a claim that *any* AI SDK backend is already compatible.

## Correctness and performance gates

The fixture subset contains 45 source-established canonical UI7 cases. The
reference Node runner executes the pinned published packages, not a hand-written
JavaScript approximation. Rust tests compare folded messages and normalized
sidecars, then replay every case with several byte partitionings. Additional
tests cover budgets, stale tickets, atomic failure, local cancellation, explicit
unknown/error policy, part-handle invalidation and transient-event retention.

The fixture's `foldedMessage` is the last upstream publication. Direct current
state can be newer: `start-step` inserts a marker but does not itself publish.
The callback publication flag exposes that distinction without forcing snapshots.

CPU replay is separate from browser/render qualification:

```sh
cargo test -p ai-ui-sdk-stream --locked
cargo clippy -p ai-ui-sdk-stream --all-targets --locked -- -D warnings
cargo check -p ai-ui-sdk-stream --lib --target wasm32-unknown-unknown --locked
cargo bench -p ai-ui-sdk-stream --bench stream_replay --locked
cargo build -p ai-ui-sdk-stream --example replay --release --locked
cd benchmarks/ai-sdk-stream
npm ci --ignore-scripts
node ai-sdk-oracle.mjs --verify
```

Record patch-oriented and per-publication-snapshot modes separately. The latter
is the closer API comparison with `readUIMessageStream`, which materializes
snapshots internally. Nine or more interleaved trials, output-equivalence checks,
toolchain/package versions and machine/load context are required before quoting
a comparative CPU result. Neither mode measures React, Slint, rendering, network
latency, model generation, physical frame pacing or full application startup.
No 100x application-speed claim follows from these tests.

Protocol correctness fixtures do
not prove complete compatibility with every backend or every malformed input.

## Sources and licenses

Primary behavior references, pinned to AI SDK 7.0.166:

- [Chunk schema](https://github.com/vercel/ai/blob/ai%407.0.166/packages/ai/src/ui-message-stream/ui-message-chunks.ts)
- [Reducer](https://github.com/vercel/ai/blob/ai%407.0.166/packages/ai/src/ui/process-ui-message-stream.ts)
- [Read helper](https://github.com/vercel/ai/blob/ai%407.0.166/packages/ai/src/ui-message-stream/read-ui-message-stream.ts)
- [SSE reader](https://github.com/vercel/ai/blob/ai%407.0.166/packages/provider-utils/src/parse-json-event-stream.ts)
- [Partial JSON repair](https://github.com/vercel/ai/blob/ai%407.0.166/packages/ai/src/util/fix-json.ts)
- [WHATWG SSE interpretation](https://html.spec.whatwg.org/multipage/server-sent-events.html#event-stream-interpretation)

The partial JSON repair is adapted from the Apache-2.0-licensed AI SDK and carries
its notice/license beside the source. assistant-ui itself is MIT; that license
must not be confused with the AI SDK source license. `serde_json` is MIT OR
Apache-2.0. The crate's own code is MIT like the rest of this workspace; the package is published on crates.io as `ai-ui-sdk-stream` 0.1.0.

## Remaining work

1. Decide how this crate and `ai-ui-stream`'s UI decoder relate: for example convert this crate's messages into
   assistant-ui parts and use its incremental updates to avoid per-update work that grows with reply length.
2. Run the audit's 45 UI7 vectors through this crate as an independent check of both implementations.
3. Wire it into a UI (viewport-sized row publication, Markdown, approval controls, tool UI).
4. Broader differential fuzzing, bounded resumable JSON preview parsing if
   measured necessary, and representative native/browser performance comparisons.
