//! Behaviour of the local thread runtime, after upstream's
//! `local-thread-runtime-core` tests. Futures are polled by hand so each
//! interleaving of user actions and model output is deterministic.

use ai_ui_core::adapter::{self, ChatModelRunOptions, RunStream};
use ai_ui_core::*;
use futures_core::Stream;
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

// ------------------------------------------------------------------ helpers

fn options() -> LocalRuntimeOptions {
    let tick = Cell::new(0);
    LocalRuntimeOptions::with_clock(Rc::new(move || {
        tick.set(tick.get() + 1);
        Timestamp(tick.get())
    }))
}

/// Poll once; `Some` when the future finished.
fn poll<T>(future: &mut Pin<Box<dyn Future<Output = T>>>) -> Option<T> {
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(value) => Some(value),
        Poll::Pending => None,
    }
}

type Item = Result<ChatModelRunResult, RunError>;

/// A stream the test feeds one result at a time.
#[derive(Clone, Default)]
struct Script(Rc<RefCell<(VecDeque<Item>, bool)>>);

impl Script {
    fn push(&self, item: Item) {
        self.0.borrow_mut().0.push_back(item);
    }
    fn close(&self) {
        self.0.borrow_mut().1 = true;
    }
    fn stream(&self) -> RunStream {
        Box::pin(self.clone())
    }
}

impl Stream for Script {
    type Item = Item;
    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Item>> {
        let mut inner = self.0.borrow_mut();
        match inner.0.pop_front() {
            Some(item) => Poll::Ready(Some(item)),
            None if inner.1 => Poll::Ready(None),
            None => Poll::Pending,
        }
    }
}

/// An adapter that records each call's options and plays queued streams.
#[derive(Clone, Default)]
struct Recorder {
    calls: Rc<RefCell<Vec<ChatModelRunOptions>>>,
    streams: Rc<RefCell<VecDeque<RunStream>>>,
}

impl Recorder {
    fn then(&self, stream: RunStream) -> &Self {
        self.streams.borrow_mut().push_back(stream);
        self
    }
    fn adapter(&self) -> impl ChatModelAdapter + 'static {
        let this = self.clone();
        move |options: ChatModelRunOptions| -> RunStream {
            this.calls.borrow_mut().push(options);
            this.streams
                .borrow_mut()
                .pop_front()
                .expect("an adapter call was scripted")
        }
    }
}

fn texts(runtime: &LocalThreadRuntime) -> Vec<String> {
    runtime.messages().iter().map(|m| m.text()).collect()
}
fn last(runtime: &LocalThreadRuntime) -> AssistantMessage {
    runtime
        .messages()
        .last()
        .unwrap()
        .as_assistant()
        .unwrap()
        .clone()
}
fn tool_call(id: &str, name: &str) -> AssistantMessagePart {
    AssistantMessagePart::ToolCall(ToolCallPart {
        tool_call_id: id.into(),
        tool_name: name.into(),
        args_text: "{}".into(),
        ..Default::default()
    })
}
const NEEDS_TOOLS: MessageStatus = MessageStatus::RequiresAction {
    reason: RequiresActionReason::ToolCalls,
};

// ------------------------------------------------------------------ tests

#[test]
fn a_user_message_starts_a_run_that_completes() {
    let recorder = Recorder::default();
    recorder.then(adapter::from_results([Ok(ChatModelRunResult::text(
        "Hello!",
    ))]));
    let runtime = LocalThreadRuntime::new(recorder.adapter(), options());
    let notified = Rc::new(Cell::new(0));
    let _subscription = {
        let notified = notified.clone();
        runtime.subscribe(move || notified.set(notified.get() + 1))
    };

    let mut run = runtime
        .append(AppendMessage::user_text(None, "Hi"))
        .unwrap();
    // The user message and a running assistant message exist before polling.
    assert!(runtime.is_running());
    assert_eq!(last(&runtime).status, MessageStatus::Running);
    assert!(notified.get() > 0);

    assert_eq!(poll(&mut run), Some(Ok(())));
    assert_eq!(texts(&runtime), ["Hi", "Hello!"]);
    assert_eq!(last(&runtime).status, MessageStatus::complete());
    assert!(!runtime.is_running());

    let call = &recorder.calls.borrow()[0];
    assert_eq!(
        call.messages.len(),
        1,
        "the model sees the thread up to the user message"
    );
    assert_eq!(call.messages[0].text(), "Hi");
    assert_eq!(&call.assistant_message_id, runtime.messages()[1].id());
}

