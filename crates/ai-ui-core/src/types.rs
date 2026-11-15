//! Thread message model, ported from `@assistant-ui/core` `types/message.ts`.
//!
//! The serialized form follows upstream's JSON: parts are tagged by `type`,
//! messages by `role`, fields are camelCase and the `unstable_*` metadata names
//! are kept. Fields this port does not model yet are preserved in each
//! struct's `extra` map, so stored or streamed messages round-trip losslessly.
//! One deliberate difference: `createdAt` is epoch milliseconds, not a `Date`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fmt;

/// A JSON object (`ReadonlyJSONObject` upstream).
pub type JsonObject = Map<String, Value>;

fn is_false(value: &bool) -> bool {
    !*value
}

/// Message identifier, unique within one repository.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MessageId(pub String);

impl MessageId {
    /// The id as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl From<&str> for MessageId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}
impl From<String> for MessageId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

/// Milliseconds since the Unix epoch.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Timestamp(pub u64);

// ---------------------------------------------------------------- statuses

/// Why a message or part stopped without completing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IncompleteReason {
    /// The user or the app stopped the run.
    Cancelled,
    /// Message status only: the step limit was reached with tool calls pending.
    ToolCalls,
    /// The model reached its output length limit.
    Length,
    /// The provider's content filter stopped the output.
    ContentFilter,
    /// Stopped for a reason the provider did not classify.
    Other,
    /// The run failed; the message carries the error detail.
    Error,
}

/// Streaming status of one part (`MessagePartStreamStatus`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum PartStatus {
    /// The part is still receiving content.
    Running,
    /// The part finished.
    Complete,
    /// The part ended early.
    Incomplete {
        /// Why it ended early.
        reason: IncompleteReason,
    },
}

/// Why an assistant message is waiting on the user or the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RequiresActionReason {
    /// Tool calls without results are waiting to be answered.
    ToolCalls,
    /// A tool asked for human input before it can continue.
    Interrupt,
}

/// Why a message completed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CompleteReason {
    /// The model stopped normally.
    Stop,
    /// Completed with no reported reason.
    Unknown,
}

/// Status of an assistant message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum MessageStatus {
    /// The run is in progress.
    Running,
    /// The run paused until the user or the app acts.
    RequiresAction {
        /// What it is waiting for.
        reason: RequiresActionReason,
    },
    /// The run finished.
    Complete {
        /// Why it finished.
        reason: CompleteReason,
    },
    /// The run ended without finishing.
    Incomplete {
        /// Why it ended early.
        reason: IncompleteReason,
        /// Error detail, as the adapter reported it, when the reason is `Error`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<Value>,
    },
}

impl MessageStatus {
    /// A completed status with no reported reason.
    pub const fn complete() -> Self {
        Self::Complete {
            reason: CompleteReason::Unknown,
        }
    }
    /// An incomplete status for a run that was stopped.
    pub const fn cancelled() -> Self {
        Self::Incomplete {
            reason: IncompleteReason::Cancelled,
            error: None,
        }
    }
    /// Whether the run is still in progress.
    pub fn is_running(&self) -> bool {
        matches!(self, Self::Running)
    }
    /// Complete or incomplete: the run for this message has ended.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Complete { .. } | Self::Incomplete { .. })
    }
}

// ---------------------------------------------------------------- parts

/// A run of text.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextPart {
    /// The part's id, when the producer gave one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The text.
    pub text: String,
    /// Streaming status of the part, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<PartStatus>,
    /// Id of the part this one is grouped under, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    /// Fields this crate does not model; they are kept so the value round-trips unchanged.
    #[serde(flatten)]
    pub extra: JsonObject,
}

impl TextPart {
    /// A text part with no id and no status.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }
}

/// The model's reasoning, shown separately from the answer.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningPart {
    /// The part's id, when the producer gave one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The reasoning text.
    pub text: String,
    /// Streaming status of the part, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<PartStatus>,
    #[serde(
        rename = "unstable_summary",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    /// A short summary of the reasoning, when the model provides one.
    pub summary: Option<String>,
    /// Id of the part this one is grouped under, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    /// Fields this crate does not model; they are kept so the value round-trips unchanged.
    #[serde(flatten)]
    pub extra: JsonObject,
}

