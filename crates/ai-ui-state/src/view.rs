//! [`ThreadView`]: the state a UI renders and the actions it calls, over any
//! [`ThreadRuntime`] (or a [`ThreadList`]'s main thread).
//!
//! The view derives [`ThreadState`] lazily and caches per row, so an update
//! that touches one message rebuilds one row. It also owns UI concerns that
//! do not belong in the runtime: smooth streaming, the "copied" confirmation
//! and which sections are expanded. Time comes from the caller: drive
//! [`ThreadView::tick`] once per frame while [`ThreadView::needs_frame`].

use crate::model::{
    ApprovalDecision, ApprovalState, MessageState, PartKind, PartState, ThreadState, ToolCallState,
    ToolPresentation, ToolStatus,
};
use ai_ui_core::{
    AssistantMessagePart, Composer, FeedbackKind, IncompleteReason, MessageId, MessageRole,
    MessageStatus, PartStatus, RunFuture, RuntimeCapabilities, RuntimeError, SmoothOptions,
    SmoothText, SourcePart, SubmittedFeedback, Subscribers, Subscription, ThreadList,
    ThreadMessage, ThreadRuntime, ToolCallPart, ToolResult, UserMessagePart,
};
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::{Rc, Weak};
use std::sync::Arc;

/// How long the "copied" confirmation stays, in milliseconds.
const COPIED_MS: f64 = 2000.0;

/// Maximum number of projected revisions retained for lagging UI consumers.
const VIEW_CHANGE_HISTORY: usize = 64;

/// A consumer's position in one [`ThreadView`]'s projected row changes.
///
/// Keep a separate cursor for each consumer and pass it to
/// [`ThreadView::state_changes_since`]. Clones of a view share a cursor domain;
/// an independently constructed view does not. A cursor does not keep the view
/// or any message snapshot alive.
#[derive(Clone)]
pub struct ViewCursor {
    view: Weak<Inner>,
    version: u64,
}

impl std::fmt::Debug for ViewCursor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ViewCursor")
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

/// A state snapshot and the rows a consumer must reconcile to display it.
#[derive(Clone, Debug)]
pub struct ViewChanges {
    /// The complete current state. Retained snapshots remain immutable.
    pub state: Arc<ThreadState>,
    /// Save this cursor after reconciling the snapshot.
    pub cursor: ViewCursor,
    /// Sorted, unique indices into [`state`](Self::state)'s messages.
    ///
    /// `Some([])` means no rows changed. `None` requires a full reconciliation:
    /// this is the first read, the cursor belongs to another view or expired,
    /// or projection needed its structural/global/full-snapshot fallback.
    /// Always read the snapshot's thread-level fields, regardless of this list.
    pub indices: Option<Vec<usize>>,
}

/// Output revisions, independent of the runtime's input-change journal. Plain
/// `state()` reads may project several revisions before a UI asks for changes.
#[derive(Default)]
struct ViewChangeJournal {
    version: u64,
    entries: VecDeque<Vec<usize>>,
}

impl ViewChangeJournal {
    fn record(&mut self, indices: Option<Vec<usize>>) {
        self.version = self.version.wrapping_add(1);
        match indices {
            Some(indices) => {
                if self.entries.len() == VIEW_CHANGE_HISTORY {
                    self.entries.pop_front();
                }
                self.entries.push_back(indices);
            }
            // A full reconciliation is a barrier: earlier row indices may no
            // longer name the same messages after a rebind or branch change.
            None => self.entries.clear(),
        }
    }

    fn since(&self, version: u64) -> Option<Vec<usize>> {
        let distance = self.version.wrapping_sub(version);
        if distance > self.entries.len() as u64 {
            return None;
        }
        let mut indices = Vec::new();
        for changed in self
            .entries
            .iter()
            .skip(self.entries.len() - distance as usize)
        {
            indices.extend_from_slice(changed);
        }
        indices.sort_unstable();
        indices.dedup();
        Some(indices)
    }
}