#[test]
fn cumulative_results_replace_the_step_content_as_they_stream() {
    let script = Script::default();
    let recorder = Recorder::default();
    recorder.then(script.stream());
    let runtime = LocalThreadRuntime::new(recorder.adapter(), options());
    let mut run = runtime
        .append(AppendMessage::user_text(None, "Hi"))
        .unwrap();

    script.push(Ok(ChatModelRunResult::text("Hel")));
    assert_eq!(poll(&mut run), None);
    assert_eq!(last(&runtime).content, [AssistantMessagePart::text("Hel")]);
    assert_eq!(last(&runtime).status, MessageStatus::Running);

    script.push(Ok(ChatModelRunResult::text("Hello").with_status(
        MessageStatus::Complete {
            reason: CompleteReason::Stop,
        },
    )));
    script.close();
    assert_eq!(poll(&mut run), Some(Ok(())));
    assert_eq!(
        last(&runtime).content,
        [AssistantMessagePart::text("Hello")]
    );
    assert_eq!(
        last(&runtime).status,
        MessageStatus::Complete {
            reason: CompleteReason::Stop
        }
    );
}

#[test]
fn cancelling_marks_the_message_and_stops_reading_the_stream() {
    let script = Script::default();
    let recorder = Recorder::default();
    recorder.then(script.stream());
    let runtime = LocalThreadRuntime::new(recorder.adapter(), options());
    let mut run = runtime
        .append(AppendMessage::user_text(None, "Hi"))
        .unwrap();
    script.push(Ok(ChatModelRunResult::text("partial")));
    assert_eq!(poll(&mut run), None);

    runtime.cancel_run();
    assert!(recorder.calls.borrow()[0].abort_signal.is_aborted());
    script.push(Ok(ChatModelRunResult::text("ignored after cancel")));
    assert_eq!(poll(&mut run), Some(Ok(())));
    assert_eq!(last(&runtime).status, MessageStatus::cancelled());
    assert_eq!(
        last(&runtime).content,
        [AssistantMessagePart::text("partial")]
    );
    assert!(!runtime.is_running());
}

#[test]
fn cancelling_before_the_run_is_polled_never_calls_the_model() {
    let recorder = Recorder::default();
    let runtime = LocalThreadRuntime::new(recorder.adapter(), options());
    let mut run = runtime
        .append(AppendMessage::user_text(None, "Hi"))
        .unwrap();
    runtime.cancel_run();
    assert_eq!(poll(&mut run), Some(Ok(())));
    assert!(recorder.calls.borrow().is_empty());
    assert_eq!(last(&runtime).status, MessageStatus::cancelled());
}

#[test]
fn an_adapter_failure_is_recorded_on_the_message_and_returned() {
    let recorder = Recorder::default();
    recorder.then(adapter::from_results([
        Ok(ChatModelRunResult::text("so far")),
        Err(RunError::Failed {
            message: "rate limited".into(),
            error: Some(json!({ "code": 429 })),
        }),
    ]));
    let runtime = LocalThreadRuntime::new(recorder.adapter(), options());
    let mut run = runtime
        .append(AppendMessage::user_text(None, "Hi"))
        .unwrap();
    let result = poll(&mut run).unwrap();
    assert!(matches!(result, Err(RunError::Failed { .. })));
    assert_eq!(
        last(&runtime).status,
        MessageStatus::Incomplete {
            reason: IncompleteReason::Error,
            error: Some(json!({ "code": 429 }))
        }
    );
    assert_eq!(
        last(&runtime).content,
        [AssistantMessagePart::text("so far")]
    );
    assert!(!runtime.is_running());
}

#[test]
fn dropping_an_unpolled_run_cancels_without_calling_the_model() {
    let recorder = Recorder::default();
    let runtime = LocalThreadRuntime::new(recorder.adapter(), options());
    let run = runtime
        .append(AppendMessage::user_text(None, "Hi"))
        .unwrap();
    let notifications = Rc::new(Cell::new(0));
    let _subscription = {
        let notifications = notifications.clone();
        runtime.subscribe(move || notifications.set(notifications.get() + 1))
    };

    drop(run);

    assert!(recorder.calls.borrow().is_empty());
    assert!(!runtime.is_running());
    assert_eq!(last(&runtime).status, MessageStatus::cancelled());
    assert_eq!(notifications.get(), 1, "the view learns the run ended");
}

