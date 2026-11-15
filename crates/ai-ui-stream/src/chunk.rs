//! Stream chunks, ported from `assistant-stream`'s `AssistantStreamChunk.ts`.

use ai_ui_core::{JsonObject, ToolApproval};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The initial shape of a new part (`PartInit`).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
#[non_exhaustive]
pub enum PartInit {
    /// A text part.
    #[serde(rename_all = "camelCase")]
    Text {
        /// Provider-specific metadata.
        #[serde(default)]
        provider_metadata: Option<JsonObject>,
        /// Id of the part this one is grouped under, if any.
        #[serde(default)]
        parent_id: Option<String>,
    },
    /// A reasoning part.
    #[serde(rename_all = "camelCase")]
    Reasoning {
        /// A short summary of the reasoning, if the model gives one.
        #[serde(rename = "unstable_summary", default)]
        summary: Option<String>,
        /// Provider-specific metadata.
        #[serde(default)]
        provider_metadata: Option<JsonObject>,
        /// Id of the part this one is grouped under, if any.
        #[serde(default)]
        parent_id: Option<String>,
    },
    /// A tool call; its arguments follow as text deltas and its result as a `Result` chunk.
    #[serde(rename_all = "camelCase")]
    ToolCall {
        /// Identifies the call.
        tool_call_id: String,
        /// Which tool is called.
        tool_name: String,
        /// Id of the part this one is grouped under, if any.
        #[serde(default)]
        parent_id: Option<String>,
    },
    /// A source the answer drew on.
    #[serde(rename_all = "camelCase")]
    Source {
        /// `url` or `document`.
        source_type: String,
        /// The source's id.
        id: String,
        /// Where the source lives (URL sources).
        #[serde(default)]
        url: Option<String>,
        /// Title to display.
        #[serde(default)]
        title: Option<String>,
        /// MIME type (document sources).
        #[serde(default)]
        media_type: Option<String>,
        /// Original file name (document sources).
        #[serde(default)]
        filename: Option<String>,
        /// Id of the part this one is grouped under, if any.
        #[serde(default)]
        parent_id: Option<String>,
    },
    /// A file.
    #[serde(rename_all = "camelCase")]
    File {
        /// The file's data: base64, or a data URL.
        data: String,
        /// MIME type of the file.
        mime_type: String,
        /// Id of the part this one is grouped under, if any.
        #[serde(default)]
        parent_id: Option<String>,
    },
    /// Structured data.
    #[serde(rename_all = "camelCase")]
    Data {
        /// What kind of data this is.
        name: String,
        /// The payload.
        data: Value,
        /// Id of the part this one is grouped under, if any.
        #[serde(default)]
        parent_id: Option<String>,
    },
    /// A part type this port does not know; kept as a placeholder so later
    /// chunks still address the right part index.
    #[serde(other)]
    Unsupported,
}

/// Why a step or message finished.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum FinishReason {
    /// The model stopped normally.
    Stop,
    /// The output length limit was reached.
    Length,
    /// The provider's content filter stopped the output.
    ContentFilter,
    /// The model stopped to call tools.
    ToolCalls,
    /// The model run failed.
    Error,
    /// Another reason.
    Other,
    /// No reason was reported.
    Unknown,
    /// AI SDK 7: the run was interrupted (for example by a human-in-the-loop pause).
    Interrupted,
}

/// Token counts reported by a step or message finish.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChunkUsage {
    /// Tokens the model read.
    #[serde(default)]
    pub input_tokens: u64,
    /// Tokens the model produced.
    #[serde(default)]
    pub output_tokens: u64,
}

/// One chunk's payload; `path` addresses the part (`[index]`) it applies to.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
#[non_exhaustive]
pub enum ChunkKind {
    /// A part begins; it is appended to the message.
    PartStart {
        /// What kind of part, with its initial fields.
        part: PartInit,
    },
    /// The part at `path` is complete.
    #[serde(rename_all = "camelCase")]
    PartFinish {
        /// Provider metadata to set on the part.
        #[serde(default)]
        provider_metadata: Option<JsonObject>,
    },
    /// The arguments of the tool call at `path` are complete.
    ToolCallArgsTextFinish,
    /// An approval request or decision for the tool call at `path`.
    ToolCallApproval {
        /// The approval; it replaces any earlier one.
        approval: ToolApproval,
    },
    /// Append text to the part at `path` (or tool arguments, for a tool call).
    #[serde(rename_all = "camelCase")]
    TextDelta {
        /// The text to append.
        text_delta: String,
        /// Provider metadata to set on the part.
        #[serde(default)]
        provider_metadata: Option<JsonObject>,
    },
    /// Append annotations to the message.
    Annotations {
        /// The annotations.
        annotations: Vec<Value>,
    },
    /// Append data entries to the message.
    Data {
        /// The entries. An entry with an `id` replaces the earlier one with the same name and id.
        data: Vec<Value>,
    },
    /// A model step begins.
    #[serde(rename_all = "camelCase")]
    StepStart {
        /// The id the provider gave the step's message.
        message_id: String,
    },
    /// A model step ends.
    #[serde(rename_all = "camelCase")]
    StepFinish {
        /// Why the step ended.
        finish_reason: FinishReason,
        /// Token counts for the step.
        #[serde(default)]
        usage: ChunkUsage,
        /// The next step continues this step's text instead of starting a new part.
        #[serde(default)]
        is_continued: bool,
    },
    /// The message ends.
    #[serde(rename_all = "camelCase")]
    MessageFinish {
        /// Why the message ended.
        finish_reason: FinishReason,
        /// Token counts for the message.
        #[serde(default)]
        usage: ChunkUsage,
    },
    /// A tool call's result.
    #[serde(rename_all = "camelCase")]
    Result {
        /// Extra output for the app, not sent to the model.
        #[serde(default)]
        artifact: Option<Value>,
        /// The result value.
        result: Value,
        /// The result is an error.
        #[serde(default)]
        is_error: bool,
        /// A later result will replace this one.
        #[serde(default)]
        is_preliminary: bool,
        /// Content to send to the model instead of the raw result.
        #[serde(default)]
        model_content: Option<Value>,
        /// Messages produced by a nested run.
        #[serde(default)]
        messages: Option<Value>,
    },
    /// The stream reports an error; the message becomes incomplete with that error.
    Error {
        /// What went wrong.
        error: String,
        /// A machine-readable code.
        #[serde(default)]
        code: Option<String>,
        /// `critical`, `warning` or `info`.
        #[serde(default)]
        severity: Option<String>,
    },
    /// Apply state operations to the message's state.
    UpdateState {
        /// The operations (`set` and `append-text`).
        operations: Value,
    },
    /// Drop the parts added since the current step started (AI SDK
    /// `reset-step`). Produced by decoders only; not a native wire chunk.
    #[serde(skip_deserializing)]
    ResetStep,
}

/// A decoded chunk (`AssistantStreamChunk`).
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Chunk {
    /// Which part the chunk applies to: `[index]`, or empty for message-level chunks.
    #[serde(default)]
    pub path: Vec<usize>,
    /// What the chunk does.
    #[serde(flatten)]
    pub kind: ChunkKind,
}

impl Chunk {
    /// A chunk for `path` with the given payload.
    pub fn new(path: Vec<usize>, kind: ChunkKind) -> Self {
        Self { path, kind }
    }
}
