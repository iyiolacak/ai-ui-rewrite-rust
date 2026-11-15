# Landing page

Astro (TypeScript, static output, no UI framework) + Tailwind CSS 4 + `src/styles/site.css`. White only.
Charts are SVG strings drawn by `src/lib/bench.ts` from `data/benchmarks.json`, the only source of numbers.
The demo is the real `examples/thread` app compiled to WebAssembly; `src/scripts/demo.ts` loads it on click.

```sh
npm ci                 # needs network once; no lockfile is committed yet (the first online install creates it)
npm run wasm           # cargo + wasm-bindgen 0.2.129 -> public/wasm (git-ignored); set WASM_BINDGEN if not on PATH
npm run dev            # or: npm run build && npm run preview
npm run test:chart     # node --experimental-strip-types, no install needed
PLAYWRIGHT=/path/to/node_modules/playwright npm run test:numbers   # every number on dist/ equals the JSON
PLAYWRIGHT=... node scripts/measure-web.mjs 3 10000 > web.json; python3 scripts/fill-web.py web.json
```

## Cloudflare Pages (configuration only, nothing is deployed from here)

| Setting | Value |
|---|---|
| Root directory | `site` |
| Build command | `npm ci && npm run build` |
| Build output directory | `dist` |

The Pages build image has no Rust toolchain: build `public/wasm` first (`npm run wasm`) and run
`npx wrangler pages deploy dist` from CI, or add the wasm step to a CI job that uploads `dist`.
`wrangler.jsonc` and `public/_headers` hold the project name, output dir and cache headers.
