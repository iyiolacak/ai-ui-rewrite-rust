//! The audit's AI SDK vectors (`docs/PROTOCOL.md`): the v7 UI message stream and the v4 data stream.
//!
//! Their oracle is the AI SDK's own `UIMessage`, which has parts we do not model (steps, custom content, steering
//! messages, reasoning files, per-part state, message metadata). So each side is reduced to what both can express:
//! text, reasoning, tool calls with their results, sources, files and data parts, in order, plus whether the run
//! failed. Everything else is a known difference (`KNOWN_DIFFERENCES`), each with the reason; the test fails when
//! the set of differing vectors changes.

use ai_ui_core::adapter::RunStream;
use ai_ui_core::*;
use ai_ui_stream::{
    run_stream, DataStreamDecoder, MessageAccumulator, SseLimits, UiMessageStreamDecoder,
};
use base64::Engine as _;
use futures_core::Stream;
use serde_json::{json, Value};
use std::pin::Pin;
use std::task::{Context, Poll};

struct Reads(Vec<Vec<u8>>);
impl Stream for Reads {
    type Item = Result<Vec<u8>, String>;
    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready((!self.0.is_empty()).then(|| Ok(self.0.remove(0))))
    }
}

fn reads(vector: &Value) -> Vec<Vec<u8>> {
    if let Some(items) = vector.get("network_chunks_utf8").and_then(Value::as_array) {
        return items
            .iter()
            .map(|s| s.as_str().unwrap().as_bytes().to_vec())
            .collect();
    }
    if let Some(items) = vector
        .get("network_chunks_base64")
        .and_then(Value::as_array)
    {
        let engine = base64::engine::general_purpose::STANDARD;
        return items
            .iter()
            .map(|s| engine.decode(s.as_str().unwrap()).unwrap())
            .collect();
    }
    vec![vector["raw_utf8"].as_str().unwrap().as_bytes().to_vec()]
}

/// Our side: run the vector through the local runtime and read the stored assistant message.
fn run(vector: &Value) -> (Vec<Value>, Vec<Value>, bool) {
    let body = reads(vector);
    let v4 = vector["oracle_profile"] == "ai-sdk-v4";
    let runtime = LocalThreadRuntime::new(
        move |_options: ChatModelRunOptions| -> RunStream {
            let body = Reads(body.clone());
            let acc = MessageAccumulator::default();
            if v4 {
                run_stream(body, DataStreamDecoder::new(16 << 20), acc)
            } else {
                run_stream(body, UiMessageStreamDecoder::new(SseLimits::default()), acc)
            }
        },
        LocalRuntimeOptions::default(),
    );
    let run = runtime
        .append(AppendMessage::user_text(None, "go"))
        .unwrap();
    let failed = futures_executor::block_on(run).is_err();
    let messages = runtime.messages();
    let assistant = messages[1].as_assistant().unwrap();
    let errored = failed
        || matches!(
            assistant.status,
            MessageStatus::Incomplete {
                reason: IncompleteReason::Error,
                ..
            }
        );
    let content = serde_json::to_value(&assistant.content).unwrap();
    // AI SDK data parts are message parts; here they live in the message's data list.
    let data = serde_json::to_value(&assistant.metadata).unwrap()["unstable_data"].clone();
    let data = data
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| json!({ "name": entry["name"], "data": entry["data"] }))
        .collect();
    (essentials_ours(content.as_array().unwrap()), data, errored)
}

fn essentials_ours(parts: &[Value]) -> Vec<Value> {
    let mut out = Vec::new();
    for part in parts {
        match part["type"].as_str().unwrap_or("") {
            "text" => out.push(json!({ "k": "text", "text": part["text"] })),
            "reasoning" => out.push(json!({ "k": "reasoning", "text": part["text"] })),
            "tool-call" => {
                let mut tool =
                    json!({ "k": "tool", "id": part["toolCallId"], "name": part["toolName"] });
                // An error (or denial) is compared as a flag; its text is represented differently.
                if part.get("isError") == Some(&json!(true)) {
                    tool["error"] = json!(true);
                } else if let Some(result) = part.get("result") {
                    tool["result"] = result.clone();
                }
                out.push(tool);
            }
            "source" => out.push(
                json!({ "k": "source", "url": part.get("url").cloned().unwrap_or(Value::Null) }),
            ),
            "file" => out.push(json!({ "k": "file" })),
            _ => {}
        }
    }
    out
}

/// The oracle's side, reduced the same way. Parts we cannot represent are skipped (see the module docs).
fn essentials_expected(parts: &[Value], v4: bool) -> Vec<Value> {
    let mut out = Vec::new();
    for part in parts {
        let kind = part["type"].as_str().unwrap_or("");
        match kind {
            "text" => out.push(json!({ "k": "text", "text": part["text"] })),
            "reasoning" => {
                // v4 calls the text `reasoning`.
                let text = part
                    .get("text")
                    .or(part.get("reasoning"))
                    .cloned()
                    .unwrap_or(Value::Null);
                out.push(json!({ "k": "reasoning", "text": text }));
            }
            "tool-invocation" if v4 => {
                let invocation = &part["toolInvocation"];
                let mut tool = json!({ "k": "tool", "id": invocation["toolCallId"], "name": invocation["toolName"] });
                if invocation["state"] == "result" {
                    tool["result"] = invocation["result"].clone();
                }
                out.push(tool);
            }
            "dynamic-tool" => out.push(tool_v7(part, part["toolName"].clone())),
            k if k.starts_with("tool-") => out.push(tool_v7(part, json!(&k["tool-".len()..]))),
            "source-url" | "source" => {
                // v4 nests the source in the part.
                let url = part.get("url").or(part["source"].get("url")).cloned();
                out.push(json!({ "k": "source", "url": url.unwrap_or(Value::Null) }));
            }
            "source-document" => out.push(json!({ "k": "source", "url": Value::Null })),
            "file" => out.push(json!({ "k": "file" })),
            _ => {}
        }
    }
    out
}

