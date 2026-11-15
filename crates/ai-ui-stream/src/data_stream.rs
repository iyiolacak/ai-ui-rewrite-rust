//! The AI SDK data stream (`data-stream`), ported from
//! `serialization/data-stream`: newline-separated `CODE:JSON` lines such as
//! `0:"text"`, `9:{toolCall}` and `d:{finish}`, plus assistant-ui's `aui-*`
//! extensions. As upstream's default (strict) decoder, a malformed value or a
//! reference to an unknown tool call fails the stream.

use crate::chunk::{Chunk, ChunkUsage, FinishReason, PartInit};
use crate::decoder::StreamDecoder;
use crate::writer::{PartWriter, ToolOutput};
use ai_ui_core::ToolApproval;
use serde_json::Value;
use std::collections::HashMap;

/// Longest accepted line, in bytes.
pub const DEFAULT_MAX_LINE_BYTES: usize = 4 * 1024 * 1024;

struct ToolCall {
    index: usize,
    has_args: bool,
    args_closed: bool,
}

/// Decodes the line-based data stream protocol (AI SDK v4 codes and assistant-ui's `aui-*` extensions).
pub struct DataStreamDecoder {
    max_line_bytes: usize,
    line: Vec<u8>,
    writer: PartWriter,
    tools: HashMap<String, ToolCall>,
    /// Tool call ids in start order, for closing.
    order: Vec<String>,
    /// A failure found mid-read, reported on the next call so the chunks
    /// decoded before it are delivered first.
    failed: Option<String>,
}

impl Default for DataStreamDecoder {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_LINE_BYTES)
    }
}

