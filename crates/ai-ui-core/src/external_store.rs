//! The external-store runtime, after upstream's `useExternalStoreRuntime`.
//!
//! The app owns the messages (and the model, transport and persistence); this
//! runtime adapts them to the [`ThreadRuntime`] interface. The app pushes its
//! current state with [`ExternalStoreRuntime::set_messages`] (or
//! [`set_repository`](ExternalStoreRuntime::set_repository) for branching
//! histories) and `set_running`; user actions arrive as [`ExternalStore`]
//! callbacks, which the app answers by updating its messages. Nothing is
//! inserted optimistically.
//!
//! Messages the app re-sends unchanged keep their `Arc` identity, so a UI can
//! find what changed by pointer comparison without comparing content.

use crate::repository::{ExportedMessage, ExportedRepository, MessageEntry, MessageRepository};
use crate::runtime::{
    ComposerHost, RunFuture, RuntimeCapabilities, RuntimeError, StartRunConfig, ThreadRuntime,
    ToolResult,
};
use crate::subscribe::{Subscribers, Subscription};
use crate::types::{AppendMessage, MessageId, SubmittedFeedback, ThreadMessage};
use crate::{adapter::RunError, Composer};
use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::rc::{Rc, Weak};
use std::sync::Arc;

/// An app callback's outcome. An error message becomes a [`RunError`].
pub type StoreFuture = Pin<Box<dyn Future<Output = Result<(), String>>>>;

/// The app side of an external-store thread (`ExternalStoreAdapter`'s
/// `onNew`, `onEdit`, `onReload`, ...). Optional callbacks return `None` when
/// unsupported; declare what is supported in [`capabilities`](Self::capabilities).
pub trait ExternalStore {
    /// A new message was sent; the app answers it by updating its messages.
    fn on_new(&self, message: AppendMessage) -> StoreFuture;

    /// A message was edited. `None` falls back to [`on_new`](Self::on_new).
    fn on_edit(&self, _message: AppendMessage) -> Option<StoreFuture> {
        None
    }
    /// Regenerate the answer to `config.parent_id`.
    fn on_reload(&self, _config: StartRunConfig) -> Option<StoreFuture> {
        None
    }
    /// Stop the run in progress.
    fn on_cancel(&self) {}
    /// A result was supplied for a tool call. `None` if the store does not support it.
    fn on_add_tool_result(&self, _result: ToolResult) -> Option<StoreFuture> {
        None
    }
    /// The user answered an approval request. `None` if the store does not support it.
    fn on_respond_to_tool_approval(
        &self,
        _message_id: &MessageId,
        _approval_id: &str,
        _approved: bool,
        _reason: Option<String>,
    ) -> Option<StoreFuture> {
        None
    }
    /// The user gave, changed or cleared feedback on a message.
    fn on_feedback(&self, _message_id: &MessageId, _feedback: Option<&SubmittedFeedback>) {}
    /// The user switched branch; `head_id` is the new head.
    fn on_branch_change(&self, _head_id: Option<&MessageId>) {}

    /// Which actions the UI offers. Branch switching and copy work out of the
    /// box; the rest need the matching callbacks.
    fn capabilities(&self) -> RuntimeCapabilities {
        RuntimeCapabilities {
            switch_to_branch: true,
            edit: false,
            reload: false,
            cancel: false,
            copy: true,
            feedback: false,
            answer_tool_call: false,
        }
    }
}

struct State {
    repository: MessageRepository,
    running: bool,
    disabled: bool,
}

struct Inner {
    me: Weak<Inner>,
    store: Rc<dyn ExternalStore>,
    state: RefCell<State>,
    subscribers: Rc<Subscribers>,
    thread_composer: RefCell<Option<Composer>>,
    edits: RefCell<HashMap<MessageId, Composer>>,
}

impl Inner {
    fn notify(&self) {
        self.subscribers.notify();
        let composers: Vec<Composer> = self
            .thread_composer
            .borrow()
            .iter()
            .cloned()
            .chain(self.edits.borrow().values().cloned())
            .collect();
        for composer in composers {
            composer.notify();
        }
    }
}

/// A thread whose messages the app owns.
#[derive(Clone)]
pub struct ExternalStoreRuntime {
    inner: Rc<Inner>,
}

fn into_run(future: StoreFuture) -> RunFuture {
    Box::pin(async move { future.await.map_err(RunError::failed) })
}

