//! The state layer over local, external-store and listed threads.

use ai_ui_core::adapter::{self, RunStream};
use ai_ui_core::*;
use ai_ui_state::*;
use futures_core::Stream;
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

type Item = Result<ChatModelRunResult, RunError>;

/// A model stream the test feeds by hand.
#[derive(Clone, Default)]
struct Script(Rc<RefCell<(VecDeque<Item>, bool)>>);
impl Script {
    fn push(&self, text: &str, status: Option<MessageStatus>) {
        let mut result = ChatModelRunResult::text(text);
        result.content = Some(vec![AssistantMessagePart::Text(TextPart {
            text: text.into(),
            status: Some(if status.is_some() {
                PartStatus::Complete
            } else {
                PartStatus::Running
            }),
            ..Default::default()
        })]);
        result.status = status;
        self.0.borrow_mut().0.push_back(Ok(result));
    }
    fn close(&self) {
        self.0.borrow_mut().1 = true;
    }
}
impl Stream for Script {
    type Item = Item;
    fn poll_next(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Item>> {
        let mut inner = self.0.borrow_mut();
        match inner.0.pop_front() {
            Some(item) => Poll::Ready(Some(item)),
            None if inner.1 => Poll::Ready(None),
            None => Poll::Pending,
        }
    }
}

fn poll(run: &mut RunFuture) -> Option<Result<(), RunError>> {
    match run.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(v) => Some(v),
        Poll::Pending => None,
    }
}

fn answering(text: &'static str) -> LocalThreadRuntime {
    LocalThreadRuntime::new(
        move |_o: ChatModelRunOptions| -> RunStream {
            adapter::from_results([Ok(ChatModelRunResult::text(text))])
        },
        LocalRuntimeOptions::default(),
    )
}

fn text_of(state: &ThreadState, message: usize, part: usize) -> String {
    match &state.messages[message].parts[part].kind {
        PartKind::Text { text, .. } => text.clone(),
        other => panic!("not text: {other:?}"),
    }
}

#[test]
fn rows_describe_the_thread_and_unchanged_rows_keep_their_identity() {
    let runtime = answering("Hello");
    let view = ThreadView::new(Rc::new(runtime.clone()), ViewOptions { smooth: None });
    assert!(view.state().is_empty);
    let mut run = view.send("Hi").unwrap().unwrap();
    assert_eq!(poll(&mut run), Some(Ok(())));
    let state = view.state();
    assert_eq!(state.messages.len(), 2);
    let (user, assistant) = (&state.messages[0], &state.messages[1]);
    assert_eq!(
        (user.role, user.is_last, user.index),
        (MessageRole::User, false, 0)
    );
    assert_eq!(
        (assistant.role, assistant.is_last),
        (MessageRole::Assistant, true)
    );
    assert!(user.can_edit && assistant.can_reload && assistant.can_copy && assistant.can_feedback);
    assert_eq!(assistant.text, "Hello");
    assert_eq!((assistant.branch_number, assistant.branch_count), (1, 1));
    assert!(
        Arc::ptr_eq(&state, &view.state()),
        "nothing changed: the same state"
    );

    // A regeneration adds a branch to the assistant only; the user row is reused.
    let user_row = state.messages[0].clone();
    let mut again = view.reload(&assistant.id).unwrap();
    poll(&mut again).unwrap().unwrap();
    let next = view.state();
    assert!(
        Arc::ptr_eq(&user_row, &next.messages[0]),
        "the user row did not change"
    );
    assert_eq!(next.messages[1].branch_count, 2);
    assert_eq!(next.messages[1].branch_number, 2);
    view.switch_branch(&next.messages[1].id, false).unwrap();
    assert_eq!(view.state().messages[1].branch_number, 1);
}

