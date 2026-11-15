//! The thread runtime interface, after upstream's `ThreadRuntimeCore`.
//!
//! UIs and the state layer bind to [`ThreadRuntime`], so they work with every
//! runtime: [`LocalThreadRuntime`](crate::LocalThreadRuntime) (this library
//! drives the model), [`ExternalStoreRuntime`](crate::ExternalStoreRuntime)
//! (the app owns the messages) and the threads managed by a
//! [`ThreadList`](crate::ThreadList).

use crate::adapter::RunError;
use crate::repository::{ExportedRepository, MessageChanges, MessageEntry, RepositoryError};
use crate::subscribe::Subscription;
use crate::types::{AppendMessage, MessageId, RunConfig, SubmittedFeedback, ThreadMessage};
use crate::Composer;
use serde_json::Value;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// A model run, or an app callback, to spawn. It resolves when the work ends:
/// `Ok` for completion or cancellation, `Err` when it failed (the message
/// records a model failure too).
pub type RunFuture = Pin<Box<dyn Future<Output = Result<(), RunError>>>>;

/// `startRun` and reload configuration (`StartRunConfig`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StartRunConfig {
    /// The message to answer; `None` answers an empty thread.
    pub parent_id: Option<MessageId>,
    /// The assistant message being regenerated, if any.
    pub source_id: Option<MessageId>,
    /// Configuration for the run.
    pub run_config: RunConfig,
}

/// `addToolResult` arguments.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolResult {
    /// The assistant message holding the call.
    pub message_id: MessageId,
    /// Which call the result answers.
    pub tool_call_id: String,
    /// The result value.
    pub result: Value,
    /// The result is an error.
    pub is_error: bool,
    /// Extra output for the app, not sent to the model.
    pub artifact: Option<Value>,
}

/// What a runtime can do (`RuntimeCapabilities`); UIs hide the matching
/// actions when a capability is missing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuntimeCapabilities {
    /// The thread can switch between branches.
    pub switch_to_branch: bool,
    /// User messages can be edited.
    pub edit: bool,
    /// An answer can be regenerated.
    pub reload: bool,
    /// A run can be stopped.
    pub cancel: bool,
    /// Message text can be copied.
    pub copy: bool,
    /// Feedback can be submitted.
    pub feedback: bool,
    /// Tool calls can be answered from the UI.
    pub answer_tool_call: bool,
}

impl RuntimeCapabilities {
    /// Every capability on.
    pub const ALL: Self = Self {
        switch_to_branch: true,
        edit: true,
        reload: true,
        cancel: true,
        copy: true,
        feedback: true,
        answer_tool_call: true,
    };
}

/// Why a runtime operation failed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RuntimeError {
    /// The message tree rejected the operation.
    Repository(RepositoryError),
    /// The operation applies to assistant messages only.
    NotAssistantMessage(MessageId),
    /// The message has no tool call with this id.
    ToolCallNotFound {
        /// The message that was searched.
        message_id: MessageId,
        /// The call id that was not found.
        tool_call_id: String,
    },
    /// The operation would rewrite history a running model is reading.
    RunInProgress,
    /// `begin_edit` for a message that already has an edit composer.
    EditInProgress(MessageId),
    /// No pending approval with this id on that message.
    ApprovalNotFound(String),
    /// This runtime does not support the operation.
    Unsupported(&'static str),
    /// The runtime was dropped.
    Disposed,
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Repository(error) => error.fmt(f),
            Self::NotAssistantMessage(id) => write!(f, "message {id} is not an assistant message"),
            Self::ToolCallNotFound {
                message_id,
                tool_call_id,
            } => {
                write!(f, "message {message_id} has no tool call {tool_call_id}")
            }
            Self::RunInProgress => f.write_str("a run is in progress"),
            Self::EditInProgress(id) => write!(f, "message {id} is already being edited"),
            Self::ApprovalNotFound(id) => write!(f, "no pending approval {id}"),
            Self::Unsupported(what) => write!(f, "this runtime does not support {what}"),
            Self::Disposed => f.write_str("the runtime was dropped"),
        }
    }
}
impl std::error::Error for RuntimeError {}
impl From<RepositoryError> for RuntimeError {
    fn from(error: RepositoryError) -> Self {
        Self::Repository(error)
    }
}

