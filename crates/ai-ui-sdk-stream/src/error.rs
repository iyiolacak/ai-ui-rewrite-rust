//! Recoverable, typed failures at the stream boundary.

use std::fmt;

/// A stream failure that preserves the last successfully applied message state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamError {
    /// An event is not a valid JSON value.
    Json(String),
    /// A server error event stopped a host configured to stop on such events.
    Server(String),
    /// A known event has invalid fields, or a strict profile rejects its type.
    Validation(String),
    /// The reference reducer cannot merge an enumerable value into primitive metadata.
    /// Corresponds to its JavaScript `TypeError`, not a wire-schema failure.
    Metadata(String),
    /// An event refers to a part that is not active or does not exist.
    Lifecycle {
        /// Event or lookup operation that failed.
        chunk_type: String,
        /// External identifier that could not be resolved.
        chunk_id: String,
    },
    /// A configured resource limit would be exceeded.
    Limit {
        /// Name of the bounded resource.
        resource: &'static str,
        /// Configured upper bound.
        limit: usize,
    },
    /// Input was supplied after the stream was explicitly closed or failed.
    Closed,
}

impl fmt::Display for StreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(message) => write!(f, "invalid JSON event: {message}"),
            Self::Server(message) => write!(f, "server stream error: {message}"),
            Self::Validation(message) => write!(f, "invalid stream event: {message}"),
            Self::Metadata(message) => write!(f, "invalid metadata merge: {message}"),
            Self::Lifecycle {
                chunk_type,
                chunk_id,
            } => write!(f, "{chunk_type} cannot resolve active ID {chunk_id:?}"),
            Self::Limit { resource, limit } => {
                write!(f, "stream {resource} limit of {limit} exceeded")
            }
            Self::Closed => f.write_str("stream is closed"),
        }
    }
}

impl std::error::Error for StreamError {}
