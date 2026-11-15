//! Replay-only helpers. No I/O or clock enters the product runtime.

use ai_ui_sdk_stream::{Config, RunState, Runtime, StreamError, Update};
use serde_json::{json, Value};
use std::time::Instant;

/// Replay a JSON request. `snapshot` mode materializes each upstream publication;
/// `patch` mode materializes current state once at the end, including changes
/// that upstream has not published. Timings must name which mode ran, and folds
/// must be checked before treating the modes as equivalent benchmark workloads.
pub fn replay(input: &Value) -> Result<Value, Box<dyn std::error::Error>> {
    let snapshots = match input.get("mode") {
        None => true,
        Some(Value::String(mode)) if mode == "snapshot" => true,
        Some(Value::String(mode)) if mode == "patch" => false,
        Some(_) => return Err("mode must be snapshot or patch".into()),
    };
    let chunks = chunks(input)?;
    let initial = input
        .get("initialMessage")
        .cloned()
        .unwrap_or_else(|| json!({"id":"m","role":"assistant","parts":[]}));
    let mut folded = initial.clone();
    let mut errors = Vec::new();
    let mut transient = Vec::new();
    let mut aborts = Vec::new();
    let mut finish_reason = None;
    let mut emissions = 0_usize;
    let mut snapshots_materialized = 0_usize;
    // Input JSON decoding, network chunk construction, and initial clone are
    // outside the timer, matching the Node oracle's preparation boundary.
    let started = Instant::now();
    let mut runtime = Runtime::with_message(initial, Config::default())?;
    let ticket = runtime.ticket();
    for chunk in &chunks {
        let result = runtime.push(ticket, chunk, |delivery| {
            for update in &delivery.applied.updates {
                match update {
                    Update::Error { error_text } => {
                        errors.push(json!({"name":"Error","message":error_text}))
                    }
                    Update::Data {
                        part,
                        transient: true,
                    } => transient.push(part.clone()),
                    Update::Abort { reason } => {
                        let mut event = json!({"type":"abort"});
                        if let Some(reason) = reason {
                            event["reason"] = json!(reason);
                        }
                        aborts.push(event);
                    }
                    Update::Finish {
                        reason: Some(reason),
                    } => finish_reason = Some(reason.clone()),
                    Update::PartAdded { .. }
                    | Update::PartChanged { .. }
                    | Update::Reset
                    | Update::MetadataChanged
                    | Update::MessageIdChanged
                    | Update::Data {
                        transient: false, ..
                    }
                    | Update::Finish { reason: None }
                    | Update::StepStarted
                    | Update::StepFinished
                    | Update::Unknown { .. } => {}
                }
            }
            if delivery.applied.published {
                emissions += 1;
                if snapshots {
                    folded = delivery.view.snapshot();
                    snapshots_materialized += 1;
                }
            }
        });
        if let Err(error) = result {
            errors.push(normalize_error(&error));
            break;
        }
    }
    if runtime.state() == &RunState::Open {
        runtime.end(ticket)?;
    }
    if !snapshots {
        folded = runtime.view().snapshot();
        snapshots_materialized += 1;
    }
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    let mut sidecars = json!({"errors":errors,"transientData":transient,"aborts":aborts});
    if let Some(reason) = finish_reason {
        sidecars["finishReason"] = json!(reason);
    }
    Ok(json!({
        "foldedMessage":folded,"sidecars":sidecars,"elapsed_ms":elapsed_ms,
        "emissions":emissions,"snapshots_materialized":snapshots_materialized,
        "events":runtime.stats().events,"wire_bytes":runtime.stats().wire_bytes,
        "mode":if snapshots { "snapshot" } else { "patch" }
    }))
}