/// How a view presents streaming content.
#[derive(Clone, Copy, Debug)]
pub struct ViewOptions {
    /// Reveal streaming text at a steady pace; `None` shows text as it arrives.
    pub smooth: Option<SmoothOptions>,
}

impl Default for ViewOptions {
    fn default() -> Self {
        Self {
            smooth: Some(SmoothOptions::default()),
        }
    }
}

type Presenter = Rc<dyn Fn(&ToolCallPart) -> ToolPresentation>;
type PartKey = (MessageId, usize);

#[derive(Default)]
struct Ui {
    smooth: HashMap<PartKey, SmoothText>,
    expanded: HashMap<PartKey, bool>,
    /// Message and the time its confirmation ends.
    copied: Option<(MessageId, f64)>,
    /// Messages whose UI-local state changed since the rows were last built.
    stale: HashSet<MessageId>,
}

impl Ui {
    fn touch(&mut self, id: &MessageId) {
        self.stale.insert(id.clone());
    }
}

#[derive(Clone, PartialEq, Eq)]
struct RowKey {
    message: usize,
    branch: (usize, usize),
    is_last: bool,
    running: bool,
    capabilities: RuntimeCapabilities,
}

struct Inner {
    me: Weak<Inner>,
    runtime: RefCell<Rc<dyn ThreadRuntime>>,
    runtime_subscription: RefCell<Option<Subscription>>,
    options: ViewOptions,
    ui: RefCell<Ui>,
    presenters: RefCell<HashMap<String, Presenter>>,
    subscribers: Rc<Subscribers>,
    dirty: Cell<bool>,
    cached: RefCell<Option<Arc<ThreadState>>>,
    rows: RefCell<Vec<(RowKey, Arc<MessageState>)>>,
    /// The runtime's structure version when `rows` were built: while it is the
    /// same, branch positions are reused instead of looked up per message.
    rows_version: Cell<Option<u64>>,
    /// Independent journal cursor for this view. Never shared with another
    /// view, or carried across a runtime rebind.
    messages_version: Cell<Option<u64>>,
    /// Projected changes for independent UI consumers, including UI-local state.
    view_changes: RefCell<ViewChangeJournal>,
    /// Set for a view over a thread list.
    list: RefCell<Option<(ThreadList, String, Subscription)>>,
}

/// A thread as a UI sees it: state to render and actions to call, over any runtime.
#[derive(Clone)]
pub struct ThreadView {
    inner: Rc<Inner>,
}

impl Inner {
    fn invalidate_and_notify(&self) {
        self.dirty.set(true);
        self.subscribers.notify();
    }

    fn runtime(&self) -> Rc<dyn ThreadRuntime> {
        self.runtime.borrow().clone()
    }

    fn bind(&self, runtime: Rc<dyn ThreadRuntime>) {
        let weak = self.me.clone();
        let subscription = runtime.subscribe_dyn(Box::new(move || {
            if let Some(inner) = weak.upgrade() {
                inner.invalidate_and_notify();
            }
        }));
        *self.runtime.borrow_mut() = runtime;
        *self.runtime_subscription.borrow_mut() = Some(subscription);
    }
}

impl ThreadView {
    /// A view over `runtime`.
    pub fn new(runtime: Rc<dyn ThreadRuntime>, options: ViewOptions) -> Self {
        let view = Self {
            inner: Rc::new_cyclic(|me| Inner {
                me: me.clone(),
                runtime: RefCell::new(runtime.clone()),
                runtime_subscription: RefCell::new(None),
                options,
                ui: RefCell::new(Ui::default()),
                presenters: RefCell::new(HashMap::new()),
                subscribers: Rc::default(),
                dirty: Cell::new(true),
                cached: RefCell::new(None),
                rows: RefCell::new(Vec::new()),
                rows_version: Cell::new(None),
                messages_version: Cell::new(None),
                view_changes: RefCell::new(ViewChangeJournal::default()),
                list: RefCell::new(None),
            }),
        };
        view.inner.bind(runtime);
        view
    }