#[test]
fn a_streaming_update_rebuilds_only_the_message_that_changed() {
    let script = Script::default();
    let runtime = LocalThreadRuntime::new(
        {
            let script = script.clone();
            move |_o: ChatModelRunOptions| -> RunStream { Box::pin(script.clone()) }
        },
        LocalRuntimeOptions::default(),
    );
    let view = ThreadView::new(Rc::new(runtime), ViewOptions { smooth: None });
    let heard = Rc::new(Cell::new(0));
    let _sub = {
        let heard = heard.clone();
        view.subscribe(move || heard.set(heard.get() + 1))
    };
    let mut run = view.send("Hi").unwrap().unwrap();
    script.push("Hel", None);
    assert_eq!(poll(&mut run), None);
    let first = view.state();
    assert!(
        first.messages[1].is_generating && !first.messages[1].can_copy,
        "no copy while generating"
    );
    script.push("Hello", None);
    poll(&mut run);
    let second = view.state();
    assert!(Arc::ptr_eq(&first.messages[0], &second.messages[0]));
    assert!(!Arc::ptr_eq(&first.messages[1], &second.messages[1]));
    assert_eq!(text_of(&second, 1, 0), "Hello");
    assert!(heard.get() >= 2);
    script.push(
        "Hello!",
        Some(MessageStatus::Complete {
            reason: CompleteReason::Stop,
        }),
    );
    script.close();
    assert_eq!(poll(&mut run), Some(Ok(())));
    assert!(view.state().messages[1].can_copy);
}

#[test]
fn smooth_streaming_reveals_text_over_ticks_and_asks_for_frames() {
    let script = Script::default();
    let runtime = LocalThreadRuntime::new(
        {
            let script = script.clone();
            move |_o: ChatModelRunOptions| -> RunStream { Box::pin(script.clone()) }
        },
        LocalRuntimeOptions::default(),
    );
    let view = ThreadView::new(Rc::new(runtime), ViewOptions::default());
    let mut run = view.send("Hi").unwrap().unwrap();
    script.push("A fairly long burst of streamed text.", None);
    poll(&mut run);
    assert_eq!(
        text_of(&view.state(), 1, 0),
        "",
        "nothing is revealed before the first frame"
    );
    assert!(view.needs_frame());
    let mut now = 0.0;
    view.tick(now);
    now += 16.0;
    assert!(view.tick(now), "the first real frame reveals text");
    let partial = text_of(&view.state(), 1, 0);
    assert!(!partial.is_empty() && partial.len() < 37, "got {partial:?}");
    while view.needs_frame() && now < 5_000.0 {
        now += 16.0;
        view.tick(now);
    }
    assert_eq!(
        text_of(&view.state(), 1, 0),
        "A fairly long burst of streamed text."
    );
    assert!(!view.tick(now + 16.0), "nothing left to animate");
}

