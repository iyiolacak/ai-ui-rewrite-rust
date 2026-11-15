//! In-memory thread runtime driving a [`ChatModelAdapter`], ported from
//! `runtimes/local/local-thread-runtime-core.ts`.
//!
//! Methods change state synchronously and notify subscribers before they
//! return; a method that starts a model run also returns a [`RunFuture`] that
//! the host spawns on its executor (for example `slint::spawn_local`). Dropping
//! that future cancels its run, even before the first poll, and aborts any
//! in-flight adapter or tool through its signal.

mod composer;
mod continuation;
mod run;
mod tools;

pub use composer::Composer;
pub use tools::{Tool, ToolContext, ToolFuture, ToolRegistry};

pub use continuation::{should_continue, with_continuation, with_external_results};

use crate::abort::AbortController;
use crate::adapter::{ChatModelAdapter, ModelContext};
use crate::repository::{ExportedRepository, MessageChanges, MessageEntry, MessageRepository};
use crate::runtime::{
    ComposerHost, RunFuture, RuntimeCapabilities, RuntimeError, StartRunConfig, ThreadRuntime,
    ToolResult,
};
use crate::subscribe::{Subscribers, Subscription};
use crate::types::{
    AppendContent, AppendMessage, AssistantMessage, AssistantMetadata, MessageId, MessageMetadata,
    MessageStatus, RequiresActionReason, RunConfig, SystemMessage, SystemMessagePart, TextPart,
    ThreadMessage, Timestamp, UserMessage,
};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::Arc;

/// Construction options (`LocalRuntimeOptions`).
#[derive(Clone)]
pub struct LocalRuntimeOptions {
    /// Model steps per run before the run stops with pending tool calls
    /// (`maxSteps`, default 2). Steps are the adapter's reported `steps` or,
    /// if it reports none, the number of steps the runtime started.
    pub max_steps: usize,
    /// Tool calls the frontend answers (`unstable_humanToolNames`); see
    /// [`should_continue`].
    pub human_tool_names: Option<Vec<String>>,
    /// Passed to the adapter as `thread_id`.
    pub thread_id: Option<String>,
    /// Message timestamps.
    pub clock: Rc<dyn Fn() -> Timestamp>,
    /// Message ids; the runtime skips any id already stored.
    pub ids: Rc<dyn Fn() -> MessageId>,
}

impl LocalRuntimeOptions {
    /// Options with an explicit clock and ids derived from it.
    pub fn with_clock(clock: Rc<dyn Fn() -> Timestamp>) -> Self {
        let prefix = format!("{:x}", clock().0);
        let counter = Cell::new(0u64);
        Self {
            max_steps: 2,
            human_tool_names: None,
            thread_id: None,
            clock,
            ids: Rc::new(move || {
                counter.set(counter.get() + 1);
                MessageId(format!("{prefix}-{}", counter.get()))
            }),
        }
    }
}

/// Uses the system clock, which `wasm32-unknown-unknown` lacks: browser hosts
/// use [`LocalRuntimeOptions::with_clock`] with `Date.now()`.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
impl Default for LocalRuntimeOptions {
    fn default() -> Self {
        Self::with_clock(Rc::new(|| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            Timestamp(now.as_millis() as u64)
        }))
    }
}

struct ActiveRun {
    id: u64,
    /// The current step's controller, while a step is in flight.
    controller: Option<AbortController>,
    /// Cancelled before a step started; the next step does not start.
    cancelled: bool,
}

struct State {
    repository: MessageRepository,
    active: Option<ActiveRun>,
    next_run: u64,
    model_context: ModelContext,
    tools: Rc<ToolRegistry>,
}

pub(crate) struct Shared {
    me: Weak<Shared>,
    state: RefCell<State>,
    adapter: Rc<dyn ChatModelAdapter>,
    options: LocalRuntimeOptions,
    subscribers: Rc<Subscribers>,
    thread_composer: RefCell<Option<Composer>>,
    edits: RefCell<HashMap<MessageId, Composer>>,
}