    /// A view that follows a thread list's main thread, switching runtimes
    /// when the list does.
    pub fn for_list(list: &ThreadList, options: ViewOptions) -> Self {
        let view = Self::new(list.main(), options);
        let weak = Rc::downgrade(&view.inner);
        let subscription = list.subscribe({
            let list = list.clone();
            move || {
                let Some(inner) = weak.upgrade() else { return };
                let current = list.main_id();
                let changed = inner
                    .list
                    .borrow()
                    .as_ref()
                    .is_some_and(|(_, bound, _)| *bound != current);
                if changed {
                    if let Some(entry) = inner.list.borrow_mut().as_mut() {
                        entry.1 = current;
                    }
                    ThreadView {
                        inner: inner.clone(),
                    }
                    .rebind(list.main());
                } else {
                    inner.invalidate_and_notify();
                }
            }
        });
        *view.inner.list.borrow_mut() = Some((list.clone(), list.main_id(), subscription));
        view
    }

    /// Follow a different runtime. UI-local state (smooth text, expanded
    /// sections) belongs to the old thread and is dropped.
    pub fn rebind(&self, runtime: Rc<dyn ThreadRuntime>) {
        *self.inner.ui.borrow_mut() = Ui::default();
        self.inner.rows.borrow_mut().clear();
        self.inner.rows_version.set(None);
        self.inner.messages_version.set(None);
        self.inner.bind(runtime);
        self.inner.invalidate_and_notify();
    }

    /// The runtime this view follows, for operations the view does not wrap.
    pub fn runtime(&self) -> Rc<dyn ThreadRuntime> {
        self.inner.runtime()
    }

    /// Called whenever [`state`](Self::state) may have changed. Drop the guard
    /// to unsubscribe.
    pub fn subscribe(&self, callback: impl Fn() + 'static) -> Subscription {
        self.inner.subscribers.subscribe(callback)
    }

    /// Replace how a tool's calls are shown.
    pub fn set_tool_presenter(
        &self,
        tool: impl Into<String>,
        presenter: impl Fn(&ToolCallPart) -> ToolPresentation + 'static,
    ) {
        self.inner
            .presenters
            .borrow_mut()
            .insert(tool.into(), Rc::new(presenter));
        self.inner.rows.borrow_mut().clear();
        self.inner.invalidate_and_notify();
    }

    // ------------------------------------------------------------ state

    /// The current state; cheap when nothing changed.
    pub fn state(&self) -> Arc<ThreadState> {
        if !self.inner.dirty.get() {
            if let Some(state) = self.inner.cached.borrow().clone() {
                return state;
            }
        }
        // When no one else holds the previous state (the usual case: a UI
        // reads it during a sync and lets go), update it in place: only rows
        // that changed are written, instead of building a new list and
        // dropping the old one, which cost two reference-count updates per
        // message of the thread on every streamed update.
        let mut cached = self.inner.cached.borrow_mut();
        if let Some(state) = cached.as_mut().and_then(Arc::get_mut) {
            self.build_into(state);
        } else {
            let mut state = ThreadState {
                messages: Vec::new(),
                is_empty: true,
                is_running: false,
                capabilities: self.inner.runtime().capabilities(),
            };
            self.build_into(&mut state);
            *cached = Some(Arc::new(state));
        }
        self.inner.dirty.set(false);
        cached.clone().expect("set above")
    }

