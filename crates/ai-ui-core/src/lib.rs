//! Rust port of [`@assistant-ui/core`](https://github.com/assistant-ui/assistant-ui):
//! the thread message model, the branching message repository, the model
//! adapter contract and the local thread runtime.
//!
//! ```
//! use ai_ui_core::{adapter, AppendMessage, ChatModelRunResult, LocalRuntimeOptions, LocalThreadRuntime};
//!
//! let runtime = LocalThreadRuntime::new(
//!     |_options| adapter::from_results([Ok(ChatModelRunResult::text("Hello!"))]),
//!     LocalRuntimeOptions::default(),
//! );
//! let run = runtime.append(AppendMessage::user_text(None, "Hi")).unwrap();
//! futures_executor::block_on(run).unwrap();
//! assert_eq!(runtime.messages().last().unwrap().text(), "Hello!");
//! ```
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod abort;
pub mod adapter;
pub mod external_store;
pub mod local;
pub mod repository;
pub mod runtime;
pub mod smooth;
mod subscribe;
pub mod thread_list;
pub mod types;

pub use abort::{AbortController, AbortSignal};
pub use adapter::{
    ChatModelAdapter, ChatModelRunOptions, ChatModelRunResult, ModelContext, RunError, RunStream,
};
pub use external_store::{ExternalStore, ExternalStoreRuntime, StoreFuture};
pub use local::{
    Composer, LocalRuntimeOptions, LocalThreadRuntime, Tool, ToolContext, ToolFuture, ToolRegistry,
};
pub use repository::{
    ExportedMessage, ExportedRepository, MessageChanges, MessageRepository, RepositoryError,
};
pub use runtime::{
    ComposerHost, RunFuture, RuntimeCapabilities, RuntimeError, StartRunConfig, ThreadRuntime,
    ToolResult,
};
pub use smooth::{SmoothOptions, SmoothText};
pub use subscribe::{Subscribers, Subscription};
pub use thread_list::{
    AdapterFuture, RemoteThread, Spawner, ThreadItem, ThreadList, ThreadListAdapter, ThreadStatus,
};
pub use types::*;
