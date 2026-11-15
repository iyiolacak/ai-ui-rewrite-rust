//! Folds stream chunks into an assistant message, ported from
//! `assistant-message-accumulator.ts`.
//!
//! The accumulator owns the message being built. After any chunk,
//! [`MessageAccumulator::result`] is the cumulative [`ChatModelRunResult`] a
//! model adapter yields to the runtime. As upstream, a chunk addressing a part
//! that does not exist or has the wrong type is dropped with a warning instead
//! of failing the stream. Timing metadata is not computed (it needs a clock).

use crate::chunk::{Chunk, ChunkKind, ChunkUsage, FinishReason, PartInit};
use crate::partial_json::{Limits as JsonLimits, Parser};
use crate::state::{decode_operations, Limits as StateLimits, State};
use ai_ui_core::adapter::RunResultMetadata;
use ai_ui_core::{
    AssistantMessagePart, ChatModelRunResult, CompleteReason, DataPart, FilePart, IncompleteReason,
    MessageStatus, PartStatus, ReasoningPart, RequiresActionReason, SourcePart, TextPart,
    ThreadStep, ToolCallPart, Usage,
};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};

/// At most this many distinct warnings are kept (`MAX_WARNED_KEYS`).
const MAX_WARNINGS: usize = 16;

/// Resource limits for content the stream controls.
#[derive(Clone, Copy, Debug, Default)]
pub struct Limits {
    /// Bounds for the incremental parser of streamed tool arguments.
    pub tool_arguments: JsonLimits,
    /// Bounds for message state operations.
    pub state: StateLimits,
}

#[derive(Default)]
struct Step {
    message_id: Option<String>,
    usage: Option<Usage>,
    finished: bool,
    finish_reason: Option<String>,
    is_continued: Option<bool>,
}

/// Folds stream chunks into one assistant message; `result()` is the cumulative message so far.
pub struct MessageAccumulator {
    limits: Limits,
    parts: Vec<AssistantMessagePart>,
    status: MessageStatus,
    annotations: Vec<Value>,
    data: Vec<Value>,
    steps: Vec<Step>,
    state: Option<State>,
    initial_state: Value,
    /// How many parts existed when the current step started (`reset-step` keeps them).
    step_boundary: usize,
    /// Incremental argument parsers by part index.
    parsers: HashMap<usize, Parser>,
    warned: BTreeSet<String>,
    warnings: Vec<String>,
}

/// Summarises state; message content is not printed.
impl std::fmt::Debug for MessageAccumulator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessageAccumulator")
            .field("parts", &self.parts.len())
            .field("status", &self.status)
            .field("steps", &self.steps.len())
            .field("warnings", &self.warnings.len())
            .finish_non_exhaustive()
    }
}

impl Default for MessageAccumulator {
    fn default() -> Self {
        Self::new(Limits::default())
    }
}

