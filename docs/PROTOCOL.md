# Wire protocols: audit and conformance record

**Provenance.** This page records a protocol audit produced by GPT-6 Pro on 2026-11-09 at the maintainer's request,
brought in by the maintainer. It cites primary sources (AI SDK and assistant-ui source at pinned tags) and ships with
96 executable vectors, saved at `crates/ai-ui-stream/tests/fixtures/protocol-vectors.json`. **We have not
re-verified its claims against upstream ourselves**; the vectors are the checkable part: 90 were executed by the
auditor against published upstream packages, and 6 are labelled `HYPOTHESIS` (proposed recovery policies, not
upstream behaviour). Where this page and a vector disagree, trust neither until checked against the cited source.
Our own conformance status is in [Conformance status](#conformance-status).

Audited versions: `ai@7.0.166` (latest; `ai@6.0.326`, `ai@5.0.286`, `ai@4.3.19` + `@ai-sdk/ui-utils@1.2.11` for
history), `assistant-stream@0.3.56`, `@assistant-ui/react@0.15.33`, `@assistant-ui/react-data-stream@0.12.41`,
`@assistant-ui/ai-sdk@0.0.17`, assistant-ui commit `eb65bd4558b769550093003c95e5867ad0ec8eba` (our reference commit).

Headline findings:

- The UI stream header `x-vercel-ai-ui-message-stream: v1` is the same in AI SDK v5, v6 and v7: it does not identify the
  SDK major version.
- AI SDK's reducer and assistant-ui's generic `UIMessageStreamDecoder` do **not** fold identical input identically; some
  differences are accommodations, others discard information (see B2).
- Decoding responses alone cannot give "any backend works unchanged": the data-stream runtime POSTs model-style
  `role`/`content` messages, `AssistantChatTransport` POSTs UI `role`/`parts` messages (B3).
- Usage, errors, aborts, transport completion and message completion are separate things, not all message parts.

Notation: `s` string, `b` boolean, `n` number, `J` any JSON (incl. `null`), `O` JSON object, `PM` =
`Record<string, O>` keyed by provider, `?` optional. "Complete" = all standard variants at the pinned releases;
`data-*` names, provider metadata, tool args/results and app state are open-ended.

## A1. AI SDK UI message stream (`ai@7.0.166`)

Framing: UTF-8 SSE, `data: {json}\n\n`, optionally ending `data: [DONE]\n\n` (an SSE event, not a JSON chunk). Marker
header: `x-vercel-ai-ui-message-stream: v1`. Sources under `packages/ai/src/` at tag `ai@7.0.166`:
`ui-message-stream/ui-message-chunks.ts`, `ui/process-ui-message-stream.ts`, `ui-message-stream/read-ui-message-stream.ts`,
`ui-message-stream/to-ui-message-chunk.ts`, `ui/chat.ts`, `util/merge-objects.ts`, `types/citation.ts`,
`ui/ui-messages.ts`; and `packages/provider-utils/src/parse-json-event-stream.ts`.

29 fixed types plus the `data-*` family. `ToolFlags` expands to four optional direct properties:
`providerExecuted?:b`, `providerMetadata?:PM`, `toolMetadata?:O`, `dynamic?:b`.

| `type` | Fields beyond `type` | Folding / lifecycle |
|---|---|---|
| `start` | `messageId?:s`, `messageMetadata?:J` | Establish/update message identity and metadata. |
| `text-start` | `id:s`, `providerMetadata?:PM` | Create an empty streaming text part; register the ID. |
| `text-delta` | `id:s`, `delta:s`, `providerMetadata?:PM` | Append to the active text part with that ID. |
| `text-end` | `id:s`, `citations?:Citation[]`, `providerMetadata?:PM` | Mark done; remove the active ID. |
| `reasoning-start` | `id:s`, `providerMetadata?:PM` | Create/register a reasoning part. |
| `reasoning-delta` | `id:s`, `delta:s`, `providerMetadata?:PM` | Append reasoning text. |
| `reasoning-end` | `id:s`, `providerMetadata?:PM` | Mark done; remove the active ID. |
| `tool-input-start` | `toolCallId:s`, `toolName:s`, `inputEncoding?:"json"\|"text"`, `title?:s`, `ToolFlags` | Begin argument streaming; absent encoding = JSON. |
| `tool-input-delta` | `toolCallId:s`, `inputTextDelta:s` | Append raw argument text; JSON preview via partial parse, or string for text encoding. |
| `tool-input-available` | `toolCallId:s`, `toolName:s`, `input:J`, `title?:s`, `ToolFlags` | Complete input; may appear with no start/deltas; `input` is authoritative. |
| `tool-input-error` | `toolCallId:s`, `toolName:s`, `input:J`, `errorText:s`, `title?:s`, `ToolFlags` | Input failed; folds to `output-error`. |
| `tool-approval-request` | `approvalId:s`, `toolCallId:s`, `approvalDescriptor?:J`, `inputSchemaInput?:J`, `reason?:s`, `isAutomatic?:b`, `signature?:s` | Attach an approval gate to an existing call. |
| `tool-approval-response` | `approvalId:s`, `approved:b`, `reason?:s`, `providerExecuted?:b`, `providerMetadata?:PM` | Find the call by approval ID; no `toolCallId` field. |
| `tool-output-available` | `toolCallId:s`, `output:J`, `preliminary?:b`, `ToolFlags` | Update an existing call's output; preliminary output may be replaced. |
| `tool-output-error` | `toolCallId:s`, `errorText:s`, `ToolFlags` | Existing call -> output error. |
| `tool-output-denied` | `toolCallId:s` | Existing call -> execution denied. |
| `source-url` | `sourceId:s`, `url:s`, `title?:s`, `providerMetadata?:PM` | Append a URL source. |
| `source-document` | `sourceId:s`, `mediaType:s`, `title:s`, `filename?:s`, `providerMetadata?:PM` | Append a document source (no URL). |
| `file` | `url:s`, `mediaType:s`, `providerMetadata?:PM` | Append an output file. |
| `reasoning-file` | `url:s`, `mediaType:s`, `providerMetadata?:PM` | Append a file belonging to reasoning. |
| `custom` | `kind:s`, `providerMetadata?:PM` | Provider-specific content; `kind` is dotted `provider.kind` in TS (not enforced at runtime). |
| `steering-message` | `id:s`, `parts:Array<TextUIPart\|FileUIPart>` | Experimental nested steering message. |
| `data-${name}` | `data:J`, `id?:s`, `transient?:b` | Persistent data upserts by **(type, id)**; no id = append; transient is event-only. |
| `start-step` | none | Append a `step-start` boundary part. |
| `finish-step` | none | Step boundary notification; **no usage or finish reason**. |
| `reset-step` | none | Remove parts after the last step-start (all parts if none); clear active maps. |
| `finish` | `finishReason?:F7`, `messageMetadata?:J` | Record finish reason/metadata; does not synthesize missing ends. |
| `abort` | `reason?:s` | Outcome event, not a content part. |
| `error` | `errorText:s` | Error handling hook; not a message part. |
| `message-metadata` | `messageMetadata:J` | Update message metadata. |

`F7` = `stop | length | content-filter | tool-calls | error | interrupted | other`.

`Citation` = `{ source: {type:"source", sourceType:"url", id, url, title?, providerMetadata?} | {type:"source",
sourceType:"document", id, mediaType, title, filename?, providerMetadata?}, startIndex?:n, endIndex?:n, citedText?:s }`
(`source.id` here vs `sourceId` on top-level source chunks). `steering-message.parts`: text
`{type:"text", text, providerMetadata?}` or file `{type:"file", mediaType, url, filename?, providerReference?:Record<string,s>,
providerMetadata?}` (do not copy `filename`/`providerReference` onto the top-level `file` chunk).

Rules that matter:

1. Text and reasoning have independent active-ID maps. A delta/end without its start raises `UIMessageStreamError`.
   Different active IDs may interleave; an ID may be reused after its end.
2. Tool input streaming is optional: start -> deltas -> available/error. There is **no `tool-input-end` chunk**.
3. Partial JSON is a preview; the reducer keeps `rawInput` while streaming and the state stays `input-streaming`
   until `tool-input-available`.
4. Approval and output chunks need an existing call; approval responses resolve by approval ID; outputs can find calls
   from earlier steps.
5. In v7, `finish-step` does **not** close active text/reasoning; explicit end chunks do.
6. Persistent data replacement is by `(type,id)` and replaces (no deep merge). Sources/files append without dedup.
7. Object message metadata merges recursively, arrays replace, top-level null is ignored. Provider metadata on
   text/reasoning replaces the previous.
8. Errors: `useChat` throws; `readUIMessageStream` defaults to `terminateOnError:false` (continues after an error
   chunk). `abort` mutates no content part.
9. Usage is not on the UI wire: `finish-step`/`finish` carry none; usage must travel in application metadata/data.
10. Validators are loose objects (extra fields allowed on known types) but reject unknown non-`data-*` types.

## A2. Legacy AI SDK data stream (`ai@4.3.19`)

`<code>:<JSON>\n`, marker `x-vercel-ai-data-stream: v1`, no SSE, no `[DONE]`. Sources: `packages/ui-utils/src/
{data-stream-parts,process-data-stream,process-chat-response}.ts` at `ai@4.3.19`. 16 codes:

| Code | Meaning | Payload |
|---|---|---|
| `0` | text | `s` |
| `2` | data | `J[]` |
| `3` | error | `s` |
| `8` | message annotations | `J[]` |
| `9` | complete tool call | `{toolCallId, toolName, args:O}` |
| `a` | tool result | `{toolCallId, result:J}` |
| `b` | tool input start | `{toolCallId, toolName}` |
| `c` | tool input delta | `{toolCallId, argsTextDelta}` |
| `d` | finish message | `{finishReason:F4, usage?:{promptTokens,completionTokens}}` |
| `e` | finish step | `{finishReason:F4, usage?:{promptTokens,completionTokens}, isContinued?:b}` |
| `f` | start step | `{messageId}` |
| `g` | reasoning text | `s` |
| `h` | source | `{sourceType:"url", id, url, title?, providerMetadata?}` |
| `i` | redacted reasoning | `{data:s}` |
| `j` | reasoning signature | `{signature:s}` |
| `k` | file | `{data:s, mimeType:s}` |

`F4` = `stop | length | content-filter | tool-calls | error | other | unknown`. Notes: `0`/`g` append to the current
part (no ids); `b` -> `c*` -> `9` streamed input, `9` may stand alone, `a` supplies the result; `j` is ignored without a
current plain reasoning detail; `e.isContinued` defaults false; the v4 hook ignores `e.usage` (uses `d.usage`); `3`
throws and stops; parser accepts arbitrary finish-reason strings and (`9.args`) null/arrays; v4 puts bare Base64 in
`k.data` (assistant-ui streams may carry a full data URL: keep the raw string).

## A3. assistant-transport SSE (`assistant-stream@0.3.56`)

Not the AI SDK UI stream. `data: <JSON>\n\n` ending `data: [DONE]\n\n`; no protocol-revision header. Sources at
`eb65bd4`: `packages/assistant-stream/src/core/{AssistantStreamChunk.ts, serialization/assistant-transport/
AssistantTransport.ts, accumulators/assistant-message-accumulator.ts, utils/types.ts, tool/tool-types.ts, gorp/types.ts}`.

13 variants, each with `path:number[]` (nonnegative ints):

| `type` | Fields beyond `type,path` |
|---|---|
| `part-start` | `part:PartInit` |
| `part-finish` | `providerMetadata?:PM` |
| `tool-call-args-text-finish` | none |
| `tool-call-approval` | `approval:Approval` |
| `text-delta` | `textDelta:s`, `providerMetadata?:PM` |
| `annotations` | `annotations:J[]` |
| `data` | `data:J[]` |
| `step-start` | `messageId:s` |
| `step-finish` | `finishReason:F4`, `usage:{inputTokens,outputTokens}`, `isContinued:b` |
| `message-finish` | `finishReason:F4`, `usage:{inputTokens,outputTokens}` |
| `result` | `result:J`, `isError:b`, `artifact?:J`, `isPreliminary?:b`, `modelContent?:ModelContent[]`, `messages?:J` |
| `error` | `error:s`, `code?:s`, `severity?:"critical"\|"warning"\|"info"` |
| `update-state` | `operations:StateOperation[]` |

`PartInit` (every row also allows `parentId?:s`): `text` (`providerMetadata?`), `reasoning` (`unstable_summary?`,
`providerMetadata?`), `tool-call` (`toolCallId`, `toolName`), `source` url (`id,url,title?`) or document (`id,title,
mediaType,filename?`), `file` (`data,mimeType`), `data` (`name,data`).
`Approval` = `{id, approved?, reason?, prompt?, isAutomatic?, signature?, requestReason?, descriptor?, inputSchemaInput?}`;
`ModelContent` = text `{type:"text",text}` or file `{type:"file",data,mediaType,filename?}`;
`StateOperation` = `{type:"set", path:string[], value:J}` or `{type:"append-text", path:string[], value:s}` (no delete).

Two path systems: chunk `path` is numeric part indices; state-operation `path` is string segments.
Qualifications: message-level chunks and `part-start` may omit `path` (normalised to `[]`); the five addressed variants
require it. The accumulator appends on `part-start` (ignores the supplied path); addressed updates need
`path.length === 1` (nested paths are dropped). `tool-call-approval` is request and response (presence of `approved`
= decision; replaces the previous approval; `approved:false` alone does not create a denied result). Preliminary results
stay unsettled; a final result clears stale `artifact`/`modelContent`/`messages` if omitted; preliminary after final is
ignored. `error` makes the message `incomplete/error` regardless of severity, and later `message-finish` does not
erase it. Step usage is kept in step metadata. `strict:true` is not schema validation: malformed JSON, unknown chunks,
invalid paths and failed guards are warned and dropped; strictness covers unknown SSE event names and a missing `[DONE]`.
No native `abort`, `message-metadata`, `reasoning-delta`, `tool-input-*`, `tool-output-*`, `source-url`, `source-document`.

## A4. assistant-ui `aui-*` extensions (data stream)

Same framing/marker as A2. Source: `.../serialization/data-stream/{chunk-types,DataStream}.ts`. Six extra codes:
`aui-state` (`StateOperation[]`), `aui-text-delta` (`{textDelta,parentId}`), `aui-reasoning-delta`
(`{reasoningDelta,parentId}`), `aui-data` (`{name,data,parentId?}`), `aui-reasoning-part-start`
(`{unstable_summary?,parentId?}`), `aui-tool-call-approval` (`{toolCallId, approval:Approval}`).
Stock codes differ: `b` adds `parentId?`; `c` adds `isFinal?` (true closes input after appending); `a` adds `artifact?,
isError?, isPreliminary?, modelContent?, messages?`; `3` is a string **or** `{error,code?,severity?}`; `h` url or
document source with `parentId?` (no provider metadata); `k` adds `parentId?`; `d`/`e` usage is
`{inputTokens,outputTokens}` not v4's `promptTokens/completionTokens` (passed through unrenamed; missing -> zeros;
missing continuation -> false). `i`/`j` are recognised and ignored. A complete `9` after streamed deltas closes the
input without replacing the streamed text. Duplicate `b` ids and unknown tool ids are strict-mode errors. A preliminary
`a` closes arguments but leaves the result open. EOF closes open argument streams; the legacy line decoder rejects a
nonempty final line without a terminator (v4's parser flushes it).

## A5. Out of scope: AI SDK `AssistantResponse`

AI SDK v4 `useAssistant`/`AssistantResponse` is a separate line protocol (`0` text, `3` error, `4` assistant message
`{id,role,content:[{type:"text",text:{value}}]}`, `5` `{threadId,messageId}`, `6` `{id?,role:"data",data}`), no Vercel
version marker, multiple messages per stream. A single cumulative assistant message cannot represent it fully.

## B1. Version differences and detection

| Transition | Decoding-relevant change |
|---|---|
| v4 -> v5 | Code lines become SSE JSON; header `x-vercel-ai-ui-message-stream`; text/reasoning get ids and start/delta/end; `args`/`result` -> `input`/`output`, `inputTextDelta`; typed `data-*`, transient data, message metadata, abort, URL/document sources, URL files; step usage no longer in UI finish-step. |
| 5.0.0 -> 5.0.286 | adds `tool-input-error`, preliminary output, `finish.finishReason`; loose validation. |
| v6.0.0 | adds `tool-approval-request`, `tool-output-denied`, tool titles; removes `"unknown"` finish reason. |
| 6.0.326 | `inputEncoding`, tool metadata, provider metadata on more tool events, approval descriptor/input-schema/signature, abort reason. |
| v7.0.0 -> 7.0.166 | wire `tool-approval-response`, automatic approval, `custom`, `reasoning-file`; later citations, steering messages, `reset-step`, `"interrupted"`, expanded approval fields, text-encoded input; lifecycle fixes for merged streams and resume. |

Detection: `x-vercel-ai-data-stream: v1` = legacy family (may include `aui-*`); `x-vercel-ai-ui-message-stream: v1` = UI
SSE, cannot tell v5/v6/v7; `text/event-stream` alone = SSE only (native vs UI undetermined); an `aui-*` code = extension
present; a newer optional field = capability, not a version. assistant-ui's data runtime picks the **response decoder**
in this order: explicit protocol, legacy header, UI header, UI fallback (both markers: legacy wins). It does not choose
the request schema.

## B2. assistant-ui's UI adapter vs the AI SDK reducer

| Feature | Generic `assistant-stream` UI decoder | AI SDK 7 |
|---|---|---|
| Persistent `data-*` ids | drops id; appends `{name,data}` to `metadata.unstable_data` | replaces `(type,id)` |
| Tool-input deltas | buffers until available/error/flush | updates progressively |
| Delta before text start | may create text implicitly | errors on fresh state |
| `reset-step` | ignored | removes current-step parts |
| `reasoning-file`, `custom`, `steering-message` | ignored | distinct parts |
| Message metadata, citations | discarded | supported |
| `inputEncoding`, `dynamic`, `title`, `toolMetadata`, provider flags | lost | supported |
| `finish-step` with active parts | closes active mappings | keeps them active |
| Abort reason | lost; becomes finish reason `other` | available |

Approval request/response chunks are supported by the generic decoder. The `@assistant-ui/ai-sdk` path lets AI SDK fold
first, then converts with losses (reasoning-file -> file, citations dropped, raw string input not representable).
**Recommendation (auditor): implement the full AI SDK wire contract and make reference-adapter compatibility a named
profile; do not copy the lossy adapter.**

## B3. Request-body contract

`useDataStreamRuntime` POSTs JSON (forced `Content-Type: application/json`, credentials default `same-origin`, run abort
signal). Precedence: `base fields < context.callSettings < context.config < resolved options.body` (later wins, even
`messages`/`tools`). Fields: `messages` (`[...messages, unstable_getMessage()]`), `tools` (name -> schema; normally `{}`),
`system?`, `unstable_assistantMessageId?` (only if truthy), `threadId?` (only if truthy), `parentId` (null survives),
`runConfig` (`{custom?}`, normally `{}`), `state?` (from `metadata.unstable_state`), flattened call settings (`maxTokens,
temperature, topP, presencePenalty, frequencyPenalty, seed, headers`), flattened config (`apiKey, baseUrl, modelName,
reasoningEffort`), custom body last. No enclosing `context`/`callSettings`/`config` object. `onResponse` is a callback
and sees non-2xx responses too.

Messages are model-style `role`/`content`: system `{role:"system",content:s}`; user text/file content arrays
(attachments appended); text `{type:"text",text}`; file `{type:"file",data,mediaType,filename?}`; assistant
`{type:"tool-call",toolCallId,toolName,input}`; tool result `{type:"tool-result",toolCallId,toolName,output}` with output
`{type:"json",value}` / `{type:"error-json",value}` / `{type:"content",value:[text|media]}` /
`{type:"execution-denied",reason?}`. One assistant message may become several assistant/tool messages.
`sendExtraMessageFields:true` adds `unstable_id` (not `id`). Reasoning, sources, annotations, custom data, provider
metadata, timestamps, statuses are not replayed. An old unfinished call gets `{"error":"Tool call was not completed"}`;
preliminary results do not count as settled. A decided approval inserts, after the call,
`{type:"tool-approval-request",approvalId,toolCallId,reason?,isAutomatic?,signature?,inputSchemaInput?}` and, in tool
content, `{type:"tool-approval-response",approvalId,approved,reason?}`.

Tools are `Record<name,{parameters:JSONSchema7, description?, providerOptions?}>` (no `execute`; non-uploadable,
disabled and backend tools filtered; names sorted). Backends read `await req.json()` and use `frontendTools(body.tools ??
{})`. No universal helper interprets `runConfig`, thread ids, state, model selection or custom fields.
v7 qualifications: system-role messages in `messages` are rejected by default (use top-level `instructions`; `system` is a
deprecated alias; `allowSystemInMessages:true` restores the old layout); the historical `media` tool-content branch is
outside the declared output schema (core passes it through to a mock model; provider acceptance unverified).

`AssistantChatTransport` POSTs a different body: `{callSettings?, system?, config?, tools, id, messages:[UI messages with
parts], trigger, messageId?, metadata?}` (settings/config nested). Its backend calls `convertToModelMessages(messages)`;
applying that to the data runtime's already model-style messages is wrong.

## C. Edge cases: established behaviour and our policy

| Case | Established upstream behaviour | Recommended Rust behaviour (auditor) |
|---|---|---|
| Interleaved ids | independent active text/reasoning/tool ids may interleave | separate namespaces, keep creation order |
| Duplicate active text id | appends a new part and replaces the active entry | diagnose active collisions; distinguish reuse after end |
| Duplicate deltas | no sequence numbers; appended again | do not dedupe by payload |
| Delta before start | fresh AI SDK errors; assistant-ui may synthesize | strict errors; optional tolerant mode with diagnostic |
| Resume mid-part | start may precede the cursor; chat keeps active state across disconnect | preserve active maps or require replay |
| Tool input never completes | partial JSON gives a repaired preview; native args-finish can close incomplete text | keep raw text, preview and completion separate; never execute from repaired JSON |
| Complete input differs from preview | AI SDK uses complete `input`; legacy assistant-ui keeps streamed text | profile-specific; keep raw input |
| Output/approval for unknown call | AI SDK errors; assistant-ui has output-only continuation accommodations | strict: require context; do not invent tool names |
| Preliminary then final | AI SDK may accept a later preliminary overwrite; native ignores preliminary after final | choose an oracle; "final wins" is a recommendation |
| Error mid-stream | v4/useChat stop; readUI helper continues; native warning/info still sets error status | keep prefix plus structured error; separate continuation policy |
| Abort | UI event with optional reason; native has no abort chunk | track as run outcome; preserve content |
| Missing `[DONE]` | AI SDK parser does not require it; native strict errors | track finish, DONE, EOF and transport failure separately |
| Events after `[DONE]` | AI SDK ignores the marker and continues; native terminates | match the profile |
| CRLF / bare CR | SSE allows LF/CRLF/CR; v4 handles CRLF in JSON whitespace but a blank CRLF line can fail | real SSE framer; legacy line framing separate |
| Multiline `data:` | joined with `\n` before JSON parsing | parse the assembled event |
| Comments, BOM, `event`, `id`, `retry` | handled by the SSE layer; SSE id != message id | keep framing out of folding |
| Missing final blank SSE line | unfinished event not dispatched | never treat a partial event as complete |
| Missing final legacy newline | AI SDK v4 flushes at EOF; assistant-ui legacy rejects | expose compatibility profiles |
| Unknown type/code | AI SDK rejects non-data types; native warns/drops; legacy strict errors | optional forward-compatible mode keeping opaque events |
| Extra fields on known chunks | current AI SDK accepts; older may reject | preserve unknown fields; validate known ones |
| Very large event/argument/message | native defaults 16 Mi UTF-16 code units; AI SDK layers have no limit | independent configurable bounds (event, raw args, part count, total) |
| Reads split UTF-8 or CRLF | decoders must keep partial state | test arbitrary byte fragmentation |
| Malformed later line in a v4 read | v4 parses the whole buffered read first; a bad later line can suppress earlier lines | process incrementally; keep read segmentation in exact-reference tests |
| State update paths | string paths; invalid ops fail or skip by mode | validate indices/bounds before mutating |

Real failures reproduced upstream: merged-stream step boundaries losing active parts (vercel/ai PR #19324); resume
after disconnect (vercel/ai issue #13160, fixed in `ai@7.0.120` by retaining active state).

## D. Test vectors

`crates/ai-ui-stream/tests/fixtures/protocol-vectors.json` (`format_version` 1, as of 2026-11-09, 96 vectors).
Each has `id`, `label` (`ESTABLISHED`/`HYPOTHESIS`), `oracle_profile`, `protocol`, `package`, `headers`, `raw_utf8`
(exact wire bytes after JSON decoding), optional `network_chunks_utf8` / `network_chunks_base64` (one array item = one
network read; the base64 form splits inside code points), `expected` (`foldedMessage` + `sidecars`), `verification`,
`source_urls`. If neither chunk field is present, deliver `raw_utf8` as one read. `null` for `foldedMessage` means no
snapshot was yielded before failure; `null` in v4 usage stands for upstream `NaN`.

| Profile | Cases | Folded message shape |
|---|---:|---|
| `ai-sdk-ui7` | 45 | AI SDK `UIMessage` (`parts`, `state`, `rawInput`, ...) |
| `ai-sdk-v4` | 20 | v4 projection plus data/finish/usage/error sidecars |
| `assistant-stream-native-and-legacy` | 17 | assistant-stream `AssistantMessage` (no `content` alias, no timings) |
| `assistant-stream-ui-adapter` | 8 | assistant-stream `AssistantMessage` via `UIMessageStreamDecoder` (differential cases) |
| `rust-preserving-recovery-v1` | 6 | `HYPOTHESIS`: proposed recovery policies |

## E. Not verified by the auditor

1. Conformance of any Rust implementation (none was supplied). 2. A universal request contract for arbitrary backends.
3. Application-defined `data-*`/metadata/tool schemas. 4. The first patch release introducing each optional field.
5. Provider acceptance of historical request representations (e.g. `media` tool content). 6. PyPI package parity and
non-JavaScript implementations. 7. Replay/idempotency guarantees of arbitrary backends and production frequency.

## Conformance status

`crates/ai-ui-stream/tests/protocol_vectors.rs` runs the 25 vectors whose oracle is assistant-stream's own
`AssistantMessage` (the shape our model mirrors), comparing the adapter's last result with the oracle: every field the
oracle has must match (we may carry more), and a run fails exactly when the oracle reports an error. Lifecycle `state`
keys and per-part `status` are compared through the message status instead (representation only).

As of 2026-11-09: **22 of 23 run vectors match; 2 skipped** (AT10 sets a size limit in UTF-16 units, ours are bytes;
AU05 uses lenient mode, we have one strict policy). Known difference: **AU06**, a final line without a newline is
accepted, as AI SDK v4 does (assistant-ui rejects it).

Fixed to reach this (each was found by a vector):
- Steps keep `finishReason` and `isContinued` (AT01).
- Malformed and unknown assistant-transport chunks are dropped with a warning instead of failing the run (AT05).
- Chunks decoded before a framing error in the same read are delivered before the error (AU04, all decoders).
- A clean end of stream settles a running message as upstream's accumulator does: `requires-action/tool-calls` when a
  tool call is still open (no result, or only a preliminary one), otherwise `complete/unknown` (AT03, AU01, AU03, AU07,
  AUI_UI08).
- Legacy tool results keep `artifact` and `modelContent`; arguments that never streamed close as `{}` (AU07).
- `interrupted` finish reason (AI SDK 7) is accepted.

### AI SDK vectors (`tests/ai_sdk_vectors.rs`)

The 45 `ai-sdk-ui7` and 20 `ai-sdk-v4` vectors have `UIMessage` / v4 oracles with parts we do not model (steps, custom
content, steering messages, reasoning files, per-part state, message metadata). Each side is therefore reduced to what
both express: text, reasoning, tool calls with their result or error flag, sources, files, data parts, and whether the
run failed. As of 2026-11-09: **52 of 65 match; 13 differ, each recorded with its reason** in `KNOWN_DIFFERENCES`.

Fixed to reach this (found by the vectors, all in the UI-stream and data-stream decoders):
- `data-*` parts with an `id` replace the earlier part with the same name and id (U18, U44), instead of appending
  duplicates: a "searching..." status that later becomes "done" now shows once. Entries keep their `id`.
- `reset-step` discards the parts the current step added (U22, U23), keeping earlier steps and calls addressable;
  previously the discarded and the replacement text were both shown.
- `finish-step` no longer closes open text and reasoning parts (U21); only their own `-end` chunks or the end of the
  message do, as in AI SDK 7.
- A v4 `e` (finish step) without `isContinued` ends the open text, so the next step starts a new part (L13).

Differences we keep, on purpose (the 13):
- **Lenient decoding.** The AI SDK raises on lifecycle and schema violations; we keep the content and warn: delta before
  start (U08, U09), output without a call (U10), repeated end (U32), unknown chunk type (U33), invalid JSON event (U36),
  unknown approval id (U39), finish reason `unknown` (U43), blank CRLF line in v4 (L16). A violation never fails a run
  whose content arrived.
- **Ordering and replay.** A late preliminary result after the final one is ignored (U11). The stream ends at `[DONE]`
  (U38). A tool-input delta with no start, as after a resume, is dropped (U41); resuming needs a replayed prefix. A v4
  error line does not stop later lines (L09).

Two assistant-stream-oracle vectors are now recorded as deliberate improvements over upstream assistant-ui, which is
lossy there (the audit recommends exactly this): AUI_UI02 (data ids) and AUI_UI03 (`reset-step`).

Still not represented (B2): `reasoning-file`, `custom`, `steering-message`, citations, message metadata, tool
`inputEncoding`/`title`/`dynamic`, abort reason. Not yet run: the 6 `HYPOTHESIS` recovery policies.

### Two implementations of the UI message stream

`crates/ai-ui-sdk-stream` (see [AI-SDK-STREAM.md](AI-SDK-STREAM.md)) folds the same wire exactly as the AI SDK 7
reducer does. Its 45 fixtures (`tests/fixtures/ui7.json`) are byte-identical to this audit's `U01` to `U45`, and it matches
all of them against the published package, including the strict lifecycle errors. `ai-ui-stream` folds the wire
into assistant-ui's message model instead and is lenient on purpose, so it differs on 11 of those 45 (the policy list
above: `U08`-`U11`, `U32`, `U33`, `U36`, `U38`, `U39`, `U41`, `U43`). A backend that sends a valid stream gets the same content from both; they
differ only on malformed or unusual streams.

### Tool safety (audit vector `H04`)

A response that ends inside a tool call's arguments never runs the tool, whichever decoder produced it
(`tests/tool_safety.rs`). The check lives in the local runtime, not the decoders: a call whose streamed argument text is
not complete JSON gets an error result (`The tool call's arguments were cut off, so the tool was not run.`) and is neither
executed nor offered for approval, so it also protects custom adapters. Found by that test: the UI message stream and data
stream decoders closed cut-off arguments at end of stream, and the runtime then ran the tool on the repaired partial value
(the native transport was already safe). Complete arguments still run (the control case).