/// A cited source: a URL or a document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "sourceType", rename_all = "camelCase")]
pub enum SourcePart {
    /// A web page the answer drew on.
    #[serde(rename_all = "camelCase")]
    Url {
        /// The source's id.
        id: String,
        /// Where the page lives.
        url: String,
        /// Title to display.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        /// Id of the part this one is grouped under, if any.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_id: Option<String>,
        /// Fields this crate does not model; they are kept so the value round-trips unchanged.
        #[serde(flatten)]
        extra: JsonObject,
    },
    /// A document the answer drew on.
    #[serde(rename_all = "camelCase")]
    Document {
        /// The source's id.
        id: String,
        /// Title to display.
        title: String,
        /// MIME type of the document.
        media_type: String,
        /// Original file name, if known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
        /// Id of the part this one is grouped under, if any.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_id: Option<String>,
        /// Fields this crate does not model; they are kept so the value round-trips unchanged.
        #[serde(flatten)]
        extra: JsonObject,
    },
}

/// An image, as a URL or data URL.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagePart {
    /// The part's id, when the producer gave one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Data URL or remote URL.
    pub image: String,
    /// Original file name, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    /// Fields this crate does not model; they are kept so the value round-trips unchanged.
    #[serde(flatten)]
    pub extra: JsonObject,
}

/// How a file part's `data` is addressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileSourceType {
    /// `data` is a URL.
    Url,
    /// `data` is a provider file id.
    Id,
}

/// A file, uploaded by the user or produced by the model.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilePart {
    /// The part's id, when the producer gave one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Original file name, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    /// Base64 data, URL or provider file id, per `source_type`.
    pub data: String,
    /// MIME type of the file.
    pub mime_type: String,
    /// How `data` is addressed; `None` means the data is inline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_type: Option<FileSourceType>,
    /// Id of the part this one is grouped under, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    /// Fields this crate does not model; they are kept so the value round-trips unchanged.
    #[serde(flatten)]
    pub extra: JsonObject,
}

/// Structured data with a name, for the app to render or ignore.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataPart {
    /// The part's id, when the producer gave one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// What kind of data this is (the part of a `data-*` chunk type after the dash).
    pub name: String,
    /// The payload.
    pub data: Value,
    /// Fields this crate does not model; they are kept so the value round-trips unchanged.
    #[serde(flatten)]
    pub extra: JsonObject,
}

/// Encoded audio.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioData {
    /// The audio bytes, base64 encoded.
    pub data: String,
    /// `mp3` or `wav` upstream.
    pub format: String,
}

/// An audio clip in a user message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioPart {
    /// The audio and its format.
    pub audio: AudioData,
    /// Fields this crate does not model; they are kept so the value round-trips unchanged.
    #[serde(flatten)]
    pub extra: JsonObject,
}

/// A generative UI tree; the spec is kept as JSON for the renderer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerativeUiPart {
    /// The UI specification; the app decides how to render it.
    pub spec: Value,
    /// The part's id, when the producer gave one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Id of the part this one is grouped under, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    /// Fields this crate does not model; they are kept so the value round-trips unchanged.
    #[serde(flatten)]
    pub extra: JsonObject,
}

/// One choice offered with an approval request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolApprovalOption {
    /// Identifier of the option, returned when it is chosen.
    pub id: String,
    /// `allow-once`, `allow-always`, `reject-once`, `reject-always` or custom.
    pub kind: String,
    /// Text to show for the option.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// A longer explanation of the option.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Fields this crate does not model; they are kept so the value round-trips unchanged.
    #[serde(flatten)]
    pub extra: JsonObject,
}

/// Why an approval request ended without an answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApprovalResolution {
    /// The user dismissed the request.
    Cancelled,
    /// The request timed out.
    Expired,
}

/// A pending or answered approval request on a tool call.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolApproval {
    /// Identifier of the approval request.
    pub id: String,
    /// What the user is asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// The decision; `None` while the request is pending.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved: Option<bool>,
    /// Why the user decided as they did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Choices offered besides approve and deny.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<ToolApprovalOption>>,
    /// The option the user chose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub option_id: Option<String>,
    /// Free text entered with the choice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Set when the request ended without an answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<ApprovalResolution>,
    /// `display`, `questions`, `answers` and other fields not yet modelled.
    #[serde(flatten)]
    pub extra: JsonObject,
}

impl ToolApproval {
    /// Waiting for the user: neither answered nor resolved.
    pub fn is_pending(&self) -> bool {
        self.approved.is_none() && self.resolution.is_none()
    }
}

/// A request for human input that pauses a tool call.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolInterrupt {
    /// Always `human` upstream.
    #[serde(rename = "type")]
    pub kind: String,
    /// What the tool wants to ask; the app interprets it.
    pub payload: Value,
}

