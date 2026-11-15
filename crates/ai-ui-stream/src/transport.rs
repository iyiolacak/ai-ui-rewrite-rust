//! Incremental, bounded decoding for assistant-ui's `AssistantTransport` SSE wire format.
//!
//! This decoder owns no I/O, authentication, retry, or conversation state. The caller
//! must capture the original run identity, apply backpressure, and interpret the chunks.
//! A `[DONE]` marker only closes the wire stream; it does not certify a run outcome.
//! Invalid or unrecognized chunks terminate decoding rather than silently losing data.
#![forbid(unsafe_code)]

use serde_json::Value;
use std::fmt;

/// Byte bounds for one push, one SSE line, and the joined `data` fields of an event.
/// These bounds do not include caller-retained frames or allocator overhead.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Largest single input slice accepted by `push`.
    pub input_bytes: usize,
    /// Longest SSE line accepted.
    pub line_bytes: usize,
    /// Largest event payload (the joined `data` fields) accepted.
    pub event_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            input_bytes: 64 * 1024,
            line_bytes: 1024 * 1024,
            event_bytes: 1024 * 1024,
        }
    }
}

/// Canonical upstream chunk, preserving all JSON fields, including unsupported metadata.
/// `path` is normalized to an empty array when omitted on message-level chunks.
/// Its fields are private so consumers cannot fabricate a mismatched normalized
/// path or bypass transport validation.
///
/// ```compile_fail
/// use crate::transport::Chunk;
/// let _ = Chunk { kind: "data".into(), path: vec![], value: serde_json::json!({}) };
/// ```
#[derive(Clone, PartialEq)]
pub struct Chunk {
    kind: String,
    path: Vec<u64>,
    value: Value,
}

// Decoded frames can contain signed URLs, file data, errors and other private
// provider content. Inspecting their values must be a deliberate host action.
impl fmt::Debug for Chunk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Chunk")
            .field("kind", &self.kind)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl Chunk {
    /// The transport-validated chunk kind.
    pub fn kind(&self) -> &str {
        &self.kind
    }
    /// The validated, normalized upstream part path.
    pub fn path(&self) -> &[u64] {
        &self.path
    }
    /// Complete decoded JSON, including fields not interpreted by the decoder.
    pub fn value(&self) -> &Value {
        &self.value
    }

    // The typed decoder consumes frames immediately. Move the JSON tree into
    // serde instead of deep-cloning every string, array and metadata object.
    pub(crate) fn into_parts(self) -> (String, Vec<u64>, Value) {
        (self.kind, self.path, self.value)
    }
}

/// Why decoding the SSE stream stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// A limit was zero or beyond the hard bound.
    InvalidLimits,
    /// A single input slice exceeds the limit.
    InputTooLarge,
    /// A line exceeds the limit.
    LineTooLarge,
    /// An event exceeds the limit.
    EventTooLarge,
    /// A line is not valid UTF-8.
    InvalidUtf8,
    /// The stream used an SSE event name other than the default.
    UnknownSseEvent,
    /// An event's data is not valid JSON.
    InvalidJson,
    /// A JSON object key such as `__proto__` is refused.
    UnsafeJsonKey,
    /// An event is JSON but not a valid chunk.
    InvalidChunk,
    /// An event has a chunk type this decoder does not know.
    UnknownChunkType,
    /// The stream ended without `[DONE]`.
    MissingDone,
    /// The decoder already failed or finished.
    Closed,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}

/// All complete chunks preceding a failure remain available to the caller.
/// On failure the decoder becomes terminal; callers must not replay the request.
pub struct Batch {
    /// The chunks decoded from this input, in order.
    pub chunks: Vec<Chunk>,
    /// `[DONE]` has been received.
    pub done: bool,
    /// Set when decoding stopped; the chunks before it are still valid.
    pub error: Option<Error>,
    /// Chunks that were malformed or of an unknown type and were skipped.
    pub dropped: Vec<String>,
}
impl fmt::Debug for Batch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Batch")
            .field("chunk_count", &self.chunks.len())
            .field("done", &self.done)
            .field("error", &self.error)
            .finish()
    }
}

