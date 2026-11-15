//! The run loop: one or more model steps for one assistant message, ported
//! from `_runLoop` and `performRoundtrip`.
//!
//! No `RefCell` borrow is held across an `.await`, and subscribers are notified
//! only after each borrow ends.

use super::continuation::{should_continue, with_continuation, with_external_results};
use super::tools::{approval_prompt, ToolContext};
use super::{Shared, State};
use crate::abort::{AbortController, AbortSignal};
use crate::adapter::{ChatModelRunOptions, ChatModelRunResult, RunError, RunStream};
use crate::types::{
    AssistantMessage, AssistantMessagePart, AssistantMetadata, IncompleteReason, MessageId,
    MessageStatus, RequiresActionReason, RunConfig, ThreadMessage,
};
use std::future::{poll_fn, Future as _};
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::task::Poll;

use crate::types::{ToolApproval, ToolCallPart};
use serde_json::{json, Value};

pub(super) fn run_loop(
    shared: Rc<Shared>,
    run: u64,
    parent: Option<MessageId>,
    message_id: MessageId,
    run_config: RunConfig,
    // Resuming after an approval: run the decided tools before the next step.
    resume_tools: bool,
) -> impl std::future::Future<Output = Result<(), RunError>> {
    // Construct the guard before returning the future: an executor may reject
    // or drop a task without polling it, after the runtime has become active.
    let guard = DropGuard {
        shared: shared.clone(),
        run,
        parent: parent.clone(),
        message_id: message_id.clone(),
        armed: true,
    };
    async move {
        let mut guard = guard;
        let mut steps_started = 0;
        let mut skip_roundtrip = resume_tools;
        let result = loop {
            if !skip_roundtrip {
                if let Err(error) = roundtrip(
                    &shared,
                    run,
                    parent.as_ref(),
                    &message_id,
                    &run_config,
                    &mut steps_started,
                )
                .await
                {
                    break Err(error);
                }
            }
            skip_roundtrip = false;
            if !shared.is_active(run) {
                break Ok(());
            }
            if !tools_step(&shared, run, parent.as_ref(), &message_id).await {
                break Ok(());
            }
            let continues = {
                let state = shared.state.borrow();
                state.repository.get(&message_id).ok().is_some_and(|entry| {
                    entry.message.as_assistant().is_some_and(|m| {
                        should_continue(m, shared.options.human_tool_names.as_deref())
                    }) && !state.repository.has_children(&message_id)
                })
            };
            if !continues {
                break Ok(());
            }
        };
        guard.armed = false;
        let ended = {
            let mut state = shared.state.borrow_mut();
            let ours = state.active.as_ref().is_some_and(|a| a.id == run);
            if ours {
                state.active = None;
            }
            ours
        };
        if ended {
            shared.notify();
        }
        result
    }
}

/// Ends the run if the host drops its future before it finishes: the message,
/// if still running, becomes cancelled and the runtime is no longer running.
struct DropGuard {
    shared: Rc<Shared>,
    run: u64,
    parent: Option<MessageId>,
    message_id: MessageId,
    armed: bool,
}

impl Drop for DropGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Ok(mut state) = self.shared.state.try_borrow_mut() else {
            return;
        };
        if state.active.as_ref().is_none_or(|a| a.id != self.run) {
            return;
        }
        let controller = state.active.take().and_then(|active| active.controller);
        if let Some(mut message) = state
            .repository
            .get(&self.message_id)
            .ok()
            .and_then(|e| e.message.as_assistant().cloned())
        {
            if matches!(
                message.status,
                MessageStatus::Running
                    | MessageStatus::RequiresAction {
                        reason: RequiresActionReason::ToolCalls
                    }
            ) {
                message.status = MessageStatus::cancelled();
                let _ = state
                    .repository
                    .add_or_update(self.parent.as_ref(), ThreadMessage::Assistant(message));
            }
        }
        drop(state);
        // Waking adapter/tool work may run host code; never do it while the
        // runtime state is borrowed, or reach a replacement run's controller.
        if let Some(controller) = controller {
            controller.abort();
        }
        self.shared.notify();
    }
}

