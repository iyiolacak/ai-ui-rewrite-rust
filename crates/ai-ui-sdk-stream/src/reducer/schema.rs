//! The loose AI SDK 7.0.166 event schema, checked before any state mutation.

use serde_json::{Map, Value};

use crate::error::StreamError;

use super::UnknownPolicy;

pub(super) fn invalid(message: impl Into<String>) -> StreamError {
    StreamError::Validation(message.into())
}

pub(super) fn string<'a>(value: &'a Value, field: &str) -> Result<&'a str, StreamError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(format!("{field} must be a string")))
}

fn strings(value: &Value, fields: &[&str]) -> Result<(), StreamError> {
    for field in fields {
        string(value, field)?;
    }
    Ok(())
}

fn present(value: &Value, field: &str) -> Result<(), StreamError> {
    if value.get(field).is_none() {
        return Err(invalid(format!("{field} is required (null is permitted)")));
    }
    Ok(())
}

/// Match provider-utils 5.0.76 secureJsonParse on already decoded JSON values.
/// Unlike schema validation this applies to unknown fields and unknown events.
/// Initial messages are not wire JSON and deliberately do not use this guard.
pub(super) fn validate_json_keys(value: &Value) -> Result<(), StreamError> {
    let mut pending = Vec::new();
    let mut node = value;
    loop {
        match node {
            Value::Object(object) => {
                if object.contains_key("__proto__")
                    || object
                        .get("constructor")
                        .and_then(Value::as_object)
                        .is_some_and(|constructor| constructor.contains_key("prototype"))
                {
                    return Err(StreamError::Json(
                        "Object contains forbidden prototype property".to_owned(),
                    ));
                }
                pending.extend(
                    object
                        .values()
                        .filter(|value| value.is_object() || value.is_array()),
                );
            }
            Value::Array(array) => {
                pending.extend(
                    array
                        .iter()
                        .filter(|value| value.is_object() || value.is_array()),
                );
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
        let Some(next) = pending.pop() else {
            return Ok(());
        };
        node = next;
    }
}

fn optional_strings(value: &Value, fields: &[&str]) -> Result<(), StreamError> {
    for field in fields {
        if let Some(item) = value.get(field) {
            if !item.is_string() {
                return Err(invalid(format!("{field} must be a string when present")));
            }
        }
    }
    Ok(())
}

fn optional_bools(value: &Value, fields: &[&str]) -> Result<(), StreamError> {
    for field in fields {
        if let Some(item) = value.get(field) {
            if !item.is_boolean() {
                return Err(invalid(format!("{field} must be a boolean when present")));
            }
        }
    }
    Ok(())
}

fn provider_metadata(value: &Value) -> Result<(), StreamError> {
    if let Some(metadata) = value.get("providerMetadata") {
        let providers = metadata
            .as_object()
            .ok_or_else(|| invalid("providerMetadata must be an object"))?;
        if providers.values().any(|provider| !provider.is_object()) {
            return Err(invalid("each providerMetadata value must be an object"));
        }
    }
    Ok(())
}

fn tool_metadata(value: &Value) -> Result<(), StreamError> {
    if let Some(metadata) = value.get("toolMetadata") {
        if !metadata.is_object() {
            return Err(invalid("toolMetadata must be an object"));
        }
    }
    Ok(())
}

fn tool_common(value: &Value) -> Result<(), StreamError> {
    strings(value, &["toolCallId"])?;
    optional_bools(value, &["providerExecuted", "dynamic"])?;
    provider_metadata(value)?;
    tool_metadata(value)
}

fn citations(value: &Value) -> Result<(), StreamError> {
    let Some(items) = value.get("citations") else {
        return Ok(());
    };
    let items = items
        .as_array()
        .ok_or_else(|| invalid("citations must be an array"))?;
    for item in items {
        if !item.is_object() {
            return Err(invalid("citation must be an object"));
        }
        optional_strings(item, &["citedText"])?;
        for field in ["startIndex", "endIndex"] {
            if let Some(index) = item.get(field) {
                let Some(number) = index.as_f64() else {
                    return Err(invalid(format!("citation {field} must be an integer")));
                };
                if !(0.0..=9_007_199_254_740_991.0).contains(&number)
                    || number.floor().total_cmp(&number) != std::cmp::Ordering::Equal
                {
                    return Err(invalid(format!(
                        "citation {field} must be a nonnegative safe integer"
                    )));
                }
            }
        }
        let source = item
            .get("source")
            .filter(|source| source.is_object())
            .ok_or_else(|| invalid("citation source must be an object"))?;
        if string(source, "type")? != "source" {
            return Err(invalid("citation source.type must be source"));
        }
        string(source, "id")?;
        match string(source, "sourceType")? {
            "url" => {
                string(source, "url")?;
                optional_strings(source, &["title"])?;
            }
            "document" => {
                strings(source, &["mediaType", "title"])?;
                optional_strings(source, &["filename"])?;
            }
            _ => return Err(invalid("unknown citation sourceType")),
        }
        provider_metadata(source)?;
    }
    Ok(())
}

fn steering(value: &Value) -> Result<(), StreamError> {
    string(value, "id")?;
    let parts = value
        .get("parts")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("steering-message.parts must be an array"))?;
    for part in parts {
        match string(part, "type")? {
            "text" => {
                string(part, "text")?;
            }
            "file" => {
                strings(part, &["mediaType", "url"])?;
                optional_strings(part, &["filename"])?;
                if let Some(reference) = part.get("providerReference") {
                    let reference = reference
                        .as_object()
                        .ok_or_else(|| invalid("providerReference must be an object"))?;
                    if reference.values().any(|item| !item.is_string()) {
                        return Err(invalid("providerReference values must be strings"));
                    }
                }
            }
            _ => return Err(invalid("steering-message parts must be text or file")),
        }
        provider_metadata(part)?;
    }
    Ok(())
}

pub(super) fn validate(value: &Value, policy: UnknownPolicy) -> Result<(), StreamError> {
    validate_json_keys(value)?;
    if !value.is_object() {
        return Err(invalid("event must be an object"));
    }
    match string(value, "type")? {
        "text-start" | "text-end" | "reasoning-start" | "reasoning-end" => {
            string(value, "id")?;
            provider_metadata(value)?;
            if string(value, "type")? == "text-end" {
                citations(value)?;
            }
        }
        "text-delta" | "reasoning-delta" => {
            strings(value, &["id", "delta"])?;
            provider_metadata(value)?;
        }
        "error" => {
            string(value, "errorText")?;
        }
        "tool-input-start" | "tool-input-available" | "tool-input-error" => {
            tool_common(value)?;
            string(value, "toolName")?;
            optional_strings(value, &["title"])?;
            match string(value, "type")? {
                "tool-input-start" => {
                    if let Some(encoding) = value.get("inputEncoding") {
                        if !matches!(encoding.as_str(), Some("json" | "text")) {
                            return Err(invalid("inputEncoding must be json or text"));
                        }
                    }
                }
                "tool-input-error" => {
                    present(value, "input")?;
                    string(value, "errorText")?;
                }
                _ => present(value, "input")?,
            }
        }
        "tool-input-delta" => {
            strings(value, &["toolCallId", "inputTextDelta"])?;
        }
        "tool-output-available" | "tool-output-error" => {
            tool_common(value)?;
            if string(value, "type")? == "tool-output-error" {
                string(value, "errorText")?;
            } else {
                present(value, "output")?;
                optional_bools(value, &["preliminary"])?;
            }
        }
        "tool-output-denied" => {
            string(value, "toolCallId")?;
        }
        "tool-approval-request" => {
            strings(value, &["toolCallId", "approvalId"])?;
            optional_strings(value, &["reason", "signature"])?;
            optional_bools(value, &["isAutomatic"])?;
        }
        "tool-approval-response" => {
            string(value, "approvalId")?;
            if !value.get("approved").is_some_and(Value::is_boolean) {
                return Err(invalid("approved must be a boolean"));
            }
            optional_strings(value, &["reason"])?;
            optional_bools(value, &["providerExecuted"])?;
            provider_metadata(value)?;
        }
        "source-url" => {
            strings(value, &["sourceId", "url"])?;
            optional_strings(value, &["title"])?;
            provider_metadata(value)?;
        }
        "source-document" => {
            strings(value, &["sourceId", "mediaType", "title"])?;
            optional_strings(value, &["filename"])?;
            provider_metadata(value)?;
        }
        "file" | "reasoning-file" => {
            strings(value, &["url", "mediaType"])?;
            provider_metadata(value)?;
        }
        "custom" => {
            string(value, "kind")?;
            provider_metadata(value)?;
        }
        "steering-message" => steering(value)?,
        "start-step" | "finish-step" | "reset-step" => {}
        "message-metadata" => present(value, "messageMetadata")?,
        "start" => optional_strings(value, &["messageId"])?,
        "finish" => {
            if let Some(reason) = value.get("finishReason") {
                if !matches!(
                    reason.as_str(),
                    Some(
                        "stop"
                            | "length"
                            | "content-filter"
                            | "tool-calls"
                            | "error"
                            | "interrupted"
                            | "other"
                    )
                ) {
                    return Err(invalid("unknown finishReason in AI SDK 7 profile"));
                }
            }
        }
        "abort" => optional_strings(value, &["reason"])?,
        kind if kind.starts_with("data-") => {
            present(value, "data")?;
            optional_strings(value, &["id"])?;
            optional_bools(value, &["transient"])?;
        }
        _ if policy == UnknownPolicy::Preserve => {}
        kind => return Err(invalid(format!("unknown event type {kind:?}"))),
    }
    Ok(())
}

pub(super) fn select(value: &Value, fields: &[&str]) -> Map<String, Value> {
    fields
        .iter()
        .filter_map(|field| {
            value
                .get(field)
                .map(|item| ((*field).to_owned(), item.clone()))
        })
        .collect()
}

pub(super) fn projected_citations(value: &Value) -> Option<Value> {
    value
        .get("citations")
        .and_then(Value::as_array)
        .map(|items| {
            Value::Array(
                items
                    .iter()
                    .map(|item| {
                        let mut citation = select(item, &["startIndex", "endIndex", "citedText"]);
                        if let Some(source) = item.get("source") {
                            let source_fields: &[&str] = if source
                                .get("sourceType")
                                .and_then(Value::as_str)
                                == Some("url")
                            {
                                &[
                                    "type",
                                    "sourceType",
                                    "id",
                                    "url",
                                    "title",
                                    "providerMetadata",
                                ]
                            } else {
                                &[
                                    "type",
                                    "sourceType",
                                    "id",
                                    "mediaType",
                                    "title",
                                    "filename",
                                    "providerMetadata",
                                ]
                            };
                            citation.insert(
                                "source".to_owned(),
                                Value::Object(select(source, source_fields)),
                            );
                        }
                        Value::Object(citation)
                    })
                    .collect(),
            )
        })
}