    /// Read the current snapshot and all row changes since `cursor`.
    ///
    /// Pass `None` on a consumer's first read. A missing, foreign or expired
    /// cursor returns `indices: None`, as do projection fallbacks after a
    /// structural/global change, rebind, unsupported runtime journal, or
    /// retained output snapshot. Reconcile the whole snapshot in that case.
    /// Otherwise only the returned indices need updating. Keep thread-level
    /// fields in sync on every read, even when the index list is empty.
    ///
    /// Reads never consume changes: another consumer, [`state`](Self::state),
    /// and [`needs_frame`](Self::needs_frame) cannot advance your cursor. The
    /// journal retains at most 64 projected revisions and no message bodies.
    pub fn state_changes_since(&self, cursor: Option<&ViewCursor>) -> ViewChanges {
        let state = self.state();
        let journal = self.inner.view_changes.borrow();
        let indices = cursor
            .filter(|cursor| Weak::ptr_eq(&cursor.view, &self.inner.me))
            .and_then(|cursor| journal.since(cursor.version));
        ViewChanges {
            state,
            cursor: ViewCursor {
                view: self.inner.me.clone(),
                version: journal.version,
            },
            indices,
        }
    }

    /// Whether [`tick`](Self::tick) has work: text is being revealed or a
    /// confirmation is waiting to end.
    pub fn needs_frame(&self) -> bool {
        let _ = self.state();
        let ui = self.inner.ui.borrow();
        ui.copied.is_some() || ui.smooth.values().any(SmoothText::is_animating)
    }

    /// Advance animations to `now_ms` (any monotonic clock, in milliseconds).
    /// Returns whether the state changed; subscribers are notified then.
    pub fn tick(&self, now_ms: f64) -> bool {
        let changed = {
            let mut ui = self.inner.ui.borrow_mut();
            let mut changed: Vec<MessageId> = Vec::new();
            for (key, smooth) in ui.smooth.iter_mut() {
                if smooth.tick(now_ms) {
                    changed.push(key.0.clone());
                }
            }
            if let Some((id, until)) = ui.copied.clone() {
                if now_ms >= until {
                    ui.copied = None;
                    changed.push(id);
                }
            }
            for id in &changed {
                ui.touch(id);
            }
            !changed.is_empty()
        };
        if changed {
            self.inner.invalidate_and_notify();
        }
        changed
    }

    fn build_into(&self, state: &mut ThreadState) {
        let runtime = self.inner.runtime();
        let messages = runtime.messages();
        let running = runtime.is_running();
        let capabilities = runtime.capabilities();
        let last = messages.len().saturating_sub(1);
        let presenters = self.inner.presenters.borrow();
        let mut ui = self.inner.ui.borrow_mut();
        if self.inner.options.smooth.is_some() {
            self.sync_smooth(&mut ui, &messages, running);
        }
        let mut rows = self.inner.rows.borrow_mut();
        rows.truncate(messages.len());
        let version = runtime.structure_version();
        let same_shape = version.is_some() && version == self.inner.rows_version.get();
        self.inner.rows_version.set(version);
        let changes = runtime.message_changes_since(self.inner.messages_version.get());
        self.inner
            .messages_version
            .set(changes.as_ref().map(|changes| changes.version));
        let out = &mut state.messages;
        out.truncate(messages.len());
        // A uniquely held ThreadState can patch only affected rows. Retained
        // snapshots, structure/global changes, and runtimes without a complete
        // journal keep the full-scan path. Never infer a delta from just the
        // latest notification: several updates may precede a view read.
        let changed = if same_shape
            && rows.len() == messages.len()
            && out.len() == messages.len()
            && state.is_running == running
            && state.capabilities == capabilities
        {
            changes.and_then(|changes| changes.indices)
        } else {
            None
        };
        let changed = changed.map(|mut indices| {
            for id in &ui.stale {
                if let Ok(entry) = runtime.message(id) {
                    if messages
                        .get(entry.index)
                        .is_some_and(|message| message.id() == id)
                    {
                        indices.push(entry.index);
                    }
                }
            }
            indices.sort_unstable();
            indices.dedup();
            indices
        });
        let mut project = |index: usize| {
            let message = &messages[index];
            // A streamed update replaces a message in place, which cannot move
            // any branch; looking positions up for every message made each
            // update cost a hash lookup per message of the thread.
            let branch = match rows.get(index) {
                Some((key, row))
                    if same_shape
                        && (key.message == Arc::as_ptr(message) as usize
                            || row.id == *message.id()) =>
                {
                    key.branch
                }
                _ => runtime.branch_position(message.id()).unwrap_or((1, 1)),
            };
            let key = RowKey {
                message: Arc::as_ptr(message) as usize,
                branch,
                is_last: index == last,
                running,
                capabilities,
            };
            let stale = !ui.stale.is_empty() && ui.stale.contains(message.id());
            let row = match rows.get(index) {
                Some((existing, row)) if *existing == key && !stale => {
                    if out.get(index).is_some_and(|shown| Arc::ptr_eq(shown, row)) {
                        return;
                    }
                    row.clone()
                }
                _ => {
                    let row = Arc::new(self.message_state(
                        &ui,
                        &presenters,
                        message,
                        index,
                        branch,
                        &key,
                        capabilities,
                    ));
                    if index < rows.len() {
                        rows[index] = (key, row.clone());
                    } else {
                        rows.push((key, row.clone()));
                    }
                    row
                }
            };
            if index < out.len() {
                out[index] = row;
            } else {
                out.push(row);
            }
        };
        match &changed {
            Some(indices) => indices.iter().copied().for_each(&mut project),
            None => (0..messages.len()).for_each(&mut project),
        }
        ui.stale.clear();
        state.is_empty = state.messages.is_empty();
        state.is_running = running;
        state.capabilities = capabilities;
        self.inner.view_changes.borrow_mut().record(changed);
    }

