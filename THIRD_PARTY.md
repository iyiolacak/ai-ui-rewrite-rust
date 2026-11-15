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


## Benchmark harnesses (`benchmarks/`)

- `benchmarks/upstream-look/src/` holds the UI of assistant-ui's `templates/minimal`, copied unchanged (MIT,
  © AgentbaseAI Inc., see [NOTICE](NOTICE)). The React harnesses install upstream's published packages from npm;
  nothing from `node_modules` is committed.

## Landing page (`site/`)

The page is not part of the crates; its material and licences are separate from the MIT code.

- **Work Sans** (SIL Open Font License 1.1, © The Work Sans Project Authors): `site/public/fonts/WorkSans-*.woff2`,
  Google Fonts' variable-weight files for the Latin and Latin Extended subsets, self-hosted so that a visit sends no
  request to a third party; licence in `site/public/fonts/WorkSans-OFL.txt`.
- **Rust logo** (CC BY 4.0, from [rust-artwork](https://github.com/rust-lang/rust-artwork)):
  `site/public/rust-logo-white-outline.svg`, byte for byte the official `logo/rust-logo-white-outline.svg`. Rust and
  the Rust logo are trademarks of the Rust Foundation; the page credits the licence and says the project is not
  affiliated with or endorsed by the Rust Foundation or the Rust Project.
- **Made with Slint badge**: `site/public/made-with-slint.svg`, Slint's official `logo/MadeWithSlint-logo-light.svg`.
  The page's WebAssembly demo uses Slint under the Slint Royalty-Free License 2.0, whose attribution term this badge
  on the page meets.
- **GSAP** (Webflow's GSAP Standard "no charge" licence, <https://gsap.com/standard-license>; proprietary, not
  open source and not covered by this project's MIT licence): installed from npm for the page's animations. The build
  keeps GSAP's licence banner in the minified bundle (`site/astro.config.mjs`), as the licence requires.
- **Astro, Tailwind CSS** and the other build tools are MIT and are not redistributed beyond the built page.
