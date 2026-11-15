//! Thread messages to AI SDK model messages, ported from assistant-stream's
//! `toGenericMessages` and react-data-stream's `toLanguageModelMessages` and
//! `toModelMessages`. The output is the `messages` array an AI SDK backend
//! passes to `streamText`.

use ai_ui_core::{
    AssistantMessage, AssistantMessagePart, ThreadMessage, ToolApproval, ToolCallPart,
    UserMessagePart,
};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

/// Result sent for a settled tool call that produced no value.
pub const NO_RESULT: &str = "<no result>";

fn data_url_media_type(value: &str) -> Option<String> {
    let rest = value
        .get(..5)
        .filter(|p| p.eq_ignore_ascii_case("data:"))
        .map(|_| &value[5..])?;
    let end = rest.find([';', ','])?;
    (end > 0).then(|| rest[..end].to_ascii_lowercase())
}

fn image_media_type(url: &str, content_type: Option<&str>) -> String {
    if let Some(declared) = content_type.map(str::to_ascii_lowercase) {
        if declared.starts_with("image/") && !declared.contains('*') {
            return declared;
        }
    }
    if let Some(media_type) = data_url_media_type(url) {
        return media_type;
    }
    let path = url.split(['?', '#']).next().unwrap_or_default();
    let extension = path
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "avif" => "image/avif",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "tiff" | "tif" => "image/tiff",
        "heic" => "image/heic",
        "heif" => "image/heif",
        _ => "image/png",
    }
    .to_owned()
}

fn file_part(data: &str, media_type: String, filename: Option<&str>) -> Value {
    let mut part = json!({ "type": "file", "data": data, "mediaType": media_type });
    if let Some(name) = filename.filter(|n| !n.is_empty()) {
        part["filename"] = Value::String(name.to_owned());
    }
    part
}

/// A user content part as a model part, if it has a model representation.
fn user_part(part: &UserMessagePart, content_type: Option<&str>) -> Option<Value> {
    match part {
        UserMessagePart::Text(text) if !text.text.is_empty() => {
            Some(json!({ "type": "text", "text": text.text }))
        }
        UserMessagePart::Text(_) | UserMessagePart::Data(_) => None,
        UserMessagePart::Image(image) if !image.image.is_empty() => Some(file_part(
            &image.image,
            image_media_type(&image.image, content_type),
            image.filename.as_deref(),
        )),
        UserMessagePart::Image(_) => None,
        UserMessagePart::File(file) => {
            let media_type = Some(file.mime_type.clone())
                .filter(|m| !m.is_empty())
                .or_else(|| data_url_media_type(&file.data))
                .unwrap_or_else(|| "application/octet-stream".into());
            Some(file_part(&file.data, media_type, file.filename.as_deref()))
        }
        UserMessagePart::Audio(audio) => {
            let media_type = format!("audio/{}", audio.audio.format);
            let data = match audio.audio.data.split_once(";base64,") {
                Some((prefix, payload)) if prefix.to_ascii_lowercase().starts_with("data:") => {
                    format!("data:{media_type};base64,{payload}")
                }
                _ => audio.audio.data.clone(),
            };
            Some(json!({ "type": "file", "data": data, "mediaType": media_type }))
        }
    }
}

/// Waiting for the user: interrupted, or an approval not yet denied or resolved.
fn awaiting_host(call: &ToolCallPart) -> bool {
    call.interrupt.is_some()
        || call
            .approval
            .as_ref()
            .is_some_and(|a| a.approved != Some(false) && a.resolution.is_none())
}

fn tool_result(call: &ToolCallPart, settled: bool) -> Value {
    let output = if !settled || call.is_error == Some(true) {
        let value = if settled {
            call.result.clone().unwrap_or(Value::Null)
        } else {
            json!({ "error": "Tool call was not completed" })
        };
        json!({ "type": "error-json", "value": value })
    } else if let Some(content) = call.extra.get("modelContent").and_then(Value::as_array) {
        let value: Vec<Value> = content
            .iter()
            .map(|c| match c.get("type").and_then(Value::as_str) {
                Some("text") => json!({ "type": "text", "text": c["text"] }),
                _ => json!({ "type": "media", "data": c["data"], "mediaType": c["mediaType"] }),
            })
            .collect();
        json!({ "type": "content", "value": value })
    } else {
        json!({ "type": "json", "value": call.result.clone().unwrap_or_else(|| Value::String(NO_RESULT.into())) })
    };
    json!({ "type": "tool-result", "toolCallId": call.tool_call_id, "toolName": call.tool_name, "output": output })
}

