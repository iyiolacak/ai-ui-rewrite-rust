//! The AI SDK UI message stream (`ui-message-stream`), ported from
//! `serialization/ui-message-stream`: SSE JSON chunks such as `text-delta`,
//! `tool-input-available` and `finish`, as served by AI SDK v5 backends.
//!
//! The normalizer maps wire variants onto one chunk set (for example tool
//! input is buffered until it is complete, as upstream does); the decoder then
//! writes parts through an internal part writer so the accumulator sees ordinary chunks.

use crate::chunk::{Chunk, ChunkUsage, FinishReason, PartInit};
use crate::decoder::StreamDecoder;
use crate::sse_json::{SseEvents, SseLimits};
use crate::writer::{PartWriter, ToolOutput};
use ai_ui_core::{JsonObject, ToolApproval};
use serde_json::{json, Value};
use std::collections::HashMap;

struct PendingTool {
    tool_call_id: String,
    tool_name: String,
    deltas: Vec<String>,
    emitted: bool,
    has_final_result: bool,
    results: Vec<(Value, bool, bool)>,
}

impl PendingTool {
    fn new(tool_call_id: &str, tool_name: Option<&str>) -> Self {
        Self {
            tool_call_id: tool_call_id.to_owned(),
            tool_name: tool_name.unwrap_or_default().to_owned(),
            deltas: Vec::new(),
            emitted: false,
            has_final_result: false,
            results: Vec::new(),
        }
    }
}

/// Decodes a UI message stream response body into accumulator chunks.
#[derive(Default)]
pub struct UiMessageStreamDecoder {
    events: SseEvents,
    writer: PartWriter,
    pending: Vec<PendingTool>,
    /// Open text (`[0]`) and reasoning (`[1]`) parts by stream id.
    text_parts: [HashMap<String, usize>; 2],
    active_text: [Option<String>; 2],
    tools: HashMap<String, usize>,
    args_open: HashMap<usize, bool>,
    active_tool: Option<String>,
    current_message_id: Option<String>,
    step_opened_by_start: bool,
    approvals: Vec<(String, ToolApproval)>,
    next_id: u64,
    warnings: Vec<String>,
}

fn s<'a>(chunk: &'a Value, key: &str) -> Option<&'a str> {
    chunk.get(key).and_then(Value::as_str)
}
fn object(chunk: &Value, key: &str) -> Option<JsonObject> {
    chunk.get(key).and_then(Value::as_object).cloned()
}
fn finish_reason(chunk: &Value) -> FinishReason {
    chunk
        .get("finishReason")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or(FinishReason::Unknown)
}
fn usage(chunk: &Value) -> ChunkUsage {
    chunk
        .get("usage")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default()
}

impl UiMessageStreamDecoder {
    /// A decoder for the AI SDK UI message stream with the given size bounds.
    pub fn new(limits: SseLimits) -> Self {
        Self {
            events: SseEvents::new(limits),
            ..Self::default()
        }
    }

