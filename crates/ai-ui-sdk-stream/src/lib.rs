//! Shared, renderer-independent assistant stream state for native and WebAssembly.
//!
//! This first compatibility profile implements the AI SDK 7.0.166 UI-message
//! protocol (`x-vercel-ai-ui-message-stream: v1`). That header identifies a wire
//! format, **not** an SDK major version. Legacy data-stream and assistant-transport
//! adapters are not implemented by this crate yet.
//!
//! Feed network bytes with a captured [`RunTicket`]. Every complete event is
//! applied in order to one mutable owner. [`Delivery`] exposes small notifications
//! and a borrowed [`MessageView`]; the ordinary delta path never clones the whole
//! message. Materialize a JSON snapshot only when an API consumer needs one.
//!
//! No I/O, renderer, async runtime, timers or platform-specific APIs live here.
//! Wire errors preserve previously committed content. Configurable limits bound
//! accepted input and part count; they are this crate's policy, not upstream protocol limits.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod engine;
mod error;
mod framing;
mod reducer;
mod update;

pub use engine::{
    Config, Delivery, FeedOutcome, Limits, MessageView, PartHandle, RunState, RunStats, RunTicket,
    Runtime, ServerErrorPolicy,
};
pub use error::StreamError;
pub use reducer::{ReducerOptions, UiReducer, UnknownPolicy};
pub use update::{ApplyResult, Update};
