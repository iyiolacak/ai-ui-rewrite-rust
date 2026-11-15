//! Frontend tools: execution, approval pause and resume, denial, cancellation.

use ai_ui_core::adapter::{self, RunStream};
use ai_ui_core::*;
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

const NEEDS_TOOLS: MessageStatus = MessageStatus::RequiresAction {
    reason: RequiresActionReason::ToolCalls,
};

/// Step one asks for `lookup`; later steps answer with the tool's result.
fn model(seen: Rc<RefCell<Vec<Value>>>) -> impl ChatModelAdapter {
    move |options: ChatModelRunOptions| -> RunStream {
        let result = options
            .message
            .tool_calls()
            .next()
            .and_then(|c| c.result.clone());
        seen.borrow_mut()
            .push(result.clone().unwrap_or(Value::Null));
        match result {
            None => adapter::from_results([Ok(ChatModelRunResult::content(vec![
                AssistantMessagePart::ToolCall(ToolCallPart {
                    tool_call_id: "c1".into(),
                    tool_name: "lookup".into(),
                    args: serde_json::from_value(json!({ "q": "rust" })).unwrap(),
                    args_text: r#"{"q":"rust"}"#.into(),
                    ..Default::default()
                }),
            ])
            .with_status(NEEDS_TOOLS))]),
            Some(result) => {
                adapter::from_results([Ok(ChatModelRunResult::text(format!("Answer: {result}")))])
            }
        }
    }
}

fn poll(run: &mut RunFuture) -> Option<Result<(), RunError>> {
    match run.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(value) => Some(value),
        Poll::Pending => None,
    }
}