impl Shared {
    /// Notify runtime subscribers, then composers (whose `can_send` and
    /// `can_cancel` can depend on runtime state).
    fn notify(&self) {
        self.subscribers.notify();
        let composers: Vec<Composer> = self
            .thread_composer
            .borrow()
            .iter()
            .cloned()
            .chain(self.edits.borrow().values().cloned())
            .collect();
        for composer in composers {
            composer.notify();
        }
    }
    fn is_active(&self, run: u64) -> bool {
        self.state
            .borrow()
            .active
            .as_ref()
            .is_some_and(|a| a.id == run)
    }
    fn fresh_id(&self, repository: &MessageRepository) -> MessageId {
        loop {
            let id = (self.options.ids)();
            if !repository.contains(&id) {
                return id;
            }
        }
    }
}

/// A single thread with its branches, driving one model adapter.
#[derive(Clone)]
pub struct LocalThreadRuntime {
    shared: Rc<Shared>,
}

impl LocalThreadRuntime {
    /// A runtime that runs `adapter` for each message sent.
    pub fn new(adapter: impl ChatModelAdapter + 'static, options: LocalRuntimeOptions) -> Self {
        Self {
            shared: Rc::new_cyclic(|me| Shared {
                me: me.clone(),
                state: RefCell::new(State {
                    repository: MessageRepository::new(),
                    active: None,
                    next_run: 0,
                    model_context: ModelContext::default(),
                    tools: Rc::default(),
                }),
                adapter: Rc::new(adapter),
                options,
                subscribers: Rc::default(),
                thread_composer: RefCell::new(None),
                edits: RefCell::new(HashMap::new()),
            }),
        }
    }

    /// Called after every change. Drop the guard to unsubscribe.
    pub fn subscribe(&self, callback: impl Fn() + 'static) -> Subscription {
        self.shared.subscribers.subscribe(callback)
    }

    /// The selected branch, root first. Cheap to call; cached between changes.
    pub fn messages(&self) -> Arc<[Arc<ThreadMessage>]> {
        self.shared.state.borrow().repository.messages()
    }
    /// Whether a run is in progress.
    pub fn is_running(&self) -> bool {
        self.shared.state.borrow().active.is_some()
    }
    /// A message and its position, by id.
    pub fn message(&self, id: &MessageId) -> Result<MessageEntry, RuntimeError> {
        Ok(self.shared.state.borrow().repository.get(id)?)
    }
    /// The message and its siblings, for a branch picker.
    pub fn branches(&self, id: &MessageId) -> Result<Vec<MessageId>, RuntimeError> {
        Ok(self.shared.state.borrow().repository.branches(id)?.to_vec())
    }
    /// Select the branch containing `id`; subscribers are notified.
    pub fn switch_to_branch(&self, id: &MessageId) -> Result<(), RuntimeError> {
        self.shared
            .state
            .borrow_mut()
            .repository
            .switch_to_branch(id)?;
        self.shared.notify();
        Ok(())
    }
    /// The composer for the next message (`thread.composer`).
    pub fn composer(&self) -> Composer {
        self.shared
            .thread_composer
            .borrow_mut()
            .get_or_insert_with(|| {
                Composer::thread(Rc::downgrade(&self.shared) as Weak<dyn ComposerHost>)
            })
            .clone()
    }

    /// Start editing a message (`beginEdit`). Sending the edit composer adds
    /// a sibling branch; cancelling discards it.
    pub fn begin_edit(&self, id: &MessageId) -> Result<Composer, RuntimeError> {
        if self.shared.edits.borrow().contains_key(id) {
            return Err(RuntimeError::EditInProgress(id.clone()));
        }
        let entry = self.message(id)?;
        let composer = Composer::edit(
            Rc::downgrade(&self.shared) as Weak<dyn ComposerHost>,
            entry.parent_id,
            &entry.message,
        );
        self.shared
            .edits
            .borrow_mut()
            .insert(id.clone(), composer.clone());
        self.shared.notify();
        Ok(composer)
    }

    /// The edit composer for a message, while it is being edited.
    pub fn edit_composer(&self, id: &MessageId) -> Option<Composer> {
        self.shared.edits.borrow().get(id).cloned()
    }

    /// Register the tools this thread offers. Their definitions are added to
    /// the model context (replacing earlier tool definitions), and open calls
    /// of tools with an `execute` function run in the app after each step.
    pub fn set_tools(&self, tools: ToolRegistry) {
        let mut state = self.shared.state.borrow_mut();
        state.model_context.tools = tools.definitions();
        state.tools = Rc::new(tools);
    }

