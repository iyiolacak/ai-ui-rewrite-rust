# ai-ui-core

Rust port of `@assistant-ui/core`: the thread message model (upstream JSON shape, unknown fields round-trip), the branching `MessageRepository`, the `ChatModelAdapter` contract, the `ThreadRuntime` trait with `LocalThreadRuntime` and `ExternalStoreRuntime`, `ThreadList`, frontend tools with approvals, composers and `SmoothText`.

```rust
use ai_ui_core::{adapter, AppendMessage, ChatModelRunResult, LocalRuntimeOptions, LocalThreadRuntime, ThreadRuntime};

let runtime = LocalThreadRuntime::new(
    |_options| adapter::from_results([Ok(ChatModelRunResult::text("Hello!"))]),
    LocalRuntimeOptions::default(),
);
let run = runtime.append(AppendMessage::user_text(None, "Hi"))?;
// Spawn `run` on any executor: slint::spawn_local, a Tokio LocalSet, wasm-bindgen-futures...
# Ok::<(), ai_ui_core::RuntimeError>(())
```

No I/O, UI, clock or randomness dependencies; builds for `wasm32-unknown-unknown`. Runtimes are `Rc`-based (single-threaded, like a UI thread).

Part of [assistant-ui for Rust](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/README.md), a port of [assistant-ui](https://github.com/assistant-ui/assistant-ui) (MIT). Not affiliated with the assistant-ui project. Status and gaps: [docs/STATUS.md](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/docs/STATUS.md).
