//! The protocol audit's vectors (`docs/PROTOCOL.md`) run through our decoders.
//!
//! Only the vectors whose oracle is assistant-stream's own `AssistantMessage` projection are compared directly: that
//! is the shape our message model mirrors. A vector passes when everything the oracle expects is present and equal in
//! our result (we may carry more), and when an error is reported exactly when the oracle reports one. Vectors we
//! intentionally do not match are listed in `KNOWN_DIFFERENCES` with the reason; the test fails if the set of
//! failing vectors changes, so both regressions and fixes show up.

use ai_ui_core::adapter::RunStream;
use ai_ui_core::*;
use ai_ui_stream::{
    run_stream, DataStreamDecoder, MessageAccumulator, SseDecoder, SseLimits,
    UiMessageStreamDecoder,
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

/// What the vector's oracle yields: the last message, and whether it failed.
struct Outcome {
    /// No content was produced (the oracle's `null` message).
    empty: bool,
    message: Option<Value>,
    error: Option<String>,
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
        return items
            .iter()
            .map(|s| {
                base64::engine::general_purpose::STANDARD
                    .decode(s.as_str().unwrap())
                    .unwrap()
            })
            .collect();
    }
    vec![vector["raw_utf8"].as_str().unwrap().as_bytes().to_vec()]
}

fn stream_for(vector: &Value) -> Option<RunStream> {
    let body = Reads(reads(vector));
    let acc = MessageAccumulator::default();
    Some(match vector["protocol"].as_str().unwrap() {
        "assistant-transport" => {
            if vector["options"].get("maxEventLength").is_some() {
                return None; // our limits are in bytes and configured elsewhere
            }
            run_stream(body, SseDecoder::default(), acc)
        }
        "aui-data-stream" => {
            if vector["options"].get("strict") == Some(&json!(false)) {
                return None; // we have a single, strict policy
            }
            run_stream(body, DataStreamDecoder::new(16 << 20), acc)
        }
        "ui-message-stream" => {
            run_stream(body, UiMessageStreamDecoder::new(SseLimits::default()), acc)
        }
        _ => return None,
    })
}

/// The adapter's last cumulative result, in the oracle's naming (`parts`, not
/// `content`). The oracle is assistant-stream's accumulator, so this compares at
/// the adapter boundary; the runtime's continuation rules are tested elsewhere.
fn project(result: &ChatModelRunResult) -> Value {
    let metadata = result.metadata.clone().unwrap_or_default();
    json!({
        "role": "assistant",
        "status": result.status.as_ref().map_or(json!({ "type": "running" }), |s| serde_json::to_value(s).unwrap()),
        "parts": serde_json::to_value(result.content.clone().unwrap_or_default()).unwrap(),
        "metadata": {
            "unstable_state": metadata.state.unwrap_or(Value::Null),
            "unstable_data": metadata.data.unwrap_or_default(),
            "unstable_annotations": metadata.annotations.unwrap_or_default(),
            "steps": serde_json::to_value(metadata.steps.unwrap_or_default()).unwrap(),
            "custom": metadata.custom.map_or(json!({}), Value::Object),
        }
    })
}

fn run(vector: &Value) -> Option<Outcome> {
    let mut stream = stream_for(vector)?;
    let mut outcome = Outcome {
        empty: true,
        message: None,
        error: None,
    };
    futures_executor::block_on(async {
        while let Some(item) = std::future::poll_fn(|cx| stream.as_mut().poll_next(cx)).await {
            match item {
                Ok(result) => {
                    outcome.empty = result.content.as_ref().is_none_or(Vec::is_empty);
                    outcome.message = Some(project(&result));
                }
                Err(error) => outcome.error = Some(error.to_string()),
            }
        }
    });
    Some(outcome)
}