impl MessageAccumulator {
    /// An accumulator with the given limits.
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            parts: Vec::new(),
            status: MessageStatus::Running,
            annotations: Vec::new(),
            data: Vec::new(),
            steps: Vec::new(),
            state: None,
            initial_state: Value::Null,
            step_boundary: 0,
            parsers: HashMap::new(),
            warned: BTreeSet::new(),
            warnings: Vec::new(),
        }
    }

    /// Start from existing thread state (`unstable_state`), which
    /// `update-state` operations then modify.
    pub fn with_state(mut self, state: Value) -> Self {
        self.initial_state = state;
        self
    }

    /// The content parts so far.
    pub fn parts(&self) -> &[AssistantMessagePart] {
        &self.parts
    }
    /// The stream ended cleanly. A message still running settles as upstream's
    /// accumulator does: `requires-action` when a tool call has no result yet,
    /// otherwise `complete` with an unknown reason.
    pub fn finish(&mut self) {
        if self.status != MessageStatus::Running {
            return;
        }
        let unresolved = self
            .parts
            .iter()
            .any(|part| matches!(part, AssistantMessagePart::ToolCall(call) if call.is_open()));
        self.status = if unresolved {
            MessageStatus::RequiresAction {
                reason: RequiresActionReason::ToolCalls,
            }
        } else {
            MessageStatus::Complete {
                reason: CompleteReason::Unknown,
            }
        };
    }

    /// The message status so far.
    pub fn status(&self) -> &MessageStatus {
        &self.status
    }
    /// Dropped-chunk warnings, deduplicated by cause.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// The message so far, as a cumulative adapter result.
    pub fn result(&self) -> ChatModelRunResult {
        ChatModelRunResult {
            content: Some(self.parts.clone()),
            status: Some(self.status.clone()),
            metadata: Some(RunResultMetadata {
                state: self.state.as_ref().map(|s| s.value().clone()),
                annotations: Some(self.annotations.clone()),
                data: Some(self.data.clone()),
                steps: Some(
                    self.steps
                        .iter()
                        .map(|s| ThreadStep {
                            message_id: s.message_id.clone(),
                            usage: s.usage.clone(),
                            finish_reason: s.finish_reason.clone(),
                            is_continued: s.is_continued,
                        })
                        .collect(),
                ),
                ..Default::default()
            }),
        }
    }

    fn warn(&mut self, key: String, message: String) {
        if self.warned.len() < MAX_WARNINGS && self.warned.insert(key) {
            self.warnings.push(message);
        }
    }

    /// The index addressed by a single-element path, if that part exists.
    fn target(&mut self, chunk: &Chunk, kind: &str) -> Option<usize> {
        match chunk.path.as_slice() {
            [index] if *index < self.parts.len() => Some(*index),
            _ => {
                self.warn(
                    format!("no-part:{kind}"),
                    format!("Dropped {kind} chunk: no part at path {:?}", chunk.path),
                );
                None
            }
        }
    }

    /// Apply one chunk. Invalid chunks are dropped with a warning, never a panic.
    pub fn push(&mut self, chunk: Chunk) {
        match chunk.kind {
            ChunkKind::PartStart { ref part } => {
                let part = self.start_part(part.clone());
                self.parts.push(part);
            }
            ChunkKind::TextDelta {
                ref text_delta,
                ref provider_metadata,
            } => {
                let Some(index) = self.target(&chunk, "text-delta") else {
                    return;
                };
                self.text_delta(index, text_delta, provider_metadata.clone());
            }
            ChunkKind::ToolCallArgsTextFinish => {
                let Some(index) = self.target(&chunk, "tool-call-args-text-finish") else {
                    return;
                };
                if !matches!(self.parts[index], AssistantMessagePart::ToolCall(_)) {
                    self.warn(
                        "wrong-part:tool-call-args-text-finish".into(),
                        "Dropped tool-call-args-text-finish chunk: part is not a tool-call".into(),
                    );
                }
            }
            ChunkKind::ToolCallApproval { ref approval } => {
                let Some(index) = self.target(&chunk, "tool-call-approval") else {
                    return;
                };
                match &mut self.parts[index] {
                    AssistantMessagePart::ToolCall(call) => call.approval = Some(approval.clone()),
                    _ => self.warn(
                        "wrong-part:tool-call-approval".into(),
                        "Dropped tool-call-approval chunk: part is not a tool-call".into(),
                    ),
                }
            }
            ChunkKind::PartFinish {
                ref provider_metadata,
            } => {
                let Some(index) = self.target(&chunk, "part-finish") else {
                    return;
                };
                finish_part(&mut self.parts[index], provider_metadata.clone());
            }
            ChunkKind::Result {
                ref artifact,
                ref result,
                is_error,
                is_preliminary,
                ref model_content,
                ref messages,
            } => {
                let Some(index) = self.target(&chunk, "result") else {
                    return;
                };
                let AssistantMessagePart::ToolCall(call) = &mut self.parts[index] else {
                    self.warn(
                        "wrong-part:result".into(),
                        "Dropped result chunk: part is not a tool-call".into(),
                    );
                    return;
                };
                // A preliminary result never replaces a final one.
                if is_preliminary && call.result.is_some() && call.is_preliminary != Some(true) {
                    return;
                }
                call.result = Some(result.clone());
                call.is_error = Some(is_error);
                call.is_preliminary = is_preliminary.then_some(true);
                call.artifact = artifact.clone();
                replace_extra(&mut call.extra, "modelContent", model_content.clone());
                replace_extra(&mut call.extra, "messages", messages.clone());
            }
            ChunkKind::MessageFinish { finish_reason, .. } => {
                let errored = matches!(
                    self.status,
                    MessageStatus::Incomplete {
                        reason: IncompleteReason::Error,
                        ..
                    }
                );
                if !errored {
                    self.status = status_for(finish_reason);
                }
            }
            ChunkKind::Annotations { annotations } => self.annotations.extend(annotations),
            ChunkKind::Data { data } => {
                for entry in data {
                    // An entry with a name and an id replaces the earlier one with both.
                    let slot = entry.get("id").filter(|id| !id.is_null()).and_then(|id| {
                        self.data.iter().position(|old| {
                            old.get("id") == Some(id) && old.get("name") == entry.get("name")
                        })
                    });
                    match slot {
                        Some(index) => self.data[index] = entry,
                        None => self.data.push(entry),
                    }
                }
            }
            ChunkKind::ResetStep => {
                self.parts.truncate(self.step_boundary);
                let boundary = self.step_boundary;
                self.parsers.retain(|index, _| *index < boundary);
            }
            ChunkKind::StepStart { message_id } => {
                self.step_boundary = self.parts.len();
                self.steps.push(Step {
                    message_id: Some(message_id),
                    ..Step::default()
                });
            }
            ChunkKind::StepFinish {
                usage,
                finish_reason,
                is_continued,
            } => self.finish_step(usage, finish_reason, is_continued),
            ChunkKind::Error {
                error,
                code,
                severity,
            } => {
                let mut detail =
                    json!({ "code": code.unwrap_or_else(|| "unknown".into()), "message": error });
                if let Some(severity) =
                    severity.filter(|s| matches!(s.as_str(), "critical" | "warning" | "info"))
                {
                    detail["severity"] = Value::String(severity);
                }
                self.status = MessageStatus::Incomplete {
                    reason: IncompleteReason::Error,
                    error: Some(detail),
                };
            }
            ChunkKind::UpdateState { ref operations } => self.update_state(operations),
        }
    }

    fn start_part(&mut self, init: PartInit) -> AssistantMessagePart {
        let running = Some(PartStatus::Running);
        match init {
            PartInit::Text {
                provider_metadata,
                parent_id,
            } => {
                let mut part = TextPart {
                    status: running,
                    parent_id,
                    ..Default::default()
                };
                set_extra(
                    &mut part.extra,
                    "providerMetadata",
                    provider_metadata.map(Value::Object),
                );
                AssistantMessagePart::Text(part)
            }
            PartInit::Reasoning {
                summary,
                provider_metadata,
                parent_id,
            } => {
                let mut part = ReasoningPart {
                    status: running,
                    summary,
                    parent_id,
                    ..Default::default()
                };
                set_extra(
                    &mut part.extra,
                    "providerMetadata",
                    provider_metadata.map(Value::Object),
                );
                AssistantMessagePart::Reasoning(part)
            }
            PartInit::ToolCall {
                tool_call_id,
                tool_name,
                parent_id,
            } => AssistantMessagePart::ToolCall(ToolCallPart {
                tool_call_id,
                tool_name,
                parent_id,
                ..Default::default()
            }),
            PartInit::Source {
                source_type,
                id,
                url,
                title,
                media_type,
                filename,
                parent_id,
            } => AssistantMessagePart::Source(if source_type == "document" {
                SourcePart::Document {
                    id,
                    title: title.unwrap_or_default(),
                    media_type: media_type.unwrap_or_else(|| "application/octet-stream".into()),
                    filename,
                    parent_id,
                    extra: Default::default(),
                }
            } else {
                SourcePart::Url {
                    id,
                    url: url.unwrap_or_default(),
                    title,
                    parent_id,
                    extra: Default::default(),
                }
            }),
            PartInit::File {
                data,
                mime_type,
                parent_id,
            } => AssistantMessagePart::File(FilePart {
                data,
                mime_type,
                parent_id,
                ..Default::default()
            }),
            PartInit::Data {
                name,
                data,
                parent_id,
            } => {
                let mut extra = serde_json::Map::new();
                set_extra(&mut extra, "parentId", parent_id.map(Value::String));
                AssistantMessagePart::Data(DataPart {
                    id: None,
                    name,
                    data,
                    extra,
                })
            }
            PartInit::Unsupported => {
                self.warn(
                    "unsupported-part-start".into(),
                    "Unsupported part-start type: inserting an empty reasoning part to preserve part indices".into(),
                );
                AssistantMessagePart::Reasoning(ReasoningPart {
                    status: running,
                    ..Default::default()
                })
            }
        }
    }

    fn text_delta(
        &mut self,
        index: usize,
        delta: &str,
        provider_metadata: Option<serde_json::Map<String, Value>>,
    ) {
        match &mut self.parts[index] {
            AssistantMessagePart::Text(part) => {
                part.text.push_str(delta);
                set_extra(
                    &mut part.extra,
                    "providerMetadata",
                    provider_metadata.map(Value::Object),
                );
            }
            AssistantMessagePart::Reasoning(part) => {
                part.text.push_str(delta);
                set_extra(
                    &mut part.extra,
                    "providerMetadata",
                    provider_metadata.map(Value::Object),
                );
            }
            AssistantMessagePart::ToolCall(call) => {
                call.args_text.push_str(delta);
                let limits = self.limits.tool_arguments;
                let parser = match self.parsers.remove(&index) {
                    Some(parser) => Ok(parser),
                    None => Parser::new(limits),
                };
                let parsed = parser.and_then(|mut parser| {
                    // A fresh parser replays everything streamed so far.
                    let text = if parser.text_len() == 0 {
                        call.args_text.as_str()
                    } else {
                        delta
                    };
                    parser.append(text).map(|()| parser)
                });
                match parsed {
                    Ok(parser) => {
                        if let Value::Object(args) = &parser.snapshot().value {
                            call.args = args.clone();
                        }
                        self.parsers.insert(index, parser);
                    }
                    Err(_) => self.warn(
                        "tool-args-parse".into(),
                        "Tool call arguments could not be parsed incrementally; argsText is kept"
                            .into(),
                    ),
                }
            }
            _ => self.warn(
                "wrong-part:text-delta".into(),
                "Dropped text-delta chunk: part is neither text nor tool-call".into(),
            ),
        }
    }

    fn finish_step(&mut self, usage: ChunkUsage, reason: FinishReason, is_continued: bool) {
        let usage = Some(Usage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
        });
        let finish_reason = serde_json::to_value(reason)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned));
        if self.steps.last().is_none_or(|s| s.finished) {
            self.steps.push(Step::default());
        }
        let step = self.steps.last_mut().expect("pushed above");
        step.usage = usage;
        step.finished = true;
        step.finish_reason = finish_reason;
        step.is_continued = Some(is_continued);
    }

    fn update_state(&mut self, operations: &Value) {
        let limits = self.limits.state;
        let applied = decode_operations(operations, limits)
            .map_err(|e| e.to_string())
            .and_then(|operations| {
                if self.state.is_none() {
                    self.state = Some(
                        State::new(self.initial_state.clone(), limits)
                            .map_err(|e| e.to_string())?,
                    );
                }
                self.state
                    .as_mut()
                    .expect("initialized above")
                    .apply(&operations)
                    .map_err(|e| e.to_string())
            });
        if let Err(error) = applied {
            self.warn(
                format!("update-state:{error}"),
                format!("Dropped update-state chunk: {error}"),
            );
        }
    }
}