/// One model step. Returns `Err` only for an adapter failure.
async fn roundtrip(
    shared: &Rc<Shared>,
    run: u64,
    parent: Option<&MessageId>,
    message_id: &MessageId,
    run_config: &RunConfig,
    steps_started: &mut usize,
) -> Result<(), RunError> {
    let Some(start) = begin_step(shared, run, parent, message_id, run_config, *steps_started)
    else {
        return Ok(());
    };
    shared.notify();
    let Step::Started {
        options,
        controller,
        initial,
    } = start
    else {
        return Ok(());
    };
    *steps_started += 1;
    // Enable the O(1) tail overlay for the streaming loop. Must be called
    // before the first `apply` so `replace_in_path` hits the overlay lane.
    {
        let state = shared.state.borrow();
        state.repository.begin_overlay();
    }
    let signal = controller.signal();
    let mut stream = shared.adapter.run(options);

    let outcome = loop {
        match next(&signal, &mut stream).await {
            Next::Aborted => break Ok(()),
            Next::Item(Some(Ok(result))) => {
                if !apply(shared, parent, message_id, &initial, result) {
                    break Ok(());
                }
                shared.notify();
            }
            Next::Item(Some(Err(RunError::Aborted))) => {
                set_status(shared, parent, message_id, MessageStatus::cancelled());
                break Ok(());
            }
            Next::Item(Some(Err(error))) => {
                let status = MessageStatus::Incomplete {
                    reason: IncompleteReason::Error,
                    error: error.status_error(),
                };
                set_status(shared, parent, message_id, status);
                break Err(error);
            }
            Next::Item(None) => break Ok(()),
        }
    };
    drop(stream);
    // Drop the overlay — the next `messages()` rebuilds from `nodes`.
    {
        let state = shared.state.borrow();
        state.repository.end_overlay();
    }

    if outcome.is_ok() {
        let status = current(shared, message_id).map(|m| m.status);
        if should_cancel(shared, run, &signal, message_id) {
            set_status(shared, parent, message_id, MessageStatus::cancelled());
        } else if status == Some(MessageStatus::Running) {
            set_status(shared, parent, message_id, MessageStatus::complete());
        }
    }
    {
        let mut state = shared.state.borrow_mut();
        if let Some(active) = state.active.as_mut().filter(|a| a.id == run) {
            active.controller = None;
        }
    }
    shared.notify();
    outcome
}

// Transient: built and matched once per step.
#[allow(clippy::large_enum_variant)]
enum Step {
    Started {
        options: ChatModelRunOptions,
        controller: AbortController,
        initial: Initial,
    },
    /// Cancelled before starting, or the step limit was reached; the message
    /// status now records which.
    Stopped,
}

