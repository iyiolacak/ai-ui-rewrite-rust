# Troubleshooting

Source: the Troubleshooting section of [Getting started](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/docs/guide/01-getting-started.md) and the [FAQ](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/docs/guide/10-faq.md).

**`fontconfig` not found when building a Slint crate.** Install the development package: `libfontconfig1-dev` on Debian and Ubuntu, `fontconfig-devel` on Fedora. For a non-standard prefix, `export PKG_CONFIG_PATH=/path/to/prefix/lib/pkgconfig`. The runtime and state crates (`ai-ui-core`, `ai-ui-state`, `ai-ui-stream`, `ai-ui-data-stream`, `ai-ui-http`) build without it.

**"requires rustc" or an old toolchain.** The Slint crates need Rust 1.92, `ai-ui-markdown` 1.88, `ai-ui-clipboard` 1.86, the rest 1.85. Run `rustup update`. Do not raise or lower crate versions to work around it.

**`wasm-bindgen` version mismatch.** `scripts/build-web-demo.sh` stops with "wasm-bindgen is X but Cargo.lock needs Y". Install the exact CLI version, or set `WASM_BINDGEN=/path/to/wasm-bindgen`. Add the target with `rustup target add wasm32-unknown-unknown`.

**A streaming Slint chat uses a lot of CPU or drops frames.** Slint 1.18 depends on a femtovg release that recompiles GPU pipelines too often for a streaming chat. Add to the workspace root `Cargo.toml`:

```toml
[patch.crates-io]
femtovg = { git = "https://github.com/iyiolacak/ai-ui-rewrite-rust" }
```

See [patches/README.md](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/patches/README.md).

**Slint version conflict.** The Slint crates pin `=1.18.1`. The app's `slint` and `slint-build` must be `=1.18.1`, with `compat-1-18` on `slint`.

**Nothing happens after `append` or `send`.** The returned `RunFuture` was not spawned. Spawn it on the project's executor ([runtime.md](runtime.md)).

**It will not compile when moving the runtime to another thread, or `tokio::spawn` complains about `Send`.** The runtime is `Rc`-based and single-threaded by design. Keep it on one thread with `slint::spawn_local`, a Tokio `LocalSet`, or `wasm-bindgen-futures`. Do not add `Arc`/`Mutex`.

**The reply arrives all at once.** A proxy is buffering the response, or the backend does not stream. Check the response headers and that no intermediary buffers.

**Text does not reveal smoothly, or `needs_frame` is never true.** A hand-written adapter must mark streaming text parts `PartStatus::Running`, and the UI must call `view.tick(now_ms)` once per frame while `view.needs_frame()` is true.

**`Status 503` (or another status) on the message.** The backend returned a non-2xx status. The response body is not read into the message. Check the server.

**The backend rejects `messages`.** AI SDK 7 rejects system-role messages inside `messages`. Send the system prompt as the top-level `system` field. Do not call `convertToModelMessages` on the posted `messages`.

**Wrong decoder for the response.** The adapter picks by response header. Without `x-vercel-ai-ui-message-stream: v1` it assumes the UI message stream. If you serve the legacy data stream without its `x-vercel-ai-data-stream: v1` header, set `DataStreamOptions::protocol`.

**Assistant-transport backend.** `DataStreamAdapter` does not select it. Wrap the response bytes with `run_stream_from_sse` in your own adapter ([backend.md](backend.md)). The server-state runtime is not ported.

**Stale or wrong rows in a custom UI after switching threads.** Reset per-conversation caches and, in Slint, change `conversation-revision`. A `ThreadView` reads changes with its own cursor; after `rebind` it resets.

**Unsure about a feature.** Check the lists of what is not ported in the README and [STATUS](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/docs/STATUS.md): assistant-transport as a runtime, attachments, speech, custom per-tool components, and more.

**Can I use another UI toolkit?** Yes. `ai-ui-core` and `ai-ui-state` have no toolkit dependency. `ThreadView` projects any runtime into plain data and actions.

**Is it on crates.io?** Yes, all ten library crates at 0.1. The example app is not published.
