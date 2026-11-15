//! Rust port of [`@assistant-ui/react-data-stream`](https://github.com/assistant-ui/assistant-ui/tree/main/packages/react-data-stream):
//! connect a thread runtime to an AI SDK backend.
//!
//! ```ignore
//! let adapter = DataStreamAdapter::new(my_http_client, DataStreamOptions::new("https://example.com/api/chat"));
//! let runtime = LocalThreadRuntime::new(adapter, LocalRuntimeOptions::default());
//! ```
//!
//! The request body matches upstream (`messages` in the AI SDK model message
//! format, `tools`, `system`, `runConfig`, …); the response may be a UI message
//! stream or a data stream, detected from the AI SDK headers.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod adapter;
pub mod convert;
pub mod http;

pub use adapter::{request_body, resolve_protocol, DataStreamAdapter, DataStreamOptions, Protocol};
pub use convert::{to_language_model_messages, to_model_messages};
pub use http::{ByteStream, HttpClient, HttpError, HttpRequest, HttpResponse, ResponseFuture};
