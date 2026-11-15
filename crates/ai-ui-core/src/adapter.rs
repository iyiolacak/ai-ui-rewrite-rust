//! The model adapter contract, ported from `chat-model-adapter.ts`.
//!
//! A [`ChatModelAdapter`] receives the thread up to the new assistant message
//! and returns a stream of [`ChatModelRunResult`]s. As upstream, each result is
//! cumulative: its `content` replaces everything the adapter produced so far in
//! this step, and a `status` or `metadata` field updates only what it names. An
//! adapter that produces one answer returns a one-item stream ([`once`]).

use crate::abort::AbortSignal;
use crate::types::{
    AssistantMessage, AssistantMessagePart, JsonObject, MessageId, MessageStatus, RunConfig,
    ThreadMessage, ThreadStep,
};
use futures_core::Stream;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

/// A tool the model may call (`Tool` in `model-context/tool.ts`, without the
/// frontend `execute`/`render` hooks, which belong to the tool registry).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolDefinition {
    /// What the tool does, for the model.
    pub description: Option<String>,
    /// JSON Schema of the arguments.
    pub parameters: Value,
}

/// System prompt, tools and settings contributed by the app (`ModelContext`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelContext {
    /// System prompt to send first.
    pub system: Option<String>,
    /// Tools the model may call, by name.
    pub tools: BTreeMap<String, ToolDefinition>,
    /// Model settings such as temperature, in the form the backend expects.
    pub call_settings: JsonObject,
    /// Backend configuration such as the model name.
    pub config: JsonObject,
}

/// Everything an adapter receives for one step (`ChatModelRunOptions`).
#[derive(Clone, Debug)]
pub struct ChatModelRunOptions {
    /// The selected path up to and including the message being answered.
    pub messages: Arc<[Arc<ThreadMessage>]>,
    /// The run's configuration.
    pub run_config: RunConfig,
    /// Fires when the run is cancelled; stop work and end the stream.
    pub abort_signal: AbortSignal,
    /// The app's system prompt, tools and settings.
    pub context: ModelContext,
    /// Id of the assistant message this step fills.
    pub assistant_message_id: MessageId,
    /// The thread's remote id, when it has one.
    pub thread_id: Option<String>,
    /// The message being answered.
    pub parent_id: Option<MessageId>,
    /// The assistant message being answered, as it was when this step started
    /// (`unstable_getMessage()` upstream). Its content is non-empty when a run
    /// continues after tool results; its metadata holds the thread state.
    pub message: AssistantMessage,
}

/// Metadata updates in a run result. Lists are appended to what the message
/// had when the step started; `custom` keys are merged.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunResultMetadata {
    /// App state to merge into the message (`aui-state` operations, applied).
    pub state: Option<Value>,
    /// Annotations to append.
    pub annotations: Option<Vec<Value>>,
    /// Data entries to append.
    pub data: Option<Vec<Value>>,
    /// The steps taken so far.
    pub steps: Option<Vec<ThreadStep>>,
    /// Timing information for the run.
    pub timing: Option<Value>,
    /// Keys to merge into the message's custom metadata.
    pub custom: Option<JsonObject>,
}

/// One cumulative update from an adapter (`ChatModelRunResult`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChatModelRunResult {
    /// The content so far; each result replaces the previous one's.
    pub content: Option<Vec<AssistantMessagePart>>,
    /// The message status, if the adapter sets one; `None` lets the runtime decide.
    pub status: Option<MessageStatus>,
    /// Metadata updates carried by this result.
    pub metadata: Option<RunResultMetadata>,
}

impl ChatModelRunResult {
    /// The content so far, with no status change.
    pub fn content(content: Vec<AssistantMessagePart>) -> Self {
        Self {
            content: Some(content),
            ..Self::default()
        }
    }
    /// A single text part.
    pub fn text(text: impl Into<String>) -> Self {
        Self::content(vec![AssistantMessagePart::text(text)])
    }
    /// Set the message status.
    pub fn with_status(mut self, status: MessageStatus) -> Self {
        self.status = Some(status);
        self
    }
}

/// Why an adapter stopped.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum RunError {
    /// The run was cancelled; the message becomes `incomplete/cancelled`.
    Aborted,
    /// The model or transport failed; the message becomes `incomplete/error`
    /// with `error`, and the runtime returns this error from the run future.
    Failed {
        /// What went wrong, for display.
        message: String,
        /// Structured error detail stored with the message, if any.
        error: Option<Value>,
    },
}

impl RunError {
    /// A failure with a message and no structured detail.
    pub fn failed(message: impl Into<String>) -> Self {
        Self::Failed {
            message: message.into(),
            error: None,
        }
    }
    /// The value stored as the message status's `error`.
    pub fn status_error(&self) -> Option<Value> {
        match self {
            Self::Aborted => None,
            Self::Failed { message, error } => Some(
                error
                    .clone()
                    .unwrap_or_else(|| Value::String(message.clone())),
            ),
        }
    }
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Aborted => f.write_str("run aborted"),
            Self::Failed { message, .. } => f.write_str(message),
        }
    }
}
impl std::error::Error for RunError {}

/// The stream an adapter returns. It is polled on the runtime's thread.
pub type RunStream = Pin<Box<dyn Stream<Item = Result<ChatModelRunResult, RunError>>>>;

/// A model backend (`ChatModelAdapter`).
pub trait ChatModelAdapter {
    /// Start one step and return the stream of cumulative results.
    fn run(&self, options: ChatModelRunOptions) -> RunStream;
}

impl<F> ChatModelAdapter for F
where
    F: Fn(ChatModelRunOptions) -> RunStream,
{
    fn run(&self, options: ChatModelRunOptions) -> RunStream {
        self(options)
    }
}

/// A stream with the single result of `future` (a `Promise`-returning adapter).
pub fn once<F>(future: F) -> RunStream
where
    F: Future<Output = Result<ChatModelRunResult, RunError>> + 'static,
{
    Box::pin(Once {
        future: Some(Box::pin(future)),
    })
}

/// A stream of ready results, for scripted adapters and tests.
pub fn from_results(
    results: impl IntoIterator<Item = Result<ChatModelRunResult, RunError>>,
) -> RunStream {
    Box::pin(Iter {
        items: results.into_iter().collect::<Vec<_>>().into_iter(),
    })
}

type ResultFuture = Pin<Box<dyn Future<Output = Result<ChatModelRunResult, RunError>>>>;

struct Once {
    future: Option<ResultFuture>,
}

impl Stream for Once {
    type Item = Result<ChatModelRunResult, RunError>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let Some(future) = self.future.as_mut() else {
            return Poll::Ready(None);
        };
        match future.as_mut().poll(cx) {
            Poll::Ready(result) => {
                self.future = None;
                Poll::Ready(Some(result))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

struct Iter {
    items: std::vec::IntoIter<Result<ChatModelRunResult, RunError>>,
}

impl Stream for Iter {
    type Item = Result<ChatModelRunResult, RunError>;
    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(self.items.next())
    }
}