#[test]
fn dropping_the_run_future_ends_the_run() {
    let script = Script::default();
    let recorder = Recorder::default();
    recorder.then(script.stream());
    let runtime = LocalThreadRuntime::new(recorder.adapter(), options());
    let mut run = runtime
        .append(AppendMessage::user_text(None, "Hi"))
        .unwrap();
    script.push(Ok(ChatModelRunResult::text("partial")));
    assert_eq!(poll(&mut run), None);
    let signal = recorder.calls.borrow()[0].abort_signal.clone();
    assert!(!signal.is_aborted());

    drop(run);

    assert!(
        signal.is_aborted(),
        "work outside the stream sees cancellation"
    );
    assert!(!runtime.is_running());
    assert_eq!(last(&runtime).status, MessageStatus::cancelled());
    assert_eq!(
        last(&runtime).content,
        [AssistantMessagePart::text("partial")]
    );
}

#[test]
fn dropping_an_old_future_cannot_cancel_its_replacement() {
    for poll_first in [false, true] {
        let recorder = Recorder::default();
        let runtime = LocalThreadRuntime::new(recorder.adapter(), options());
        let mut first = runtime
            .append(AppendMessage::user_text(None, "first"))
            .unwrap();
        if poll_first {
            recorder.then(Script::default().stream());
            assert_eq!(poll(&mut first), None);
        }
        let parent = runtime.messages()[1].id().clone();
        let second = Script::default();
        recorder.then(second.stream());
        let mut replacement = runtime
            .append(AppendMessage::user_text(Some(parent), "second"))
            .unwrap();
        assert_eq!(poll(&mut replacement), None);
        let signal = recorder.calls.borrow().last().unwrap().abort_signal.clone();

        drop(first);

        assert!(runtime.is_running());
        assert!(!signal.is_aborted());
        assert_eq!(last(&runtime).status, MessageStatus::Running);
        second.push(Ok(ChatModelRunResult::text("replacement answer")));
        second.close();
        assert_eq!(poll(&mut replacement), Some(Ok(())));
        assert_eq!(last(&runtime).status, MessageStatus::complete());
        assert_eq!(
            runtime.messages().last().unwrap().text(),
            "replacement answer"
        );
    }
}

#[test]
fn appending_during_a_run_cancels_it_and_answers_the_new_message() {
    let first = Script::default();
    let recorder = Recorder::default();
    recorder
        .then(first.stream())
        .then(adapter::from_results([Ok(ChatModelRunResult::text(
            "second answer",
        ))]));
    let runtime = LocalThreadRuntime::new(recorder.adapter(), options());
    let mut run1 = runtime
        .append(AppendMessage::user_text(None, "first"))
        .unwrap();
    assert_eq!(poll(&mut run1), None);
    let first_answer = runtime.messages()[1].id().clone();

    // The composer sends after the in-progress answer, as upstream does.
    let mut run2 = runtime
        .append(AppendMessage::user_text(
            Some(first_answer.clone()),
            "second",
        ))
        .unwrap();
    assert_eq!(poll(&mut run1), Some(Ok(())));
    assert_eq!(
        runtime.message(&first_answer).unwrap().message.status(),
        Some(&MessageStatus::cancelled())
    );
    assert!(
        runtime.is_running(),
        "the old run ending does not end the new one"
    );
    assert_eq!(poll(&mut run2), Some(Ok(())));
    assert_eq!(texts(&runtime), ["first", "", "second", "second answer"]);
}