    /// Point each streaming part's animator at its latest text. Only the last
    /// message can stream, so this looks at it alone; an animator ends once
    /// its part stops streaming and everything is revealed. Parts that were
    /// never streaming have no animator and show their text as it is.
    fn sync_smooth(&self, ui: &mut Ui, messages: &[Arc<ThreadMessage>], running: bool) {
        let options = self.inner.options.smooth.unwrap_or_default();
        let mut streaming_now: Vec<PartKey> = Vec::new();
        if let Some(assistant) = messages.last().and_then(|m| m.as_assistant()) {
            for (i, part) in assistant.content.iter().enumerate() {
                let (text, status) = match part {
                    AssistantMessagePart::Text(t) => (&t.text, &t.status),
                    AssistantMessagePart::Reasoning(r) => (&r.text, &r.status),
                    _ => continue,
                };
                let streaming = running && *status == Some(PartStatus::Running);
                let key = (assistant.id.clone(), i);
                match ui.smooth.get_mut(&key) {
                    Some(smooth) => smooth.set_target(text, streaming),
                    None if streaming => {
                        ui.smooth
                            .insert(key.clone(), SmoothText::new(text, true, options));
                    }
                    None => {}
                }
                if streaming {
                    streaming_now.push(key);
                }
            }
        }
        ui.smooth
            .retain(|key, smooth| smooth.is_animating() || streaming_now.contains(key));
    }

