//! Ready-made [`HttpClient`](ai_ui_data_stream::HttpClient)s: a native
//! client (ureq with rustls, one worker thread per request) and, on
//! `wasm32`, a browser `fetch` client.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(not(target_arch = "wasm32"))]
mod native;

#[cfg(target_arch = "wasm32")]
pub use browser::FetchClient;
#[cfg(not(target_arch = "wasm32"))]
pub use native::NativeClient;