fn assistant(message: &AssistantMessage, is_last: bool, out: &mut Vec<Value>) {
    let in_flight = is_last && !message.status.is_terminal();
    let mut content: Vec<Value> = Vec::new();
    let mut results: Vec<Value> = Vec::new();
    let flush = |content: &mut Vec<Value>, results: &mut Vec<Value>, out: &mut Vec<Value>| {
        if !content.is_empty() {
            out.push(json!({ "role": "assistant", "content": std::mem::take(content) }));
        }
        if !results.is_empty() {
            out.push(json!({ "role": "tool", "content": std::mem::take(results) }));
        }
    };
    for part in &message.content {
        let value = match part {
            AssistantMessagePart::Text(text) if !text.text.is_empty() => {
                Some(json!({ "type": "text", "text": text.text }))
            }
            AssistantMessagePart::File(file) => {
                let media_type = Some(file.mime_type.clone())
                    .filter(|m| !m.is_empty())
                    .unwrap_or_else(|| "application/octet-stream".into());
                Some(file_part(&file.data, media_type, file.filename.as_deref()))
            }
            AssistantMessagePart::Image(image) if !image.image.is_empty() => Some(file_part(
                &image.image,
                image_media_type(&image.image, None),
                image.filename.as_deref(),
            )),
            AssistantMessagePart::ToolCall(call) => {
                if call.tool_call_id.is_empty() || call.tool_name.is_empty() {
                    continue;
                }
                content.push(json!({ "type": "tool-call", "toolCallId": call.tool_call_id, "toolName": call.tool_name, "input": call.args }));
                let settled = call.is_preliminary != Some(true) && call.result.is_some();
                if !settled && in_flight && awaiting_host(call) {
                    continue;
                }
                results.push(tool_result(call, settled));
                continue;
            }
            _ => None,
        };
        if let Some(value) = value {
            // A text or file after tool calls starts a new assistant turn.
            if !results.is_empty() {
                flush(&mut content, &mut results, out);
            }
            content.push(value);
        }
    }
    flush(&mut content, &mut results, out);
}

/// `toLanguageModelMessages`: system, user, assistant and tool messages.
pub fn to_language_model_messages<'a>(
    messages: impl IntoIterator<Item = &'a ThreadMessage>,
) -> Vec<Value> {
    let messages: Vec<&ThreadMessage> = messages.into_iter().collect();
    let mut out = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        match message {
            ThreadMessage::System(system) => {
                if let Some(text) = system
                    .content
                    .iter()
                    .map(|ai_ui_core::SystemMessagePart::Text(t)| &t.text)
                    .find(|t| !t.is_empty())
                {
                    out.push(json!({ "role": "system", "content": text }));
                }
            }
            ThreadMessage::User(user) => {
                let mut content: Vec<Value> = user
                    .content
                    .iter()
                    .filter_map(|p| user_part(p, None))
                    .collect();
                for attachment in &user.attachments {
                    content.extend(
                        attachment
                            .content
                            .iter()
                            .filter_map(|p| user_part(p, attachment.content_type.as_deref())),
                    );
                }
                if !content.is_empty() {
                    out.push(json!({ "role": "user", "content": content }));
                }
            }
            ThreadMessage::Assistant(message) => {
                assistant(message, index + 1 == messages.len(), &mut out)
            }
        }
    }
    out
}

/// `toModelMessages`: language model messages plus the AI SDK's tool approval
/// request/response parts for decided approvals, and `execution-denied`
/// outputs for denied calls.
pub fn to_model_messages<'a>(
    messages: impl IntoIterator<Item = &'a ThreadMessage> + Clone,
) -> Vec<Value> {
    let mut approvals: HashMap<String, ToolApproval> = HashMap::new();
    for message in messages.clone() {
        if let ThreadMessage::Assistant(assistant) = message {
            for call in assistant.tool_calls() {
                if let Some(approval) = call.approval.as_ref().filter(|a| a.approved.is_some()) {
                    approvals.insert(call.tool_call_id.clone(), approval.clone());
                }
            }
        }
    }
    let converted = to_language_model_messages(messages);
    if approvals.is_empty() {
        return converted;
    }
    let mut out = Vec::new();
    let mut responses: Vec<Value> = Vec::new();
    for mut message in converted {
        match message["role"].as_str() {
            Some("tool") => {
                let mut content = std::mem::take(&mut responses);
                for mut part in message["content"].as_array().cloned().unwrap_or_default() {
                    let id = part["toolCallId"].as_str().unwrap_or_default();
                    if let Some(approval) = approvals.get(id).filter(|a| a.approved == Some(false))
                    {
                        let mut output = json!({ "type": "execution-denied" });
                        if let Some(reason) = &approval.reason {
                            output["reason"] = Value::String(reason.clone());
                        }
                        part["output"] = output;
                    }
                    content.push(part);
                }
                message["content"] = Value::Array(content);
                out.push(message);
                continue;
            }
            Some("assistant") => {
                if !responses.is_empty() {
                    out.push(json!({ "role": "tool", "content": std::mem::take(&mut responses) }));
                }
                let mut content = Vec::new();
                for part in message["content"].as_array().cloned().unwrap_or_default() {
                    let approval = part["toolCallId"].as_str().and_then(|id| approvals.get(id));
                    content.push(part.clone());
                    if let (Some(approval), Some("tool-call")) = (approval, part["type"].as_str()) {
                        let mut response = json!({ "type": "tool-approval-response", "approvalId": approval.id, "approved": approval.approved });
                        if let Some(reason) = &approval.reason {
                            response["reason"] = Value::String(reason.clone());
                        }
                        responses.push(response);
                        let mut request = Map::new();
                        request.insert("type".into(), json!("tool-approval-request"));
                        request.insert("approvalId".into(), json!(approval.id));
                        request.insert("toolCallId".into(), part["toolCallId"].clone());
                        if let Some(reason) = approval.extra.get("requestReason") {
                            request.insert("reason".into(), reason.clone());
                        }
                        for key in ["isAutomatic", "signature", "inputSchemaInput"] {
                            if let Some(value) = approval.extra.get(key) {
                                request.insert(key.into(), value.clone());
                            }
                        }
                        content.push(Value::Object(request));
                    }
                }
                message["content"] = Value::Array(content);
            }
            _ => {
                if !responses.is_empty() {
                    out.push(json!({ "role": "tool", "content": std::mem::take(&mut responses) }));
                }
            }
        }
        out.push(message);
    }
    if !responses.is_empty() {
        out.push(json!({ "role": "tool", "content": responses }));
    }
    out
}