    #[allow(clippy::too_many_arguments)]
    fn message_state(
        &self,
        ui: &Ui,
        presenters: &HashMap<String, Presenter>,
        message: &Arc<ThreadMessage>,
        index: usize,
        branch: (usize, usize),
        key: &RowKey,
        capabilities: RuntimeCapabilities,
    ) -> MessageState {
        let id = message.id().clone();
        let running = key.running;
        let generating = running && key.is_last;
        let shown = |part_index: usize, raw: &str, streaming: bool| -> (String, bool) {
            match ui.smooth.get(&(id.clone(), part_index)) {
                Some(smooth) => (smooth.text().to_owned(), streaming || smooth.is_animating()),
                None => (raw.to_owned(), streaming),
            }
        };
        let mut parts = Vec::new();
        let mut status = None;
        let mut feedback = None;
        match &**message {
            ThreadMessage::User(user) => {
                for (i, part) in user.content.iter().enumerate() {
                    let kind = match part {
                        UserMessagePart::Text(t) => PartKind::Text {
                            text: t.text.clone(),
                            streaming: false,
                        },
                        UserMessagePart::Image(image) => PartKind::Image {
                            url: image.image.clone(),
                        },
                        UserMessagePart::File(file) => PartKind::File {
                            filename: file.filename.clone(),
                            mime_type: file.mime_type.clone(),
                        },
                        UserMessagePart::Data(data) => PartKind::Data {
                            name: data.name.clone(),
                        },
                        UserMessagePart::Audio(_) => PartKind::File {
                            filename: None,
                            mime_type: "audio".into(),
                        },
                    };
                    parts.push(PartState { index: i, kind });
                }
            }
            ThreadMessage::System(system) => {
                for (i, ai_ui_core::SystemMessagePart::Text(t)) in system.content.iter().enumerate()
                {
                    parts.push(PartState {
                        index: i,
                        kind: PartKind::Text {
                            text: t.text.clone(),
                            streaming: false,
                        },
                    });
                }
            }
            ThreadMessage::Assistant(assistant) => {
                status = Some(assistant.status.clone());
                feedback = assistant
                    .metadata
                    .submitted_feedback
                    .as_ref()
                    .map(|f| f.kind);
                for (i, part) in assistant.content.iter().enumerate() {
                    let part_streaming =
                        |s: &Option<PartStatus>| generating && *s == Some(PartStatus::Running);
                    let kind = match part {
                        AssistantMessagePart::Text(t) => {
                            let (text, streaming) = shown(i, &t.text, part_streaming(&t.status));
                            PartKind::Text { text, streaming }
                        }
                        AssistantMessagePart::Reasoning(r) => {
                            let (text, streaming) = shown(i, &r.text, part_streaming(&r.status));
                            let expanded = ui
                                .expanded
                                .get(&(id.clone(), i))
                                .copied()
                                .unwrap_or(streaming);
                            PartKind::Reasoning {
                                text,
                                streaming,
                                expanded,
                            }
                        }
                        AssistantMessagePart::ToolCall(call) => {
                            let expanded =
                                ui.expanded.get(&(id.clone(), i)).copied().unwrap_or(false);
                            let incomplete =
                                matches!(assistant.status, MessageStatus::Incomplete { .. });
                            PartKind::ToolCall(Box::new(tool_state(
                                call, expanded, incomplete, presenters,
                            )))
                        }
                        AssistantMessagePart::Source(SourcePart::Url { title, url, .. }) => {
                            PartKind::Source {
                                title: title.clone().unwrap_or_else(|| url.clone()),
                                url: Some(url.clone()),
                            }
                        }
                        AssistantMessagePart::Source(SourcePart::Document { title, .. }) => {
                            PartKind::Source {
                                title: title.clone(),
                                url: None,
                            }
                        }
                        AssistantMessagePart::Image(image) => PartKind::Image {
                            url: image.image.clone(),
                        },
                        AssistantMessagePart::File(file) => PartKind::File {
                            filename: file.filename.clone(),
                            mime_type: file.mime_type.clone(),
                        },
                        AssistantMessagePart::Data(data) => PartKind::Data {
                            name: data.name.clone(),
                        },
                        AssistantMessagePart::GenerativeUi(_) => PartKind::GenerativeUi,
                    };
                    parts.push(PartState { index: i, kind });
                }
            }
        }
        let text = message.text();
        let terminal = status.as_ref().is_some_and(MessageStatus::is_terminal);
        let role = message.role();
        MessageState {
            error: status.as_ref().and_then(error_text),
            is_copied: ui.copied.as_ref().is_some_and(|(copied, _)| *copied == id),
            can_edit: role == MessageRole::User && !running && capabilities.edit,
            can_reload: role == MessageRole::Assistant && !running && capabilities.reload,
            can_copy: capabilities.copy && !generating && !text.is_empty(),
            can_feedback: role == MessageRole::Assistant
                && capabilities.feedback
                && terminal
                && !generating,
            id,
            role,
            index,
            is_last: key.is_last,
            branch_number: branch.0,
            branch_count: branch.1,
            parts,
            text,
            status,
            is_generating: generating,
            feedback,
            message: message.clone(),
        }
    }

