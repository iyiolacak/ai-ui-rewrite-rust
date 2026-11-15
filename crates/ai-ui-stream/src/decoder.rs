//! The common interface of wire-format decoders.

use crate::chunk::Chunk;

/// Turns a response body, delivered in arbitrary slices, into chunks.
pub trait StreamDecoder {
    /// Decode more bytes. An error ends the stream.
    fn push(&mut self, input: &[u8]) -> Result<Vec<Chunk>, String>;
    /// End of input: any buffered chunks, or an error if the stream was cut short.
    fn finish(&mut self) -> Result<Vec<Chunk>, String>;
}
