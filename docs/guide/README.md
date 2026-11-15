---
title: Guide
description: A complete guide to ai-ui, the Rust port of assistant-ui for native and WebAssembly apps.
order: 0
section: Start
---

# ai-ui guide

This guide is written to be published: every page is plain Markdown with front matter, so the landing site
(repository `ai-ui-site`) can render it without changes. The reference documents in `docs/` (ARCHITECTURE,
PROTOCOL, AI-SDK-STREAM, PERFORMANCE, STATUS, RESULTS, WEB) stay the detailed source; the guide explains,
links to them and never contradicts them.

| Page | Section | What it covers |
|---|---|---|
| [Why ai-ui](00-why-ai-ui.md) | Start | The problem, what you get, a like-for-like results table, who it is for, and the limits |
| [Getting started](01-getting-started.md) | Start | Install, a first runtime in 20 lines, a first window, a first web build |
| [Concepts](02-concepts.md) | Start | Messages, branches, runtimes, thread lists, tools and approvals, smooth streaming, state |
| [Streaming and backends](03-streaming-and-backends.md) | Build | Wire protocols, the AI SDK and assistant-transport decoders, the HTTP adapter, connecting a real backend |
| [Building a UI with Slint](04-ui-slint.md) | Build | The thread UI, Markdown, settings, theming, keyboard and focus, scaling |
| [Running on the web](05-web.md) | Build | WebAssembly build, what works in the browser, limits, embedding the demo |
| [Crate reference](06-crates.md) | Reference | Every crate: purpose, key types, features, wasm32 support, a runnable example |
| [Performance](07-performance.md) | Reference | Why it is fast, how it is measured, how to reproduce every number |
| [Coming from assistant-ui](08-from-assistant-ui.md) | Reference | Parity table, differences, migration notes |
| [Contributing and testing](09-contributing.md) | Reference | Repo layout, the gate, benchmarks, A/B method, conventions |
| [FAQ](10-faq.md) | Reference | Short answers to the questions people ask first |

## Conventions for every page

- **Front matter:** `title`, `description` (one sentence), `order` (the number in the file name), `section`
  (Start, Build or Reference).
- **Numbers:** every performance figure is copied from `docs/RESULTS.md` or `docs/results.json` with its
  conditions (workload, median of how many pairs, date, what each side measures). Never invent, round up or
  mix a Rust "first frame" time with a React "fully rendered" time. If a figure is not in RESULTS.md, do not
  write it.
- **Claims:** compare only with upstream assistant-ui. Say nothing about other products. State limits.
- **Code:** snippets must match real APIs in this repository (check the source, not memory). A snippet that is
  not compiled by tests is fenced as `rust,ignore`. Commands must be ones that work from a clean checkout.
- **Links:** relative links between pages and to `../` reference documents; every link must resolve
  (`scripts/check-guide.py` checks front matter and links).
- **Voice:** direct and plain. Short paragraphs. No hype words, no marketing adjectives, no emoji.
- **Tone, banned words:** do not use blazingly, revolutionary, game-changing, powerful, seamless, effortless,
  next-generation, cutting-edge, best-in-class, lightning, supercharge, unleash, magic, rock-solid, incredible,
  exclamation marks, emoji, or a rhetorical question as a headline.
- **Tone, claims:** a claim needs a number and its condition, or a file, command or type that shows it. If neither
  exists, remove the claim.
- **Tone, limits:** state limits before a reader asks. A benefit that has a caveat carries the caveat in the same
  paragraph or table.
- **Tone, comparisons:** compare only with upstream assistant-ui, and only with figures from `docs/RESULTS.md`.