/// Prepare a step under one borrow: install its abort controller, mark the
/// message running, and snapshot the model's input.
fn begin_step(
    shared: &Shared,
    run: u64,
    parent: Option<&MessageId>,
    message_id: &MessageId,
    run_config: &RunConfig,
    steps_started: usize,
) -> Option<Step> {
    let mut state = shared.state.borrow_mut();
    let State {
        repository,
        active,
        model_context,
        ..
    } = &mut *state;
    let active = active.as_mut().filter(|a| a.id == run)?;
    let mut message = repository
        .get(message_id)
        .ok()?
        .message
        .as_assistant()?
        .clone();
    if active.cancelled {
        message.status = MessageStatus::cancelled();
        repository
            .add_or_update(parent, ThreadMessage::Assistant(message))
            .ok()?;
        return Some(Step::Stopped);
    }
    let steps = message.metadata.steps.len().max(steps_started);
    if steps >= shared.options.max_steps {
        message.status = MessageStatus::Incomplete {
            reason: IncompleteReason::ToolCalls,
            error: None,
        };
        repository
            .add_or_update(parent, ThreadMessage::Assistant(message))
            .ok()?;
        return Some(Step::Stopped);
    }
    let messages: Arc<[Arc<ThreadMessage>]> = match parent {
        Some(parent) => repository.messages_to(parent).ok()?.into(),
        None => Arc::from(Vec::new()),
    };
    let controller = AbortController::new();
    active.controller = Some(controller.clone());
    let initial = Initial {
        content: message.content.clone(),
        metadata: message.metadata.clone(),
    };
    let options = ChatModelRunOptions {
        messages,
        run_config: run_config.clone(),
        abort_signal: controller.signal(),
        context: model_context.clone(),
        assistant_message_id: message_id.clone(),
        thread_id: shared.options.thread_id.clone(),
        parent_id: parent.cloned(),
        message: message.clone(),
    };
    message.status = MessageStatus::Running;
    repository
        .add_or_update(parent, ThreadMessage::Assistant(message))
        .ok()?;
    Some(Step::Started {
        options,
        controller,
        initial,
    })
}

/// What the message looked like when the step began; adapter results build on it.
struct Initial {
    content: Vec<AssistantMessagePart>,
    metadata: AssistantMetadata,
}

fn current(shared: &Shared, id: &MessageId) -> Option<AssistantMessage> {
    shared
        .state
        .borrow()
        .repository
        .get(id)
        .ok()?
        .message
        .as_assistant()
        .cloned()
}

/// Merge one cumulative adapter result. Returns false when the message is gone.
fn apply(
    shared: &Shared,
    parent: Option<&MessageId>,
    id: &MessageId,
    initial: &Initial,
    result: ChatModelRunResult,
) -> bool {
    let mut state = shared.state.borrow_mut();
    let Some(mut message) = state
        .repository
        .get(id)
        .ok()
        .and_then(|e| e.message.as_assistant().cloned())
    else {
        return false;
    };
    let base = &initial.metadata;
    if let Some(content) = result.content {
        let merged = with_continuation(&initial.content, content);
        message.content = with_external_results(&message.content, merged);
    }
    if let Some(status) = result.status {
        message.status = status;
    }
    if let Some(update) = result.metadata {
        let metadata = &mut message.metadata;
        if let Some(state) = update.state {
            metadata.state = state;
        }
        if let Some(annotations) = update.annotations {
            metadata.annotations = [base.annotations.clone(), annotations].concat();
        }
        if let Some(data) = update.data {
            metadata.data = [base.data.clone(), data].concat();
        }
        if let Some(steps) = update.steps {
            metadata.steps = [base.steps.clone(), steps].concat();
        }
        if let Some(timing) = update.timing {
            metadata.timing = Some(timing);
        }
        if let Some(custom) = update.custom {
            let mut merged = base.custom.clone();
            merged.extend(custom);
            metadata.custom = merged;
        }
    }
    state
        .repository
        .add_or_update(parent, ThreadMessage::Assistant(message))
        .is_ok()
}

fn set_status(shared: &Shared, parent: Option<&MessageId>, id: &MessageId, status: MessageStatus) {
    let mut state = shared.state.borrow_mut();
    if let Some(mut message) = state
        .repository
        .get(id)
        .ok()
        .and_then(|e| e.message.as_assistant().cloned())
    {
        if message.status != status {
            message.status = status;
            let _ = state
                .repository
                .add_or_update(parent, ThreadMessage::Assistant(message));
        }
    }
}

/// Cancelled if aborted while running, or while paused for tool results that
/// this run would otherwise have continued with.
fn should_cancel(shared: &Shared, run: u64, signal: &AbortSignal, id: &MessageId) -> bool {
    if !signal.is_aborted() {
        return false;
    }
    let Some(message) = current(shared, id) else {
        return false;
    };
    match message.status {
        MessageStatus::Running => true,
        MessageStatus::RequiresAction {
            reason: RequiresActionReason::ToolCalls,
        } => {
            !shared.is_active(run)
                || should_continue(&message, shared.options.human_tool_names.as_deref())
        }
        _ => false,
    }
}

