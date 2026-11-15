//! Indexed, in-place AI SDK 7 UI-message reduction with explicit snapshots.
//!
//! Wire behavior follows `ai@7.0.166`'s UI-message chunk schema and reducer.
//! Rendering is a separate consumer. No UI, clock, network, or executor lives here.

mod partial_json;
mod schema;
mod tool;

#[cfg(test)]
mod tests;

use std::collections::HashMap;

use serde_json::{json, Map, Value};

use crate::error::StreamError;
use crate::update::{ApplyResult, Update};

use schema::{invalid, select, string};

/// How unrecognized non-data event types are handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UnknownPolicy {
    /// Match AI SDK 7: reject types outside its current wire schema.
    #[default]
    Reject,
    /// Forward an opaque notification without guessing its message semantics.
    Preserve,
}

/// Limits and explicit compatibility choices for one reducer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReducerOptions {
    /// Maximum retained parts. The default is unbounded, matching upstream.
    /// Hosts should choose a finite application-appropriate limit.
    pub max_parts: usize,
    /// Unknown-type handling; known types are always validated.
    pub unknown_policy: UnknownPolicy,
    /// Rebuild the message when a start event supplies a different ID.
    /// Enabled by default, matching `readUIMessageStream`.
    pub reset_on_message_id_change: bool,
}

impl Default for ReducerOptions {
    fn default() -> Self {
        Self {
            max_parts: usize::MAX,
            unknown_policy: UnknownPolicy::Reject,
            reset_on_message_id_change: true,
        }
    }
}

#[derive(Debug)]
struct PartialTool {
    part: usize,
    name: String,
    dynamic: bool,
    encoding: Option<String>,
    title: Option<Value>,
    metadata: Option<Value>,
}

#[derive(Debug)]
struct DeferredInput {
    raw: String,
    encoding: Option<String>,
    expose: bool,
    raw_visible: bool,
    received_delta: bool,
}

impl DeferredInput {
    fn input(&self) -> Option<Value> {
        if self.encoding.as_deref() == Some("text") {
            Some(Value::String(self.raw.clone()))
        } else {
            partial_json::preview(&self.raw)
        }
    }
}

/// A single-owner cumulative assistant message with indexed active parts.
///
/// Applying text and tool-input deltas never clones the complete message or
/// reparses accumulated tool JSON. Partial tool previews are computed only by
/// explicit snapshot reads and the control transitions that need their value.
#[derive(Debug)]
pub struct UiReducer {
    id: String,
    metadata: Option<Value>,
    parts: Vec<Value>,
    options: ReducerOptions,
    text: HashMap<String, usize>,
    reasoning: HashMap<String, usize>,
    partial_tools: HashMap<String, PartialTool>,
    deferred: HashMap<usize, DeferredInput>,
    static_tools: HashMap<String, usize>,
    dynamic_tools: HashMap<String, usize>,
    latest_tools: HashMap<String, usize>,
    approvals: HashMap<String, usize>,
    data: HashMap<(String, String), usize>,
    step: Option<usize>,
}

impl UiReducer {
    /// Create an empty assistant message. The ID is opaque and may be empty.
    #[must_use]
    pub fn new(message_id: String, options: ReducerOptions) -> Self {
        Self {
            id: message_id,
            metadata: None,
            parts: Vec::new(),
            options,
            text: HashMap::new(),
            reasoning: HashMap::new(),
            partial_tools: HashMap::new(),
            deferred: HashMap::new(),
            static_tools: HashMap::new(),
            dynamic_tools: HashMap::new(),
            latest_tools: HashMap::new(),
            approvals: HashMap::new(),
            data: HashMap::new(),
            step: None,
        }
    }

