//! Tool lifecycle reduction and deferred argument materialization.

use serde_json::{Map, Value};

use crate::error::StreamError;
use crate::update::ApplyResult;

use super::{
    assign, copy_if_present, part_result, schema::invalid, string, DeferredInput, PartialTool,
    UiReducer,
};

impl UiReducer {
    fn current_tool(&self, id: &str) -> Option<usize> {
        match (self.static_tools.get(id), self.dynamic_tools.get(id)) {
            (Some(left), Some(right)) => Some((*left).min(*right)),
            (Some(index), None) | (None, Some(index)) => Some(*index),
            (None, None) => None,
        }
    }

    fn output_tool(&self, id: &str) -> Result<usize, StreamError> {
        self.current_tool(id)
            .or_else(|| self.latest_tools.get(id).copied())
            .ok_or_else(|| StreamError::Lifecycle {
                chunk_type: "tool-invocation".to_owned(),
                chunk_id: id.to_owned(),
            })
    }

    fn tool_slot(
        &mut self,
        id: &str,
        name: &str,
        dynamic: bool,
    ) -> Result<(usize, bool), StreamError> {
        let current = if dynamic {
            self.dynamic_tools.get(id)
        } else {
            self.static_tools.get(id)
        };
        if let Some(index) = current {
            return Ok((*index, false));
        }
        self.reserve_part()?;
        let mut part = Map::new();
        part.insert(
            "type".to_owned(),
            Value::String(if dynamic {
                "dynamic-tool".to_owned()
            } else {
                format!("tool-{name}")
            }),
        );
        if dynamic {
            part.insert("toolName".to_owned(), Value::String(name.to_owned()));
        }
        part.insert("toolCallId".to_owned(), Value::String(id.to_owned()));
        let index = self.append(Value::Object(part))?;
        if dynamic {
            self.dynamic_tools.insert(id.to_owned(), index);
        } else {
            self.static_tools.insert(id.to_owned(), index);
        }
        self.latest_tools.insert(id.to_owned(), index);
        Ok((index, true))
    }

    pub(super) fn tool_start(&mut self, chunk: &Value) -> Result<ApplyResult, StreamError> {
        let id = string(chunk, "toolCallId")?;
        let name = string(chunk, "toolName")?;
        let dynamic = chunk.get("dynamic").and_then(Value::as_bool) == Some(true);
        let (index, added) = self.tool_slot(id, name, dynamic)?;
        let part = self.object_mut(index)?;
        tool_identity(part, name, dynamic);
        part.insert(
            "state".to_owned(),
            Value::String("input-streaming".to_owned()),
        );
        clear(
            part,
            &["input", "output", "errorText", "rawInput", "preliminary"],
        );
        assign(part, "inputEncoding", chunk.get("inputEncoding").cloned());
        copy_if_present(part, chunk, "providerExecuted", "providerExecuted");
        copy_if_present(part, chunk, "providerMetadata", "callProviderMetadata");
        copy_if_present(part, chunk, "title", "title");
        copy_if_present(part, chunk, "toolMetadata", "toolMetadata");
        let encoding = chunk
            .get("inputEncoding")
            .and_then(Value::as_str)
            .map(str::to_owned);
        self.partial_tools.insert(
            id.to_owned(),
            PartialTool {
                part: index,
                name: name.to_owned(),
                dynamic,
                encoding: encoding.clone(),
                title: chunk.get("title").cloned(),
                metadata: chunk.get("toolMetadata").cloned(),
            },
        );
        self.deferred.insert(
            index,
            DeferredInput {
                raw: String::new(),
                encoding,
                expose: false,
                raw_visible: true,
                received_delta: false,
            },
        );
        Ok(part_result(index, added))
    }