fn finish_part(
    part: &mut AssistantMessagePart,
    provider_metadata: Option<serde_json::Map<String, Value>>,
) {
    match part {
        AssistantMessagePart::ToolCall(call) if call.is_preliminary == Some(true) => {}
        AssistantMessagePart::Text(text) => {
            text.status = Some(PartStatus::Complete);
            set_extra(
                &mut text.extra,
                "providerMetadata",
                provider_metadata.map(Value::Object),
            );
        }
        AssistantMessagePart::Reasoning(reasoning) => {
            reasoning.status = Some(PartStatus::Complete);
            set_extra(
                &mut reasoning.extra,
                "providerMetadata",
                provider_metadata.map(Value::Object),
            );
        }
        _ => {}
    }
}

fn status_for(reason: FinishReason) -> MessageStatus {
    match reason {
        FinishReason::ToolCalls => MessageStatus::RequiresAction {
            reason: RequiresActionReason::ToolCalls,
        },
        FinishReason::Stop => MessageStatus::Complete {
            reason: CompleteReason::Stop,
        },
        FinishReason::Unknown => MessageStatus::Complete {
            reason: CompleteReason::Unknown,
        },
        FinishReason::Length => MessageStatus::Incomplete {
            reason: IncompleteReason::Length,
            error: None,
        },
        FinishReason::ContentFilter => MessageStatus::Incomplete {
            reason: IncompleteReason::ContentFilter,
            error: None,
        },
        FinishReason::Error => MessageStatus::Incomplete {
            reason: IncompleteReason::Error,
            error: None,
        },
        FinishReason::Other | FinishReason::Interrupted => MessageStatus::Incomplete {
            reason: IncompleteReason::Other,
            error: None,
        },
    }
}

/// Set an unmodelled field when a value is present; keep it otherwise.
fn set_extra(extra: &mut serde_json::Map<String, Value>, key: &str, value: Option<Value>) {
    if let Some(value) = value {
        extra.insert(key.to_owned(), value);
    }
}

/// Set an unmodelled field, or remove it when absent.
fn replace_extra(extra: &mut serde_json::Map<String, Value>, key: &str, value: Option<Value>) {
    match value {
        Some(value) => extra.insert(key.to_owned(), value),
        None => extra.remove(key),
    };
}