    /// Resume an assistant message, reconstructing current-step tool streams.
    ///
    /// Text and reasoning streams still need a fresh start event, as upstream
    /// does not restore their wire IDs. Returns validation or configured-limit
    /// errors without constructing a partially usable reducer.
    pub fn with_message(message: Value, options: ReducerOptions) -> Result<Self, StreamError> {
        let id = string(&message, "id")?.to_owned();
        if string(&message, "role")? != "assistant" {
            return Err(invalid("initial message role must be assistant"));
        }
        let values = message
            .get("parts")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("initial message parts must be an array"))?;
        if values.len() > options.max_parts {
            return Err(StreamError::Limit {
                resource: "parts",
                limit: options.max_parts,
            });
        }
        for part in values {
            string(part, "type")?;
            if is_tool(part) {
                string(part, "toolCallId")?;
                string(part, "state")?;
                if part.get("type").and_then(Value::as_str) == Some("dynamic-tool") {
                    string(part, "toolName")?;
                }
                if let Some(raw) = part.get("rawInput") {
                    if !raw.is_string()
                        && part.get("state").and_then(Value::as_str) == Some("input-streaming")
                    {
                        return Err(invalid("streaming initial rawInput must be a string"));
                    }
                }
            }
        }
        let mut result = Self::new(id, options);
        let mut message = message;
        if let Some(object) = message.as_object_mut() {
            result.metadata = object.remove("metadata");
            if let Some(Value::Array(parts)) = object.remove("parts") {
                result.parts = parts;
            }
        }
        result.rebuild_indexes();
        let start = result.step.map_or(0, |index| index + 1);
        for (index, part) in result.parts.iter().enumerate().skip(start) {
            if !is_tool(part)
                || part.get("state").and_then(Value::as_str) != Some("input-streaming")
            {
                continue;
            }
            let id = string(part, "toolCallId")?.to_owned();
            let dynamic = part.get("type").and_then(Value::as_str) == Some("dynamic-tool");
            let name = tool_name(part)?.to_owned();
            let encoding = part
                .get("inputEncoding")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let raw = part
                .get("rawInput")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            result.partial_tools.insert(
                id,
                PartialTool {
                    part: index,
                    name,
                    dynamic,
                    encoding: encoding.clone(),
                    title: part.get("title").cloned(),
                    metadata: part.get("toolMetadata").cloned(),
                },
            );
            result.deferred.insert(
                index,
                DeferredInput {
                    raw,
                    encoding,
                    expose: false,
                    raw_visible: true,
                    received_delta: part.get("rawInput").is_some(),
                },
            );
        }
        Ok(result)
    }

    /// Apply one complete event after validating all known fields.
    ///
    /// A validation, lifecycle, or part-limit failure leaves the prior message
    /// intact. Explicit server `error` events are notifications, not failures.
    pub fn apply(&mut self, chunk: Value) -> Result<ApplyResult, StreamError> {
        schema::validate(&chunk, self.options.unknown_policy)?;
        let kind = string(&chunk, "type")?;
        self.validate_metadata_merge(&chunk, kind)?;
        match kind {
            "text-start" | "reasoning-start" => self.text_start(&chunk, kind == "reasoning-start"),
            "text-delta" | "reasoning-delta" => self.text_delta(&chunk, kind == "reasoning-delta"),
            "text-end" | "reasoning-end" => self.text_end(&chunk, kind == "reasoning-end"),
            "tool-input-start" => self.tool_start(&chunk),
            "tool-input-delta" => self.tool_delta(&chunk),
            "tool-input-available" | "tool-input-error" => self.tool_input(&chunk),
            "tool-output-available" | "tool-output-error" | "tool-output-denied" => {
                self.tool_output(&chunk)
            }
            "tool-approval-request" | "tool-approval-response" => self.tool_approval(&chunk),
            "source-url" | "source-document" | "file" | "reasoning-file" | "custom"
            | "steering-message" => self.simple_part(&chunk),
            "start-step" => {
                let index = self.append(json!({"type":"step-start"}))?;
                self.step = Some(index);
                self.static_tools.clear();
                self.dynamic_tools.clear();
                Ok(ApplyResult {
                    updates: vec![Update::PartAdded { index }, Update::StepStarted],
                    published: false,
                })
            }
            "finish-step" => Ok(ApplyResult {
                updates: vec![Update::StepFinished],
                published: false,
            }),
            "reset-step" => Ok(self.reset_step()),
            "start" => Ok(self.start(&chunk)),
            "finish" => {
                let reason = chunk
                    .get("finishReason")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let mut result = ApplyResult {
                    updates: vec![Update::Finish { reason }],
                    published: false,
                };
                self.metadata_update(&chunk, &mut result);
                Ok(result)
            }
            "message-metadata" => {
                let mut result = ApplyResult::default();
                self.metadata_update(&chunk, &mut result);
                Ok(result)
            }
            "abort" => Ok(ApplyResult {
                updates: vec![Update::Abort {
                    reason: chunk
                        .get("reason")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                }],
                published: false,
            }),
            "error" => Ok(ApplyResult {
                updates: vec![Update::Error {
                    error_text: string(&chunk, "errorText")?.to_owned(),
                }],
                published: false,
            }),
            kind if kind.starts_with("data-") => self.data_part(chunk),
            _ => Ok(ApplyResult {
                updates: vec![Update::Unknown { chunk }],
                published: false,
            }),
        }
    }

    /// Materialize a canonical cumulative AI SDK message, including previews.
    /// This is proportional to the retained message size; it is not a hot-path notification.
    #[must_use]
    pub fn snapshot(&self) -> Value {
        let mut message = Map::new();
        message.insert("id".to_owned(), Value::String(self.id.clone()));
        message.insert("role".to_owned(), Value::String("assistant".to_owned()));
        message.insert(
            "parts".to_owned(),
            Value::Array(
                self.parts
                    .iter()
                    .enumerate()
                    .map(|(index, part)| self.project_part(index, part))
                    .collect(),
            ),
        );
        if let Some(metadata) = &self.metadata {
            message.insert("metadata".to_owned(), metadata.clone());
        }
        Value::Object(message)
    }

    /// Number of ordered retained parts, without materializing their contents.
    #[must_use]
    pub fn part_count(&self) -> usize {
        self.parts.len()
    }

    /// Borrow a stored structural part without cloning or computing tool previews.
    /// For a streaming tool, use [`Self::raw_tool_input`] or [`Self::part_snapshot`]
    /// when input/rawInput is needed. These fields are deliberately deferred.
    #[must_use]
    pub fn part(&self, index: usize) -> Option<&Value> {
        self.parts.get(index)
    }

    /// Materialize only the requested part, including its current tool preview.
    #[must_use]
    pub fn part_snapshot(&self, index: usize) -> Option<Value> {
        self.parts
            .get(index)
            .map(|part| self.project_part(index, part))
    }

    /// Borrow accumulated tool-input bytes after any delta, even after completion.
    /// They are never interpreted as a complete tool call merely because JSON parses.
    #[must_use]
    pub fn raw_tool_input(&self, index: usize) -> Option<&str> {
        self.deferred
            .get(&index)
            .filter(|input| input.received_delta)
            .map(|input| input.raw.as_str())
    }

    fn project_part(&self, index: usize, part: &Value) -> Value {
        let mut projected = part.clone();
        if let Some(input) = self.deferred.get(&index).filter(|input| input.expose) {
            if let Some(object) = projected.as_object_mut() {
                assign(object, "input", input.input());
                if input.raw_visible {
                    object.insert("rawInput".to_owned(), Value::String(input.raw.clone()));
                } else {
                    object.remove("rawInput");
                }
            }
        }
        projected
    }

    fn reserve_part(&self) -> Result<(), StreamError> {
        if self.parts.len() >= self.options.max_parts {
            return Err(StreamError::Limit {
                resource: "parts",
                limit: self.options.max_parts,
            });
        }
        Ok(())
    }

    fn append(&mut self, part: Value) -> Result<usize, StreamError> {
        self.reserve_part()?;
        let index = self.parts.len();
        self.parts.push(part);
        Ok(index)
    }

    fn object_mut(&mut self, index: usize) -> Result<&mut Map<String, Value>, StreamError> {
        self.parts
            .get_mut(index)
            .and_then(Value::as_object_mut)
            .ok_or_else(|| invalid("invalid internal part handle"))
    }

    fn text_start(&mut self, chunk: &Value, reasoning: bool) -> Result<ApplyResult, StreamError> {
        let id = string(chunk, "id")?;
        let mut part = select(chunk, &["providerMetadata"]);
        part.insert(
            "type".to_owned(),
            Value::String(if reasoning { "reasoning" } else { "text" }.to_owned()),
        );
        part.insert("text".to_owned(), Value::String(String::new()));
        part.insert("state".to_owned(), Value::String("streaming".to_owned()));
        if reasoning {
            part.insert("id".to_owned(), Value::String(id.to_owned()));
        }
        let index = self.append(Value::Object(part))?;
        if reasoning {
            self.reasoning.insert(id.to_owned(), index);
        } else {
            self.text.insert(id.to_owned(), index);
        }
        Ok(part_result(index, true))
    }

    fn text_index(&self, chunk: &Value, reasoning: bool) -> Result<usize, StreamError> {
        let id = string(chunk, "id")?;
        let index = if reasoning {
            self.reasoning.get(id)
        } else {
            self.text.get(id)
        };
        index.copied().ok_or_else(|| StreamError::Lifecycle {
            chunk_type: string(chunk, "type").unwrap_or_default().to_owned(),
            chunk_id: id.to_owned(),
        })
    }

    fn text_delta(&mut self, chunk: &Value, reasoning: bool) -> Result<ApplyResult, StreamError> {
        let index = self.text_index(chunk, reasoning)?;
        let part = self.object_mut(index)?;
        let Some(Value::String(text)) = part.get_mut("text") else {
            return Err(invalid("active text part has no string text"));
        };
        text.push_str(string(chunk, "delta")?);
        copy_if_present(part, chunk, "providerMetadata", "providerMetadata");
        Ok(part_result(index, false))
    }

    fn text_end(&mut self, chunk: &Value, reasoning: bool) -> Result<ApplyResult, StreamError> {
        let index = self.text_index(chunk, reasoning)?;
        let part = self.object_mut(index)?;
        part.insert("state".to_owned(), Value::String("done".to_owned()));
        copy_if_present(part, chunk, "providerMetadata", "providerMetadata");
        if !reasoning {
            if let Some(citations) = schema::projected_citations(chunk) {
                part.insert("citations".to_owned(), citations);
            }
        }
        let id = string(chunk, "id")?;
        if reasoning {
            self.reasoning.remove(id);
        } else {
            self.text.remove(id);
        }
        Ok(part_result(index, false))
    }

    fn simple_part(&mut self, chunk: &Value) -> Result<ApplyResult, StreamError> {
        let fields: &[&str] = match string(chunk, "type")? {
            "source-url" => &["type", "sourceId", "url", "title", "providerMetadata"],
            "source-document" => &[
                "type",
                "sourceId",
                "mediaType",
                "title",
                "filename",
                "providerMetadata",
            ],
            "file" | "reasoning-file" => &["type", "mediaType", "url", "providerMetadata"],
            "custom" => &["type", "kind", "providerMetadata"],
            "steering-message" => &["type", "id", "parts"],
            _ => return Err(invalid("unexpected simple part kind")),
        };
        let index = self.append(Value::Object(select(chunk, fields)))?;
        Ok(part_result(index, true))
    }

    fn data_part(&mut self, chunk: Value) -> Result<ApplyResult, StreamError> {
        let transient = chunk.get("transient").and_then(Value::as_bool) == Some(true);
        if transient {
            return Ok(ApplyResult {
                updates: vec![Update::Data {
                    part: chunk,
                    transient: true,
                }],
                published: false,
            });
        }
        let key = chunk.get("id").and_then(Value::as_str).map(|id| {
            (
                string(&chunk, "type").unwrap_or_default().to_owned(),
                id.to_owned(),
            )
        });
        let existing = key.as_ref().and_then(|key| self.data.get(key)).copied();
        let mut result = if let Some(index) = existing {
            assign(self.object_mut(index)?, "data", chunk.get("data").cloned());
            part_result(index, false)
        } else {
            let index = self.append(chunk.clone())?;
            if let Some(key) = key {
                self.data.insert(key, index);
            }
            part_result(index, true)
        };
        result.updates.push(Update::Data {
            part: chunk,
            transient: false,
        });
        Ok(result)
    }

    fn validate_metadata_merge(&self, chunk: &Value, kind: &str) -> Result<(), StreamError> {
        if !matches!(kind, "start" | "finish" | "message-metadata") {
            return Ok(());
        }
        if kind == "start"
            && self.options.reset_on_message_id_change
            && chunk
                .get("messageId")
                .and_then(Value::as_str)
                .is_some_and(|id| id != self.id)
        {
            return Ok(());
        }
        let Some(base) = self.metadata.as_ref() else {
            return Ok(());
        };
        if base.is_null() || base.is_object() || base.is_array() {
            return Ok(());
        }
        let has_keys = match chunk.get("messageMetadata") {
            Some(Value::Object(object)) => object
                .keys()
                .any(|key| !matches!(key.as_str(), "__proto__" | "constructor" | "prototype")),
            Some(Value::Array(array)) => !array.is_empty(),
            Some(Value::String(text)) => !text.is_empty(),
            Some(Value::Null | Value::Bool(_) | Value::Number(_)) | None => false,
        };
        if has_keys {
            return Err(StreamError::Metadata(
                "reference mergeObjects uses the in operator on primitive metadata".to_owned(),
            ));
        }
        Ok(())
    }

    fn metadata_update(&mut self, chunk: &Value, result: &mut ApplyResult) {
        if let Some(metadata) = chunk
            .get("messageMetadata")
            .filter(|value| !value.is_null())
        {
            self.metadata = Some(match self.metadata.take() {
                Some(base) if !base.is_null() => merge_metadata(base, metadata),
                Some(Value::Null) => metadata.clone(),
                Some(_) => metadata.clone(),
                None => metadata.clone(),
            });
            result.updates.push(Update::MetadataChanged);
            result.published = true;
        }
    }

    fn start(&mut self, chunk: &Value) -> ApplyResult {
        let id = chunk.get("messageId").and_then(Value::as_str);
        let mut result = ApplyResult::default();
        if self.options.reset_on_message_id_change && id.is_some_and(|id| id != self.id) {
            let next = id.unwrap_or_default().to_owned();
            *self = Self::new(next, self.options.clone());
            result.updates.push(Update::Reset);
            result.published = true;
        }
        if let Some(id) = id {
            self.id = id.to_owned();
            result.updates.push(Update::MessageIdChanged);
            result.published = true;
        }
        self.metadata_update(chunk, &mut result);
        result
    }

    fn reset_step(&mut self) -> ApplyResult {
        let keep = self.step.map_or(0, |index| index + 1);
        let published = self.parts.len() > keep;
        self.parts.truncate(keep);
        self.text.clear();
        self.reasoning.clear();
        self.partial_tools.clear();
        self.deferred.retain(|index, _| *index < keep);
        self.rebuild_indexes();
        ApplyResult {
            updates: vec![Update::Reset],
            published,
        }
    }

    fn rebuild_indexes(&mut self) {
        self.step = self
            .parts
            .iter()
            .rposition(|part| part.get("type").and_then(Value::as_str) == Some("step-start"));
        self.static_tools.clear();
        self.dynamic_tools.clear();
        self.latest_tools.clear();
        self.approvals.clear();
        self.data.clear();
        let start = self.step.map_or(0, |index| index + 1);
        for (index, part) in self.parts.iter().enumerate() {
            if is_tool(part) {
                if let Some(id) = part.get("toolCallId").and_then(Value::as_str) {
                    self.latest_tools.insert(id.to_owned(), index);
                    if index >= start {
                        let map =
                            if part.get("type").and_then(Value::as_str) == Some("dynamic-tool") {
                                &mut self.dynamic_tools
                            } else {
                                &mut self.static_tools
                            };
                        map.entry(id.to_owned()).or_insert(index);
                    }
                }
            }
            if is_tool(part) {
                if let Some(approval) = part
                    .get("approval")
                    .and_then(|value| value.get("id"))
                    .and_then(Value::as_str)
                {
                    self.approvals.entry(approval.to_owned()).or_insert(index);
                }
            }
            if let (Some(kind), Some(id)) = (
                part.get("type").and_then(Value::as_str),
                part.get("id").and_then(Value::as_str),
            ) {
                if kind.starts_with("data-") {
                    self.data
                        .entry((kind.to_owned(), id.to_owned()))
                        .or_insert(index);
                }
            }
        }
    }
}

