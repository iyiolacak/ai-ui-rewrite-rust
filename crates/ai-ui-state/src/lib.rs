//! UI-agnostic state for assistant-ui, after upstream's scoped state layer.
//!
//! A UI never reads a runtime directly. It renders [`ThreadState`] from a
//! [`ThreadView`] and calls the view's actions, whatever toolkit it uses and
//! whichever runtime (local, external-store, or a thread list's main thread)
//! sits below.
//!
//! ```
//! use ai_ui_core::{adapter, ChatModelRunResult, LocalRuntimeOptions, LocalThreadRuntime};
//! use ai_ui_state::{PartKind, ThreadView, ViewOptions};
//! use std::rc::Rc;
//!
//! let runtime = LocalThreadRuntime::new(
//!     |_options| adapter::from_results([Ok(ChatModelRunResult::text("Hello!"))]),
//!     LocalRuntimeOptions::default(),
//! );
//! let view = ThreadView::new(Rc::new(runtime), ViewOptions { smooth: None });
//! let run = view.send("Hi").unwrap().unwrap();
//! futures_executor::block_on(run).unwrap();
//! let state = view.state();
//! assert_eq!(state.messages.len(), 2);
//! assert!(matches!(&state.messages[1].parts[0].kind, PartKind::Text { text, .. } if text == "Hello!"));
//! ```
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod model;
mod view;

pub use model::{
    ApprovalDecision, ApprovalState, MessageState, PartKind, PartState, ThreadState, ToolCallState,
    ToolPresentation, ToolStatus,
};
pub use view::{ThreadView, ViewChanges, ViewCursor, ViewOptions};
