# React baseline

Upstream assistant-ui, the published packages of our reference commit
(`@assistant-ui/react` 0.15.33, `@assistant-ui/react-markdown` 0.14.25), on the
same workload as `ai-ui-example-thread --bench N --bench-reply-kb K`:

- a history of N messages (alternating a short question and a Markdown reply
  with a list, code block and table);
- then the demo model's reply to a K KB prompt: reasoning, then the answer,
  one word every 18 ms, each step a cumulative snapshot (both sides quote the
  prompt, so the reply is roughly 2K KB);
- measured over the same fixed 60 s window.

The React thread follows upstream's default `thread.aui.tsx` structure
(viewport, messages, parts with reasoning, GFM Markdown with `defer`, action
bar, branch picker, composer) with plain CSS instead of Tailwind and Radix, so
it does less work than upstream's full default thread.
It does **not** virtualise the message list (upstream's default does not either; upstream has a separate virtualised
example), so every message is in the DOM. A virtualised variant has not been measured yet.

## Run

```sh
npm ci --ignore-scripts && npm run build
export PLAYWRIGHT=/path/to/node_modules/playwright          # any Playwright package
export CHROME=/path/to/chrome-headless-shell                 # if Playwright's browser is not installed
node bench.mjs 1000 9 60                                     # one React run, JSON on stdout
./compare.sh 1000 9 5                                        # five interleaved Rust/React pairs
```

## What is compared

| | Rust (`--bench-render`) | React (`bench.mjs`) |
|---|---|---|
| Busy time while streaming | app work + layout + software paint, one thread | Chromium main thread (`TaskDuration`); raster and compositing run on other threads and are not counted |
| Opening the thread | `open (formatted and painted, like for like)`: import and first sync, then Markdown formatted off the UI thread, then a forced frame (the first-sync part alone is printed too and is not comparable) | `open_formatted_ms`: navigation until all N messages are in the DOM with their Markdown rendered, plus two frames (`open_ms` is the DOM-only figure) |
| Memory | resident memory of the one process | RSS and PSS summed over every Chromium process of the run |

Both differences in method favour React. Results are recorded in
[`docs/STATUS.md`](../../docs/STATUS.md#performance).
