# Assistant stream oracle and baseline

This is an isolated, development-only Node harness for **AI SDK `ai@7.0.166`**
and **`@ai-sdk/provider-utils@5.0.76`**. Both direct dependencies are exact pins;
`package-lock.json` pins the full installation. The package is private and is not
part of the product's runtime or browser bundle.

It runs the published pipeline:

1. Network-byte stream into `parseJsonEventStream` with `uiMessageChunkSchema`.
2. Observation of transient data, aborts and finish reasons.
3. `readUIMessageStream` with its default `terminateOnError: false` behavior.
4. Consumption of **every upstream message snapshot**, retaining only the last.

This is a decoder/fold/snapshot baseline, **not a React rendering benchmark**.
It makes no model, backend or provider requests. No browser or visible window is
opened. It cannot establish application startup, frame pacing, input latency,
memory usage in a rendered conversation, or a general Rust-versus-React speedup.

## Install and check

From this directory, with Node 22 or newer:

```sh
npm ci --ignore-scripts
npm run verify
```

`verify` checks all 45 cases in
[`ui7.json`](../../crates/ai-ui-sdk-stream/tests/fixtures/ui7.json), comparing the
complete `foldedMessage` and `sidecars` projections. It prints one JSON summary
and exits nonzero for a mismatch. Failure diagnostics contain the fixture ID and
mismatched field, never raw input, snapshots, or stack traces.

The fixtures are a mechanical, unchanged subset of the earlier protocol-audit
corpus, selected by `oracle_profile: "ai-sdk-ui7"`. Every case has the label
`ESTABLISHED`, a description, exact input bytes encoded in JSON, expected results,
and primary source URLs. They cover valid lifecycles **and observed malformed-input
behavior**. `ESTABLISHED` does not mean a malformed case conforms to the protocol,
nor does it mean a Rust implementation already passes the case.

## JSON-lines interface

Run the persistent process directly, avoiding npm's command banners on stdout:

```sh
node benchmarks/ai-sdk-stream/ai-sdk-oracle.mjs < requests.jsonl
```

The command above is relative to the repository root. Each nonblank input line
is one JSON request. Each produces one JSON response on stdout. Requests are
processed in order, each with fresh stream and reducer state.

| Request field | Meaning |
|---|---|
| `raw_utf8` | Complete wire stream as a JSON string; encoded to UTF-8 without adding bytes. Required unless an explicit network-chunk array is present. |
| `initialMessage` | Optional initial UI message. Defaults to `{"id":"m","role":"assistant","parts":[]}`. The supplied value is copied before replay and is not mutated. |
| `chunk_bytes` | Optional positive safe integer: partition the encoded `raw_utf8` into at most this many bytes per network read. UTF-8 code points and CRLF pairs can be split. Without it, deliver one read. |
| `network_chunks_base64` | Optional array of canonical padded base64 strings, each decoded as one network read. Takes precedence over `raw_utf8` and `chunk_bytes`. |
| `network_chunks_utf8` | Optional array of strings, each encoded separately as one network read. Same precedence. Do not supply both explicit chunk-array fields. |
| `mode` | Optional string accepted for shared benchmark-driver input. It **does not alter this oracle**: upstream always produces snapshots. The Rust driver accepts exactly `snapshot` (default) or `patch` and rejects other values. Use only those values for shared requests. |

Example request:

```json
{"raw_utf8":"data: {\"type\":\"text-start\",\"id\":\"t\"}\n\ndata: {\"type\":\"text-delta\",\"id\":\"t\",\"delta\":\"Hello\"}\n\ndata: {\"type\":\"text-end\",\"id\":\"t\"}\n\ndata: [DONE]\n\n","chunk_bytes":17}
```

Response shape (`elapsed_ms` is runtime-dependent):

```json
{
  "foldedMessage": {
    "id": "m",
    "role": "assistant",
    "parts": [{"type":"text","text":"Hello","state":"done"}]
  },
  "sidecars": {"errors":[],"transientData":[],"aborts":[]},
  "elapsed_ms": 0.0,
  "emissions": 3
}
```

