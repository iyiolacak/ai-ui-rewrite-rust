//! The README quickstart: one local thread with a canned model reply.
use ai_ui_core::{
    adapter, AppendMessage, ChatModelRunResult, LocalRuntimeOptions, LocalThreadRuntime,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let runtime = LocalThreadRuntime::new(
        |_options| adapter::from_results([Ok(ChatModelRunResult::text("Hello!"))]),
        LocalRuntimeOptions::default(),
    );
    let run = runtime.append(AppendMessage::user_text(None, "Hi"))?;
    // Any executor works: slint::spawn_local, a Tokio LocalSet, wasm-bindgen-futures...
    let _ = futures_executor::block_on(run);
    Ok(())
}
