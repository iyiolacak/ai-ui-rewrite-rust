# Upstream look

Upstream assistant-ui's default thread for side-by-side UX review: the UI of
`templates/minimal` (components, `globals.css`; MIT, © AgentbaseAI Inc., see
the repository NOTICE) copied unchanged into `src/`, on the published packages
of our reference commit (`@assistant-ui/react` 0.15.33,
`@assistant-ui/react-markdown` 0.14.25), with Inter and IBM Plex Mono from
`@fontsource` instead of `next/font`, and the Rust example's demo model.

```sh
npm ci --ignore-scripts && npm run build
PLAYWRIGHT=/path/to/node_modules/playwright CHROME=/path/to/chrome-headless-shell node shots.mjs
# shots/empty.png, shots/conversation.png, shots/stream.png (900x760, dark)
```

The minimal template renders no reasoning and has copy, refresh and a "more"
menu in its action bar; the fuller `packages/ui` thread, which our port follows,
also shows reasoning, tool groups and feedback. Markdown styling (`markdown-text`)
is shared by both.