`emissions` counts actual snapshots yielded by `readUIMessageStream`. Snapshot
construction is intrinsic to the upstream helper and remains in the measured
work even though old snapshots are not retained by this consumer.

`sidecars` are harness observations, not fields added to the AI SDK UI message:

- `errors`: upstream `onError` callbacks, preserving error class; lifecycle errors
  also preserve `chunkType` and optional `chunkId`. A plain explicit `Error`
  preserves its message. Schema diagnostic strings and JSON parser excerpts are
  omitted, matching the original corpus projection.
- `transientData`: transient `data-*` chunks observed after schema validation.
- `aborts`: observed `abort` chunks.
- `finishReason`: last supplied non-null finish reason, omitted when none arrived.

An explicit wire `error` chunk follows the upstream helper's default continuation
policy; this is not `useChat`'s throwing error policy. Parsing/lifecycle failures
can leave an earlier snapshot as the last visible result. The oracle neither
repairs invalid events nor adds recovery policies absent from the pinned SDK.

Malformed harness requests produce `{"error":{"name":"OracleInputError",...}}`
and set the process exit status to nonzero; later JSON lines are still processed.
Unexpected harness failures return the fixed code `REPLAY_FAILED` or
`HARNESS_FAILED` without dumping input. Successful response payloads necessarily
contain the requested folded data; there is no additional payload logging.

## Timing boundary and comparison rules

The timer starts **after** the input JSON is parsed, network byte chunks are
prepared, and the initial message is copied. It includes stream construction and
enqueuing, SSE framing, JSON/schema decoding, sidecar observation, reducer work,
upstream snapshot production and consumption. It stops immediately after the
final snapshot iterator completes. JSON normalization and output serialization
are outside the interval. No network latency or model generation is measured.

Keep network partitioning identical between the implementations. First compare
the final folds and sidecars; reject timing results if correctness differs.

Report Rust patch mode and Rust snapshot mode separately. Comparing a patch-only
Rust consumer to this snapshot-producing helper measures the benefit of a changed
consumer contract as well as implementation differences. It is not an isolated
JSON-parser or language comparison. `emissions` is not a rendering count.

The Rust driver's `snapshot` result is the **last published snapshot**. Its `patch`
result is **current reducer state at EOF/failure**, materialized once. These are
deliberately different contracts: a sole `start-step` event changes current parts
to `[{"type":"step-start"}]` but publishes no snapshot, so upstream and Rust
snapshot mode still return the initial empty parts. The same difference occurs
after a trailing, unpublished `start-step`. Do not report those as equal-fold
benchmark workloads. First establish equality for the particular workload, then
compare timings and explicitly name both modes and their snapshot counts.

Warm both runtimes, then use at least **nine interleaved samples per configuration**,
alternating which runs first, as required by [`docs/profiling.md`](../../docs/profiling.md).
Record Node/Rust versions, optimization profile, CPU count, `load_1m`, workload
bytes/chunk counts and distributions. Report medians and spread. Setup, process
startup, output serialization and rendered UI need separate measurements if they
matter to the claim. Do not infer a 100× application speedup from this harness.

## Reproduce the extended gate and CPU comparison

From the repository root, after the locked Node installation:

```sh
cargo build -p ai-ui-sdk-stream --example replay --release --locked
node --test benchmarks/ai-sdk-stream/compare.test.mjs
node benchmarks/ai-sdk-stream/differential.mjs target/release/examples/replay
node benchmarks/ai-sdk-stream/compare.mjs --binary target/release/examples/replay --out assistant-replay-result.json
```

Substitute the actual executable location if `CARGO_TARGET_DIR` is set. The
output path must be new; the driver will not overwrite an earlier result.
The driver records a SHA-256 of the supplied executable; it cannot infer build
flags or compiler provenance from it. The caller must use the release build
above. Its `rust` version field records the compiler on `PATH`.