/// A tool call and, once it has run, its result.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallPart {
    /// Identifies the call; the result carries the same id.
    pub tool_call_id: String,
    /// Which tool was called.
    pub tool_name: String,
    /// The arguments as a JSON object (parsed from `args_text`).
    #[serde(default)]
    pub args: JsonObject,
    /// The arguments as streamed text; may be partial JSON while running.
    #[serde(default)]
    pub args_text: String,
    /// The result, once there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// The result is an error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    /// The result is partial and a later one will replace it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_preliminary: Option<bool>,
    /// Extra output for the app, not sent to the model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<Value>,
    /// An approval request or decision for this call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<ToolApproval>,
    /// A request for human input, if the call is paused on one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interrupt: Option<ToolInterrupt>,
    /// Id of the part this one is grouped under, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    /// `timing`, `mcp`, `modelContent`, nested `messages` and other fields.
    #[serde(flatten)]
    pub extra: JsonObject,
}

impl ToolCallPart {
    /// No final result yet (none, or only a preliminary one).
    pub fn is_open(&self) -> bool {
        self.result.is_none() || self.is_preliminary == Some(true)
    }
}

/// Content of a user message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum UserMessagePart {
    /// Text.
    Text(TextPart),
    /// An image.
    Image(ImagePart),
    /// A file.
    File(FilePart),
    /// Structured data.
    Data(DataPart),
    /// An audio clip.
    Audio(AudioPart),
}

/// Content of an assistant message. Variants stay unboxed so callers can
/// match on them directly; a message holds few parts.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum AssistantMessagePart {
    /// Text.
    Text(TextPart),
    /// The model's reasoning.
    Reasoning(ReasoningPart),
    /// A tool call and its result.
    ToolCall(ToolCallPart),
    /// A source the answer drew on.
    Source(SourcePart),
    /// A file.
    File(FilePart),
    /// An image.
    Image(ImagePart),
    /// Structured data.
    Data(DataPart),
    /// A generated UI specification.
    #[serde(rename = "generative-ui")]
    GenerativeUi(GenerativeUiPart),
}

impl AssistantMessagePart {
    /// A text part with no id and no status.
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(TextPart::new(text))
    }
    /// The tool call, if this part is one.
    pub fn as_tool_call(&self) -> Option<&ToolCallPart> {
        match self {
            Self::ToolCall(part) => Some(part),
            _ => None,
        }
    }
}

/// Content of a system message: a single text part upstream.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum SystemMessagePart {
    /// Text.
    Text(TextPart),
}

// ---------------------------------------------------------------- attachments

/// A finished attachment on a user message (`CompleteAttachment`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompleteAttachment {
    /// The attachment's id.
    pub id: String,
    /// `image`, `document`, `file` or a custom kind.
    #[serde(rename = "type")]
    pub kind: String,
    /// File name shown to the user.
    pub name: String,
    /// MIME type, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    /// What the attachment contributes to the model request.
    #[serde(default)]
    pub content: Vec<UserMessagePart>,
    /// Fields this crate does not model; they are kept so the value round-trips unchanged.
    #[serde(flatten)]
    pub extra: JsonObject,
}

// ---------------------------------------------------------------- messages

/// Token counts for one model step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    /// Tokens the model read.
    #[serde(rename = "inputTokens")]
    pub input_tokens: u64,
    /// Tokens the model produced.
    #[serde(rename = "outputTokens")]
    pub output_tokens: u64,
}

/// One model step of an assistant message.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadStep {
    /// The id the provider gave the step's message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    /// Token counts for the step, once known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    /// Why the step ended (`stop`, `tool-calls`, ...), once it has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    /// The model continues the same message in the next step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_continued: Option<bool>,
}

/// A thumbs up or down.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FeedbackKind {
    /// Thumbs up.
    Positive,
    /// Thumbs down.
    Negative,
}

/// The user's feedback on a message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SubmittedFeedback {
    /// Up or down.
    #[serde(rename = "type")]
    pub kind: FeedbackKind,
    /// Optional free text from the user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

/// Metadata of user and system messages.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageMetadata {
    /// Shown before the backend confirmed it; excluded from export.
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_optimistic: bool,
    /// App-defined data stored with the message.
    #[serde(default)]
    pub custom: JsonObject,
    /// Fields this crate does not model; they are kept so the value round-trips unchanged.
    #[serde(flatten)]
    pub extra: JsonObject,
}