fn normalize_error(error: &StreamError) -> Value {
    match error {
        StreamError::Json(_) => json!({"name":"AI_JSONParseError"}),
        StreamError::Validation(_) => json!({"name":"AI_TypeValidationError"}),
        StreamError::Metadata(_) => json!({"name":"TypeError"}),
        StreamError::Lifecycle {
            chunk_type,
            chunk_id,
        } => json!({"name":"AI_UIMessageStreamError","chunkType":chunk_type,"chunkId":chunk_id}),
        StreamError::Server(message) => json!({"name":"Error","message":message}),
        StreamError::Limit { resource, limit } => {
            json!({"name":"StreamLimit","resource":resource,"limit":limit})
        }
        StreamError::Closed => json!({"name":"StreamClosed"}),
    }
}

fn chunks(input: &Value) -> Result<Vec<Vec<u8>>, Box<dyn std::error::Error>> {
    let encoded = input.get("network_chunks_base64");
    let utf8 = input.get("network_chunks_utf8");
    if encoded.is_some() && utf8.is_some() {
        return Err("network chunk fields are mutually exclusive".into());
    }
    // Validate even when explicit reads override partitioning, just as the Node
    // oracle does. Otherwise an invalid shared request can silently diverge.
    let width = input.get("chunk_bytes").map(chunk_width).transpose()?;
    if let Some(encoded) = encoded {
        let encoded = encoded.as_array().ok_or("base64 chunks must be array")?;
        return encoded
            .iter()
            .map(|value| base64(value.as_str().ok_or("base64 chunk must be string")?))
            .collect();
    }
    if let Some(chunks) = utf8 {
        let chunks = chunks.as_array().ok_or("UTF-8 chunks must be array")?;
        return chunks
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(|text| text.as_bytes().to_vec())
                    .ok_or_else(|| "UTF-8 chunk must be string".into())
            })
            .collect();
    }
    let raw = input
        .get("raw_utf8")
        .and_then(Value::as_str)
        .ok_or("raw_utf8 required")?
        .as_bytes();
    let width = width.unwrap_or(raw.len().max(1));
    Ok(raw.chunks(width).map(<[u8]>::to_vec).collect())
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the checked positive integral JS-safe-number range fits u64 exactly"
)]
fn chunk_width(value: &Value) -> Result<usize, Box<dyn std::error::Error>> {
    let number = value
        .as_f64()
        .ok_or("chunk_bytes must be positive safe integer")?;
    if !(1.0..=9_007_199_254_740_991.0).contains(&number) || number.fract() > 0.0 {
        return Err("chunk_bytes must be positive safe integer".into());
    }
    // A valid width beyond this platform's address space still means one read.
    Ok(usize::try_from(number as u64).unwrap_or(usize::MAX))
}

fn base64(text: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    if text.len() % 4 != 0 {
        return Err("base64 chunk must use canonical padding".into());
    }
    let quartet_count = text.len() / 4;
    let mut result = Vec::with_capacity(quartet_count * 3);
    for (index, quartet) in text.as_bytes().chunks_exact(4).enumerate() {
        let [a, b, c, d] = quartet else {
            return Err("invalid base64 quartet".into());
        };
        let a = base64_digit(*a)?;
        let b = base64_digit(*b)?;
        let last = index + 1 == quartet_count;
        if *c == b'=' {
            if *d != b'=' || !last || b & 0x0f != 0 {
                return Err("noncanonical base64 padding".into());
            }
            result.push((a << 2) | (b >> 4));
        } else {
            let c = base64_digit(*c)?;
            if *d == b'=' {
                if !last || c & 0x03 != 0 {
                    return Err("noncanonical base64 padding".into());
                }
                result.extend([(a << 2) | (b >> 4), (b << 4) | (c >> 2)]);
            } else {
                let d = base64_digit(*d)?;
                result.extend([(a << 2) | (b >> 4), (b << 4) | (c >> 2), (c << 6) | d]);
            }
        }
    }
    Ok(result)
}

fn base64_digit(byte: u8) -> Result<u8, Box<dyn std::error::Error>> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err("invalid base64 chunk".into()),
    }
}