    // ---------------------------------------------------------- actions

    /// Send `text` as the next message. Spawn the returned future, if any.
    pub fn send(&self, text: &str) -> Result<Option<RunFuture>, RuntimeError> {
        let composer = self.runtime().composer();
        composer.set_text(text);
        composer.send()
    }

    /// The composer for the next message, for drafts, attachments and a
    /// disabled state.
    pub fn composer(&self) -> Composer {
        self.runtime().composer()
    }

    /// Stop the run in progress.
    pub fn stop(&self) {
        self.runtime().cancel_run();
    }

    /// Answer the message's parent again.
    pub fn reload(&self, id: &MessageId) -> Result<RunFuture, RuntimeError> {
        let entry = self.runtime().message(id)?;
        self.runtime().reload(ai_ui_core::StartRunConfig {
            parent_id: entry.parent_id,
            source_id: Some(id.clone()),
            ..Default::default()
        })
    }

    /// Show the next or previous sibling branch of a message.
    pub fn switch_branch(&self, id: &MessageId, forward: bool) -> Result<(), RuntimeError> {
        let runtime = self.runtime();
        let branches = runtime.branches(id)?;
        let Some(index) = branches.iter().position(|b| b == id) else {
            return Ok(());
        };
        let target = if forward {
            index + 1
        } else {
            index.wrapping_sub(1)
        };
        match branches.get(target) {
            Some(target) => runtime.switch_to_branch(target),
            None => Ok(()),
        }
    }

    /// Start editing a message; send or cancel the returned composer.
    pub fn begin_edit(&self, id: &MessageId) -> Result<Composer, RuntimeError> {
        self.runtime().begin_edit(id)
    }

    /// The composer of a message being edited, if any.
    pub fn edit_composer(&self, id: &MessageId) -> Option<Composer> {
        self.runtime().edit_composer(id)
    }

    /// The text to put on the clipboard.
    pub fn copy_text(&self, id: &MessageId) -> Option<String> {
        self.runtime().message(id).ok().map(|e| e.message.text())
    }

    /// Show the "copied" confirmation on a message for two seconds.
    pub fn mark_copied(&self, id: &MessageId, now_ms: f64) {
        {
            let mut ui = self.inner.ui.borrow_mut();
            if let Some((previous, _)) = ui.copied.replace((id.clone(), now_ms + COPIED_MS)) {
                ui.touch(&previous);
            }
            ui.touch(id);
        }
        self.inner.invalidate_and_notify();
    }

    /// Expand or collapse a reasoning or tool part.
    pub fn toggle_part(&self, id: &MessageId, part_index: usize) {
        let current = self
            .state()
            .messages
            .iter()
            .find(|m| m.id == *id)
            .and_then(|m| {
                m.parts
                    .iter()
                    .find(|p| p.index == part_index)
                    .and_then(|p| match &p.kind {
                        PartKind::Reasoning { expanded, .. } => Some(*expanded),
                        PartKind::ToolCall(call) => Some(call.expanded),
                        _ => None,
                    })
            });
        let Some(current) = current else { return };
        {
            let mut ui = self.inner.ui.borrow_mut();
            ui.expanded.insert((id.clone(), part_index), !current);
            ui.touch(id);
        }
        self.inner.invalidate_and_notify();
    }

    /// Answer a tool approval; spawn the returned future, which continues the run.
    pub fn respond_to_approval(
        &self,
        id: &MessageId,
        approval_id: &str,
        approved: bool,
    ) -> Result<Option<RunFuture>, RuntimeError> {
        let reason = (!approved).then(|| "The user declined".to_owned());
        self.runtime()
            .respond_to_tool_approval(id, approval_id, approved, reason)
    }

