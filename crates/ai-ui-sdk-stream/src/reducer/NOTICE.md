# AI SDK source adaptation

The partial JSON repair state machine in `partial_json.rs` is adapted from
Vercel AI SDK **7.0.166**, `packages/ai/src/util/fix-json.ts`:

https://github.com/vercel/ai/blob/ai%407.0.166/packages/ai/src/util/fix-json.ts

Copyright 2023 Vercel, Inc.

Licensed under the Apache License, Version 2.0. The full license is reproduced
in `LICENSE-APACHE.txt`. The Rust adaptation uses typed states and byte-safe
Unicode traversal. The repair is performed on demand; it is not represented
as an incremental JSON parser.

The event schema and reducer behavior are implemented against these primary
sources at the same version:

- `packages/ai/src/ui-message-stream/ui-message-chunks.ts`
- `packages/ai/src/ui/process-ui-message-stream.ts`
- `packages/ai/src/ui-message-stream/read-ui-message-stream.ts`
- `packages/provider-utils/src/parse-json-event-stream.ts`
- `packages/provider-utils/src/secure-json-parse.ts` (published
  `@ai-sdk/provider-utils@5.0.76`): prototype-key rejection in wire JSON and
  partial JSON previews. The Rust implementation validates decoded values
  iteratively rather than applying JavaScript source-text regular expressions.

JSON values use Rust Unicode scalar strings and `serde_json` number/depth rules.
They do not reproduce JavaScript lone-surrogate strings, UTF-16 string property
enumeration, numeric overflow, or arbitrary parser-depth behavior. In particular,
non-BMP string metadata spread and a partial escaped surrogate can differ from
JavaScript materializations. Raw streamed tool input is retained independently.

These references identify compatibility behavior, not a claim that Vercel
maintains, approves, or supports this implementation.