    pub(super) fn tool_delta(&mut self, chunk: &Value) -> Result<ApplyResult, StreamError> {
        let id = string(chunk, "toolCallId")?;
        let delta = string(chunk, "inputTextDelta")?;
        let partial = self
            .partial_tools
            .get(id)
            .ok_or_else(|| StreamError::Lifecycle {
                chunk_type: "tool-input-delta".to_owned(),
                chunk_id: id.to_owned(),
            })?;
        let current = if partial.dynamic {
            self.dynamic_tools.get(id)
        } else {
            self.static_tools.get(id)
        }
        .copied();
        let source = partial.part;
        if current == Some(source)
            && self
                .parts
                .get(source)
                .and_then(|part| part.get("state"))
                .and_then(Value::as_str)
                == Some("input-streaming")
        {
            let deferred = self
                .deferred
                .get_mut(&source)
                .ok_or_else(|| invalid("missing deferred tool input"))?;
            deferred.raw.push_str(delta);
            deferred.expose = true;
            deferred.raw_visible = true;
            deferred.received_delta = true;
            // Resumed input/rawInput is superseded by the deferred current view.
            clear(
                self.object_mut(source)?,
                &["input", "rawInput", "output", "errorText", "preliminary"],
            );
            return Ok(part_result(source, false));
        }

        // A control transition or step boundary can make a later delta reopen
        // this call. Copy configuration only on this uncommon transition, never
        // for each ordinary token. Preserve prior-step snapshots independently.
        let name = partial.name.clone();
        let dynamic = partial.dynamic;
        let encoding = partial.encoding.clone();
        let title = partial.title.clone();
        let metadata = partial.metadata.clone();
        if current.is_none() {
            self.reserve_part()?;
        }
        let existing = self
            .deferred
            .get(&source)
            .ok_or_else(|| invalid("missing deferred tool input"))?;
        let copied_raw = if current != Some(source) {
            Some(existing.raw.clone())
        } else {
            None
        };
        let (index, added) = self.tool_slot(id, &name, dynamic)?;
        if let Some(raw) = copied_raw {
            self.deferred.insert(
                index,
                DeferredInput {
                    raw,
                    encoding: encoding.clone(),
                    expose: true,
                    raw_visible: true,
                    received_delta: true,
                },
            );
        }
        let deferred = self
            .deferred
            .get_mut(&index)
            .ok_or_else(|| invalid("missing deferred tool input"))?;
        deferred.raw.push_str(delta);
        deferred.expose = true;
        // The published dynamic-tool constructor omits rawInput. An existing
        // part receives it on the next delta; the raw bytes are retained either way.
        deferred.raw_visible = !(dynamic && added);
        deferred.received_delta = true;
        let part = self.object_mut(index)?;
        tool_identity(part, &name, dynamic);
        part.insert(
            "state".to_owned(),
            Value::String("input-streaming".to_owned()),
        );
        clear(
            part,
            &["input", "output", "errorText", "rawInput", "preliminary"],
        );
        assign(part, "inputEncoding", encoding.map(Value::String));
        if let Some(title) = title {
            part.insert("title".to_owned(), title);
        }
        if let Some(metadata) = metadata {
            part.insert("toolMetadata".to_owned(), metadata);
        }
        if let Some(partial) = self.partial_tools.get_mut(id) {
            partial.part = index;
        }
        Ok(part_result(index, added))
    }

    pub(super) fn tool_input(&mut self, chunk: &Value) -> Result<ApplyResult, StreamError> {
        let id = string(chunk, "toolCallId")?;
        let name = string(chunk, "toolName")?;
        let error = string(chunk, "type")? == "tool-input-error";
        let requested_dynamic = chunk.get("dynamic").and_then(Value::as_bool) == Some(true);
        let dynamic = if error {
            self.current_tool(id)
                .and_then(|index| self.parts.get(index))
                .map_or(requested_dynamic, |part| {
                    part.get("type").and_then(Value::as_str) == Some("dynamic-tool")
                })
        } else {
            requested_dynamic
        };
        let (index, added) = self.tool_slot(id, name, dynamic)?;
        let part = self.object_mut(index)?;
        tool_identity(part, name, dynamic);
        part.insert(
            "state".to_owned(),
            Value::String(
                if error {
                    "output-error"
                } else {
                    "input-available"
                }
                .to_owned(),
            ),
        );
        clear(
            part,
            &[
                "output",
                "errorText",
                "rawInput",
                "inputEncoding",
                "preliminary",
            ],
        );
        assign(part, "input", chunk.get("input").cloned());
        if error {
            copy_if_present(part, chunk, "errorText", "errorText");
        }
        copy_if_present(part, chunk, "providerExecuted", "providerExecuted");
        copy_if_present(
            part,
            chunk,
            "providerMetadata",
            if error {
                "resultProviderMetadata"
            } else {
                "callProviderMetadata"
            },
        );
        copy_if_present(part, chunk, "toolMetadata", "toolMetadata");
        if !error {
            copy_if_present(part, chunk, "title", "title");
        }
        if let Some(deferred) = self.deferred.get_mut(&index) {
            deferred.expose = false;
        }
        Ok(part_result(index, added))
    }