/// The `assistant-transport` SSE decoder: bytes in, chunks out, with explicit limits.
pub struct Decoder {
    limits: Limits,
    line: Vec<u8>,
    data: Vec<u8>,
    data_seen: bool,
    event_name: Option<String>,
    first_line: bool,
    pending_lf: bool,
    state: State,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Open,
    Done,
    Failed,
}

impl Decoder {
    /// A decoder with the given limits; fails if a limit is zero.
    pub fn new(limits: Limits) -> Result<Self, Error> {
        if limits.input_bytes == 0 || limits.line_bytes == 0 || limits.event_bytes == 0 {
            return Err(Error::InvalidLimits);
        }
        Ok(Self {
            limits,
            line: Vec::new(),
            data: Vec::new(),
            data_seen: false,
            event_name: None,
            first_line: true,
            pending_lf: false,
            state: State::Open,
        })
    }

    /// Feed a bounded piece of response bytes. CR, LF, CRLF, and split UTF-8 sequences
    /// are supported. After `[DONE]`, any remaining bytes in this push are ignored.
    pub fn push(&mut self, input: &[u8]) -> Batch {
        let mut batch = Batch {
            chunks: Vec::new(),
            dropped: Vec::new(),
            done: self.state == State::Done,
            error: None,
        };
        match self.state {
            State::Open => {}
            // After `[DONE]` the response is complete: further bytes (a trailing newline in a
            // later read, say) are ignored, exactly as when they arrive in the same read.
            State::Done => return batch,
            State::Failed => {
                batch.error = Some(Error::Closed);
                return batch;
            }
        }
        if input.len() > self.limits.input_bytes {
            self.state = State::Failed;
            batch.error = Some(Error::InputTooLarge);
            return batch;
        }
        for &byte in input {
            if self.pending_lf {
                self.pending_lf = false;
                if byte == b'\n' {
                    continue;
                }
            }
            let result = match byte {
                b'\r' => {
                    self.pending_lf = true;
                    self.end_line(&mut batch)
                }
                b'\n' => self.end_line(&mut batch),
                _ if self.line.len() >= self.limits.line_bytes => Err(Error::LineTooLarge),
                _ => {
                    self.line.push(byte);
                    Ok(())
                }
            };
            if let Err(error) = result {
                self.state = State::Failed;
                batch.error = Some(error);
                break;
            }
            if self.state == State::Done {
                batch.done = true;
                break;
            }
        }
        batch
    }

    /// Ends the response. Unterminated frames are dropped and are never a valid
    /// `[DONE]` marker, matching the strict upstream decoder.
    pub fn finish(&mut self) -> Result<(), Error> {
        match self.state {
            State::Done => Ok(()),
            State::Failed => Err(Error::Closed),
            State::Open => {
                self.state = State::Failed;
                Err(Error::MissingDone)
            }
        }
    }

    fn end_line(&mut self, batch: &mut Batch) -> Result<(), Error> {
        let mut line = std::mem::take(&mut self.line);
        let result = self.parse_line(&line, batch);
        line.clear();
        self.line = line;
        result
    }

    fn parse_line(&mut self, line: &[u8], batch: &mut Batch) -> Result<(), Error> {
        let mut line = line;
        if self.first_line {
            self.first_line = false;
            line = line.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(line);
        }
        let line = std::str::from_utf8(line).map_err(|_| Error::InvalidUtf8)?;
        if line.is_empty() {
            return self.dispatch(batch);
        }
        if line.starts_with(':') {
            return Ok(());
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "data" => {
                let extra = value.len().saturating_add(usize::from(self.data_seen));
                if extra > self.limits.event_bytes.saturating_sub(self.data.len()) {
                    return Err(Error::EventTooLarge);
                }
                if self.data_seen {
                    self.data.push(b'\n');
                }
                self.data.extend_from_slice(value.as_bytes());
                self.data_seen = true;
            }
            "event" => self.event_name = Some(value.to_owned()),
            // Reconnection is deliberately host-owned; id/retry are not acted on.
            _ => {}
        }
        Ok(())
    }