/// Metadata of an assistant message.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantMetadata {
    /// Id of the run that produced the message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// App state streamed by the backend (`aui-state` operations applied).
    #[serde(rename = "unstable_state", default)]
    pub state: Value,
    /// Annotations the backend attached.
    #[serde(rename = "unstable_annotations", default)]
    pub annotations: Vec<Value>,
    /// Data parts that are kept beside the content (`data-*` chunks).
    #[serde(rename = "unstable_data", default)]
    pub data: Vec<Value>,
    /// One entry per model step.
    #[serde(default)]
    pub steps: Vec<ThreadStep>,
    /// The user's feedback, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submitted_feedback: Option<SubmittedFeedback>,
    /// Timing information reported by the run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timing: Option<Value>,
    /// The message was added locally and the backend has not confirmed it yet.
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_optimistic: bool,
    /// App-defined data stored with the message.
    #[serde(default)]
    pub custom: JsonObject,
    /// Fields this crate does not model; they are kept so the value round-trips unchanged.
    #[serde(flatten)]
    pub extra: JsonObject,
}

/// A system message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemMessage {
    /// The message id.
    pub id: MessageId,
    /// When the message was created.
    pub created_at: Timestamp,
    /// The message's parts.
    pub content: Vec<SystemMessagePart>,
    /// Metadata stored with the message.
    #[serde(default)]
    pub metadata: MessageMetadata,
}

/// A message from the user.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserMessage {
    /// The message id.
    pub id: MessageId,
    /// When the message was created.
    pub created_at: Timestamp,
    /// The message's parts.
    pub content: Vec<UserMessagePart>,
    /// Files attached to the message.
    #[serde(default)]
    pub attachments: Vec<CompleteAttachment>,
    /// Metadata stored with the message.
    #[serde(default)]
    pub metadata: MessageMetadata,
}

/// A message from the model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantMessage {
    /// The message id.
    pub id: MessageId,
    /// When the message was created.
    pub created_at: Timestamp,
    /// The message's parts.
    pub content: Vec<AssistantMessagePart>,
    /// Whether the run is still going, finished or ended early.
    pub status: MessageStatus,
    /// Metadata stored with the message.
    #[serde(default)]
    pub metadata: AssistantMetadata,
}

impl AssistantMessage {
    /// The message's tool calls, in order.
    pub fn tool_calls(&self) -> impl Iterator<Item = &ToolCallPart> {
        self.content
            .iter()
            .filter_map(AssistantMessagePart::as_tool_call)
    }
}

/// A message in a thread (`ThreadMessage`). Always stored behind an `Arc`.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "lowercase")]
pub enum ThreadMessage {
    /// A system message.
    System(SystemMessage),
    /// A message from the user.
    User(UserMessage),
    /// A message from the model.
    Assistant(AssistantMessage),
}

/// Who wrote a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    /// The system prompt.
    System,
    /// The user.
    User,
    /// The model.
    Assistant,
}

impl ThreadMessage {
    /// The message id.
    pub fn id(&self) -> &MessageId {
        match self {
            Self::System(m) => &m.id,
            Self::User(m) => &m.id,
            Self::Assistant(m) => &m.id,
        }
    }
    /// When the message was created.
    pub fn created_at(&self) -> Timestamp {
        match self {
            Self::System(m) => m.created_at,
            Self::User(m) => m.created_at,
            Self::Assistant(m) => m.created_at,
        }
    }
    /// Who wrote the message.
    pub fn role(&self) -> MessageRole {
        match self {
            Self::System(_) => MessageRole::System,
            Self::User(_) => MessageRole::User,
            Self::Assistant(_) => MessageRole::Assistant,
        }
    }
    /// Added locally and not yet confirmed by the backend.
    pub fn is_optimistic(&self) -> bool {
        match self {
            Self::System(m) => m.metadata.is_optimistic,
            Self::User(m) => m.metadata.is_optimistic,
            Self::Assistant(m) => m.metadata.is_optimistic,
        }
    }
    /// The assistant message, if this is one.
    pub fn as_assistant(&self) -> Option<&AssistantMessage> {
        match self {
            Self::Assistant(m) => Some(m),
            _ => None,
        }
    }
    /// The status of an assistant message; `None` for the other roles.
    pub fn status(&self) -> Option<&MessageStatus> {
        self.as_assistant().map(|m| &m.status)
    }
    /// Concatenated text parts, as used for copy and speech.
    pub fn text(&self) -> String {
        let texts: Vec<&str> = match self {
            Self::System(m) => m
                .content
                .iter()
                .map(|SystemMessagePart::Text(t)| t.text.as_str())
                .collect(),
            Self::User(m) => m
                .content
                .iter()
                .filter_map(|p| match p {
                    UserMessagePart::Text(t) => Some(t.text.as_str()),
                    _ => None,
                })
                .collect(),
            Self::Assistant(m) => m
                .content
                .iter()
                .filter_map(|p| match p {
                    AssistantMessagePart::Text(t) => Some(t.text.as_str()),
                    _ => None,
                })
                .collect(),
        };
        texts.join("\n\n")
    }
}

