//! The HTTP boundary. The adapter only needs one streaming POST; hosts supply
//! a client for their platform (a native client, browser `fetch`, or a test
//! double), which keeps this crate free of network and TLS dependencies.

use ai_ui_core::AbortSignal;
use futures_core::Stream;
use std::fmt;
use std::future::Future;
use std::pin::Pin;

/// A JSON POST.
#[derive(Clone)]
pub struct HttpRequest {
    /// The URL to post to.
    pub url: String,
    /// Request headers as name and value pairs.
    pub headers: Vec<(String, String)>,
    /// The request body (JSON).
    pub body: Vec<u8>,
}

/// Not printed: URLs, headers and bodies can contain credentials and
/// conversation content.
impl fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpRequest")
            .field("body_bytes", &self.body.len())
            .finish_non_exhaustive()
    }
}

/// Response body chunks as they arrive.
pub type ByteStream = Pin<Box<dyn Stream<Item = Result<Vec<u8>, HttpError>>>>;

/// A response whose body can be read as it arrives.
pub struct HttpResponse {
    /// The HTTP status code.
    pub status: u16,
    /// Header names in any case.
    pub headers: Vec<(String, String)>,
    /// The response body as a stream of byte slices.
    pub body: ByteStream,
}

impl HttpResponse {
    /// A response header by name, ignoring case.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// A request that could not be completed; the text is for the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpError(pub String);

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for HttpError {}

/// The future of a POST: a response, or why there is none.
pub type ResponseFuture = Pin<Box<dyn Future<Output = Result<HttpResponse, HttpError>>>>;

/// Sends one request. Implementations should stop reading once `signal`
/// aborts; the adapter also stops polling the response then.
pub trait HttpClient {
    /// Send `request`; abandon it when `signal` fires.
    fn post(&self, request: HttpRequest, signal: AbortSignal) -> ResponseFuture;
}