fn runtime(tool: Tool) -> (LocalThreadRuntime, Rc<RefCell<Vec<Value>>>) {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let runtime = LocalThreadRuntime::new(model(seen.clone()), LocalRuntimeOptions::default());
    runtime.set_tools(ToolRegistry::new().with("lookup", tool));
    (runtime, seen)
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

fn counting(calls: Rc<Cell<u32>>) -> Tool {
    Tool::new(json!({ "type": "object" }))
        .description("Look something up")
        .execute(move |args, _| {
            calls.set(calls.get() + 1);
            async move { Ok(json!({ "found": args["q"] })) }
        })
}

#[test]
fn a_registered_tool_runs_and_its_result_reaches_the_model() {
    let calls = Rc::new(Cell::new(0));
    let (runtime, seen) = runtime(counting(calls.clone()));
    let mut run = runtime
        .append(AppendMessage::user_text(None, "Find rust"))
        .unwrap();
    assert_eq!(poll(&mut run), Some(Ok(())));
    assert_eq!(calls.get(), 1);
    let message = last(&runtime);
    assert_eq!(
        message.tool_calls().next().unwrap().result,
        Some(json!({ "found": "rust" }))
    );
    assert_eq!(runtime.messages()[1].text(), r#"Answer: {"found":"rust"}"#);
    assert_eq!(message.status, MessageStatus::complete());
    assert_eq!(*seen.borrow(), [Value::Null, json!({ "found": "rust" })]);
}

#[test]
fn definitions_reach_the_model_context() {
    let names = Rc::new(RefCell::new(None));
    let probe = LocalThreadRuntime::new(
        {
            let names = names.clone();
            move |o: ChatModelRunOptions| -> RunStream {
                *names.borrow_mut() = Some(o.context.tools.keys().cloned().collect::<Vec<_>>());
                adapter::from_results([Ok(ChatModelRunResult::text("x"))])
            }
        },
        LocalRuntimeOptions::default(),
    );
    probe.set_tools(ToolRegistry::new().with("lookup", counting(Rc::default())));
    let mut run = probe.append(AppendMessage::user_text(None, "hi")).unwrap();
    poll(&mut run);
    assert_eq!(names.borrow().clone(), Some(vec!["lookup".to_owned()]));
}

#[test]
fn an_approval_pauses_the_run_until_the_user_decides() {
    let calls = Rc::new(Cell::new(0));
    let (runtime, _) = runtime(
        counting(calls.clone())
            .requires_approval()
            .approval_prompt(|name, args| format!("Run {name} for {}?", args["q"])),
    );
    let mut run = runtime
        .append(AppendMessage::user_text(None, "Find rust"))
        .unwrap();
    assert_eq!(
        poll(&mut run),
        Some(Ok(())),
        "the run ends while waiting for the user"
    );
    assert!(!runtime.is_running());
    assert_eq!(calls.get(), 0, "nothing ran before approval");
    let message = last(&runtime);
    let approval = message
        .tool_calls()
        .next()
        .unwrap()
        .approval
        .clone()
        .unwrap();
    assert!(approval.is_pending());
    assert_eq!(
        approval.prompt.as_deref(),
        Some(r#"Run lookup for "rust"?"#)
    );
    assert_eq!(message.status, NEEDS_TOOLS);

    let id = message.id.clone();
    let mut resumed = runtime
        .respond_to_tool_approval(&id, &approval.id, true, None)
        .unwrap()
        .expect("resumes the message");
    assert_eq!(poll(&mut resumed), Some(Ok(())));
    assert_eq!(calls.get(), 1);
    assert_eq!(runtime.messages()[1].text(), r#"Answer: {"found":"rust"}"#);
    assert_eq!(last(&runtime).status, MessageStatus::complete());
    assert_eq!(
        runtime
            .respond_to_tool_approval(&id, &approval.id, true, None)
            .err(),
        Some(RuntimeError::ApprovalNotFound(approval.id)),
        "a decided approval cannot be answered again"
    );
}

#[test]
fn a_denied_tool_never_runs_and_the_model_hears_about_it() {
    let calls = Rc::new(Cell::new(0));
    let (runtime, seen) = runtime(counting(calls.clone()).requires_approval());
    let mut run = runtime
        .append(AppendMessage::user_text(None, "Find rust"))
        .unwrap();
    poll(&mut run);
    let message = last(&runtime);
    let approval = message
        .tool_calls()
        .next()
        .unwrap()
        .approval
        .clone()
        .unwrap();
    let mut resumed = runtime
        .respond_to_tool_approval(&message.id, &approval.id, false, Some("No thanks".into()))
        .unwrap()
        .unwrap();
    assert_eq!(poll(&mut resumed), Some(Ok(())));
    assert_eq!(calls.get(), 0);
    let call = last(&runtime).tool_calls().next().unwrap().clone();
    assert_eq!(
        (call.result, call.is_error),
        (Some(json!({ "error": "No thanks" })), Some(true))
    );
    assert_eq!(seen.borrow().last(), Some(&json!({ "error": "No thanks" })));
}

#[test]
fn cancelling_while_a_tool_runs_stops_it() {
    let tool = Tool::new(json!({})).execute(|_, context| async move {
        // Never finishes by itself.
        context.abort_signal.aborted().await;
        std::future::pending::<Result<Value, String>>().await
    });
    let (runtime, _) = runtime(tool);
    let mut run = runtime
        .append(AppendMessage::user_text(None, "go"))
        .unwrap();
    assert_eq!(poll(&mut run), None, "waiting on the tool");
    runtime.cancel_run();
    assert_eq!(poll(&mut run), Some(Ok(())));
    assert_eq!(last(&runtime).status, MessageStatus::cancelled());
    assert!(last(&runtime).tool_calls().next().unwrap().is_open());
    assert!(!runtime.is_running());
}

#[test]
fn dropping_a_pending_tool_run_aborts_the_tool_and_settles_the_message() {
    let signal = Rc::new(RefCell::new(None));
    let tool = Tool::new(json!({})).execute({
        let signal = signal.clone();
        move |_, context| {
            *signal.borrow_mut() = Some(context.abort_signal);
            std::future::pending::<Result<Value, String>>()
        }
    });
    let (runtime, seen) = runtime(tool);
    let mut run = runtime
        .append(AppendMessage::user_text(None, "go"))
        .unwrap();
    assert_eq!(poll(&mut run), None, "waiting on the tool");
    assert!(!signal.borrow().as_ref().unwrap().is_aborted());

    drop(run);

    assert!(signal.borrow().as_ref().unwrap().is_aborted());
    assert!(!runtime.is_running());
    assert_eq!(last(&runtime).status, MessageStatus::cancelled());
    assert!(last(&runtime).tool_calls().next().unwrap().is_open());
    assert_eq!(seen.borrow().len(), 1, "no continuation after cancellation");
}

#[test]
fn dropping_an_unpolled_approval_resume_never_executes_the_tool() {
    let calls = Rc::new(Cell::new(0));
    let (runtime, _) = runtime(counting(calls.clone()).requires_approval());
    let mut run = runtime
        .append(AppendMessage::user_text(None, "go"))
        .unwrap();
    assert_eq!(poll(&mut run), Some(Ok(())));
    let message = last(&runtime);
    let approval = message
        .tool_calls()
        .next()
        .unwrap()
        .approval
        .as_ref()
        .unwrap();
    let resumed = runtime
        .respond_to_tool_approval(&message.id, &approval.id, true, None)
        .unwrap()
        .unwrap();
    assert!(runtime.is_running());

    drop(resumed);

    assert_eq!(calls.get(), 0);
    assert!(!runtime.is_running());
    assert_eq!(last(&runtime).status, MessageStatus::cancelled());
}