impl ExternalStoreRuntime {
    /// A runtime over `store`, with no messages yet.
    pub fn new(store: impl ExternalStore + 'static) -> Self {
        Self {
            inner: Rc::new_cyclic(|me| Inner {
                me: me.clone(),
                store: Rc::new(store),
                state: RefCell::new(State {
                    repository: MessageRepository::new(),
                    running: false,
                    disabled: false,
                }),
                subscribers: Rc::default(),
                thread_composer: RefCell::new(None),
                edits: RefCell::new(HashMap::new()),
            }),
        }
    }

    /// Called after every change. Drop the guard to unsubscribe.
    pub fn subscribe(&self, callback: impl Fn() + 'static) -> Subscription {
        self.inner.subscribers.subscribe(callback)
    }

    /// The app's current messages as a linear history (`messages`).
    /// Messages equal to what is already shown keep their `Arc`.
    pub fn set_messages<M: Into<Arc<ThreadMessage>>>(&self, messages: impl IntoIterator<Item = M>) {
        let mut parent: Option<MessageId> = None;
        let items = messages
            .into_iter()
            .map(|message| {
                let message: Arc<ThreadMessage> = message.into();
                let item = ExportedMessage {
                    message: message.clone(),
                    parent_id: parent.take(),
                    run_config: None,
                };
                parent = Some(message.id().clone());
                item
            })
            .collect();
        self.set_repository(ExportedRepository {
            head_id: None,
            messages: items,
        });
    }

    /// The app's messages with branches (`messageRepository`).
    pub fn set_repository(&self, mut exported: ExportedRepository) {
        {
            let mut state = self.inner.state.borrow_mut();
            // Reuse the Arc of every message that did not change.
            for item in &mut exported.messages {
                if let Ok(existing) = state.repository.get(item.message.id()) {
                    if Arc::ptr_eq(&existing.message, &item.message)
                        || *existing.message == *item.message
                    {
                        item.message = existing.message;
                    }
                }
            }
            // A linear history keeps the user's head only if it is still valid.
            let keep_head = exported
                .head_id
                .is_none()
                .then(|| state.repository.head_id().cloned());
            state.repository.clear();
            let _ = state.repository.import(exported);
            if let Some(Some(head)) = keep_head {
                if state.repository.contains(&head) {
                    let _ = state.repository.switch_to_branch(&head);
                }
            }
        }
        self.inner.notify();
    }

    /// Tell the runtime whether the app is generating a response.
    pub fn set_running(&self, running: bool) {
        let changed = {
            let mut state = self.inner.state.borrow_mut();
            std::mem::replace(&mut state.running, running) != running
        };
        if changed {
            self.inner.notify();
        }
    }

    /// Disable sending (for example while the app is offline).
    pub fn set_disabled(&self, disabled: bool) {
        let changed = {
            let mut state = self.inner.state.borrow_mut();
            std::mem::replace(&mut state.disabled, disabled) != disabled
        };
        if changed {
            self.inner.notify();
        }
    }

    fn handle(inner: &Rc<Inner>) -> Self {
        Self {
            inner: inner.clone(),
        }
    }
}

impl ComposerHost for Inner {
    fn last_message_id(&self) -> Option<MessageId> {
        self.state
            .borrow()
            .repository
            .messages()
            .last()
            .map(|m| m.id().clone())
    }
    fn append(&self, message: AppendMessage) -> Result<RunFuture, RuntimeError> {
        let inner = self.me.upgrade().ok_or(RuntimeError::Disposed)?;
        ExternalStoreRuntime::handle(&inner).append(message)
    }
    fn cancel_run(&self) {
        self.store.on_cancel();
    }
    fn is_running(&self) -> bool {
        self.state.borrow().running
    }
    fn end_edit(&self, id: &MessageId) {
        self.edits.borrow_mut().remove(id);
        self.notify();
    }
}