#[test]
fn tool_approval_flows_through_the_view() {
    let calls = Rc::new(Cell::new(0));
    let runtime = LocalThreadRuntime::new(
        |o: ChatModelRunOptions| -> RunStream {
            if let Some(result) = o.message.tool_calls().find_map(|c| c.result.clone()) {
                return adapter::from_results([Ok(ChatModelRunResult::text(format!(
                    "Sent: {result}"
                )))]);
            }
            adapter::from_results([Ok(ChatModelRunResult::content(vec![
                AssistantMessagePart::ToolCall(ToolCallPart {
                    tool_call_id: "c".into(),
                    tool_name: "send".into(),
                    args: serde_json::from_value(json!({ "to": "team" })).unwrap(),
                    args_text: r#"{"to":"team"}"#.into(),
                    ..Default::default()
                }),
            ])
            .with_status(MessageStatus::RequiresAction {
                reason: RequiresActionReason::ToolCalls,
            }))])
        },
        LocalRuntimeOptions::default(),
    );
    runtime.set_tools(
        ToolRegistry::new().with(
            "send",
            Tool::new(json!({}))
                .requires_approval()
                .approval_prompt(|_, a| format!("Send to {}?", a["to"]))
                .execute({
                    let calls = calls.clone();
                    move |_, _| {
                        calls.set(calls.get() + 1);
                        async { Ok(json!("ok")) }
                    }
                }),
        ),
    );
    let view = ThreadView::new(Rc::new(runtime), ViewOptions { smooth: None });
    view.set_tool_presenter("send", |call| ToolPresentation {
        title: Some("Email".into()),
        detail: Some(format!("to {}", call.args["to"])),
        result_text: None,
    });
    let mut run = view.send("Email the team").unwrap().unwrap();
    assert_eq!(poll(&mut run), Some(Ok(())));
    let state = view.state();
    let PartKind::ToolCall(call) = &state.messages[1].parts[0].kind else {
        panic!("tool call expected")
    };
    assert_eq!(call.status, ToolStatus::AwaitingApproval);
    let approval = call.approval.clone().unwrap();
    assert_eq!(
        (approval.decision, approval.prompt.as_str()),
        (ApprovalDecision::Pending, r#"Send to "team"?"#)
    );
    assert_eq!(
        (
            call.presentation.title.as_deref(),
            call.presentation.detail.as_deref()
        ),
        (Some("Email"), Some(r#"to "team""#))
    );
    assert_eq!(calls.get(), 0);

    let mut resumed = view
        .respond_to_approval(&state.messages[1].id, &approval.id, true)
        .unwrap()
        .unwrap();
    assert_eq!(poll(&mut resumed), Some(Ok(())));
    let state = view.state();
    let PartKind::ToolCall(call) = &state.messages[1].parts[0].kind else {
        panic!()
    };
    assert_eq!((call.status, calls.get()), (ToolStatus::Complete, 1));
    assert_eq!(text_of(&state, 1, 1), r#"Sent: "ok""#);

    // A presenter changes projection without changing runtime versions. Drop
    // the old snapshot so the fallback is required by presenter invalidation,
    // rather than by retaining an immutable ThreadState.
    let cursor = view.state_changes_since(None).cursor;
    drop(state);
    view.set_tool_presenter("send", |_| ToolPresentation {
        title: Some("Sent email".into()),
        ..Default::default()
    });
    let changed = view.state_changes_since(Some(&cursor));
    assert!(changed.indices.is_none());
    let PartKind::ToolCall(call) = &changed.state.messages[1].parts[0].kind else {
        panic!("tool call expected")
    };
    assert_eq!(call.presentation.title.as_deref(), Some("Sent email"));
}

struct Store;
impl ExternalStore for Store {
    fn on_new(&self, _: AppendMessage) -> StoreFuture {
        Box::pin(async { Ok(()) })
    }
}

#[test]
fn an_external_store_thread_hides_unsupported_actions() {
    let runtime = ExternalStoreRuntime::new(Store);
    let message = |role: &str, id: &str, text: &str| -> ThreadMessage {
        let mut v = json!({ "role": role, "id": id, "createdAt": 0, "content": [{ "type": "text", "text": text }] });
        if role == "assistant" {
            v["status"] = json!({ "type": "complete", "reason": "stop" });
        }
        serde_json::from_value(v).unwrap()
    };
    runtime.set_messages([
        message("user", "u", "Hi"),
        message("assistant", "a", "Hello"),
    ]);
    let view = ThreadView::new(Rc::new(runtime.clone()), ViewOptions { smooth: None });
    let before = view.state();
    assert!(
        !before.messages[0].can_edit
            && !before.messages[1].can_reload
            && !before.messages[1].can_feedback
    );
    assert!(before.messages[1].can_copy);

    // The app streams more text: the view follows and keeps the first row.
    runtime.set_messages([
        message("user", "u", "Hi"),
        message("assistant", "a", "Hello there"),
    ]);
    let after = view.state();
    assert!(Arc::ptr_eq(&before.messages[0], &after.messages[0]));
    assert_eq!(text_of(&after, 1, 0), "Hello there");
    assert!(view.reload(&"a".into()).is_err());
}

#[test]
fn copy_confirmation_expires_and_sections_toggle() {
    let runtime = LocalThreadRuntime::new(
        |_o: ChatModelRunOptions| -> RunStream {
            adapter::from_results([Ok(ChatModelRunResult::content(vec![
                AssistantMessagePart::Reasoning(ReasoningPart {
                    text: "thinking".into(),
                    status: Some(PartStatus::Complete),
                    ..Default::default()
                }),
                AssistantMessagePart::text("answer"),
            ]))])
        },
        LocalRuntimeOptions::default(),
    );
    let view = ThreadView::new(Rc::new(runtime), ViewOptions { smooth: None });
    futures_executor::block_on(view.send("Hi").unwrap().unwrap()).unwrap();
    let id = view.state().messages[1].id.clone();
    assert_eq!(view.copy_text(&id).as_deref(), Some("answer"));
    view.mark_copied(&id, 1000.0);
    assert!(view.state().messages[1].is_copied && view.needs_frame());
    assert!(!view.tick(2500.0));
    assert!(view.tick(3000.0));
    assert!(!view.state().messages[1].is_copied && !view.needs_frame());

    let expanded = |view: &ThreadView| match &view.state().messages[1].parts[0].kind {
        PartKind::Reasoning { expanded, .. } => *expanded,
        _ => panic!(),
    };
    assert!(
        !expanded(&view),
        "finished reasoning is collapsed by default"
    );
    view.toggle_part(&id, 0);
    assert!(expanded(&view));
    view.toggle_part(&id, 0);
    assert!(!expanded(&view));
}

#[test]
fn a_view_over_a_thread_list_follows_the_main_thread() {
    let list = ThreadList::new(
        |_id| -> Rc<dyn ThreadRuntime> { Rc::new(answering("answer")) },
        Rc::new(|_| {}),
        None,
    );
    let view = ThreadView::for_list(&list, ViewOptions { smooth: None });
    let first = list.main_id();
    futures_executor::block_on(view.send("one").unwrap().unwrap()).unwrap();
    assert_eq!(view.state().messages.len(), 2);

    list.switch_to_new();
    assert_ne!(list.main_id(), first);
    assert!(view.state().is_empty, "the view moved to the new thread");
    list.switch_to(&first);
    assert_eq!(
        view.state().messages.len(),
        2,
        "and back, with the first thread's messages"
    );
}

#[test]
fn independent_views_catch_up_after_coalesced_updates_and_keep_old_snapshots() {
    let script = Script::default();
    let runtime = LocalThreadRuntime::new(
        {
            let script = script.clone();
            move |_o: ChatModelRunOptions| -> RunStream { Box::pin(script.clone()) }
        },
        LocalRuntimeOptions::default(),
    );
    let fast = ThreadView::new(Rc::new(runtime.clone()), ViewOptions { smooth: None });
    let slow = ThreadView::new(Rc::new(runtime.clone()), ViewOptions { smooth: None });
    let mut run = fast.send("Hi").unwrap().unwrap();
    script.push("initial", None);
    poll(&mut run);
    let old = slow.state();
    let unchanged_user = fast.state().messages[0].clone();
    for i in 1..=100 {
        let text = format!("update {i}");
        script.push(&text, None);
        poll(&mut run);
        let shown = fast.state();
        assert_eq!(shown.messages[1].text, text);
        assert!(Arc::ptr_eq(&unchanged_user, &shown.messages[0]));
        if i == 20 || i == 100 {
            // The second gap exceeds the journal, and the first retained
            // snapshot forces a new output allocation. Neither loses data.
            assert_eq!(shown, slow.state());
        }
    }
    assert_eq!(old.messages[1].text, "initial");
    assert!(old.is_running);
    script.close();
    assert_eq!(poll(&mut run), Some(Ok(())));
    assert!(!fast.state().is_running);
    assert!(fast.state().messages[0].can_edit);
    assert!(fast.state().messages[1].can_reload);
    let reference = ThreadView::new(Rc::new(runtime), ViewOptions { smooth: None });
    assert_eq!(fast.state(), reference.state());
    assert_eq!(slow.state(), reference.state());
}

#[test]
fn deltas_include_older_messages_and_ui_local_changes() {
    let runtime = answering("answer");
    let view = ThreadView::new(Rc::new(runtime.clone()), ViewOptions { smooth: None });
    futures_executor::block_on(view.send("one").unwrap().unwrap()).unwrap();
    futures_executor::block_on(view.send("two").unwrap().unwrap()).unwrap();
    let ids: Vec<_> = view.state().messages.iter().map(|m| m.id.clone()).collect();
    view.submit_feedback(&ids[1], FeedbackKind::Positive)
        .unwrap();
    view.submit_feedback(&ids[3], FeedbackKind::Negative)
        .unwrap();
    let reference = ThreadView::new(Rc::new(runtime), ViewOptions { smooth: None });
    assert_eq!(view.state(), reference.state());

    view.mark_copied(&ids[0], 0.0);
    assert!(view.state().messages[0].is_copied);
    view.mark_copied(&ids[2], 100.0);
    assert!(!view.state().messages[0].is_copied);
    assert!(view.state().messages[2].is_copied);
    view.tick(2200.0);
    assert_eq!(view.state(), reference.state());
}

#[test]
fn rebind_discards_change_cursors_even_if_versions_and_ids_match() {
    let first = answering("first answer");
    let second = answering("second answer");
    for runtime in [&first, &second] {
        futures_executor::block_on(
            runtime
                .append(AppendMessage::user_text(None, "Hi"))
                .unwrap(),
        )
        .unwrap();
    }
    assert_eq!(first.structure_version(), second.structure_version());
    assert_eq!(
        first.message_changes_since(None).unwrap().version,
        second.message_changes_since(None).unwrap().version
    );
    let view = ThreadView::new(Rc::new(first), ViewOptions { smooth: None });
    assert_eq!(view.state().messages[1].text, "first answer");
    view.rebind(Rc::new(second));
    assert_eq!(view.state().messages[1].text, "second answer");
}

#[test]
fn nested_notifications_do_not_drop_another_messages_update() {
    let runtime = answering("answer");
    let view = ThreadView::new(Rc::new(runtime.clone()), ViewOptions { smooth: None });
    futures_executor::block_on(view.send("one").unwrap().unwrap()).unwrap();
    futures_executor::block_on(view.send("two").unwrap().unwrap()).unwrap();
    let ids: Vec<_> = view.state().messages.iter().map(|m| m.id.clone()).collect();
    let nested = Rc::new(Cell::new(false));
    let _subscription = view.subscribe({
        let view = view.clone();
        let nested = nested.clone();
        let id = ids[3].clone();
        move || {
            if !nested.replace(true) {
                view.submit_feedback(&id, FeedbackKind::Negative).unwrap();
            }
            let _ = view.state();
        }
    });
    view.submit_feedback(&ids[1], FeedbackKind::Positive)
        .unwrap();
    let reference = ThreadView::new(Rc::new(runtime), ViewOptions { smooth: None });
    assert_eq!(view.state(), reference.state());
}

#[test]
fn row_change_consumers_are_independent_and_plain_reads_do_not_consume_changes() {
    let view = ThreadView::new(Rc::new(answering("answer")), ViewOptions { smooth: None });
    futures_executor::block_on(view.send("one").unwrap().unwrap()).unwrap();
    futures_executor::block_on(view.send("two").unwrap().unwrap()).unwrap();
    let (cursor, ids) = {
        let changes = view.state_changes_since(None);
        assert!(changes.indices.is_none());
        let ids: Vec<_> = changes
            .state
            .messages
            .iter()
            .map(|m| m.id.clone())
            .collect();
        (changes.cursor, ids)
    };
    let slow_cursor = cursor.clone();
    view.submit_feedback(&ids[1], FeedbackKind::Positive)
        .unwrap();
    // A different subscriber may read the projection before either host does.
    drop(view.state());
    view.mark_copied(&ids[0], 0.0);
    assert!(view.needs_frame());
    let fast = view.state_changes_since(Some(&cursor));
    assert_eq!(fast.indices, Some(vec![0, 1]));
    assert!(fast.state.messages[0].is_copied);
    assert_eq!(
        fast.state.messages[1].feedback,
        Some(FeedbackKind::Positive)
    );
    let slow = view.state_changes_since(Some(&slow_cursor));
    assert_eq!(slow.indices, Some(vec![0, 1]));
    assert_eq!(slow.state, fast.state);
    assert_eq!(
        view.state_changes_since(Some(&fast.cursor)).indices,
        Some(Vec::new())
    );
}

#[test]
fn streaming_row_changes_and_run_end_are_reported_to_the_host() {
    let script = Script::default();
    let runtime = LocalThreadRuntime::new(
        {
            let script = script.clone();
            move |_o: ChatModelRunOptions| -> RunStream { Box::pin(script.clone()) }
        },
        LocalRuntimeOptions::default(),
    );
    let view = ThreadView::new(Rc::new(runtime), ViewOptions { smooth: None });
    let mut run = view.send("Hi").unwrap().unwrap();
    script.push("first", None);
    assert_eq!(poll(&mut run), None);
    let cursor = view.state_changes_since(None).cursor;
    script.push("second", None);
    assert_eq!(poll(&mut run), None);
    let changes = view.state_changes_since(Some(&cursor));
    assert_eq!(changes.indices, Some(vec![1]));
    assert_eq!(changes.state.messages[1].text, "second");
    let cursor = changes.cursor;
    drop(changes.state);
    script.close();
    assert_eq!(poll(&mut run), Some(Ok(())));
    let finished = view.state_changes_since(Some(&cursor));
    assert!(
        finished.indices.is_none(),
        "run end changes every row's actions"
    );
    assert!(!finished.state.is_running);
    assert!(finished.state.messages[0].can_edit);
}

#[test]
fn row_changes_include_smoothing_copy_expiry_and_the_previous_copied_row() {
    let script = Script::default();
    let runtime = LocalThreadRuntime::new(
        {
            let script = script.clone();
            move |_o: ChatModelRunOptions| -> RunStream { Box::pin(script.clone()) }
        },
        LocalRuntimeOptions::default(),
    );
    let view = ThreadView::new(Rc::new(runtime), ViewOptions::default());
    let mut run = view.send("Hi").unwrap().unwrap();
    script.push("A fairly long burst of streamed text.", None);
    poll(&mut run);
    let cursor = view.state_changes_since(None).cursor;
    view.tick(0.0);
    assert!(view.tick(16.0));
    let changes = view.state_changes_since(Some(&cursor));
    assert_eq!(changes.indices, Some(vec![1]));
    let user = changes.state.messages[0].id.clone();
    let assistant = changes.state.messages[1].id.clone();
    let cursor = changes.cursor;
    drop(changes.state);
    view.mark_copied(&user, 16.0);
    drop(view.state());
    view.mark_copied(&assistant, 32.0);
    let changes = view.state_changes_since(Some(&cursor));
    assert_eq!(changes.indices, Some(vec![0, 1]));
    assert!(!changes.state.messages[0].is_copied);
    assert!(changes.state.messages[1].is_copied);
    let cursor = changes.cursor;
    drop(changes.state);
    assert!(view.tick(3000.0));
    let expired = view.state_changes_since(Some(&cursor));
    assert_eq!(expired.indices, Some(vec![1]));
    assert!(!expired.state.messages[1].is_copied);
}

#[test]
fn row_changes_fall_back_after_retained_snapshots_and_keep_them_immutable() {
    let view = ThreadView::new(Rc::new(answering("answer")), ViewOptions { smooth: None });
    futures_executor::block_on(view.send("one").unwrap().unwrap()).unwrap();
    let retained = view.state_changes_since(None);
    let id = retained.state.messages[0].id.clone();
    view.mark_copied(&id, 0.0);
    let changed = view.state_changes_since(Some(&retained.cursor));
    assert!(changed.indices.is_none());
    assert!(!retained.state.messages[0].is_copied);
    assert!(changed.state.messages[0].is_copied);
    assert!(!Arc::ptr_eq(&retained.state, &changed.state));
}

#[test]
fn expired_row_cursors_and_structural_changes_require_full_reconciliation() {
    let view = ThreadView::new(Rc::new(answering("answer")), ViewOptions { smooth: None });
    futures_executor::block_on(view.send("one").unwrap().unwrap()).unwrap();
    let (old, id) = {
        let changes = view.state_changes_since(None);
        (changes.cursor, changes.state.messages[0].id.clone())
    };
    let mut fresh = old.clone();
    for revision in 0..65 {
        view.mark_copied(&id, f64::from(revision));
        let changes = view.state_changes_since(Some(&fresh));
        assert_eq!(changes.indices, Some(vec![0]));
        fresh = changes.cursor;
    }
    assert!(view.state_changes_since(Some(&old)).indices.is_none());
    assert_eq!(
        view.state_changes_since(Some(&fresh)).indices,
        Some(Vec::new())
    );
    futures_executor::block_on(view.send("two").unwrap().unwrap()).unwrap();
    let changed = view.state_changes_since(Some(&fresh));
    assert!(changed.indices.is_none());
    assert_eq!(changed.state.messages.len(), 4);
}

#[test]
fn row_cursors_are_scoped_to_a_view_and_rebind_is_a_barrier() {
    let first = answering("first");
    let second = answering("second");
    for runtime in [&first, &second] {
        futures_executor::block_on(
            runtime
                .append(AppendMessage::user_text(None, "Hi"))
                .unwrap(),
        )
        .unwrap();
    }
    let view = ThreadView::new(Rc::new(first.clone()), ViewOptions { smooth: None });
    let other = ThreadView::new(Rc::new(first), ViewOptions { smooth: None });
    let cursor = view.state_changes_since(None).cursor;
    let foreign = other.state_changes_since(Some(&cursor));
    assert!(foreign.indices.is_none());
    assert_eq!(
        view.clone().state_changes_since(Some(&cursor)).indices,
        Some(Vec::new())
    );
    view.rebind(Rc::new(second));
    let rebound = view.state_changes_since(Some(&cursor));
    assert!(rebound.indices.is_none());
    assert_eq!(rebound.state.messages[1].text, "second");
}

#[test]
fn runtimes_without_a_change_journal_keep_the_full_reconciliation_path() {
    let runtime = ExternalStoreRuntime::new(Store);
    let message = |text: &str| -> ThreadMessage {
        serde_json::from_value(json!({
            "role": "user", "id": "u", "createdAt": 0,
            "content": [{ "type": "text", "text": text }]
        }))
        .unwrap()
    };
    runtime.set_messages([message("before")]);
    let view = ThreadView::new(Rc::new(runtime.clone()), ViewOptions { smooth: None });
    let cursor = view.state_changes_since(None).cursor;
    runtime.set_messages([message("after")]);
    let changed = view.state_changes_since(Some(&cursor));
    assert!(changed.indices.is_none());
    assert_eq!(changed.state.messages[0].text, "after");
}

#[test]
fn switching_a_branch_requires_full_host_reconciliation_even_at_the_same_length() {
    let view = ThreadView::new(Rc::new(answering("answer")), ViewOptions { smooth: None });
    futures_executor::block_on(view.send("Hi").unwrap().unwrap()).unwrap();
    let first = view.state().messages[1].id.clone();
    futures_executor::block_on(view.reload(&first).unwrap()).unwrap();
    let (cursor, second) = {
        let changes = view.state_changes_since(None);
        assert_eq!(changes.state.messages[1].branch_number, 2);
        (changes.cursor, changes.state.messages[1].id.clone())
    };
    view.switch_branch(&second, false).unwrap();
    let changed = view.state_changes_since(Some(&cursor));
    assert!(changed.indices.is_none());
    assert_eq!(changed.state.messages.len(), 2);
    assert_eq!(changed.state.messages[1].id, first);
    assert_eq!(changed.state.messages[1].branch_number, 1);
}
