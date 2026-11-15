//! What a UI renders, after upstream's scoped state (`ThreadState`,
//! `MessageState`, `PartState`). Plain data: no toolkit types, no callbacks.

use ai_ui_core::{
    FeedbackKind, JsonObject, MessageId, MessageRole, MessageStatus, RuntimeCapabilities,
    ThreadMessage,
};
use serde_json::Value;
use std::sync::Arc;

/// Where a tool call is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolStatus {
    /// The call is running or its arguments are still arriving.
    Running,
    /// The call waits for the user to approve it.
    AwaitingApproval,
    /// The call has a result.
    Complete,
    /// The call failed or was denied.
    Error,
}

/// The user's answer to an approval request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalDecision {
    /// Not answered yet.
    Pending,
    /// The user approved the call.
    Approved,
    /// The user denied the call.
    Denied,
    /// The run ended before the user answered.
    Unavailable,
}

/// A tool call's approval request, as a UI shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovalState {
    /// Identifier to answer the request with.
    pub id: String,
    /// What the user is asked.
    pub prompt: String,
    /// Whether, and how, the user answered.
    pub decision: ApprovalDecision,
}

/// How a tool call is shown. An app registers a presenter per tool to
/// replace the generic title and result text with its own.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ToolPresentation {
    /// Replaces the tool's name.
    pub title: Option<String>,
    /// A line under the title, for example a summary of the arguments.
    pub detail: Option<String>,
    /// Replaces the pretty-printed JSON result.
    pub result_text: Option<String>,
}

/// A tool call as a UI shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolCallState {
    /// Identifies the call.
    pub call_id: String,
    /// Which tool was called.
    pub name: String,
    /// The arguments as streamed text.
    pub args_text: String,
    /// The arguments as a JSON object.
    pub args: JsonObject,
    /// The result, once there is one.
    pub result: Option<Value>,
    /// The result is an error.
    pub is_error: bool,
    /// Where the call is in its life.
    pub status: ToolStatus,
    /// The approval request, if the call needs one.
    pub approval: Option<ApprovalState>,
    /// Whether the user opened the call's details.
    pub expanded: bool,
    /// How the app wants the call shown.
    pub presentation: ToolPresentation,
}

/// What a part contains, in the form a UI renders.
#[derive(Clone, Debug, PartialEq)]
pub enum PartKind {
    /// `text` is what to show now; with smooth streaming that is a prefix of
    /// the received text.
    Text {
        /// The text to show (smooth streaming may reveal it gradually).
        text: String,
        /// More text is still arriving or being revealed.
        streaming: bool,
    },
    /// The model's reasoning.
    Reasoning {
        /// The text to show.
        text: String,
        /// More text is still arriving or being revealed.
        streaming: bool,
        /// Whether the user opened the reasoning.
        expanded: bool,
    },
    /// A tool call.
    ToolCall(Box<ToolCallState>),
    /// A source the answer drew on.
    Source {
        /// Title to show.
        title: String,
        /// Where the source lives, if it is a web page.
        url: Option<String>,
    },
    /// An image.
    Image {
        /// The image URL or data URL.
        url: String,
    },
    /// A file.
    File {
        /// File name, if known.
        filename: Option<String>,
        /// MIME type of the file.
        mime_type: String,
    },
    /// Structured data.
    Data {
        /// What kind of data this is.
        name: String,
    },
    /// A generated UI specification, which the app renders itself.
    GenerativeUi,
}

/// One part of a message.
#[derive(Clone, Debug, PartialEq)]
pub struct PartState {
    /// Position of the part in the message.
    pub index: usize,
    /// What the part contains.
    pub kind: PartKind,
}

/// One message as a UI shows it, with the actions available on it.
#[derive(Clone, Debug, PartialEq)]
pub struct MessageState {
    /// The message id.
    pub id: MessageId,
    /// Who wrote the message.
    pub role: MessageRole,
    /// Position of the message in the thread.
    pub index: usize,
    /// Whether this is the newest message.
    pub is_last: bool,
    /// One-based position among this message's sibling branches.
    pub branch_number: usize,
    /// How many sibling branches this message has (at least 1).
    pub branch_count: usize,
    /// The message's parts, in order.
    pub parts: Vec<PartState>,
    /// The concatenated text, for copying.
    pub text: String,
    /// The status of an assistant message; `None` for other roles.
    pub status: Option<MessageStatus>,
    /// A readable failure, when the message ended in an error.
    pub error: Option<String>,
    /// The newest message while a run is in progress.
    pub is_generating: bool,
    /// The "copied" confirmation is showing.
    pub is_copied: bool,
    /// The user can edit this message.
    pub can_edit: bool,
    /// The answer can be regenerated.
    pub can_reload: bool,
    /// The text can be copied.
    pub can_copy: bool,
    /// The user can rate this message.
    pub can_feedback: bool,
    /// The user's rating, if any.
    pub feedback: Option<FeedbackKind>,
    /// The underlying message.
    pub message: Arc<ThreadMessage>,
}

/// Everything a UI renders for a thread.
#[derive(Clone, Debug, PartialEq)]
pub struct ThreadState {
    /// In order. A message that did not change since the previous state is
    /// the same `Arc`, so UIs can update only the rows whose pointer differs.
    pub messages: Vec<Arc<MessageState>>,
    /// The thread has no messages.
    pub is_empty: bool,
    /// A run is in progress.
    pub is_running: bool,
    /// What the UI may offer for this thread.
    pub capabilities: RuntimeCapabilities,
}