impl ThreadRuntime for ExternalStoreRuntime {
    fn messages(&self) -> Arc<[Arc<ThreadMessage>]> {
        self.inner.state.borrow().repository.messages()
    }
    fn message(&self, id: &MessageId) -> Result<MessageEntry, RuntimeError> {
        Ok(self.inner.state.borrow().repository.get(id)?)
    }
    fn is_running(&self) -> bool {
        self.inner.state.borrow().running
    }
    fn capabilities(&self) -> RuntimeCapabilities {
        self.inner.store.capabilities()
    }
    fn branches(&self, id: &MessageId) -> Result<Vec<MessageId>, RuntimeError> {
        Ok(self.inner.state.borrow().repository.branches(id)?.to_vec())
    }
    fn branch_position(&self, id: &MessageId) -> Result<(usize, usize), RuntimeError> {
        Ok(self.inner.state.borrow().repository.branch_position(id)?)
    }
    fn message_at(&self, index: usize) -> Option<Arc<ThreadMessage>> {
        self.inner.state.borrow().repository.message_at(index)
    }
    fn len_selected(&self) -> usize {
        self.inner.state.borrow().repository.len_selected()
    }
    fn begin_overlay(&self) {
        self.inner.state.borrow().repository.begin_overlay()
    }
    fn end_overlay(&self) {
        self.inner.state.borrow().repository.end_overlay()
    }
    fn switch_to_branch(&self, id: &MessageId) -> Result<(), RuntimeError> {
        let head = {
            let mut state = self.inner.state.borrow_mut();
            state.repository.switch_to_branch(id)?;
            state.repository.head_id().cloned()
        };
        self.inner.notify();
        self.inner.store.on_branch_change(head.as_ref());
        Ok(())
    }
    fn append(&self, message: AppendMessage) -> Result<RunFuture, RuntimeError> {
        if self.inner.state.borrow().disabled {
            return Err(RuntimeError::Unsupported("sending while disabled"));
        }
        let editing = message.source_id.is_some();
        let edit = editing
            .then(|| self.inner.store.on_edit(message.clone()))
            .flatten();
        Ok(into_run(
            edit.unwrap_or_else(|| self.inner.store.on_new(message)),
        ))
    }
    fn reload(&self, config: StartRunConfig) -> Result<RunFuture, RuntimeError> {
        self.inner
            .store
            .on_reload(config)
            .map(into_run)
            .ok_or(RuntimeError::Unsupported("reload"))
    }
    fn cancel_run(&self) {
        self.inner.store.on_cancel();
    }
    fn add_tool_result(&self, result: ToolResult) -> Result<Option<RunFuture>, RuntimeError> {
        self.inner
            .store
            .on_add_tool_result(result)
            .map(|f| Some(into_run(f)))
            .ok_or(RuntimeError::Unsupported("tool results"))
    }
    fn respond_to_tool_approval(
        &self,
        message_id: &MessageId,
        approval_id: &str,
        approved: bool,
        reason: Option<String>,
    ) -> Result<Option<RunFuture>, RuntimeError> {
        self.inner
            .store
            .on_respond_to_tool_approval(message_id, approval_id, approved, reason)
            .map(|f| Some(into_run(f)))
            .ok_or(RuntimeError::Unsupported("tool approvals"))
    }
    fn submit_feedback(
        &self,
        id: &MessageId,
        feedback: Option<SubmittedFeedback>,
    ) -> Result<(), RuntimeError> {
        if !self.inner.store.capabilities().feedback {
            return Err(RuntimeError::Unsupported("feedback"));
        }
        self.message(id)?;
        self.inner.store.on_feedback(id, feedback.as_ref());
        Ok(())
    }
    fn composer(&self) -> Composer {
        self.inner
            .thread_composer
            .borrow_mut()
            .get_or_insert_with(|| {
                Composer::thread(Rc::downgrade(&self.inner) as Weak<dyn ComposerHost>)
            })
            .clone()
    }
    fn begin_edit(&self, id: &MessageId) -> Result<Composer, RuntimeError> {
        if self.inner.edits.borrow().contains_key(id) {
            return Err(RuntimeError::EditInProgress(id.clone()));
        }
        let entry = ThreadRuntime::message(self, id)?;
        let composer = Composer::edit(
            Rc::downgrade(&self.inner) as Weak<dyn ComposerHost>,
            entry.parent_id,
            &entry.message,
        );
        self.inner
            .edits
            .borrow_mut()
            .insert(id.clone(), composer.clone());
        self.inner.notify();
        Ok(composer)
    }
    fn edit_composer(&self, id: &MessageId) -> Option<Composer> {
        self.inner.edits.borrow().get(id).cloned()
    }
    fn subscribe_dyn(&self, callback: Box<dyn Fn()>) -> Subscription {
        self.inner.subscribers.subscribe(callback)
    }
    fn export(&self) -> ExportedRepository {
        self.inner.state.borrow().repository.export()
    }
    fn import(&self, exported: ExportedRepository) -> Result<(), RuntimeError> {
        self.set_repository(exported);
        Ok(())
    }
}