/// Drop what only differs in representation: lifecycle `state` keys and the
/// per-part `status` (our parts record completion through the message).
fn representation_free(value: &Value, in_parts: bool) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(k, _)| *k != "state" && !(in_parts && *k == "status"))
                .map(|(k, v)| (k.clone(), representation_free(v, k == "parts")))
                .collect(),
        ),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|v| representation_free(v, in_parts))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Every key the oracle has must be present and equal in ours; extra keys of ours are fine.
fn covers(expected: &Value, ours: &Value, path: &str, diffs: &mut Vec<String>) {
    match (expected, ours) {
        (Value::Object(e), Value::Object(o)) => {
            for (key, value) in e {
                match o.get(key) {
                    Some(mine) => covers(value, mine, &format!("{path}.{key}"), diffs),
                    None => diffs.push(format!("{path}.{key}: missing (expected {value})")),
                }
            }
        }
        (Value::Array(e), Value::Array(o)) => {
            if e.len() != o.len() {
                diffs.push(format!("{path}: {} items, expected {}", o.len(), e.len()));
            }
            for (i, (a, b)) in e.iter().zip(o).enumerate() {
                covers(a, b, &format!("{path}[{i}]"), diffs);
            }
        }
        _ if expected == ours => {}
        _ => diffs.push(format!("{path}: {ours}, expected {expected}")),
    }
}

fn compare(vector: &Value, outcome: &Outcome) -> Vec<String> {
    let mut diffs = Vec::new();
    let expected = &vector["expected"];
    let expects_error = !expected["sidecars"]["error"].is_null();
    if expects_error != outcome.error.is_some() {
        diffs.push(format!(
            "error: ours {:?}, oracle {}",
            outcome.error, expected["sidecars"]["error"]
        ));
    }
    match (&expected["foldedMessage"], &outcome.message) {
        (Value::Null, _) if outcome.empty => {}
        (Value::Null, _) => diffs.push("produced content, oracle yielded no message".into()),
        (_, None) => diffs.push("yielded no message".into()),
        (e, Some(ours)) => covers(&representation_free(e, false), ours, "message", &mut diffs),
    }
    diffs
}

/// Vectors we do not match, and why. Keep this list honest: each entry is a decision or a bug to fix.
const KNOWN_DIFFERENCES: &[(&str, &str)] = &[(
    "AU06",
    "a final line without a newline is accepted, as AI SDK v4 does; assistant-ui's legacy decoder rejects it",
), (
    "AUI_UI02",
    "data-* parts with an id replace the earlier one with the same name and id (AI SDK 7); upstream assistant-ui drops the id and appends",
), (
    "AUI_UI03",
    "reset-step discards the current step's parts (AI SDK 7); upstream assistant-ui ignores it and shows both versions",
)];

#[test]
fn assistant_stream_vectors_against_our_decoders() {
    let file: Value = serde_json::from_str(include_str!("fixtures/protocol-vectors.json")).unwrap();
    let mut failing: Vec<String> = Vec::new();
    let (mut ran, mut skipped) = (0, 0);
    for vector in file["vectors"].as_array().unwrap() {
        if !vector["oracle_profile"]
            .as_str()
            .unwrap()
            .starts_with("assistant-stream")
        {
            continue;
        }
        let id = vector["id"].as_str().unwrap();
        let Some(outcome) = run(vector) else {
            skipped += 1;
            continue;
        };
        ran += 1;
        let diffs = compare(vector, &outcome);
        if !diffs.is_empty() {
            println!(
                "{id}: {}\n    {}",
                vector["description"],
                diffs.join("\n    ")
            );
            failing.push(id.to_owned());
        }
    }
    println!(
        "ran {ran}, skipped {skipped}, differing {}: {failing:?}",
        failing.len()
    );
    let known: Vec<&str> = KNOWN_DIFFERENCES.iter().map(|(id, _)| *id).collect();
    let mut failing_sorted = failing.iter().map(String::as_str).collect::<Vec<_>>();
    failing_sorted.sort_unstable();
    let mut known_sorted = known.clone();
    known_sorted.sort_unstable();
    assert_eq!(
        failing_sorted, known_sorted,
        "the set of differing vectors changed"
    );
}
