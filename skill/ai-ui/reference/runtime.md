# Runtime: first thread, streaming, reading state

Source: [Getting started](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/docs/guide/01-getting-started.md), [Concepts](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/docs/guide/02-concepts.md) and `crates/ai-ui-core/examples/quickstart.rs`. The snippets are built by the repository's tests or examples.

## Dependencies

```toml
[dependencies]
ai-ui-core = "0.1"
ai-ui-state = "0.1"
# Only if you talk to an AI SDK backend:
ai-ui-data-stream = "0.1"
ai-ui-http = "0.1"
# The examples on this page drive futures with a small executor:
futures-executor = "0.3"
```

`futures-executor` is for these examples and tests only. In an app, use the project's own executor (below).

## A first runtime

A runtime holds one thread. Its model is a `ChatModelAdapter`: any closure from the run options to a stream of results works. This one answers every request with canned text.

```rust
use ai_ui_core::{
    adapter, AppendMessage, ChatModelRunResult, LocalRuntimeOptions, LocalThreadRuntime,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The adapter is the model. This one answers every step with a canned text.
    let runtime = LocalThreadRuntime::new(
        |_options| adapter::from_results([Ok(ChatModelRunResult::text("Hello!"))]),
        LocalRuntimeOptions::default(),
    );

    // `append` changes the thread now: the user message and an empty, running
    // assistant message exist before the run has been polled.
    let run = runtime.append(AppendMessage::user_text(None, "Hi"))?;
    println!("before the run: {} messages", runtime.messages().len());

    // The returned future does the model work. Any executor can drive it.
    futures_executor::block_on(run)?;

    for message in runtime.messages().iter() {
        println!("{:?}: {}", message.role(), message.text());
    }
    Ok(())
}
```

Output:

```text
before the run: 2 messages
User: Hi
Assistant: Hello!
```

Two points. `append` does its synchronous work immediately, so a UI can show the user message and the running assistant message in the same frame. The future it returns is the model run, and nothing happens until you spawn it. The same code is built as `cargo run --example quickstart -p ai-ui-core`.

## Stream a reply and read state through ThreadView

Each item an adapter yields is the whole reply so far, not a delta. A UI holds a `ThreadView` from `ai-ui-state`, reads `view.state()` when told something changed, and calls the view's actions (`send`, `stop`, `reload`, `switch_branch`, ...).

```rust
use ai_ui_core::{
    adapter, AssistantMessagePart, ChatModelRunResult, LocalRuntimeOptions, LocalThreadRuntime,
    PartStatus, RunError, TextPart,
};
use ai_ui_state::{PartKind, ThreadView, ViewOptions};
use std::cell::Cell;
use std::rc::Rc;

/// One cumulative update: the whole reply so far, and whether the part is still running.
fn step(text: &str, status: PartStatus) -> Result<ChatModelRunResult, RunError> {
    Ok(ChatModelRunResult::content(vec![
        AssistantMessagePart::Text(TextPart {
            status: Some(status),
            ..TextPart::new(text)
        }),
    ]))
}

fn reply(view: &ThreadView) -> String {
    match &view.state().messages[1].parts[0].kind {
        PartKind::Text { text, .. } => format!("{text:?}"),
        _ => String::new(),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let runtime = LocalThreadRuntime::new(
        |_options| {
            adapter::from_results([
                step("Rust", PartStatus::Running),
                step("Rust is a", PartStatus::Running),
                step("Rust is a systems language.", PartStatus::Complete),
            ])
        },
        LocalRuntimeOptions::default(),
    );
    let view = ThreadView::new(Rc::new(runtime), ViewOptions::default());

    // A UI reads `state()` whenever it is told something changed. That is also
    // how the view notices a part that is still streaming and starts to pace it.
    let changes = Rc::new(Cell::new(0u32));
    let _subscription = view.subscribe({
        let (view, changes) = (view.clone(), changes.clone());
        move || {
            changes.set(changes.get() + 1);
            let _ = view.state();
        }
    });

    let run = view
        .send("What is Rust?")?
        .expect("the composer was not empty");
    futures_executor::block_on(run)?;
    println!("run finished, shown so far: {}", reply(&view));

    // Once per frame the UI hands the view a clock reading.
    let mut now_ms = 0.0;
    while view.needs_frame() {
        now_ms += 16.0;
        view.tick(now_ms);
        if now_ms == 64.0 {
            println!("after 64 ms: {}", reply(&view));
        }
    }
    println!("after {now_ms} ms: {}", reply(&view));
    println!(
        "running: {}, notifications: {}",
        view.state().is_running,
        changes.get()
    );
    Ok(())
}
```

The view reveals text at a steady pace, like upstream's `useSmooth`. A frame loop calls `view.tick(now_ms)` while `view.needs_frame()` is true. In a real app, the subscription callback asks the toolkit for a redraw instead of reading state itself. The clock is yours: the core has no ambient clock.

The `PartStatus::Running` marker is how the view knows a part is streaming. The wire decoders set it; a hand-written adapter must too.

## Executor patterns

The runtime never spawns. Spawn the returned `RunFuture` on the project's executor and ignore or log the result: failures are recorded on the message.

**Slint** (from `examples/thread/src/app.rs`):

```rust,ignore
fn spawn(run: RunFuture) {
    // The message records failures; nothing else to report here.
    let _ = slint::spawn_local(async move {
        let _ = run.await;
    });
}
```

**Tokio.** Run the runtime and its futures on one thread, inside a `tokio::task::LocalSet`, and spawn the run future with `spawn_local` on that set. Because the runtime is `Rc`-based, do not use `tokio::spawn` (it requires `Send`). The repository has no Tokio example; this follows from the `Rc` design in [Concepts](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/docs/guide/02-concepts.md#the-threadruntime-trait). Verify it compiles in the user's project.

**WebAssembly.** Spawn with `wasm-bindgen-futures`. The repository's own browser demo uses `slint::spawn_local` (`examples/thread/src/web.rs`).

**Tests and scripts.** `futures_executor::block_on(run)`.

## Thread lists

`ThreadList::new(factory, spawner, adapter)` manages many threads and needs a `Spawner` (`Rc<dyn Fn(Pin<Box<dyn Future<Output = ()>>>)>`) for background work. The example app's spawner calls `slint::spawn_local(task)`. A runnable version with `futures_executor::block_on` as the spawner is in the Concepts guide, section "The ThreadRuntime trait". `ThreadView::for_list(&list, options)` follows the main thread.

## Rules for state code

- Read `view.state()` for rendering; do not read the runtime directly from a UI.
- A row that did not change keeps the same `Arc<MessageState>`. Compare pointers to find changed rows.
- When the selected conversation changes, reset any per-conversation UI caches. After rebinding a view to a new runtime (`rebind`), the view resets its cursors.
