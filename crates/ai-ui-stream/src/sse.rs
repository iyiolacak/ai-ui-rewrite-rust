//! assistant-stream's `assistant-transport` SSE serialization: response bytes
//! to typed [`Chunk`]s, using the bounded decoder in [`crate::transport`].

use crate::chunk::Chunk;
use crate::transport::{Decoder, Error as TransportError, Limits};
use std::fmt;

/// Why decoding an assistant-transport stream stopped.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum DecodeError {
    /// The byte stream violated the SSE framing or a limit.
    Transport(TransportError),
    /// A frame was valid JSON but not a chunk this port understands.
    /// A chunk was recognised but its fields were invalid.
    /// A chunk was recognised but its fields were invalid.
    InvalidChunk {
        /// The chunk type.
        kind: String,
        /// What was wrong with it.
        message: String,
    },
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(error) => write!(f, "stream framing: {error}"),
            Self::InvalidChunk { kind, message } => write!(f, "invalid {kind} chunk: {message}"),
        }
    }
}
impl std::error::Error for DecodeError {}

/// Chunks decoded from one input slice.
#[derive(Debug, Default)]
pub struct Decoded {
    /// The chunks decoded from this input.
    pub chunks: Vec<Chunk>,
    /// The `[DONE]` terminator was seen.
    pub done: bool,
    /// Decoding stopped; chunks before the error are still returned.
    pub error: Option<DecodeError>,
    /// Malformed or unknown chunks that were skipped, as upstream does.
    pub dropped: Vec<String>,
}

/// Decodes the `assistant-transport` SSE protocol into chunks.
pub struct SseDecoder {
    decoder: Decoder,
    limits: Limits,
    /// A failure reported on the next call, after the chunks before it.
    failed: Option<String>,
}

impl SseDecoder {
    /// A decoder with the given limits; fails if a limit is zero.
    pub fn new(limits: Limits) -> Result<Self, DecodeError> {
        Ok(Self {
            decoder: Decoder::new(limits).map_err(DecodeError::Transport)?,
            limits,
            failed: None,
        })
    }

    /// Decode any amount of input, in slices the transport decoder accepts.
    pub fn push(&mut self, input: &[u8]) -> Decoded {
        let mut decoded = Decoded::default();
        for slice in input.chunks(self.limits.input_bytes.max(1)) {
            let batch = self.decoder.push(slice);
            decoded.dropped.extend(batch.dropped);
            for chunk in batch.chunks {
                let (kind, path, value) = chunk.into_parts();
                let path = path.into_iter().map(|i| i as usize).collect();
                match serde_json::from_value::<Chunk>(value) {
                    Ok(mut typed) => {
                        typed.path = path;
                        decoded.chunks.push(typed);
                    }
                    Err(error) => decoded
                        .dropped
                        .push(format!("dropped invalid {kind} chunk: {error}")),
                }
            }
            decoded.done |= batch.done;
            if let Some(error) = batch.error {
                decoded.error = Some(DecodeError::Transport(error));
                return decoded;
            }
        }
        decoded
    }

    /// End of input: an error unless the stream terminated cleanly.
    pub fn finish(&mut self) -> Result<(), DecodeError> {
        self.decoder.finish().map_err(DecodeError::Transport)
    }
}

impl Default for SseDecoder {
    fn default() -> Self {
        Self::new(Limits::default()).expect("default limits are valid")
    }
}

impl crate::decoder::StreamDecoder for SseDecoder {
    fn push(&mut self, input: &[u8]) -> Result<Vec<Chunk>, String> {
        if let Some(error) = self.failed.take() {
            return Err(error);
        }
        let decoded = SseDecoder::push(self, input);
        // Chunks before a failure are delivered first; the failure follows.
        self.failed = decoded.error.map(|error| error.to_string());
        Ok(decoded.chunks)
    }
    fn finish(&mut self) -> Result<Vec<Chunk>, String> {
        if let Some(error) = self.failed.take() {
            return Err(error);
        }
        SseDecoder::finish(self)
            .map(|()| Vec::new())
            .map_err(|e| e.to_string())
    }
}
