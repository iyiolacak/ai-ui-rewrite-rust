//! Composers, ported from `default-thread-composer-runtime-core.ts`,
//! `default-edit-composer-runtime-core.ts` and the parts of
//! `base-composer-runtime-core.ts` that do not involve attachment adapters
//! or dictation.
//!
//! The thread composer drafts the next message; sending appends it after the
//! last message and clears the draft. An edit composer drafts a replacement
//! for one message; sending appends a sibling (a new branch) and ends the edit.

use crate::runtime::{ComposerHost, RunFuture, RuntimeError};
use crate::subscribe::{Subscribers, Subscription};
use crate::types::{
    AppendContent, AppendMessage, AssistantMessagePart, CompleteAttachment, JsonObject, MessageId,
    MessageRole, RunConfig, TextPart, ThreadMessage, UserMessagePart,
};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

#[derive(Default)]
struct Draft {
    text: String,
    role: Option<MessageRole>,
    attachments: Vec<CompleteAttachment>,
    run_config: RunConfig,
}

enum Mode {
    Thread,
    Edit {
        message_id: MessageId,
        parent_id: Option<MessageId>,
        /// Non-text content of the edited message, kept in the replacement.
        user_parts: Vec<UserMessagePart>,
        assistant_parts: Vec<AssistantMessagePart>,
        custom: JsonObject,
        ended: Cell<bool>,
    },
}

struct Inner {
    runtime: Weak<dyn ComposerHost>,
    mode: Mode,
    draft: RefCell<Draft>,
    subscribers: Rc<Subscribers>,
}

/// A message draft (`ComposerRuntime`). Clones are handles to one composer.
#[derive(Clone)]
pub struct Composer {
    inner: Rc<Inner>,
}

impl Composer {
    pub(crate) fn thread(runtime: Weak<dyn ComposerHost>) -> Self {
        Self::new(runtime, Mode::Thread, Draft::default())
    }

    pub(crate) fn edit(
        runtime: Weak<dyn ComposerHost>,
        parent_id: Option<MessageId>,
        message: &ThreadMessage,
    ) -> Self {
        let (role, attachments, user_parts, assistant_parts, custom) = match message {
            ThreadMessage::User(m) => (
                MessageRole::User,
                m.attachments.clone(),
                m.content
                    .iter()
                    .filter(|p| !matches!(p, UserMessagePart::Text(_)))
                    .cloned()
                    .collect(),
                Vec::new(),
                m.metadata.custom.clone(),
            ),
            ThreadMessage::Assistant(m) => (
                MessageRole::Assistant,
                Vec::new(),
                Vec::new(),
                m.content
                    .iter()
                    .filter(|p| !matches!(p, AssistantMessagePart::Text(_)))
                    .cloned()
                    .collect(),
                m.metadata.custom.clone(),
            ),
            ThreadMessage::System(m) => (
                MessageRole::System,
                Vec::new(),
                Vec::new(),
                Vec::new(),
                m.metadata.custom.clone(),
            ),
        };
        let draft = Draft {
            text: message.text(),
            role: Some(role),
            attachments,
            run_config: RunConfig::default(),
        };
        let mode = Mode::Edit {
            message_id: message.id().clone(),
            parent_id,
            user_parts,
            assistant_parts,
            custom,
            ended: Cell::new(false),
        };
        Self::new(runtime, mode, draft)
    }

    fn new(runtime: Weak<dyn ComposerHost>, mode: Mode, draft: Draft) -> Self {
        Self {
            inner: Rc::new(Inner {
                runtime,
                mode,
                draft: RefCell::new(draft),
                subscribers: Rc::default(),
            }),
        }
    }

    pub(crate) fn notify(&self) {
        self.inner.subscribers.notify();
    }
    fn changed(&self) {
        self.notify();
    }

    /// Called after draft changes and after runtime changes that affect
    /// `can_send`/`can_cancel`. Drop the guard to unsubscribe.
    pub fn subscribe(&self, callback: impl Fn() + 'static) -> Subscription {
        self.inner.subscribers.subscribe(callback)
    }

    /// Whether this composer edits an existing message.
    pub fn is_editing(&self) -> bool {
        matches!(self.inner.mode, Mode::Edit { .. })
    }
    /// The message being edited, for an edit composer.
    pub fn source_id(&self) -> Option<&MessageId> {
        match &self.inner.mode {
            Mode::Edit { message_id, .. } => Some(message_id),
            Mode::Thread => None,
        }
    }

    /// The text typed so far.
    pub fn text(&self) -> String {
        self.inner.draft.borrow().text.clone()
    }
    /// Replace the text.
    pub fn set_text(&self, text: impl Into<String>) {
        let text = text.into();
        {
            let mut draft = self.inner.draft.borrow_mut();
            if draft.text == text {
                return;
            }
            draft.text = text;
        }
        self.changed();
    }
    /// User by default.
    pub fn role(&self) -> MessageRole {
        self.inner.draft.borrow().role.unwrap_or(MessageRole::User)
    }
    /// Set the role of the message to send.
    pub fn set_role(&self, role: MessageRole) {
        self.inner.draft.borrow_mut().role = Some(role);
        self.changed();
    }
    /// The run configuration sent with the message.
    pub fn run_config(&self) -> RunConfig {
        self.inner.draft.borrow().run_config.clone()
    }
    /// Replace the run configuration.
    pub fn set_run_config(&self, run_config: RunConfig) {
        self.inner.draft.borrow_mut().run_config = run_config;
        self.changed();
    }

