# Third-party material

- **assistant-ui** (MIT, © AgentbaseAI Inc.): this project is a port; see [NOTICE](NOTICE) and
  `crates/ai-ui-slint/ui/assets/assistant-ui-MIT-LICENSE.txt`.
- **Inter** (SIL Open Font License 1.1, © The Inter Project Authors): `crates/ai-ui-slint/ui/assets/Inter-*.ttf`
  (Regular, Medium, SemiBold), subset to Latin, Latin-1, Latin Extended-A and common punctuation with fontTools;
  licence in `Inter-OFL.txt`. Upstream's default thread uses Inter.
- **IBM Plex Mono** (SIL Open Font License 1.1, © IBM Corp., Reserved Font Name "Plex"): unmodified
  `crates/ai-ui-slint/ui/assets/IBMPlexMono-Regular.ttf`, used for code (upstream's code font); licence in
  `IBMPlexMono-OFL.txt`. Not subset, because a modified copy may not keep the reserved name.
- **Lucide icons** (ISC): the SVG icons in `crates/ai-ui-slint/ui/assets/`, licence in `Lucide-LICENSE.txt`.
- **Slint** (GPL-3.0, Slint Royalty-Free Desktop/Mobile/Web/Embedded licence, or commercial): required by
  `ai-ui-slint`, `ai-ui-markdown-slint` and `examples/thread` only. This project's own code is MIT;
  an application using the Slint crates must comply with the Slint licence it chooses (the royalty-free licence
  requires an attribution badge). The runtime crates (`core`, `stream`, `data-stream`, `http`, `state`,
  `markdown`, `clipboard`) do not depend on Slint.
- **Vercel AI SDK** (Apache-2.0): `crates/ai-ui-sdk-stream/src/reducer/partial_json.rs` is adapted from
  `packages/ai/src/util/fix-json.ts` (AI SDK 7.0.166, Copyright 2023 Vercel, Inc.); the licence and the adaptation
  notice are beside it (`LICENSE-APACHE.txt`, `NOTICE.md`). The rest of that crate implements the AI SDK's documented
  stream behaviour from its public sources. Not affiliated with or endorsed by Vercel.