    /// Answer a tool call yourself (a "human" tool).
    pub fn add_tool_result(&self, result: ToolResult) -> Result<Option<RunFuture>, RuntimeError> {
        self.runtime().add_tool_result(result)
    }

    /// Give feedback on a message; choosing the active feedback again clears it.
    pub fn submit_feedback(&self, id: &MessageId, kind: FeedbackKind) -> Result<(), RuntimeError> {
        let runtime = self.runtime();
        let current = runtime
            .message(id)?
            .message
            .as_assistant()
            .and_then(|m| m.metadata.submitted_feedback.as_ref().map(|f| f.kind));
        let next = (current != Some(kind)).then_some(SubmittedFeedback {
            kind,
            comment: None,
        });
        runtime.submit_feedback(id, next)
    }
}

fn error_text(status: &MessageStatus) -> Option<String> {
    match status {
        MessageStatus::Incomplete {
            reason: IncompleteReason::Error,
            error,
        } => Some(
            error
                .as_ref()
                .and_then(|e| {
                    e.get("message")
                        .and_then(Value::as_str)
                        .or_else(|| e.as_str())
                })
                .map_or_else(|| "The response failed.".to_owned(), str::to_owned),
        ),
        MessageStatus::Incomplete {
            reason: IncompleteReason::ToolCalls,
            ..
        } => Some("Stopped after reaching the step limit.".into()),
        _ => None,
    }
}

fn tool_state(
    call: &ToolCallPart,
    expanded: bool,
    message_ended_early: bool,
    presenters: &HashMap<String, Presenter>,
) -> ToolCallState {
    let approval = call.approval.as_ref().map(|a| ApprovalState {
        id: a.id.clone(),
        prompt: a.prompt.clone().unwrap_or_default(),
        decision: match (a.approved, a.resolution.is_some()) {
            (Some(true), _) => ApprovalDecision::Approved,
            (Some(false), _) => ApprovalDecision::Denied,
            (None, true) => ApprovalDecision::Unavailable,
            (None, false) => ApprovalDecision::Pending,
        },
    });
    let is_error = call.is_error == Some(true);
    let awaiting = approval
        .as_ref()
        .is_some_and(|a| a.decision == ApprovalDecision::Pending);
    let status = if awaiting {
        ToolStatus::AwaitingApproval
    } else if call.is_open() && message_ended_early {
        // The run stopped before this call finished.
        ToolStatus::Error
    } else if call.is_open() {
        ToolStatus::Running
    } else if is_error {
        ToolStatus::Error
    } else {
        ToolStatus::Complete
    };
    ToolCallState {
        call_id: call.tool_call_id.clone(),
        name: call.tool_name.clone(),
        args_text: call.args_text.clone(),
        args: call.args.clone(),
        result: call.result.clone(),
        is_error,
        status,
        approval,
        expanded,
        presentation: presenters
            .get(&call.tool_name)
            .map(|p| p(call))
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod change_journal_tests {
    use super::ViewChangeJournal;

    #[test]
    fn cursor_distance_wraps_and_full_reconciliation_is_a_barrier() {
        let mut journal = ViewChangeJournal {
            version: u64::MAX - 1,
            ..Default::default()
        };
        let before = journal.version;
        journal.record(Some(vec![3]));
        journal.record(Some(vec![1, 3]));
        assert_eq!(journal.version, 0);
        assert_eq!(journal.since(before), Some(vec![1, 3]));
        assert_eq!(journal.since(0), Some(Vec::new()));
        assert_eq!(journal.since(1), None, "a future cursor is not current");
        journal.record(None);
        assert_eq!(journal.since(before), None);
        let barrier = journal.version;
        journal.record(Some(vec![2]));
        assert_eq!(journal.since(barrier), Some(vec![2]));
        assert_eq!(journal.since(0), None);
    }
}