`differential.mjs` exercises 257 additional deterministic cases against the
published oracle. The observed result is 247 exact matches and ten narrowly
checked Unicode differences, with zero unexpected mismatches. Known exceptions
substitute only the documented metadata/preview projection and then compare the
whole result; they do not suppress unrelated errors or fields. Unexpected
differences fail the process. This is focused testing, not exhaustive fuzzing.

`compare.mjs` uses one persistent Rust process, excludes process startup from
internal replay clocks, and tests text, fragmented tool JSON and a message with
1,000 existing parts. It warms each mode twice and uses nine balanced interleaved
trials. Every trial checks the final fold, sidecars and publication count before
accepting its timing. It does not compare every intermediate snapshot. Child
writes, reads and shutdown have a 30-second bound. No backend is contacted.

The initial [Linux report](results-linux-x86_64-2026-11-09.json) and
[validation analysis](../../VALIDATION.md) record the observed results and their
limits. Snapshot-demand-equivalent CPU medians were 2.12–3.10× faster in Rust.
Update-oriented consumption was 18.48–693.63× faster on those synthetic cases,
but omits intermediate cumulative snapshots/previews. That difference is the
purpose of the API design, not evidence of a 100× faster rendered application.

## Primary sources and version boundary

The fixtures and oracle target AI SDK `7.0.166`, source commit
`afc151d0b5db9adfce6e131fbd56e544cbffc577`, checked on 2026-11-09:

- [UI message chunk schema](https://github.com/vercel/ai/blob/ai%407.0.166/packages/ai/src/ui-message-stream/ui-message-chunks.ts)
- [UI message reducer](https://github.com/vercel/ai/blob/ai%407.0.166/packages/ai/src/ui/process-ui-message-stream.ts)
- [Snapshot-producing read helper](https://github.com/vercel/ai/blob/ai%407.0.166/packages/ai/src/ui-message-stream/read-ui-message-stream.ts)
- [JSON/SSE parser](https://github.com/vercel/ai/blob/ai%407.0.166/packages/provider-utils/src/parse-json-event-stream.ts)

This harness does not cover AI SDK v4 legacy data streams, assistant-transport,
assistant-ui's `aui-*` extensions, requests to real backends, or React/Slint UI
behavior. Those require their own adapters and tests.

## Known Rust profile and harness limitations

Passing the 45 fixtures is evidence for those cases, not a universal claim that
every JavaScript JSON value or upstream extension has identical Rust behavior:

- Rust's `serde_json` event parser rejects out-of-range numbers such as `1e400`;
  JavaScript accepts an infinite numeric value, which this oracle's JSON output
  normalizes to `null`. Rust also rejects escaped lone UTF-16 surrogates such as
  `"\ud800"`, which JavaScript can retain in a string.
- Integers beyond JavaScript's exact-number range can remain exact in Rust while
  JavaScript rounds them. For example, `9007199254740993` is emitted exactly by
  Rust but becomes `9007199254740992` in JavaScript. Parsing Rust output with
  JavaScript before comparing can mask this distinction; numeric-boundary tests
  need an integer-preserving comparison as well.
- The default `serde_json` nesting limit is an additional Rust parser bound; a
  data value containing 140 nested arrays was accepted by this pinned oracle and
  rejected by Rust. Configured event-byte, run-byte and part-count bounds are
  separate application policy, not upstream wire-protocol limits.
- Rust replay abort sidecars currently reconstruct only known `type` and `reason`
  fields. This Node oracle observes the validated wire chunk, so loose additional
  fields such as `{"type":"abort","future":7}` survive here but not in the Rust
  replay sidecar. Sidecar equality must account for this harness limitation.

Rust request validation rejects unknown modes, nonpositive/nonintegral/unsafe
chunk widths, ambiguous explicit chunk arrays, and noncanonical base64 (including
nonzero padding bits). Explicit chunks override `raw_utf8` partitioning, but do
not bypass validation of a supplied `chunk_bytes`. These checks occur before the
replay timer and prevent accidentally timing different byte inputs or modes.