/// Per-run configuration passed through to the model adapter.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RunConfig {
    /// App-defined data passed to the adapter with the run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom: Option<JsonObject>,
}

/// The new message's content and role for [`AppendMessage`].
#[derive(Clone, Debug, PartialEq)]
pub enum AppendContent {
    /// A user message.
    User {
        /// The message's parts.
        content: Vec<UserMessagePart>,
        /// Files attached to the message.
        attachments: Vec<CompleteAttachment>,
        /// App-defined metadata.
        custom: JsonObject,
    },
    /// An assistant message, for restoring history.
    Assistant {
        /// The message's parts.
        content: Vec<AssistantMessagePart>,
        /// App-defined metadata.
        custom: JsonObject,
    },
    /// A system message.
    System {
        /// The system prompt text.
        text: String,
    },
}

/// A message to append (`AppendMessage`): content plus where it goes.
#[derive(Clone, Debug, PartialEq)]
pub struct AppendMessage {
    /// The message it follows; `None` starts the thread.
    pub parent_id: Option<MessageId>,
    /// The message this one replaces when editing, if any.
    pub source_id: Option<MessageId>,
    /// Settings for the run this message starts.
    pub run_config: RunConfig,
    /// Whether to start a run; by default only for user messages.
    pub start_run: Option<bool>,
    /// What to append.
    pub content: AppendContent,
}

impl AppendMessage {
    /// A plain-text user message after `parent_id`.
    pub fn user_text(parent_id: Option<MessageId>, text: impl Into<String>) -> Self {
        Self {
            parent_id,
            source_id: None,
            run_config: RunConfig::default(),
            start_run: None,
            content: AppendContent::User {
                content: vec![UserMessagePart::Text(TextPart::new(text))],
                attachments: Vec::new(),
                custom: JsonObject::new(),
            },
        }
    }
    /// Whether appending this message starts a model run (user messages do).
    pub fn starts_run(&self) -> bool {
        self.start_run
            .unwrap_or(matches!(self.content, AppendContent::User { .. }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn upstream_assistant_json_round_trips_with_unmodelled_fields() {
        let upstream = json!({
            "role": "assistant", "id": "a1", "createdAt": 1700000000000u64,
            "status": { "type": "requires-action", "reason": "tool-calls" },
            "content": [
                { "type": "reasoning", "text": "thinking", "unstable_summary": "s" },
                { "type": "text", "text": "hi", "status": { "type": "complete" } },
                { "type": "tool-call", "toolCallId": "c1", "toolName": "search",
                  "args": { "q": "x" }, "argsText": "{\"q\":\"x\"}",
                  "timing": { "startedAt": 1 }, "mcp": { "app": { "resourceUri": "ui://a" } } },
                { "type": "source", "sourceType": "url", "id": "s1", "url": "https://e.com" },
                { "type": "generative-ui", "spec": { "root": "x" } }
            ],
            "metadata": { "unstable_state": null, "unstable_annotations": [], "unstable_data": [],
                          "steps": [ { "usage": { "inputTokens": 1, "outputTokens": 2 } } ],
                          "custom": {}, "futureField": true }
        });
        let message: ThreadMessage = serde_json::from_value(upstream.clone()).unwrap();
        let assistant = message.as_assistant().unwrap();
        let call = assistant.tool_calls().next().unwrap();
        assert_eq!(call.tool_name, "search");
        assert!(call.is_open());
        assert_eq!(call.extra["timing"], json!({ "startedAt": 1 }));
        assert_eq!(serde_json::to_value(&message).unwrap(), upstream);
    }

    #[test]
    fn user_message_defaults_missing_metadata_and_attachments() {
        let message: ThreadMessage = serde_json::from_value(json!({
            "role": "user", "id": "u1", "createdAt": 5,
            "content": [ { "type": "text", "text": "hello" } ]
        }))
        .unwrap();
        assert_eq!(message.text(), "hello");
        assert_eq!(message.role(), MessageRole::User);
        assert!(!message.is_optimistic());
    }

    #[test]
    fn incomplete_status_carries_error_only_when_present() {
        let status = MessageStatus::Incomplete {
            reason: IncompleteReason::Error,
            error: Some(json!("boom")),
        };
        assert_eq!(
            serde_json::to_value(&status).unwrap(),
            json!({ "type": "incomplete", "reason": "error", "error": "boom" })
        );
        assert_eq!(
            serde_json::to_value(MessageStatus::cancelled()).unwrap(),
            json!({ "type": "incomplete", "reason": "cancelled" })
        );
    }
}