/// One thread: its messages, branches, composers and actions.
///
/// Methods change state synchronously and notify subscribers. A method that
/// needs asynchronous work returns a [`RunFuture`] for the host to spawn.
pub trait ThreadRuntime {
    /// The selected branch, root first.
    fn messages(&self) -> Arc<[Arc<ThreadMessage>]>;
    /// A message and its position, by id.
    fn message(&self, id: &MessageId) -> Result<MessageEntry, RuntimeError>;
    /// Whether a run is in progress.
    fn is_running(&self) -> bool;
    /// What the UI may offer for this thread.
    fn capabilities(&self) -> RuntimeCapabilities;
    /// The message and its siblings.
    fn branches(&self, id: &MessageId) -> Result<Vec<MessageId>, RuntimeError>;
    /// `(position, count)` of the message among its siblings, 1-based. The
    /// default builds [`branches`](Self::branches); runtimes override it to
    /// avoid the allocation, which a UI pays once per message per update.
    fn branch_position(&self, id: &MessageId) -> Result<(usize, usize), RuntimeError> {
        let branches = self.branches(id)?;
        let position = branches.iter().position(|b| b == id).map_or(1, |p| p + 1);
        Ok((position, branches.len().max(1)))
    }
    /// A number that changes whenever branches or the selected path change,
    /// but not when a message is updated in place. While it is unchanged a UI
    /// may keep every message's [`branch_position`](Self::branch_position).
    /// `None` (the default) means unknown: positions must be read again.
    fn structure_version(&self) -> Option<u64> {
        None
    }
    /// Changed selected-message indices since a consumer's last version.
    /// `None` (the default) means this runtime does not track changes: scan
    /// [`messages`](Self::messages). A supported runtime returns a current
    /// version even when the journal requires a full read. Changes must cover
    /// every selected message update and be valid for the current `messages()`
    /// snapshot; versions are local to this runtime.
    /// Consumers must still refresh when running state or capabilities change.
    fn message_changes_since(&self, _version: Option<u64>) -> Option<MessageChanges> {
        None
    }
    /// Select the branch containing `id`.
    fn switch_to_branch(&self, id: &MessageId) -> Result<(), RuntimeError>;
    /// Add a message; a user message answers it (see [`AppendMessage`]).
    fn append(&self, message: AppendMessage) -> Result<RunFuture, RuntimeError>;
    /// Answer `parent_id` again (regenerate).
    fn reload(&self, config: StartRunConfig) -> Result<RunFuture, RuntimeError>;
    /// Stop the run in progress, if any.
    fn cancel_run(&self);
    /// Answer a tool call; may resume the run, which is then returned to spawn.
    fn add_tool_result(&self, result: ToolResult) -> Result<Option<RunFuture>, RuntimeError>;
    /// Approve or deny a pending tool approval; may resume the run, which is then returned to spawn.
    fn respond_to_tool_approval(
        &self,
        message_id: &MessageId,
        approval_id: &str,
        approved: bool,
        reason: Option<String>,
    ) -> Result<Option<RunFuture>, RuntimeError>;
    /// `None` clears feedback.
    fn submit_feedback(
        &self,
        id: &MessageId,
        feedback: Option<SubmittedFeedback>,
    ) -> Result<(), RuntimeError>;
    /// The composer for the next message.
    fn composer(&self) -> Composer;
    /// Start editing a message and return its composer.
    fn begin_edit(&self, id: &MessageId) -> Result<Composer, RuntimeError>;
    /// The composer of a message being edited, if any.
    fn edit_composer(&self, id: &MessageId) -> Option<Composer>;
    /// Called after every change; drop the guard to unsubscribe.
    fn subscribe_dyn(&self, callback: Box<dyn Fn()>) -> Subscription;
    /// Persisted messages and the selected head.
    fn export(&self) -> ExportedRepository;
    /// Load history.
    fn import(&self, exported: ExportedRepository) -> Result<(), RuntimeError>;
}

/// What a composer needs from its runtime.
pub trait ComposerHost {
    /// The newest message on the selected path, which a new message answers.
    fn last_message_id(&self) -> Option<MessageId>;
    /// Send a message.
    fn append(&self, message: AppendMessage) -> Result<RunFuture, RuntimeError>;
    /// Stop the run in progress, if any.
    fn cancel_run(&self);
    /// Whether a run is in progress.
    fn is_running(&self) -> bool;
    /// An edit composer finished (sent or cancelled).
    fn end_edit(&self, id: &MessageId);
}
