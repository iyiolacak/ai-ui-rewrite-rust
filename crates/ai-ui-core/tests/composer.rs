//! Thread and edit composers, after upstream's composer runtime behaviour.

use ai_ui_core::adapter::{self, RunStream};
use ai_ui_core::*;
use std::cell::Cell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

fn runtime_answering(answers: &'static [&'static str]) -> LocalThreadRuntime {
    let next = Cell::new(0);
    let tick = Cell::new(0);
    LocalThreadRuntime::new(
        move |_options: ChatModelRunOptions| -> RunStream {
            let answer = answers[next.get() % answers.len()];
            next.set(next.get() + 1);
            adapter::from_results([Ok(ChatModelRunResult::text(answer))])
        },
        LocalRuntimeOptions::with_clock(Rc::new(move || {
            tick.set(tick.get() + 1);
            Timestamp(tick.get())
        })),
    )
}

fn finish(run: Option<RunFuture>) {
    let mut run = run.expect("a run was started");
    match Pin::new(&mut run).poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(result) => result.unwrap(),
        Poll::Pending => panic!("scripted runs finish in one poll"),
    }
}

fn texts(runtime: &LocalThreadRuntime) -> Vec<String> {
    runtime.messages().iter().map(|m| m.text()).collect()
}

#[test]
fn sending_appends_after_the_last_message_and_clears_the_draft() {
    let runtime = runtime_answering(&["one", "two"]);
    let composer = runtime.composer();
    assert!(composer.is_empty() && !composer.can_send());

    composer.set_text("first");
    assert!(composer.can_send());
    finish(composer.send().unwrap());
    assert_eq!(composer.text(), "");
    composer.set_text("second");
    finish(composer.send().unwrap());
    assert_eq!(texts(&runtime), ["first", "one", "second", "two"]);
}

#[test]
fn whitespace_alone_is_not_sendable() {
    let runtime = runtime_answering(&["x"]);
    let composer = runtime.composer();
    composer.set_text("  \n ");
    assert!(!composer.can_send());
    assert!(composer.send().unwrap().is_none());
    assert!(runtime.messages().is_empty());
}

#[test]
fn an_attachment_alone_is_sendable_and_travels_with_the_message() {
    let runtime = runtime_answering(&["seen"]);
    let composer = runtime.composer();
    composer.add_attachment(CompleteAttachment {
        id: "att-1".into(),
        kind: "image".into(),
        name: "cat.png".into(),
        content_type: Some("image/png".into()),
        content: vec![UserMessagePart::Image(ImagePart {
            image: "data:image/png;base64,AA==".into(),
            ..Default::default()
        })],
        extra: Default::default(),
    });
    assert!(composer.can_send());
    finish(composer.send().unwrap());
    let ThreadMessage::User(user) = &*runtime.messages()[0] else {
        panic!("user message first")
    };
    assert_eq!(user.attachments.len(), 1);
    assert!(composer.attachments().is_empty());
}

#[test]
fn the_thread_composer_cancels_the_run_in_progress() {
    let runtime = LocalThreadRuntime::new(
        |_options: ChatModelRunOptions| -> RunStream { Box::pin(PendingStream) },
        LocalRuntimeOptions::default(),
    );
    let composer = runtime.composer();
    composer.set_text("hi");
    let mut run = composer.send().unwrap().unwrap();
    assert_eq!(
        Pin::new(&mut run).poll(&mut Context::from_waker(Waker::noop())),
        Poll::Pending
    );
    assert!(composer.can_cancel());
    composer.cancel();
    assert_eq!(
        Pin::new(&mut run).poll(&mut Context::from_waker(Waker::noop())),
        Poll::Ready(Ok(()))
    );
    assert!(!composer.can_cancel());
    assert_eq!(
        runtime.messages()[1].status(),
        Some(&MessageStatus::cancelled())
    );
}

#[test]
fn editing_a_user_message_branches_and_ends_the_edit() {
    let runtime = runtime_answering(&["a1", "a2"]);
    let composer = runtime.composer();
    composer.set_text("original");
    finish(composer.send().unwrap());
    let original = runtime.messages()[0].id().clone();

    let edit = runtime.begin_edit(&original).unwrap();
    assert!(edit.is_editing());
    assert_eq!(edit.text(), "original");
    assert_eq!(
        runtime.begin_edit(&original).err(),
        Some(RuntimeError::EditInProgress(original.clone()))
    );
    edit.set_text("revised");
    finish(edit.send().unwrap());

    assert_eq!(texts(&runtime), ["revised", "a2"]);
    assert_eq!(runtime.branches(&original).unwrap().len(), 2);
    assert!(
        runtime.edit_composer(&original).is_none(),
        "sending ends the edit"
    );
    assert!(!edit.can_send());
}

#[test]
fn cancelling_an_edit_leaves_the_thread_unchanged() {
    let runtime = runtime_answering(&["a1"]);
    let composer = runtime.composer();
    composer.set_text("keep me");
    finish(composer.send().unwrap());
    let before = runtime.messages();
    let id = before[0].id().clone();

    let edit = runtime.begin_edit(&id).unwrap();
    edit.set_text("discarded");
    edit.cancel();
    assert!(runtime.edit_composer(&id).is_none());
    assert_eq!(runtime.messages(), before);
}

#[test]
fn editing_an_assistant_message_keeps_its_non_text_parts_and_starts_no_run() {
    let runtime = runtime_answering(&["answer"]);
    let composer = runtime.composer();
    composer.set_text("q");
    finish(composer.send().unwrap());
    let answer = runtime.messages()[1].id().clone();
    let edit = runtime.begin_edit(&answer).unwrap();
    assert_eq!(edit.role(), MessageRole::Assistant);
    edit.set_text("corrected answer");
    let run = edit.send().unwrap();
    finish(run);
    assert_eq!(texts(&runtime), ["q", "corrected answer"]);
    assert!(!runtime.is_running());
}

#[test]
fn composer_subscribers_hear_draft_and_run_changes() {
    let runtime = runtime_answering(&["x"]);
    let composer = runtime.composer();
    let heard = Rc::new(Cell::new(0));
    let _subscription = {
        let heard = heard.clone();
        composer.subscribe(move || heard.set(heard.get() + 1))
    };
    composer.set_text("a");
    composer.set_text("a");
    assert_eq!(heard.get(), 1, "an unchanged value does not notify");
    finish(composer.send().unwrap());
    assert!(
        heard.get() > 2,
        "sending and the run's state changes notify"
    );
}

/// A stream that never yields, for cancellation tests.
struct PendingStream;
impl futures_core::Stream for PendingStream {
    type Item = Result<ChatModelRunResult, RunError>;
    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Pending
    }
}