fn is_tool(part: &Value) -> bool {
    part.get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind == "dynamic-tool" || kind.starts_with("tool-"))
}

fn tool_name(part: &Value) -> Result<&str, StreamError> {
    let kind = string(part, "type")?;
    if kind == "dynamic-tool" {
        string(part, "toolName")
    } else {
        kind.strip_prefix("tool-")
            .ok_or_else(|| invalid("part is not a tool"))
    }
}

fn assign(target: &mut Map<String, Value>, field: &str, value: Option<Value>) {
    if let Some(value) = value {
        target.insert(field.to_owned(), value);
    } else {
        target.remove(field);
    }
}

fn copy_if_present(target: &mut Map<String, Value>, source: &Value, from: &str, to: &str) {
    if let Some(value) = source.get(from) {
        target.insert(to.to_owned(), value.clone());
    }
}

fn part_result(index: usize, added: bool) -> ApplyResult {
    ApplyResult {
        updates: vec![if added {
            Update::PartAdded { index }
        } else {
            Update::PartChanged { index }
        }],
        published: true,
    }
}

fn enumerable(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(object) => object,
        Value::Array(array) => array
            .into_iter()
            .enumerate()
            .map(|(index, value)| (index.to_string(), value))
            .collect(),
        Value::String(text) => text
            .chars()
            .enumerate()
            .map(|(index, value)| (index.to_string(), Value::String(value.to_string())))
            .collect(),
        Value::Null | Value::Bool(_) | Value::Number(_) => Map::new(),
    }
}

fn merge_metadata(base: Value, overrides: &Value) -> Value {
    let mut result = enumerable(base);
    for (key, value) in enumerable(overrides.clone()) {
        if matches!(key.as_str(), "__proto__" | "constructor" | "prototype") {
            continue;
        }
        let merged = match result.remove(&key) {
            Some(base) if base.is_object() && value.is_object() => merge_metadata(base, &value),
            Some(_) | None => value,
        };
        result.insert(key, merged);
    }
    Value::Object(result)
}