    pub(super) fn tool_output(&mut self, chunk: &Value) -> Result<ApplyResult, StreamError> {
        let id = string(chunk, "toolCallId")?;
        let kind = string(chunk, "type")?;
        let index = self.output_tool(id)?;
        if kind == "tool-output-denied" {
            self.object_mut(index)?.insert(
                "state".to_owned(),
                Value::String("output-denied".to_owned()),
            );
            return Ok(part_result(index, false));
        }
        let dynamic = self
            .parts
            .get(index)
            .and_then(|part| part.get("type"))
            .and_then(Value::as_str)
            == Some("dynamic-tool");
        let error = kind == "tool-output-error";
        let materialized = self
            .deferred
            .get(&index)
            .filter(|input| input.expose)
            .map(|input| {
                (
                    input.input(),
                    if error && !dynamic {
                        Some(Value::String(input.raw.clone()))
                    } else {
                        None
                    },
                )
            });
        let part = self.object_mut(index)?;
        if let Some((input, raw)) = materialized {
            assign(part, "input", input);
            assign(part, "rawInput", raw);
        }
        part.insert(
            "state".to_owned(),
            Value::String(
                if error {
                    "output-error"
                } else {
                    "output-available"
                }
                .to_owned(),
            ),
        );
        part.remove("inputEncoding");
        if !error || dynamic {
            part.remove("rawInput");
        }
        assign(
            part,
            "output",
            if error {
                None
            } else {
                chunk.get("output").cloned()
            },
        );
        assign(
            part,
            "errorText",
            if error {
                chunk.get("errorText").cloned()
            } else {
                None
            },
        );
        assign(
            part,
            "preliminary",
            if error {
                None
            } else {
                chunk.get("preliminary").cloned()
            },
        );
        copy_if_present(part, chunk, "providerExecuted", "providerExecuted");
        copy_if_present(part, chunk, "providerMetadata", "resultProviderMetadata");
        copy_if_present(part, chunk, "toolMetadata", "toolMetadata");
        if let Some(deferred) = self.deferred.get_mut(&index) {
            deferred.expose = false;
        }
        Ok(part_result(index, false))
    }

    pub(super) fn tool_approval(&mut self, chunk: &Value) -> Result<ApplyResult, StreamError> {
        let approval_id = string(chunk, "approvalId")?;
        if string(chunk, "type")? == "tool-approval-request" {
            let index = self.output_tool(string(chunk, "toolCallId")?)?;
            let mut approval = Map::new();
            approval.insert("id".to_owned(), Value::String(approval_id.to_owned()));
            if let Some(descriptor) = chunk
                .get("approvalDescriptor")
                .filter(|value| !value.is_null())
            {
                approval.insert("descriptor".to_owned(), descriptor.clone());
            }
            copy_if_present(&mut approval, chunk, "inputSchemaInput", "inputSchemaInput");
            copy_if_present(&mut approval, chunk, "reason", "requestReason");
            copy_if_present(&mut approval, chunk, "signature", "signature");
            if chunk.get("isAutomatic").and_then(Value::as_bool) == Some(true) {
                approval.insert("isAutomatic".to_owned(), Value::Bool(true));
            }
            let part = self.object_mut(index)?;
            part.insert(
                "state".to_owned(),
                Value::String("approval-requested".to_owned()),
            );
            part.insert("approval".to_owned(), Value::Object(approval));
            // Duplicate approval IDs resolve to the first tool part upstream.
            // Rebuild on this control transition, not on streamed deltas.
            self.rebuild_indexes();
            return Ok(part_result(index, false));
        }
        let index =
            self.approvals
                .get(approval_id)
                .copied()
                .ok_or_else(|| StreamError::Lifecycle {
                    chunk_type: "tool-approval-response".to_owned(),
                    chunk_id: approval_id.to_owned(),
                })?;
        let part = self.object_mut(index)?;
        let mut approval = part
            .remove("approval")
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        approval.insert("id".to_owned(), Value::String(approval_id.to_owned()));
        copy_if_present(&mut approval, chunk, "approved", "approved");
        copy_if_present(&mut approval, chunk, "reason", "reason");
        part.insert("approval".to_owned(), Value::Object(approval));
        part.insert(
            "state".to_owned(),
            Value::String("approval-responded".to_owned()),
        );
        copy_if_present(part, chunk, "providerExecuted", "providerExecuted");
        copy_if_present(part, chunk, "providerMetadata", "callProviderMetadata");
        Ok(part_result(index, false))
    }
}

fn clear(part: &mut Map<String, Value>, fields: &[&str]) {
    for field in fields {
        part.remove(*field);
    }
}

fn tool_identity(part: &mut Map<String, Value>, name: &str, dynamic: bool) {
    if dynamic {
        part.insert("toolName".to_owned(), Value::String(name.to_owned()));
    } else {
        part.insert("type".to_owned(), Value::String(format!("tool-{name}")));
    }
}