    fn dispatch(&mut self, batch: &mut Batch) -> Result<(), Error> {
        let name = self.event_name.take();
        if !self.data_seen {
            return Ok(());
        }
        self.data_seen = false;
        if name
            .as_deref()
            .is_some_and(|name| !name.is_empty() && name != "message")
        {
            return Err(Error::UnknownSseEvent);
        }
        if self.data == b"[DONE]" {
            self.data.clear();
            self.state = State::Done;
            return Ok(());
        }
        // As upstream: a malformed or unknown chunk is dropped with a warning
        // and the stream continues (forward compatibility). Unsafe keys stay fatal.
        match parse_chunk(&self.data) {
            Ok(chunk) => batch.chunks.push(chunk),
            Err(error @ (Error::InvalidJson | Error::InvalidChunk | Error::UnknownChunkType)) => {
                let preview: String = String::from_utf8_lossy(&self.data)
                    .chars()
                    .take(200)
                    .collect();
                batch
                    .dropped
                    .push(format!("dropped {error:?} chunk: {preview}"));
            }
            Err(error) => {
                self.data.clear();
                return Err(error);
            }
        }
        self.data.clear();
        Ok(())
    }
}

fn parse_chunk(data: &[u8]) -> Result<Chunk, Error> {
    let mut value: Value = serde_json::from_slice(data).map_err(|_| Error::InvalidJson)?;
    if contains_unsafe_keys(&value) {
        return Err(Error::UnsafeJsonKey);
    }
    let object = value.as_object_mut().ok_or(Error::InvalidChunk)?;
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or(Error::InvalidChunk)?
        .to_owned();
    let required_string = |key: &str| object.get(key).is_some_and(Value::is_string);
    let required_array = |key: &str| object.get(key).is_some_and(Value::is_array);
    let (message_level, valid) = match kind.as_str() {
        "part-start" => (true, object.get("part").is_some_and(Value::is_object)),
        "part-finish" | "tool-call-args-text-finish" => (false, true),
        "tool-call-approval" => (
            false,
            object
                .get("approval")
                .and_then(Value::as_object)
                .is_some_and(|a| {
                    a.get("id").is_some_and(Value::is_string)
                        && a.get("approved").is_none_or(Value::is_boolean)
                }),
        ),
        "text-delta" => (false, required_string("textDelta")),
        "annotations" => (true, required_array("annotations")),
        "data" => (true, required_array("data")),
        "step-start" => (true, true),
        "step-finish" | "message-finish" => (true, required_string("finishReason")),
        "result" => (false, object.get("isError").is_none_or(Value::is_boolean)),
        "error" => (true, true),
        "update-state" => (true, required_array("operations")),
        _ => return Err(Error::UnknownChunkType),
    };
    if !valid {
        return Err(Error::InvalidChunk);
    }
    if !object.contains_key("path") && message_level {
        object.insert("path".into(), Value::Array(Vec::new()));
    }
    let path = object
        .get("path")
        .and_then(Value::as_array)
        .ok_or(Error::InvalidChunk)?
        .iter()
        .map(path_index)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Chunk { kind, path, value })
}

// Match JavaScript Number.isInteger for decimal and exponent syntax, including
// negative zero, while deliberately limiting identifiers to safe integers.
fn path_index(value: &Value) -> Result<u64, Error> {
    const MAX_SAFE: u64 = 9_007_199_254_740_991;
    if let Some(integer) = value.as_u64() {
        return (integer <= MAX_SAFE)
            .then_some(integer)
            .ok_or(Error::InvalidChunk);
    }
    let number = value.as_f64().ok_or(Error::InvalidChunk)?;
    if number >= 0.0 && number <= MAX_SAFE as f64 && number.fract() == 0.0 {
        Ok(number as u64)
    } else {
        Err(Error::InvalidChunk)
    }
}

// secure-json-parse's default behavior rejects these prototype-pollution keys
// anywhere in a JSON tree. Do this before returning any object to a host, which
// may later pass retained metadata into JavaScript.
fn contains_unsafe_keys(value: &Value) -> bool {
    match value {
        Value::Object(object) => {
            object.contains_key("__proto__")
                || object
                    .get("constructor")
                    .and_then(Value::as_object)
                    .is_some_and(|constructor| constructor.contains_key("prototype"))
                || object.values().any(contains_unsafe_keys)
        }
        Value::Array(values) => values.iter().any(contains_unsafe_keys),
        _ => false,
    }
}