    /// Record the user's answer to a pending tool approval (`respond to a
    /// tool approval`). An approved call runs; a denied one answers the model
    /// with an error. Returns the run that continues the message, if none is
    /// in progress, for the host to spawn.
    pub fn respond_to_tool_approval(
        &self,
        message_id: &MessageId,
        approval_id: &str,
        approved: bool,
        reason: Option<String>,
    ) -> Result<Option<RunFuture>, RuntimeError> {
        let (resume, parent) = {
            let mut state = self.shared.state.borrow_mut();
            let entry = state.repository.get(message_id)?;
            let Some(assistant) = entry.message.as_assistant() else {
                return Err(RuntimeError::NotAssistantMessage(message_id.clone()));
            };
            let mut updated = assistant.clone();
            let approval = updated
                .content
                .iter_mut()
                .filter_map(|p| match p {
                    crate::types::AssistantMessagePart::ToolCall(c) => c.approval.as_mut(),
                    _ => None,
                })
                .find(|a| a.id == approval_id && a.is_pending())
                .ok_or_else(|| RuntimeError::ApprovalNotFound(approval_id.to_owned()))?;
            approval.approved = Some(approved);
            approval.reason = reason;
            let paused = matches!(
                updated.status,
                MessageStatus::RequiresAction {
                    reason: RequiresActionReason::ToolCalls
                }
            );
            let ready =
                paused && state.active.is_none() && !state.repository.has_children(message_id);
            state
                .repository
                .add_or_update(entry.parent_id.as_ref(), ThreadMessage::Assistant(updated))?;
            let resume = ready.then(|| Self::activate(&mut state));
            (resume, entry.parent_id)
        };
        self.shared.notify();
        Ok(resume.map(|run| -> RunFuture {
            Box::pin(run::run_loop(
                self.shared.clone(),
                run,
                parent,
                message_id.clone(),
                RunConfig::default(),
                true,
            ))
        }))
    }

    /// Replace the system prompt, tools and settings sent with later runs.
    pub fn set_model_context(&self, context: ModelContext) {
        self.shared.state.borrow_mut().model_context = context;
    }

    /// Record feedback on an assistant message (`submitFeedback`); `None`
    /// clears it.
    pub fn submit_feedback(
        &self,
        id: &MessageId,
        feedback: Option<crate::types::SubmittedFeedback>,
    ) -> Result<(), RuntimeError> {
        {
            let mut state = self.shared.state.borrow_mut();
            let entry = state.repository.get(id)?;
            let Some(assistant) = entry.message.as_assistant() else {
                return Err(RuntimeError::NotAssistantMessage(id.clone()));
            };
            let mut updated = assistant.clone();
            updated.metadata.submitted_feedback = feedback;
            state
                .repository
                .add_or_update(entry.parent_id.as_ref(), ThreadMessage::Assistant(updated))?;
        }
        self.shared.notify();
        Ok(())
    }

    /// Persisted messages and the selected head.
    pub fn export(&self) -> ExportedRepository {
        self.shared.state.borrow().repository.export()
    }
    /// Load history (for example from a thread history adapter).
    pub fn import(&self, exported: ExportedRepository) -> Result<(), RuntimeError> {
        {
            let mut state = self.shared.state.borrow_mut();
            if state.active.is_some() {
                return Err(RuntimeError::RunInProgress);
            }
            state.repository.import(exported)?;
        }
        self.shared.notify();
        Ok(())
    }

