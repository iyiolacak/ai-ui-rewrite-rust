//! Builds accumulator [`Chunk`]s from part-level operations, like upstream's
//! `AssistantStreamController` (`addTextPart`, `appendText`, `addToolCallPart`…).
//! Protocol decoders use it so every wire format ends in the same chunks.

use crate::chunk::{Chunk, ChunkKind, ChunkUsage, FinishReason, PartInit};
use ai_ui_core::{JsonObject, ToolApproval};
use serde_json::Value;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Open {
    Text,
    Reasoning,
    Other,
    Closed,
}

#[derive(Default)]
pub(crate) struct PartWriter {
    /// Open state and parent id of each part, by index.
    parts: Vec<(Open, Option<String>)>,
    /// How many parts existed when the current step started.
    boundary: usize,
    out: Vec<Chunk>,
}

impl PartWriter {
    pub(crate) fn take(&mut self) -> Vec<Chunk> {
        std::mem::take(&mut self.out)
    }

    fn emit(&mut self, path: Vec<usize>, kind: ChunkKind) {
        self.out.push(Chunk::new(path, kind));
    }

    /// Start a part and return its index.
    pub(crate) fn add(&mut self, part: PartInit) -> usize {
        let open = match part {
            PartInit::Text { .. } => Open::Text,
            PartInit::Reasoning { .. } => Open::Reasoning,
            _ => Open::Other,
        };
        let parent = parent_of(&part);
        self.emit(Vec::new(), ChunkKind::PartStart { part });
        self.parts.push((open, parent));
        self.parts.len() - 1
    }

    pub(crate) fn add_text(
        &mut self,
        provider_metadata: Option<JsonObject>,
        reasoning: bool,
    ) -> usize {
        self.add_text_under(provider_metadata, reasoning, None, None)
    }

    pub(crate) fn add_text_under(
        &mut self,
        provider_metadata: Option<JsonObject>,
        reasoning: bool,
        parent_id: Option<String>,
        summary: Option<String>,
    ) -> usize {
        self.add(if reasoning {
            PartInit::Reasoning {
                summary,
                provider_metadata,
                parent_id,
            }
        } else {
            PartInit::Text {
                provider_metadata,
                parent_id,
            }
        })
    }

    pub(crate) fn delta(
        &mut self,
        index: usize,
        text: &str,
        provider_metadata: Option<JsonObject>,
    ) {
        if !text.is_empty() || provider_metadata.is_some() {
            self.emit(
                vec![index],
                ChunkKind::TextDelta {
                    text_delta: text.to_owned(),
                    provider_metadata,
                },
            );
        }
    }

    /// Append to the last part if it is an open part of that kind under the
    /// same parent, else start one (`appendText` / `appendReasoning`).
    pub(crate) fn append_text(&mut self, text: &str, reasoning: bool) {
        self.append_text_under(text, reasoning, None);
    }

    pub(crate) fn append_text_under(
        &mut self,
        text: &str,
        reasoning: bool,
        parent_id: Option<String>,
    ) {
        let want = if reasoning {
            Open::Reasoning
        } else {
            Open::Text
        };
        let index = match self.parts.last() {
            Some((open, parent)) if *open == want && *parent == parent_id => self.parts.len() - 1,
            _ => self.add_text_under(None, reasoning, parent_id, None),
        };
        self.delta(index, text, None);
    }

    /// Close every part still open as text or reasoning.
    pub(crate) fn close_open_text(&mut self) {
        let open: Vec<usize> = (0..self.parts.len())
            .filter(|&i| matches!(self.parts[i].0, Open::Text | Open::Reasoning))
            .collect();
        for index in open {
            self.close(index, None);
        }
    }

    /// Finish a part once; later calls are ignored.
    pub(crate) fn close(&mut self, index: usize, provider_metadata: Option<JsonObject>) {
        if let Some((open, _)) = self.parts.get_mut(index) {
            if *open != Open::Closed {
                *open = Open::Closed;
                self.emit(vec![index], ChunkKind::PartFinish { provider_metadata });
            }
        }
    }

    pub(crate) fn add_closed(&mut self, part: PartInit) -> usize {
        let index = self.add(part);
        self.close(index, None);
        index
    }

    pub(crate) fn args_finish(&mut self, index: usize) {
        self.emit(vec![index], ChunkKind::ToolCallArgsTextFinish);
    }

    pub(crate) fn approval(&mut self, index: usize, approval: ToolApproval) {
        self.emit(vec![index], ChunkKind::ToolCallApproval { approval });
    }

    pub(crate) fn result(&mut self, index: usize, result: ToolOutput) {
        self.emit(
            vec![index],
            ChunkKind::Result {
                artifact: result.artifact,
                result: result.value,
                is_error: result.is_error,
                is_preliminary: result.is_preliminary,
                model_content: result.model_content,
                messages: result.messages,
            },
        );
    }

    pub(crate) fn step_start(&mut self, message_id: String) {
        self.boundary = self.parts.len();
        self.emit(Vec::new(), ChunkKind::StepStart { message_id });
    }

    /// Parts at or after this index belong to the current step.
    pub(crate) fn boundary(&self) -> usize {
        self.boundary
    }

    /// Discard the current step's parts (`reset-step`).
    pub(crate) fn reset_step(&mut self) {
        self.parts.truncate(self.boundary);
        self.emit(Vec::new(), ChunkKind::ResetStep);
    }

    pub(crate) fn step_finish(
        &mut self,
        finish_reason: FinishReason,
        usage: ChunkUsage,
        is_continued: bool,
    ) {
        self.emit(
            Vec::new(),
            ChunkKind::StepFinish {
                finish_reason,
                usage,
                is_continued,
            },
        );
    }

    pub(crate) fn message_finish(&mut self, finish_reason: FinishReason, usage: ChunkUsage) {
        self.emit(
            Vec::new(),
            ChunkKind::MessageFinish {
                finish_reason,
                usage,
            },
        );
    }

    pub(crate) fn data(&mut self, data: Vec<Value>) {
        self.emit(Vec::new(), ChunkKind::Data { data });
    }

    pub(crate) fn annotations(&mut self, annotations: Vec<Value>) {
        self.emit(Vec::new(), ChunkKind::Annotations { annotations });
    }

    pub(crate) fn error(&mut self, error: String) {
        self.error_with(error, None, None);
    }

    pub(crate) fn error_with(
        &mut self,
        error: String,
        code: Option<String>,
        severity: Option<String>,
    ) {
        self.emit(
            Vec::new(),
            ChunkKind::Error {
                error,
                code,
                severity,
            },
        );
    }

    pub(crate) fn update_state(&mut self, operations: Value) {
        self.emit(Vec::new(), ChunkKind::UpdateState { operations });
    }
}

fn parent_of(part: &PartInit) -> Option<String> {
    match part {
        PartInit::Text { parent_id, .. }
        | PartInit::Reasoning { parent_id, .. }
        | PartInit::ToolCall { parent_id, .. }
        | PartInit::Source { parent_id, .. }
        | PartInit::File { parent_id, .. }
        | PartInit::Data { parent_id, .. } => parent_id.clone(),
        PartInit::Unsupported => None,
    }
}

/// A tool result as the protocols carry it.
#[derive(Default)]
pub(crate) struct ToolOutput {
    pub value: Value,
    pub is_error: bool,
    pub is_preliminary: bool,
    pub artifact: Option<Value>,
    pub model_content: Option<Value>,
    pub messages: Option<Value>,
}