fn s(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn finish_fields(value: &Value) -> (FinishReason, ChunkUsage) {
    let reason = value
        .get("finishReason")
        .and_then(|r| serde_json::from_value(r.clone()).ok())
        .unwrap_or(FinishReason::Other);
    let usage = value
        .get("usage")
        .and_then(|u| serde_json::from_value(u.clone()).ok())
        .unwrap_or_default();
    (reason, usage)
}

impl DataStreamDecoder {
    /// A decoder that accepts lines up to `max_line_bytes`.
    pub fn new(max_line_bytes: usize) -> Self {
        Self {
            max_line_bytes,
            line: Vec::new(),
            writer: PartWriter::default(),
            tools: HashMap::new(),
            failed: None,
            order: Vec::new(),
        }
    }

    fn tool(&mut self, id: &str, what: &str) -> Result<&mut ToolCall, String> {
        self.tools
            .get_mut(id)
            .ok_or_else(|| format!("Encountered {what} with unknown id: {id}"))
    }

    fn start_tool(&mut self, id: String, name: String, parent_id: Option<String>) -> usize {
        let index = self.writer.add(PartInit::ToolCall {
            tool_call_id: id.clone(),
            tool_name: name,
            parent_id,
        });
        self.tools.insert(
            id.clone(),
            ToolCall {
                index,
                has_args: false,
                args_closed: false,
            },
        );
        self.order.push(id);
        index
    }

    fn close_args(&mut self, id: &str) {
        if let Some(call) = self.tools.get_mut(id) {
            if !call.args_closed {
                call.args_closed = true;
                let index = call.index;
                // Arguments that never streamed close as an empty object, as upstream.
                if !call.has_args {
                    call.has_args = true;
                    self.writer.delta(index, "{}", None);
                }
                self.writer.args_finish(index);
            }
        }
    }

    fn close_open_args(&mut self) {
        for id in self.order.clone() {
            self.close_args(&id);
        }
    }

    fn line(&mut self, line: &str) -> Result<(), String> {
        if line.is_empty() {
            return Ok(());
        }
        let (code, json) = line
            .split_once(':')
            .ok_or_else(|| format!("Invalid data stream line: {}", preview(line)))?;
        let value: Value = serde_json::from_str(json)
            .map_err(|_| format!("Invalid JSON for data-stream chunk type \"{code}\""))?;
        let invalid = || {
            format!(
                "Invalid value for data-stream chunk type \"{code}\": {}",
                preview(json)
            )
        };
        match code {
            "0" => self
                .writer
                .append_text(value.as_str().ok_or_else(invalid)?, false),
            "g" => self
                .writer
                .append_text(value.as_str().ok_or_else(invalid)?, true),
            "aui-text-delta" => {
                let text = s(&value, "textDelta").ok_or_else(invalid)?;
                self.writer
                    .append_text_under(&text, false, s(&value, "parentId"));
            }
            "aui-reasoning-delta" => {
                let text = s(&value, "reasoningDelta").ok_or_else(invalid)?;
                self.writer
                    .append_text_under(&text, true, s(&value, "parentId"));
            }
            "aui-reasoning-part-start" => {
                self.writer.add_text_under(
                    None,
                    true,
                    s(&value, "parentId"),
                    s(&value, "unstable_summary"),
                );
            }
            "b" => {
                let id = s(&value, "toolCallId").ok_or_else(invalid)?;
                if self.tools.contains_key(&id) {
                    return Err(format!("Encountered duplicate tool call id: {id}"));
                }
                self.start_tool(
                    id,
                    s(&value, "toolName").unwrap_or_default(),
                    s(&value, "parentId"),
                );
            }
            "c" => {
                let id = s(&value, "toolCallId").ok_or_else(invalid)?;
                let delta = s(&value, "argsTextDelta").ok_or_else(invalid)?;
                let call = self.tool(&id, "tool call")?;
                if call.args_closed {
                    return Ok(());
                }
                call.has_args |= !delta.is_empty();
                let index = call.index;
                self.writer.delta(index, &delta, None);
                if value.get("isFinal").and_then(Value::as_bool) == Some(true) {
                    self.close_args(&id);
                }
            }
            "9" => {
                let id = s(&value, "toolCallId").ok_or_else(invalid)?;
                if !self.tools.contains_key(&id) {
                    self.start_tool(id.clone(), s(&value, "toolName").unwrap_or_default(), None);
                }
                let call = self.tools.get_mut(&id).expect("started above");
                if let Some(args) = value.get("args").filter(|a| !a.is_null()) {
                    if !call.has_args && !call.args_closed {
                        call.has_args = true;
                        let index = call.index;
                        self.writer.delta(index, &args.to_string(), None);
                    }
                }
                self.close_args(&id);
            }
            "a" => {
                let id = s(&value, "toolCallId").ok_or_else(invalid)?;
                let index = self.tool(&id, "tool call result")?.index;
                self.close_args(&id);
                let flag = |key| value.get(key).and_then(Value::as_bool) == Some(true);
                let field = |key| value.get(key).filter(|v: &&Value| !v.is_null()).cloned();
                self.writer.result(
                    index,
                    ToolOutput {
                        value: value.get("result").cloned().unwrap_or(Value::Null),
                        is_error: flag("isError"),
                        is_preliminary: flag("isPreliminary"),
                        artifact: field("artifact"),
                        model_content: field("modelContent"),
                        messages: field("messages"),
                    },
                );
            }
            "aui-tool-call-approval" => {
                let id = s(&value, "toolCallId").ok_or_else(invalid)?;
                let index = self.tool(&id, "tool call approval")?.index;
                let approval: ToolApproval =
                    serde_json::from_value(value["approval"].clone()).map_err(|_| invalid())?;
                self.writer.approval(index, approval);
            }
            "d" => {
                self.close_open_args();
                let (reason, usage) = finish_fields(&value);
                self.writer.message_finish(reason, usage);
            }
            "e" => {
                self.close_open_args();
                let (reason, usage) = finish_fields(&value);
                let continued = value
                    .get("isContinued")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if !continued {
                    // The next step's text is a new part; a continued step extends this one.
                    self.writer.close_open_text();
                }
                self.writer.step_finish(reason, usage, continued);
            }
            "f" => self
                .writer
                .step_start(s(&value, "messageId").unwrap_or_default()),
            "2" => self
                .writer
                .data(value.as_array().cloned().ok_or_else(invalid)?),
            "8" => self
                .writer
                .annotations(value.as_array().cloned().ok_or_else(invalid)?),
            "h" => {
                self.writer.add_closed(PartInit::Source {
                    source_type: s(&value, "sourceType").unwrap_or_else(|| "url".into()),
                    id: s(&value, "id").ok_or_else(invalid)?,
                    url: s(&value, "url"),
                    title: s(&value, "title"),
                    media_type: s(&value, "mediaType"),
                    filename: s(&value, "filename"),
                    parent_id: s(&value, "parentId"),
                });
            }
            "k" => {
                self.writer.add_closed(PartInit::File {
                    data: s(&value, "data").ok_or_else(invalid)?,
                    mime_type: s(&value, "mimeType").ok_or_else(invalid)?,
                    parent_id: s(&value, "parentId"),
                });
            }
            "aui-data" => {
                self.writer.add_closed(PartInit::Data {
                    name: s(&value, "name").ok_or_else(invalid)?,
                    data: value.get("data").cloned().unwrap_or(Value::Null),
                    parent_id: s(&value, "parentId"),
                });
            }
            "aui-state" => {
                self.writer.update_state(value);
            }
            "3" => match value {
                Value::String(error) => self.writer.error(error),
                other => {
                    let error = s(&other, "error").ok_or_else(invalid)?;
                    self.writer
                        .error_with(error, s(&other, "code"), s(&other, "severity"));
                }
            },
            // Redacted reasoning and reasoning signatures carry nothing to show.
            "i" | "j" => {}
            other => return Err(format!("unsupported chunk type: {other}")),
        }
        Ok(())
    }
}

fn preview(text: &str) -> String {
    text.chars().take(200).collect()
}

impl DataStreamDecoder {
    fn push_bytes(&mut self, input: &[u8]) -> Result<(), String> {
        for &byte in input {
            if byte == b'\n' {
                let mut line = std::mem::take(&mut self.line);
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                let line = String::from_utf8(line)
                    .map_err(|_| "Data stream line is not UTF-8".to_owned())?;
                self.line(&line)?;
            } else if self.line.len() >= self.max_line_bytes {
                return Err("Data stream line exceeds the limit".into());
            } else {
                self.line.push(byte);
            }
        }
        Ok(())
    }
}

impl StreamDecoder for DataStreamDecoder {
    fn push(&mut self, input: &[u8]) -> Result<Vec<Chunk>, String> {
        if let Some(error) = self.failed.take() {
            return Err(error);
        }
        if let Err(error) = self.push_bytes(input) {
            self.failed = Some(error);
        }
        Ok(self.writer.take())
    }

    fn finish(&mut self) -> Result<Vec<Chunk>, String> {
        if let Some(error) = self.failed.take() {
            return Err(error);
        }
        let mut chunks = if self.line.is_empty() {
            Vec::new()
        } else {
            self.push(b"\n")?
        };
        if let Some(error) = self.failed.take() {
            return Err(error);
        }
        self.close_open_args();
        self.writer.close_open_text();
        chunks.extend(self.writer.take());
        Ok(chunks)
    }
}
