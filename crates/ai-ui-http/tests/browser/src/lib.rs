//! Drives the browser `fetch` client through the real adapter and runtime.

use ai_ui_core::{
    AppendMessage, AssistantMessagePart, LocalRuntimeOptions, LocalThreadRuntime, MessageStatus,
    Timestamp,
};
use ai_ui_data_stream::{DataStreamAdapter, DataStreamOptions};
use ai_ui_http::FetchClient;
use std::cell::Cell;
use std::rc::Rc;
use wasm_bindgen::prelude::*;

fn runtime(api: String) -> LocalThreadRuntime {
    LocalThreadRuntime::new(
        DataStreamAdapter::new(FetchClient::default(), DataStreamOptions::new(api)),
        LocalRuntimeOptions::with_clock(Rc::new(|| Timestamp(js_sys::Date::now() as u64))),
    )
}

/// The assistant's text and a status tag: `text|complete`, `text|incomplete`...
fn summary(runtime: &LocalThreadRuntime) -> String {
    let messages = runtime.messages();
    let Some(answer) = messages.get(1).and_then(|m| m.as_assistant()) else {
        return "|no-answer".into();
    };
    let text: String = answer
        .content
        .iter()
        .filter_map(|part| match part {
            AssistantMessagePart::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect();
    let status = match &answer.status {
        MessageStatus::Complete { .. } => "complete",
        MessageStatus::Incomplete { .. } => "incomplete",
        MessageStatus::Running => "running",
        MessageStatus::RequiresAction { .. } => "requires-action",
    };
    format!("{text}|{status}")
}

/// Send "Hi" to `api` and wait for the run; returns [`summary`] and, when the run
/// failed, `|error:<message>` after it.
#[wasm_bindgen]
pub async fn chat(api: String) -> String {
    let runtime = runtime(api);
    let run = runtime
        .append(AppendMessage::user_text(None, "Hi"))
        .expect("append");
    let outcome = run.await;
    let mut summary = summary(&runtime);
    if let Err(error) = outcome {
        summary.push_str(&format!("|error:{error}"));
    }
    summary
}

async fn sleep(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .unwrap();
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

/// Start a run, cancel it after `ms` milliseconds, and report [`summary`] plus
/// whether the run future resolved: `text|status|done` or `text|status|pending`.
#[wasm_bindgen]
pub async fn chat_and_cancel(api: String, ms: i32) -> String {
    let runtime = runtime(api);
    let done = Rc::new(Cell::new(false));
    let run = runtime
        .append(AppendMessage::user_text(None, "Hi"))
        .expect("append");
    let flag = done.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let _ = run.await;
        flag.set(true);
    });
    sleep(ms).await;
    runtime.cancel_run();
    sleep(300).await;
    format!(
        "{}|{}",
        summary(&runtime),
        if done.get() { "done" } else { "pending" }
    )
}