// Transient: one value per adapter poll.
#[allow(clippy::large_enum_variant)]
enum Next {
    Aborted,
    Item(Option<Result<ChatModelRunResult, RunError>>),
}

/// The next adapter result, or `Aborted` as soon as the signal fires.
async fn next(signal: &AbortSignal, stream: &mut RunStream) -> Next {
    let mut aborted = signal.aborted();
    poll_fn(|cx| {
        if Pin::new(&mut aborted).poll(cx).is_ready() {
            return Poll::Ready(Next::Aborted);
        }
        stream.as_mut().poll_next(cx).map(Next::Item)
    })
    .await
}

// ---------------------------------------------------------------- frontend tools

enum Action {
    /// A pending approval was just added to a call.
    Approval,
    Deny {
        call_id: String,
        reason: String,
    },
    Run {
        call_id: String,
        tool: super::tools::Tool,
        args: crate::types::JsonObject,
    },
}

/// The call's arguments; falls back to the streamed text when the parsed
/// object is empty.
fn arguments(call: &ToolCallPart) -> crate::types::JsonObject {
    if call.args.is_empty() && !call.args_text.is_empty() {
        if let Ok(Value::Object(args)) = serde_json::from_str(&call.args_text) {
            return args;
        }
    }
    call.args.clone()
}

/// Whether the streamed argument text is complete JSON (or there is none, for a call without arguments).
fn arguments_complete(call: &crate::types::ToolCallPart) -> bool {
    call.args_text.trim().is_empty() || serde_json::from_str::<Value>(&call.args_text).is_ok()
}

/// Decide what to do next for the message's open calls to registered tools.
/// Requesting approval is applied here; running and denying are returned.
fn next_action(shared: &Shared, parent: Option<&MessageId>, id: &MessageId) -> Option<Action> {
    let mut state = shared.state.borrow_mut();
    let tools = state.tools.clone();
    let mut message = state
        .repository
        .get(id)
        .ok()?
        .message
        .as_assistant()?
        .clone();
    let mut request = None;
    let mut action = None;
    for (index, part) in message.content.iter().enumerate() {
        let AssistantMessagePart::ToolCall(call) = part else {
            continue;
        };
        if !call.is_open() {
            continue;
        }
        let Some(tool) = tools.get(&call.tool_name).filter(|t| t.execute.is_some()) else {
            continue;
        };
        if !arguments_complete(call) {
            // The response ended inside the arguments. Whatever a parser could repair them to, the
            // model never said it, so the tool does not run and the user is not asked to approve it.
            action = Some(Action::Deny {
                call_id: call.tool_call_id.clone(),
                reason: "The tool call's arguments were cut off, so the tool was not run.".into(),
            });
            break;
        }
        match &call.approval {
            Some(a) if a.is_pending() => continue,
            Some(a) if a.approved == Some(false) => {
                let reason = a
                    .reason
                    .clone()
                    .filter(|r| !r.is_empty())
                    .unwrap_or_else(|| "Tool approval denied".into());
                action = Some(Action::Deny {
                    call_id: call.tool_call_id.clone(),
                    reason,
                });
            }
            Some(a) if a.resolution.is_some() => continue,
            Some(_) => {
                action = Some(Action::Run {
                    call_id: call.tool_call_id.clone(),
                    tool: tool.clone(),
                    args: arguments(call),
                });
            }
            None if tool.requires_approval => {
                request = Some((
                    index,
                    approval_prompt(tool, &call.tool_name, &arguments(call)),
                ))
            }
            None => {
                action = Some(Action::Run {
                    call_id: call.tool_call_id.clone(),
                    tool: tool.clone(),
                    args: arguments(call),
                });
            }
        }
        break;
    }
    if let Some((index, prompt)) = request {
        if let AssistantMessagePart::ToolCall(call) = &mut message.content[index] {
            call.approval = Some(ToolApproval {
                id: format!("approval-{}", call.tool_call_id),
                prompt: Some(prompt),
                ..Default::default()
            });
        }
        state
            .repository
            .add_or_update(parent, ThreadMessage::Assistant(message))
            .ok()?;
        return Some(Action::Approval);
    }
    action
}