    /// Add a message. A user message (or `start_run: Some(true)`) starts a run
    /// answering it, cancelling any run in progress. Editing is an append with
    /// the edited message's parent and `source_id`.
    pub fn append(&self, message: AppendMessage) -> Result<RunFuture, RuntimeError> {
        let start = message.starts_run();
        let id = {
            let mut state = self.shared.state.borrow_mut();
            let id = self.shared.fresh_id(&state.repository);
            let created_at = (self.shared.options.clock)();
            let new = match message.content {
                AppendContent::User {
                    content,
                    attachments,
                    custom,
                } => ThreadMessage::User(UserMessage {
                    id: id.clone(),
                    created_at,
                    content,
                    attachments,
                    metadata: MessageMetadata {
                        custom,
                        ..Default::default()
                    },
                }),
                AppendContent::Assistant { content, custom } => {
                    ThreadMessage::Assistant(AssistantMessage {
                        id: id.clone(),
                        created_at,
                        content,
                        status: MessageStatus::complete(),
                        metadata: AssistantMetadata {
                            custom,
                            ..Default::default()
                        },
                    })
                }
                AppendContent::System { text } => ThreadMessage::System(SystemMessage {
                    id: id.clone(),
                    created_at,
                    content: vec![SystemMessagePart::Text(TextPart::new(text))],
                    metadata: MessageMetadata::default(),
                }),
            };
            state
                .repository
                .add_or_update(message.parent_id.as_ref(), new)?;
            if !start {
                state.repository.switch_to_branch(&id)?;
            }
            id
        };
        if !start {
            self.shared.notify();
            return Ok(Box::pin(async { Ok(()) }));
        }
        self.start_run(StartRunConfig {
            parent_id: Some(id),
            source_id: message.source_id,
            run_config: message.run_config,
        })
    }

    /// Add a running assistant message after `parent_id` and answer it: a new
    /// answer, or a regeneration when `source_id` names the previous one.
    pub fn start_run(&self, config: StartRunConfig) -> Result<RunFuture, RuntimeError> {
        let (run, message_id) = {
            let mut state = self.shared.state.borrow_mut();
            let id = self.shared.fresh_id(&state.repository);
            let message = AssistantMessage {
                id: id.clone(),
                created_at: (self.shared.options.clock)(),
                content: Vec::new(),
                status: MessageStatus::Running,
                metadata: AssistantMetadata::default(),
            };
            state
                .repository
                .add_or_update(config.parent_id.as_ref(), ThreadMessage::Assistant(message))?;
            state.repository.switch_to_branch(&id)?;
            (Self::activate(&mut state), id)
        };
        self.shared.notify();
        Ok(Box::pin(run::run_loop(
            self.shared.clone(),
            run,
            config.parent_id,
            message_id,
            config.run_config,
            false,
        )))
    }

    /// Cancel any run in progress; its message becomes `incomplete/cancelled`.
    pub fn cancel_run(&self) {
        let controller = self
            .shared
            .state
            .borrow_mut()
            .active
            .as_mut()
            .and_then(|a| {
                a.cancelled = true;
                a.controller.take()
            });
        if let Some(controller) = controller {
            controller.abort();
        }
    }

    /// Record a tool result. If that answers the last open call of a paused
    /// message, the run continues and its future is returned.
    pub fn add_tool_result(&self, result: ToolResult) -> Result<Option<RunFuture>, RuntimeError> {
        let (resume, parent) = {
            let mut state = self.shared.state.borrow_mut();
            let entry = state.repository.get(&result.message_id)?;
            let Some(assistant) = entry.message.as_assistant() else {
                return Err(RuntimeError::NotAssistantMessage(result.message_id));
            };
            let mut updated = assistant.clone();
            let mut added = false;
            let call = updated
                .content
                .iter_mut()
                .filter_map(|p| match p {
                    crate::types::AssistantMessagePart::ToolCall(c) => Some(c),
                    _ => None,
                })
                .find(|c| c.tool_call_id == result.tool_call_id)
                .ok_or_else(|| RuntimeError::ToolCallNotFound {
                    message_id: result.message_id.clone(),
                    tool_call_id: result.tool_call_id.clone(),
                })?;
            added |= call.is_open();
            call.result = Some(result.result);
            call.is_error = Some(result.is_error);
            call.is_preliminary = None;
            if result.artifact.is_some() {
                call.artifact = result.artifact;
            }
            let ready = added
                && state.active.is_none()
                && should_continue(&updated, self.shared.options.human_tool_names.as_deref())
                && !state.repository.has_children(&result.message_id);
            state
                .repository
                .add_or_update(entry.parent_id.as_ref(), ThreadMessage::Assistant(updated))?;
            let resume = ready.then(|| Self::activate(&mut state));
            (resume, entry.parent_id)
        };
        self.shared.notify();
        Ok(resume.map(|run| -> RunFuture {
            Box::pin(run::run_loop(
                self.shared.clone(),
                run,
                parent,
                result.message_id,
                RunConfig::default(),
                false,
            ))
        }))
    }