    /// Chunks dropped as malformed, for diagnostics.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    fn generate_id(&mut self) -> String {
        self.next_id += 1;
        format!("ui-msg-{}", self.next_id)
    }

    fn pending_index(&mut self, tool_call_id: &str, tool_name: Option<&str>) -> usize {
        match self
            .pending
            .iter()
            .position(|t| t.tool_call_id == tool_call_id)
        {
            Some(index) => index,
            None => {
                self.pending.push(PendingTool::new(tool_call_id, tool_name));
                self.pending.len() - 1
            }
        }
    }

    fn emit_pending(&mut self, index: usize, out: &mut Vec<Value>) {
        let tool = &mut self.pending[index];
        if !tool.emitted {
            tool.emitted = true;
            out.push(json!({ "type": "tool-call-start", "toolCallId": tool.tool_call_id, "toolName": tool.tool_name }));
            out.extend(
                tool.deltas
                    .iter()
                    .map(|d| json!({ "type": "tool-call-delta", "argsText": d })),
            );
            out.push(json!({ "type": "tool-call-end" }));
        }
        for (result, is_error, is_preliminary) in tool.results.drain(..) {
            let mut chunk =
                json!({ "type": "tool-result", "toolCallId": tool.tool_call_id, "result": result });
            if is_error {
                chunk["isError"] = Value::Bool(true);
            }
            if is_preliminary {
                chunk["isPreliminary"] = Value::Bool(true);
            }
            out.push(chunk);
        }
    }

    /// `createChunkNormalizer`: wire chunk → zero or more normalized chunks.
    fn normalize(&mut self, mut chunk: Value) -> Vec<Value> {
        let mut out = Vec::new();
        let kind = s(&chunk, "type").unwrap_or_default().to_owned();
        match kind.as_str() {
            "abort" => {
                self.pending.clear();
                out.push(json!({ "type": "finish", "finishReason": "other", "usage": { "inputTokens": 0, "outputTokens": 0 } }));
            }
            "start" => {
                if s(&chunk, "messageId").is_none() {
                    chunk["messageId"] = Value::String(self.generate_id());
                }
                out.push(chunk);
            }
            "source-url" => {
                let mut source =
                    json!({ "sourceType": "url", "id": chunk["sourceId"], "url": chunk["url"] });
                if let Some(title) = s(&chunk, "title").filter(|t| !t.is_empty()) {
                    source["title"] = Value::String(title.to_owned());
                }
                out.push(json!({ "type": "source", "source": source }));
            }
            "source" if chunk.get("source").is_none_or(Value::is_null) => {}
            "file" if chunk.get("file").is_none_or(Value::is_null) => {
                if let Some(url) = chunk.get("url").filter(|u| !u.is_null()) {
                    out.push(json!({ "type": "file", "file": { "mimeType": chunk["mediaType"], "data": url } }));
                }
            }
            "tool-input-start" => {
                if let Some(id) = s(&chunk, "toolCallId") {
                    if !self.pending.iter().any(|t| t.tool_call_id == id) {
                        self.pending
                            .push(PendingTool::new(id, s(&chunk, "toolName")));
                    }
                }
            }
            "tool-input-delta" => {
                if let (Some(id), Some(delta)) =
                    (s(&chunk, "toolCallId"), s(&chunk, "inputTextDelta"))
                {
                    if let Some(tool) = self
                        .pending
                        .iter_mut()
                        .find(|t| t.tool_call_id == id && !t.emitted)
                    {
                        tool.deltas.push(delta.to_owned());
                    }
                }
            }
            "tool-input-available" | "tool-input-error" => {
                let Some(id) = s(&chunk, "toolCallId").map(str::to_owned) else {
                    return out;
                };
                let index = self.pending_index(&id, s(&chunk, "toolName"));
                if self.pending[index].emitted {
                    return out;
                }
                if let Some(input) = chunk.get("input") {
                    let text = match (kind.as_str(), input) {
                        ("tool-input-error", Value::String(raw)) => raw.clone(),
                        _ => input.to_string(),
                    };
                    self.pending[index].deltas = vec![text];
                }
                if kind == "tool-input-error" {
                    let error = s(&chunk, "errorText").unwrap_or_default().to_owned();
                    self.pending[index]
                        .results
                        .push((Value::String(error), true, false));
                    self.pending[index].has_final_result = true;
                }
                self.emit_pending(index, &mut out);
            }
            "tool-output-available" | "tool-output-error" => {
                let Some(id) = s(&chunk, "toolCallId").map(str::to_owned) else {
                    return out;
                };
                if kind == "tool-output-available" && chunk.get("output").is_none() {
                    return out;
                }
                let opens_call = !self.pending.iter().any(|t| t.tool_call_id == id);
                let index = self.pending_index(&id, None);
                if self.pending[index].has_final_result {
                    return out;
                }
                let (result, is_error, preliminary) = if kind == "tool-output-error" {
                    (
                        Value::String(s(&chunk, "errorText").unwrap_or_default().to_owned()),
                        true,
                        false,
                    )
                } else {
                    (
                        chunk["output"].clone(),
                        false,
                        chunk.get("preliminary").and_then(Value::as_bool) == Some(true),
                    )
                };
                let tool = &mut self.pending[index];
                tool.results.push((result, is_error, preliminary));
                if !preliminary {
                    tool.has_final_result = true;
                }
                if tool.emitted || opens_call {
                    self.emit_pending(index, &mut out);
                }
            }
            "tool-approval-request" => {
                if let (Some(tool_call_id), Some(approval_id)) =
                    (s(&chunk, "toolCallId"), s(&chunk, "approvalId"))
                {
                    let mut approval = json!({ "id": approval_id });
                    if let Some(descriptor) =
                        chunk.get("approvalDescriptor").filter(|v| !v.is_null())
                    {
                        approval["descriptor"] = descriptor.clone();
                    }
                    if let Some(input) = chunk.get("inputSchemaInput") {
                        approval["inputSchemaInput"] = input.clone();
                    }
                    if let Some(reason) = s(&chunk, "reason") {
                        approval["prompt"] = Value::String(reason.to_owned());
                        approval["requestReason"] = Value::String(reason.to_owned());
                    }
                    if let Some(automatic) = chunk.get("isAutomatic").filter(|v| v.is_boolean()) {
                        approval["isAutomatic"] = automatic.clone();
                    }
                    if let Some(signature) = s(&chunk, "signature") {
                        approval["signature"] = Value::String(signature.to_owned());
                    }
                    out.push(json!({ "type": "tool-approval-request", "toolCallId": tool_call_id, "approval": approval }));
                }
            }
            _ => out.push(chunk),
        }
        out
    }

    fn close_text_parts(&mut self) {
        for kind in 0..2 {
            let mut open: Vec<usize> = self.text_parts[kind]
                .drain()
                .map(|(_, index)| index)
                .collect();
            open.sort_unstable();
            for index in open {
                self.writer.close(index, None);
            }
            self.active_text[kind] = None;
        }
    }

    fn close_args(&mut self, index: usize) {
        if self.args_open.insert(index, false) == Some(true) {
            self.writer.args_finish(index);
        }
    }

    /// The decoder proper, on normalized chunks.
    fn transform(&mut self, chunk: Value) -> Result<(), String> {
        let kind = s(&chunk, "type").unwrap_or_default();
        if let Some(name) = kind.strip_prefix("data-") {
            if chunk.get("transient").and_then(Value::as_bool) != Some(true) {
                let data = chunk.get("data").cloned().unwrap_or(Value::Null);
                let mut entry = json!({ "name": name, "data": data });
                // A part with an id is replaced by the next one with the same name and id.
                if let Some(id) = s(&chunk, "id") {
                    entry["id"] = json!(id);
                }
                self.writer.data(vec![entry]);
            }
            return Ok(());
        }
        match kind {
            "start" => {
                let id = s(&chunk, "messageId").unwrap_or_default().to_owned();
                self.current_message_id = Some(id.clone());
                self.step_opened_by_start = true;
                self.writer.step_start(id);
            }
            "text-start" | "reasoning-start" => {
                let slot = usize::from(kind == "reasoning-start");
                let id = s(&chunk, "id").unwrap_or_default().to_owned();
                if let Some(previous) = self.text_parts[slot].remove(&id) {
                    self.writer.close(previous, None);
                }
                let index = self
                    .writer
                    .add_text(object(&chunk, "providerMetadata"), slot == 1);
                self.text_parts[slot].insert(id.clone(), index);
                self.active_text[slot] = Some(id);
            }
            "text-delta" | "reasoning-delta" => {
                let slot = usize::from(kind == "reasoning-delta");
                let delta = if slot == 0 {
                    chunk.get("textDelta").or_else(|| chunk.get("delta"))
                } else {
                    chunk.get("delta")
                }
                .and_then(Value::as_str)
                .unwrap_or_default();
                let id = s(&chunk, "id").or(self.active_text[slot].as_deref());
                match id.and_then(|id| self.text_parts[slot].get(id).copied()) {
                    Some(index) => {
                        self.writer
                            .delta(index, delta, object(&chunk, "providerMetadata"))
                    }
                    None => self.writer.append_text(delta, slot == 1),
                }
            }
            "text-end" | "reasoning-end" => {
                let slot = usize::from(kind == "reasoning-end");
                if let Some(id) = s(&chunk, "id")
                    .map(str::to_owned)
                    .or_else(|| self.active_text[slot].clone())
                {
                    if let Some(index) = self.text_parts[slot].remove(&id) {
                        self.writer.close(index, object(&chunk, "providerMetadata"));
                    }
                    if self.active_text[slot].as_deref() == Some(id.as_str()) {
                        self.active_text[slot] = None;
                    }
                }
            }
            "source" => {
                let source = &chunk["source"];
                self.writer.add_closed(PartInit::Source {
                    source_type: s(source, "sourceType").unwrap_or("url").to_owned(),
                    id: s(source, "id").unwrap_or_default().to_owned(),
                    url: s(source, "url").map(str::to_owned),
                    title: s(source, "title").map(str::to_owned),
                    media_type: None,
                    filename: None,
                    parent_id: None,
                });
            }
            "source-document" => {
                self.writer.add_closed(PartInit::Source {
                    source_type: "document".into(),
                    id: s(&chunk, "sourceId").unwrap_or_default().to_owned(),
                    url: None,
                    title: s(&chunk, "title").map(str::to_owned),
                    media_type: s(&chunk, "mediaType").map(str::to_owned),
                    filename: s(&chunk, "filename").map(str::to_owned),
                    parent_id: None,
                });
            }
            "file" => {
                let file = &chunk["file"];
                self.writer.add_closed(PartInit::File {
                    data: s(file, "data").unwrap_or_default().to_owned(),
                    mime_type: s(file, "mimeType")
                        .unwrap_or("application/octet-stream")
                        .to_owned(),
                    parent_id: None,
                });
            }
            "tool-call-start" => {
                if let Some(active) = self.active_tool.take() {
                    if let Some(&index) = self.tools.get(&active) {
                        self.close_args(index);
                    }
                }
                let id = s(&chunk, "toolCallId").unwrap_or_default().to_owned();
                if !self.tools.contains_key(&id) {
                    let index = self.writer.add(PartInit::ToolCall {
                        tool_call_id: id.clone(),
                        tool_name: s(&chunk, "toolName").unwrap_or_default().to_owned(),
                        parent_id: None,
                    });
                    self.tools.insert(id.clone(), index);
                    self.args_open.insert(index, true);
                }
                self.active_tool = Some(id);
            }
            "tool-call-delta" => {
                if let Some(index) = self
                    .active_tool
                    .as_ref()
                    .and_then(|id| self.tools.get(id))
                    .copied()
                {
                    if self.args_open.get(&index) == Some(&true) {
                        self.writer
                            .delta(index, s(&chunk, "argsText").unwrap_or_default(), None);
                    }
                }
            }
            "tool-call-end" => {
                if let Some(index) = self
                    .active_tool
                    .take()
                    .and_then(|id| self.tools.get(&id).copied())
                {
                    self.close_args(index);
                }
            }
            "tool-result" => {
                let id = s(&chunk, "toolCallId").unwrap_or_default().to_owned();
                if self.active_tool.as_deref() == Some(id.as_str()) {
                    self.active_tool = None;
                }
                let index = *self
                    .tools
                    .get(&id)
                    .ok_or_else(|| format!("Encountered tool result with unknown id: {id}"))?;
                self.close_args(index);
                let flag = |key| chunk.get(key).and_then(Value::as_bool) == Some(true);
                self.writer.result(
                    index,
                    ToolOutput {
                        value: chunk["result"].clone(),
                        is_error: flag("isError"),
                        is_preliminary: flag("isPreliminary"),
                        messages: chunk.get("messages").cloned(),
                        ..ToolOutput::default()
                    },
                );
            }
            "tool-approval-request" => {
                let id = s(&chunk, "toolCallId").unwrap_or_default().to_owned();
                if let Some(&index) = self.tools.get(&id) {
                    if let Ok(approval) =
                        serde_json::from_value::<ToolApproval>(chunk["approval"].clone())
                    {
                        self.approvals.retain(|(tool, _)| *tool != id);
                        self.approvals.push((id, approval.clone()));
                        self.writer.approval(index, approval);
                    }
                }
            }
            "tool-approval-response" => {
                let approval_id = s(&chunk, "approvalId").unwrap_or_default();
                if let Some((tool, approval)) =
                    self.approvals.iter_mut().find(|(_, a)| a.id == approval_id)
                {
                    approval.approved = chunk.get("approved").and_then(Value::as_bool);
                    if let Some(reason) = s(&chunk, "reason") {
                        approval.reason = Some(reason.to_owned());
                    }
                    let (tool, approval) = (tool.clone(), approval.clone());
                    if let Some(&index) = self.tools.get(&tool) {
                        self.writer.approval(index, approval);
                    }
                }
            }
            "tool-output-denied" => {
                let id = s(&chunk, "toolCallId").unwrap_or_default().to_owned();
                if let Some(&index) = self.tools.get(&id) {
                    let reason = self
                        .approvals
                        .iter()
                        .find(|(tool, _)| *tool == id)
                        .and_then(|(_, a)| a.reason.clone())
                        .filter(|r| !r.is_empty())
                        .unwrap_or_else(|| "Tool approval denied".into());
                    self.close_args(index);
                    self.writer.result(
                        index,
                        ToolOutput {
                            value: json!({ "error": reason }),
                            is_error: true,
                            ..ToolOutput::default()
                        },
                    );
                }
            }
            "start-step" => {
                let id = s(&chunk, "messageId")
                    .map(str::to_owned)
                    .or_else(|| self.current_message_id.clone())
                    .unwrap_or_else(|| self.generate_id());
                let continuing =
                    self.step_opened_by_start && Some(&id) == self.current_message_id.as_ref();
                self.step_opened_by_start = false;
                if !continuing {
                    self.writer.step_start(id);
                }
            }
            "reset-step" => {
                // Discard what this step added; calls from earlier steps stay addressable.
                self.writer.reset_step();
                let boundary = self.writer.boundary();
                for slot in &mut self.text_parts {
                    slot.retain(|_, index| *index < boundary);
                }
                self.active_text.fill(None);
                self.tools.retain(|_, index| *index < boundary);
                self.args_open.retain(|index, _| *index < boundary);
                self.pending.retain(|tool| tool.emitted);
            }
            "finish-step" => {
                // Text and reasoning parts stay open across a step boundary, as in the AI SDK
                // (v7): only their own `-end` chunks (or the end of the message) close them.
                self.step_opened_by_start = false;
                let continued = chunk
                    .get("isContinued")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                self.writer
                    .step_finish(finish_reason(&chunk), usage(&chunk), continued);
            }
            "finish" => {
                self.close_text_parts();
                self.writer
                    .message_finish(finish_reason(&chunk), usage(&chunk));
            }
            "error" => self
                .writer
                .error(s(&chunk, "errorText").unwrap_or("unknown error").to_owned()),
            _ => {}
        }
        Ok(())
    }

    fn handle(&mut self, payload: &str) -> Result<(), String> {
        let chunk: Value = match serde_json::from_str(payload) {
            Ok(value @ Value::Object(_)) if value.get("type").is_some_and(Value::is_string) => {
                value
            }
            _ => {
                if self.warnings.len() < 16 {
                    self.warnings.push(format!(
                        "Dropped invalid UIMessageStream chunk: {}",
                        payload.chars().take(200).collect::<String>()
                    ));
                }
                return Ok(());
            }
        };
        // Most events in a live response are text/reasoning deltas. Their
        // aliases can be read directly without building a normalized event,
        // copying the delta, or allocating a one-element normalization Vec.
        if matches!(s(&chunk, "type"), Some("text-delta" | "reasoning-delta")) {
            return self.transform(chunk);
        }
        for normalized in self.normalize(chunk) {
            self.transform(normalized)?;
        }
        Ok(())
    }
}

impl StreamDecoder for UiMessageStreamDecoder {
    fn push(&mut self, input: &[u8]) -> Result<Vec<Chunk>, String> {
        for payload in self.events.push(input)? {
            self.handle(&payload)?;
        }
        Ok(self.writer.take())
    }

    fn finish(&mut self) -> Result<Vec<Chunk>, String> {
        for payload in self.events.finish()? {
            self.handle(&payload)?;
        }
        let mut flushed = Vec::new();
        for index in 0..self.pending.len() {
            self.emit_pending(index, &mut flushed);
        }
        for chunk in flushed {
            self.transform(chunk)?;
        }
        self.close_text_parts();
        let mut open: Vec<usize> = self
            .args_open
            .iter()
            .filter(|(_, open)| **open)
            .map(|(i, _)| *i)
            .collect();
        open.sort_unstable();
        for index in open {
            self.close_args(index);
        }
        Ok(self.writer.take())
    }
}