fn tool_v7(part: &Value, name: Value) -> Value {
    let mut tool = json!({ "k": "tool", "id": part["toolCallId"], "name": name });
    match part["state"].as_str() {
        Some("output-available") => tool["result"] = part["output"].clone(),
        Some("output-error" | "output-denied") => tool["error"] = json!(true),
        _ => {}
    }
    tool
}

fn expected_data(parts: &[Value]) -> Vec<Value> {
    parts
        .iter()
        .filter_map(|part| {
            let name = part["type"].as_str()?.strip_prefix("data-")?;
            Some(json!({ "name": name, "data": part["data"] }))
        })
        .collect()
}

fn expects_error(vector: &Value) -> bool {
    let sidecars = &vector["expected"]["sidecars"];
    sidecars["error"].as_object().is_some()
        || sidecars["error"].is_string()
        || sidecars["errors"].as_array().is_some_and(|e| !e.is_empty())
}

/// Vectors we do not match, and why. Each entry is a decision or a bug to fix.
const KNOWN_DIFFERENCES: &[(&str, &str)] = &[
    // Policy: the decoders are lenient (like upstream assistant-ui's), where the AI SDK raises on lifecycle and
    // schema violations. Content received is kept and the oddity is a warning, never a failed run.
    ("U08", "delta before text-start: we create the text part; the AI SDK raises"),
    ("U09", "tool delta before tool-input-start: lenient; the AI SDK raises"),
    ("U10", "tool output without a call: we show an unnamed call; the AI SDK raises"),
    ("U32", "repeated text-end: ignored; the AI SDK raises"),
    ("U33", "unknown non-data chunk type: dropped with a warning; the AI SDK rejects it"),
    ("U36", "invalid JSON event: dropped with a warning (as the native transport does); the AI SDK fails the stream"),
    ("U39", "approval response for an unknown id: ignored; the AI SDK raises"),
    ("U43", "finish reason `unknown`: accepted; the v7 schema rejects it"),
    ("L16_crlf_blank_line", "a blank CRLF line in a v4 stream: skipped; v4's parser fails"),
    // Decisions about replay and ordering.
    ("U11", "a late preliminary result after the final one: final wins (monotonic); the AI SDK overwrites"),
    ("U38", "events after [DONE]: the stream ends at [DONE] (as the native transport does); the AI SDK keeps folding"),
    ("U41", "tool-input delta with no start (resume mid-tool): dropped; resuming needs a replayed prefix"),
    ("L09_error_mid_stream", "v4 stops at an error line; we record the error and fold what follows"),
];

#[test]
fn ai_sdk_vectors_against_our_decoders() {
    let file: Value = serde_json::from_str(include_str!("fixtures/protocol-vectors.json")).unwrap();
    let mut failing: Vec<String> = Vec::new();
    let mut ran = 0;
    for vector in file["vectors"].as_array().unwrap() {
        let profile = vector["oracle_profile"].as_str().unwrap();
        if profile != "ai-sdk-ui7" && profile != "ai-sdk-v4" {
            continue;
        }
        ran += 1;
        let id = vector["id"].as_str().unwrap();
        let v4 = profile == "ai-sdk-v4";
        let (ours, our_data, errored) = run(vector);
        let message = &vector["expected"]["foldedMessage"];
        let parts = message["parts"].as_array().cloned().unwrap_or_default();
        let expected = essentials_expected(&parts, v4);
        // v4 reports data (`2:`) beside the message, not as parts.
        let expected_data = if v4 {
            our_data.clone()
        } else {
            expected_data(&parts)
        };
        let mut diffs = Vec::new();
        if errored != expects_error(vector) {
            diffs.push(format!(
                "error: ours {errored}, oracle {}",
                expects_error(vector)
            ));
        }
        if our_data != expected_data {
            diffs.push(format!(
                "data: ours {}\n         oracle {}",
                json!(our_data),
                json!(expected_data)
            ));
        }
        if ours != expected {
            diffs.push(format!(
                "parts: ours {}\n         oracle {}",
                json!(ours),
                json!(expected)
            ));
        }
        if !diffs.is_empty() {
            let description = vector["description"]
                .as_str()
                .or(vector["evidence"].as_str())
                .unwrap_or("");
            println!("{id}: {description}\n    {}", diffs.join("\n    "));
            failing.push(id.to_owned());
        }
    }
    println!("ran {ran}, differing {}: {failing:?}", failing.len());
    let mut failing_sorted: Vec<&str> = failing.iter().map(String::as_str).collect();
    failing_sorted.sort_unstable();
    let mut known: Vec<&str> = KNOWN_DIFFERENCES.iter().map(|(id, _)| *id).collect();
    known.sort_unstable();
    assert_eq!(
        failing_sorted, known,
        "the set of differing vectors changed"
    );
}