    /// Mark a new run active, cancelling the previous one.
    fn activate(state: &mut State) -> u64 {
        if let Some(previous) = state.active.take() {
            if let Some(controller) = previous.controller {
                controller.abort();
            }
        }
        let id = state.next_run;
        state.next_run += 1;
        state.active = Some(ActiveRun {
            id,
            controller: None,
            cancelled: false,
        });
        id
    }
}

impl ComposerHost for Shared {
    fn last_message_id(&self) -> Option<MessageId> {
        self.state
            .borrow()
            .repository
            .messages()
            .last()
            .map(|m| m.id().clone())
    }
    fn append(&self, message: AppendMessage) -> Result<RunFuture, RuntimeError> {
        let shared = self.me.upgrade().ok_or(RuntimeError::Disposed)?;
        LocalThreadRuntime { shared }.append(message)
    }
    fn cancel_run(&self) {
        if let Some(shared) = self.me.upgrade() {
            LocalThreadRuntime { shared }.cancel_run();
        }
    }
    fn is_running(&self) -> bool {
        self.state.borrow().active.is_some()
    }
    fn end_edit(&self, id: &MessageId) {
        self.edits.borrow_mut().remove(id);
        self.notify();
    }
}

impl ThreadRuntime for LocalThreadRuntime {
    fn messages(&self) -> Arc<[Arc<ThreadMessage>]> {
        Self::messages(self)
    }
    fn message(&self, id: &MessageId) -> Result<MessageEntry, RuntimeError> {
        Self::message(self, id)
    }
    fn is_running(&self) -> bool {
        Self::is_running(self)
    }
    fn capabilities(&self) -> RuntimeCapabilities {
        RuntimeCapabilities::ALL
    }
    fn branches(&self, id: &MessageId) -> Result<Vec<MessageId>, RuntimeError> {
        Self::branches(self, id)
    }
    fn branch_position(&self, id: &MessageId) -> Result<(usize, usize), RuntimeError> {
        Ok(self.shared.state.borrow().repository.branch_position(id)?)
    }
    fn structure_version(&self) -> Option<u64> {
        Some(self.shared.state.borrow().repository.shape_version())
    }
    fn message_changes_since(&self, version: Option<u64>) -> Option<MessageChanges> {
        Some(
            self.shared
                .state
                .borrow()
                .repository
                .message_changes_since(version),
        )
    }
    fn switch_to_branch(&self, id: &MessageId) -> Result<(), RuntimeError> {
        Self::switch_to_branch(self, id)
    }
    fn append(&self, message: AppendMessage) -> Result<RunFuture, RuntimeError> {
        Self::append(self, message)
    }
    fn reload(&self, config: StartRunConfig) -> Result<RunFuture, RuntimeError> {
        self.start_run(config)
    }
    fn cancel_run(&self) {
        Self::cancel_run(self)
    }
    fn add_tool_result(&self, result: ToolResult) -> Result<Option<RunFuture>, RuntimeError> {
        Self::add_tool_result(self, result)
    }
    fn respond_to_tool_approval(
        &self,
        message_id: &MessageId,
        approval_id: &str,
        approved: bool,
        reason: Option<String>,
    ) -> Result<Option<RunFuture>, RuntimeError> {
        Self::respond_to_tool_approval(self, message_id, approval_id, approved, reason)
    }
    fn submit_feedback(
        &self,
        id: &MessageId,
        feedback: Option<crate::types::SubmittedFeedback>,
    ) -> Result<(), RuntimeError> {
        Self::submit_feedback(self, id, feedback)
    }
    fn composer(&self) -> Composer {
        Self::composer(self)
    }
    fn begin_edit(&self, id: &MessageId) -> Result<Composer, RuntimeError> {
        Self::begin_edit(self, id)
    }
    fn edit_composer(&self, id: &MessageId) -> Option<Composer> {
        Self::edit_composer(self, id)
    }
    fn subscribe_dyn(&self, callback: Box<dyn Fn()>) -> Subscription {
        self.shared.subscribers.subscribe(callback)
    }
    fn export(&self) -> ExportedRepository {
        Self::export(self)
    }
    fn import(&self, exported: ExportedRepository) -> Result<(), RuntimeError> {
        Self::import(self, exported)
    }
}
