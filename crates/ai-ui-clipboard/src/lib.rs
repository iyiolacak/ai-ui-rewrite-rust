//! Host-owned, result-aware clipboard text operations for assistant presentation.
//!
//! This crate has no Slint or conversation-state dependency. Invoke
//! [`Clipboard::copy`] in the actual user gesture on web: the browser operation
//! starts before this method returns and settles asynchronously. On native,
//! a bounded worker owns the OS clipboard and invokes the completion on that
//! worker; dispatch UI updates back to the UI thread. A queued request is not a
//! successful copy. A caller must retain the native handle while its clipboard
//! ownership is needed, and scope or discard late completion notifications.
//! Call `shutdown` when a session ends; dropping the handle also shuts it down.
//! Writes already admitted to the platform cannot be revoked.
//! The browser accepts one outstanding read or write per adapter; if its
//! promise never settles, later operations report busy for that adapter's life.
//! Clipboard reads may contain private data: invoke them only for an explicit
//! paste gesture, fence delayed results to the original editor selection and
//! session, and never log their contents. Browser paste-event data is a separate
//! input path; clipboard permission and user-activation policies vary by host.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::fmt;

/// Maximum UTF-8 byte length accepted for one copy operation.
pub const MAX_COPY_BYTES: usize = 1024 * 1024;
/// Maximum UTF-8 bytes accepted from one clipboard read. A platform may have
/// already allocated its response before this boundary can reject it.
pub const MAX_READ_BYTES: usize = 4 * 1024 * 1024;

/// No clipboard content or platform exception text is included in errors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CopyError {
    /// Text exceeded [`MAX_COPY_BYTES`].
    TooLarge,
    /// No clipboard capability or secure browser context is available.
    Unavailable,
    /// The bounded native queue or the browser's single write slot is occupied.
    Busy,
    /// The operating system or browser rejected the write.
    Rejected,
    /// The session ended before the write was admitted to the platform.
    Cancelled,
    /// The native worker has stopped and can no longer accept requests.
    WorkerStopped,
}

impl fmt::Display for CopyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TooLarge => "Text is too large to copy",
            Self::Unavailable => "Clipboard is unavailable",
            Self::Busy => "Clipboard is busy",
            Self::Rejected => "Clipboard write was rejected",
            Self::Cancelled => "Clipboard request was cancelled",
            Self::WorkerStopped => "Clipboard worker stopped",
        })
    }
}

impl std::error::Error for CopyError {}

/// Completion is delivered exactly once for a normally running accepted write.
/// On desktop it runs on the clipboard worker, and on web on the browser event
/// loop. Never update a Slint UI directly from the desktop worker.
pub type Completion = Box<dyn FnOnce(Result<(), CopyError>) + Send + 'static>;

/// No clipboard contents or platform exception text are included in errors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadError {
    /// Platform text exceeded [`MAX_READ_BYTES`].
    TooLarge,
    /// The clipboard, secure browser context or text-read API is unavailable.
    Unavailable,
    /// The shared desktop queue or browser operation slot is occupied.
    Busy,
    /// The platform refused the read or did not return text.
    Rejected,
    /// Shutdown happened before the request was admitted to the platform.
    Cancelled,
    /// The desktop worker is no longer accepting requests.
    WorkerStopped,
}
impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TooLarge => "Clipboard text is too large",
            Self::Unavailable => "Clipboard is unavailable",
            Self::Busy => "Clipboard is busy",
            Self::Rejected => "Clipboard read was rejected",
            Self::Cancelled => "Clipboard request was cancelled",
            Self::WorkerStopped => "Clipboard worker stopped",
        })
    }
}
impl std::error::Error for ReadError {}

/// An accepted read completes on the desktop worker or browser event loop;
/// callers must marshal desktop results to the UI thread and fence stale work.
pub type ReadCompletion = Box<dyn FnOnce(Result<String, ReadError>) + Send + 'static>;

#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
mod native;
#[cfg(not(any(
    target_os = "linux",
    target_os = "windows",
    target_os = "macos",
    all(target_arch = "wasm32", target_os = "unknown")
)))]
mod unsupported;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod web;

#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
pub use native::Clipboard;
#[cfg(not(any(
    target_os = "linux",
    target_os = "windows",
    target_os = "macos",
    all(target_arch = "wasm32", target_os = "unknown")
)))]
pub use unsupported::Clipboard;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub use web::Clipboard;
