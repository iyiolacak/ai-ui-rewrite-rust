//! Rust port of [`assistant-stream`](https://github.com/assistant-ui/assistant-ui/tree/main/packages/assistant-stream):
//! typed stream chunks, the message accumulator that folds them into an
//! assistant message, and the `assistant-transport` SSE serialization.
//!
//! A model adapter wraps its response byte stream with [`run_stream_from_sse`],
//! which decodes with [`SseDecoder`], folds each [`Chunk`] into a
//! [`MessageAccumulator`] and yields the cumulative result to the runtime.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod accumulator;
pub mod chunk;
pub mod data_stream;
pub mod decoder;
pub mod partial_json;
mod results;
pub mod sse;
mod sse_json;
pub mod state;
pub mod transport;
pub mod ui_message;
mod writer;

pub use accumulator::{Limits as AccumulatorLimits, MessageAccumulator};
pub use chunk::{Chunk, ChunkKind, FinishReason, PartInit};
pub use data_stream::DataStreamDecoder;
pub use decoder::StreamDecoder;
pub use results::{run_stream, run_stream_from_sse};
pub use sse::{DecodeError, Decoded, SseDecoder};
pub use sse_json::SseLimits;
pub use ui_message::UiMessageStreamDecoder;