    /// The attachments added so far.
    pub fn attachments(&self) -> Vec<CompleteAttachment> {
        self.inner.draft.borrow().attachments.clone()
    }
    /// Add an attachment that is ready to send. An attachment with the same id
    /// replaces the earlier one.
    pub fn add_attachment(&self, attachment: CompleteAttachment) {
        {
            let mut draft = self.inner.draft.borrow_mut();
            draft.attachments.retain(|a| a.id != attachment.id);
            draft.attachments.push(attachment);
        }
        self.changed();
    }
    /// Remove an attachment by id; returns whether there was one.
    pub fn remove_attachment(&self, id: &str) -> bool {
        let removed = {
            let mut draft = self.inner.draft.borrow_mut();
            let before = draft.attachments.len();
            draft.attachments.retain(|a| a.id != id);
            draft.attachments.len() != before
        };
        if removed {
            self.changed();
        }
        removed
    }

    /// No text (ignoring whitespace) and no attachments.
    pub fn is_empty(&self) -> bool {
        let draft = self.inner.draft.borrow();
        draft.text.trim().is_empty() && draft.attachments.is_empty()
    }

    /// Whether sending would do something: the draft is not empty and the runtime is still there.
    pub fn can_send(&self) -> bool {
        if self.is_empty() || self.inner.runtime.upgrade().is_none() {
            return false;
        }
        match &self.inner.mode {
            Mode::Thread => true,
            Mode::Edit { ended, .. } => !ended.get(),
        }
    }

    /// Thread composer: a run is in progress. Edit composer: always, since
    /// cancelling ends the edit.
    pub fn can_cancel(&self) -> bool {
        match &self.inner.mode {
            Mode::Thread => self
                .inner
                .runtime
                .upgrade()
                .is_some_and(|host| host.is_running()),
            Mode::Edit { ended, .. } => !ended.get(),
        }
    }

    /// Send the draft. Returns the run to spawn, if the message starts one,
    /// or `None` when there was nothing to send. On error the draft is kept.
    pub fn send(&self) -> Result<Option<RunFuture>, RuntimeError> {
        if !self.can_send() {
            return Ok(None);
        }
        let Some(host) = self.inner.runtime.upgrade() else {
            return Ok(None);
        };
        let draft = std::mem::take(&mut *self.inner.draft.borrow_mut());
        let message = self.build(&*host, &draft);
        match host.append(message) {
            Ok(run) => {
                if let Mode::Thread = self.inner.mode {
                    // Keep the role and run configuration for the next message.
                    let mut next = self.inner.draft.borrow_mut();
                    next.role = draft.role;
                    next.run_config = draft.run_config;
                }
                self.end_edit(&*host);
                self.changed();
                Ok(Some(run))
            }
            Err(error) => {
                *self.inner.draft.borrow_mut() = draft;
                Err(error)
            }
        }
    }

    fn build(&self, host: &dyn ComposerHost, draft: &Draft) -> AppendMessage {
        let text = (!draft.text.is_empty()).then(|| TextPart::new(draft.text.clone()));
        let (parent_id, source_id, user_parts, assistant_parts, custom) = match &self.inner.mode {
            Mode::Thread => (
                host.last_message_id(),
                None,
                &[][..],
                &[][..],
                JsonObject::new(),
            ),
            Mode::Edit {
                message_id,
                parent_id,
                user_parts,
                assistant_parts,
                custom,
                ..
            } => (
                parent_id.clone(),
                Some(message_id.clone()),
                &user_parts[..],
                &assistant_parts[..],
                custom.clone(),
            ),
        };
        let content = match draft.role.unwrap_or(MessageRole::User) {
            MessageRole::User => AppendContent::User {
                content: text
                    .into_iter()
                    .map(UserMessagePart::Text)
                    .chain(user_parts.iter().cloned())
                    .collect(),
                attachments: draft.attachments.clone(),
                custom,
            },
            MessageRole::Assistant => AppendContent::Assistant {
                content: text
                    .into_iter()
                    .map(AssistantMessagePart::Text)
                    .chain(assistant_parts.iter().cloned())
                    .collect(),
                custom,
            },
            MessageRole::System => AppendContent::System {
                text: draft.text.clone(),
            },
        };
        AppendMessage {
            parent_id,
            source_id,
            run_config: draft.run_config.clone(),
            start_run: None,
            content,
        }
    }

    /// Thread composer: cancel the run in progress. Edit composer: discard the
    /// edit without changing the thread.
    pub fn cancel(&self) {
        let Some(host) = self.inner.runtime.upgrade() else {
            return;
        };
        match &self.inner.mode {
            Mode::Thread => host.cancel_run(),
            Mode::Edit { .. } => {
                self.end_edit(&*host);
                self.changed();
            }
        }
    }

    fn end_edit(&self, host: &dyn ComposerHost) {
        if let Mode::Edit {
            message_id, ended, ..
        } = &self.inner.mode
        {
            if !ended.replace(true) {
                host.end_edit(message_id);
            }
        }
    }
}