#[test]
fn regenerating_adds_a_sibling_branch() {
    let recorder = Recorder::default();
    recorder
        .then(adapter::from_results([Ok(ChatModelRunResult::text("one"))]))
        .then(adapter::from_results([Ok(ChatModelRunResult::text("two"))]));
    let runtime = LocalThreadRuntime::new(recorder.adapter(), options());
    poll(
        &mut runtime
            .append(AppendMessage::user_text(None, "Hi"))
            .unwrap(),
    )
    .unwrap()
    .unwrap();
    let user = runtime.messages()[0].id().clone();
    let first = runtime.messages()[1].id().clone();

    let reload = StartRunConfig {
        parent_id: Some(user),
        source_id: Some(first.clone()),
        ..Default::default()
    };
    poll(&mut runtime.start_run(reload).unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(texts(&runtime), ["Hi", "two"]);
    let branches = runtime.branches(&first).unwrap();
    assert_eq!(branches.len(), 2);
    runtime.switch_to_branch(&first).unwrap();
    assert_eq!(texts(&runtime), ["Hi", "one"]);
}

#[test]
fn editing_a_user_message_branches_from_its_parent() {
    let recorder = Recorder::default();
    recorder
        .then(adapter::from_results([Ok(ChatModelRunResult::text("a1"))]))
        .then(adapter::from_results([Ok(ChatModelRunResult::text("a2"))]));
    let runtime = LocalThreadRuntime::new(recorder.adapter(), options());
    poll(
        &mut runtime
            .append(AppendMessage::user_text(None, "original"))
            .unwrap(),
    )
    .unwrap()
    .unwrap();
    let original = runtime.messages()[0].id().clone();

    let mut edit = AppendMessage::user_text(None, "edited");
    edit.source_id = Some(original.clone());
    poll(&mut runtime.append(edit).unwrap()).unwrap().unwrap();
    assert_eq!(texts(&runtime), ["edited", "a2"]);
    assert_eq!(runtime.branches(&original).unwrap().len(), 2);
    assert_eq!(
        recorder.calls.borrow()[1].messages.len(),
        1,
        "the edit's run sees only the edited path"
    );
}

#[test]
fn tool_results_continue_the_run_and_settle_the_call_in_place() {
    let recorder = Recorder::default();
    recorder
        .then(adapter::from_results([Ok(ChatModelRunResult::content(
            vec![
                AssistantMessagePart::text("Let me check."),
                tool_call("c1", "weather"),
            ],
        )
        .with_status(NEEDS_TOOLS))]))
        .then(adapter::from_results([Ok(ChatModelRunResult::text(
            "It is sunny.",
        ))]));
    let runtime = LocalThreadRuntime::new(recorder.adapter(), options());
    poll(
        &mut runtime
            .append(AppendMessage::user_text(None, "Weather?"))
            .unwrap(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(last(&runtime).status, NEEDS_TOOLS);
    assert!(!runtime.is_running(), "the run pauses for the tool result");

    let message_id = runtime.messages()[1].id().clone();
    let mut resumed = runtime
        .add_tool_result(ToolResult {
            message_id,
            tool_call_id: "c1".into(),
            result: json!({ "sky": "clear" }),
            is_error: false,
            artifact: None,
        })
        .unwrap()
        .expect("the last open call was answered");
    assert_eq!(poll(&mut resumed), Some(Ok(())));

    let message = last(&runtime);
    assert_eq!(message.status, MessageStatus::complete());
    assert_eq!(
        message.content.len(),
        3,
        "earlier parts stay; the new step appends"
    );
    assert_eq!(
        message.content[1].as_tool_call().unwrap().result,
        Some(json!({ "sky": "clear" }))
    );
    assert_eq!(
        message.content[2],
        AssistantMessagePart::text("It is sunny.")
    );
    let second = &recorder.calls.borrow()[1];
    assert_eq!(second.message.content.len(), 2);
    assert_eq!(&second.assistant_message_id, &message.id);
}

#[test]
fn a_run_stops_at_max_steps_with_tool_calls_pending() {
    let recorder = Recorder::default();
    let answered = |id: &str| {
        let mut call = tool_call(id, "lookup");
        if let AssistantMessagePart::ToolCall(c) = &mut call {
            c.result = Some(json!(true));
        }
        ChatModelRunResult::content(vec![call]).with_status(NEEDS_TOOLS)
    };
    recorder
        .then(adapter::from_results([Ok(answered("c1"))]))
        .then(adapter::from_results([Ok(answered("c2"))]));
    let runtime = LocalThreadRuntime::new(recorder.adapter(), options());
    poll(
        &mut runtime
            .append(AppendMessage::user_text(None, "loop"))
            .unwrap(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(recorder.calls.borrow().len(), 2, "max_steps defaults to 2");
    assert_eq!(
        last(&runtime).status,
        MessageStatus::Incomplete {
            reason: IncompleteReason::ToolCalls,
            error: None
        }
    );
}

#[test]
fn only_human_tools_hold_a_run_when_listed() {
    let recorder = Recorder::default();
    recorder.then(adapter::from_results([Ok(ChatModelRunResult::content(
        vec![tool_call("h1", "ask_user")],
    )
    .with_status(NEEDS_TOOLS))]));
    let mut opts = options();
    opts.human_tool_names = Some(vec!["ask_user".into()]);
    let runtime = LocalThreadRuntime::new(recorder.adapter(), opts);
    poll(
        &mut runtime
            .append(AppendMessage::user_text(None, "go"))
            .unwrap(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(recorder.calls.borrow().len(), 1);
    assert_eq!(last(&runtime).status, NEEDS_TOOLS);
}

#[test]
fn a_tool_result_for_an_unknown_call_is_rejected_without_change() {
    let recorder = Recorder::default();
    recorder.then(adapter::from_results([Ok(ChatModelRunResult::text("x"))]));
    let runtime = LocalThreadRuntime::new(recorder.adapter(), options());
    poll(
        &mut runtime
            .append(AppendMessage::user_text(None, "Hi"))
            .unwrap(),
    )
    .unwrap()
    .unwrap();
    let message_id = runtime.messages()[1].id().clone();
    let before = runtime.messages();
    let error = runtime
        .add_tool_result(ToolResult {
            message_id: message_id.clone(),
            tool_call_id: "nope".into(),
            result: json!(1),
            is_error: false,
            artifact: None,
        })
        .err();
    assert_eq!(
        error,
        Some(RuntimeError::ToolCallNotFound {
            message_id,
            tool_call_id: "nope".into()
        })
    );
    assert_eq!(before, runtime.messages());
}

#[test]
fn metadata_lists_append_per_step_and_custom_merges() {
    let recorder = Recorder::default();
    let with_meta = |text: &str, step: u64| ChatModelRunResult {
        content: Some(vec![AssistantMessagePart::text(text)]),
        metadata: Some(adapter::RunResultMetadata {
            steps: Some(vec![ThreadStep {
                message_id: None,
                usage: Some(Usage {
                    input_tokens: step,
                    output_tokens: step,
                }),
                ..Default::default()
            }]),
            custom: Some(serde_json::from_value(json!({ "model": "m" })).unwrap()),
            ..Default::default()
        }),
        ..Default::default()
    };
    // Two cumulative snapshots of one step must not double the step list.
    recorder.then(adapter::from_results([
        Ok(with_meta("a", 1)),
        Ok(with_meta("ab", 1)),
    ]));
    let runtime = LocalThreadRuntime::new(recorder.adapter(), options());
    poll(
        &mut runtime
            .append(AppendMessage::user_text(None, "Hi"))
            .unwrap(),
    )
    .unwrap()
    .unwrap();
    let message = last(&runtime);
    assert_eq!(message.metadata.steps.len(), 1);
    assert_eq!(message.metadata.custom["model"], json!("m"));
}

#[test]
fn export_and_import_restore_the_thread() {
    let recorder = Recorder::default();
    recorder.then(adapter::from_results([Ok(ChatModelRunResult::text(
        "answer",
    ))]));
    let runtime = LocalThreadRuntime::new(recorder.adapter(), options());
    poll(
        &mut runtime
            .append(AppendMessage::user_text(None, "question"))
            .unwrap(),
    )
    .unwrap()
    .unwrap();
    let exported = runtime.export();

    let restored = LocalThreadRuntime::new(Recorder::default().adapter(), options());
    restored.import(exported).unwrap();
    assert_eq!(texts(&restored), ["question", "answer"]);
    assert_eq!(restored.messages(), runtime.messages());
}

#[test]
fn feedback_is_recorded_on_the_assistant_message_only() {
    let recorder = Recorder::default();
    recorder.then(adapter::from_results([Ok(ChatModelRunResult::text("x"))]));
    let runtime = LocalThreadRuntime::new(recorder.adapter(), options());
    poll(
        &mut runtime
            .append(AppendMessage::user_text(None, "Hi"))
            .unwrap(),
    )
    .unwrap()
    .unwrap();
    let (user, answer) = (
        runtime.messages()[0].id().clone(),
        runtime.messages()[1].id().clone(),
    );
    let positive = SubmittedFeedback {
        kind: FeedbackKind::Positive,
        comment: None,
    };
    runtime
        .submit_feedback(&answer, Some(positive.clone()))
        .unwrap();
    assert_eq!(
        last(&runtime).metadata.submitted_feedback,
        Some(positive.clone())
    );
    assert_eq!(
        runtime.submit_feedback(&user, Some(positive)),
        Err(RuntimeError::NotAssistantMessage(user))
    );
}