/// Store a tool's outcome on its call.
fn record(
    shared: &Shared,
    parent: Option<&MessageId>,
    id: &MessageId,
    call_id: &str,
    result: Value,
    is_error: bool,
) {
    let mut state = shared.state.borrow_mut();
    let Some(mut message) = state
        .repository
        .get(id)
        .ok()
        .and_then(|e| e.message.as_assistant().cloned())
    else {
        return;
    };
    for part in &mut message.content {
        if let AssistantMessagePart::ToolCall(call) = part {
            if call.tool_call_id == call_id {
                call.result = Some(result.clone());
                call.is_error = Some(is_error);
                call.is_preliminary = None;
            }
        }
    }
    let _ = state
        .repository
        .add_or_update(parent, ThreadMessage::Assistant(message));
}

fn install_controller(shared: &Shared, run: u64) -> Option<AbortController> {
    let mut state = shared.state.borrow_mut();
    let active = state.active.as_mut().filter(|a| a.id == run)?;
    if active.cancelled {
        return None;
    }
    let controller = AbortController::new();
    active.controller = Some(controller.clone());
    Some(controller)
}

/// Run `future` until it finishes or the signal aborts (`None`).
async fn until_aborted<T>(
    signal: &AbortSignal,
    mut future: Pin<Box<dyn std::future::Future<Output = T>>>,
) -> Option<T> {
    let mut aborted = signal.aborted();
    poll_fn(|cx| {
        if Pin::new(&mut aborted).poll(cx).is_ready() {
            return Poll::Ready(None);
        }
        future.as_mut().poll(cx).map(Some)
    })
    .await
}

/// Execute the message's open calls to registered tools, asking for approval
/// where required. Returns false when the run was cancelled.
async fn tools_step(
    shared: &Rc<Shared>,
    run: u64,
    parent: Option<&MessageId>,
    id: &MessageId,
) -> bool {
    while let Some(action) = next_action(shared, parent, id) {
        match action {
            Action::Approval => shared.notify(),
            Action::Deny { call_id, reason } => {
                record(
                    shared,
                    parent,
                    id,
                    &call_id,
                    json!({ "error": reason }),
                    true,
                );
                shared.notify();
            }
            Action::Run {
                call_id,
                tool,
                args,
            } => {
                let Some(controller) = install_controller(shared, run) else {
                    set_status(shared, parent, id, MessageStatus::cancelled());
                    shared.notify();
                    return false;
                };
                let signal = controller.signal();
                let execute = tool.execute.clone().expect("filtered on execute");
                let context = ToolContext {
                    tool_call_id: call_id.clone(),
                    abort_signal: signal.clone(),
                };
                let outcome = until_aborted(&signal, execute(args, context)).await;
                if let Some(active) = shared
                    .state
                    .borrow_mut()
                    .active
                    .as_mut()
                    .filter(|a| a.id == run)
                {
                    active.controller = None;
                }
                match outcome {
                    None => {
                        set_status(shared, parent, id, MessageStatus::cancelled());
                        shared.notify();
                        return false;
                    }
                    Some(Ok(value)) => record(shared, parent, id, &call_id, value, false),
                    Some(Err(error)) => {
                        record(shared, parent, id, &call_id, Value::String(error), true)
                    }
                }
                shared.notify();
            }
        }
    }
    true
}
