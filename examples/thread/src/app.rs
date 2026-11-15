//! Binds the toolkit-agnostic [`ThreadView`] and [`ThreadList`] to Slint.
//!
//! All behaviour (branching, smooth streaming, approvals, row caching) lives in
//! `ai-ui-state`; this file only projects [`MessageState`] into Slint
//! rows and forwards the window's callbacks to the view's actions. A row is
//! rebuilt only when its `Arc<MessageState>` changed.

use crate::chunks::{Chunks, MIN_CHUNKED};
use crate::markdown::Markdown;
use crate::paging::Pages;
use crate::{
    AppWindow, ApprovalChoice, ApprovalRequest, ApprovalState, AssistantSuggestion,
    AssistantThreadListItem, ChatPart, ChatPartKind, ChatRow, MessageFeedback, MessageRole,
    MessageStatusTone, PartIcon,
};
use ai_ui_core::{
    FeedbackKind, MessageId, MessageStatus, RunFuture, ThreadItem, ThreadList, ThreadStatus,
};
use ai_ui_state::{
    ApprovalDecision, MessageState, PartKind, ThreadView, ToolCallState, ToolStatus, ViewCursor,
};
use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::sync::{Arc, Weak as ArcWeak};
use std::time::Duration;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

pub const SUGGESTIONS: [&str; 4] = [
    "What's the weather in Tokyo?",
    "Send an email to the team",
    "What can you do?",
    "Explain message branching",
];

/// Message counts a host already knows for stored threads, by remote ID.
type KnownCounts = Box<dyn Fn(&str) -> Option<usize>>;

/// From this many messages a thread is the long one the demo is about.
const LARGE_THREAD: usize = 1000;

pub struct App {
    me: Weak<App>,
    view: ThreadView,
    list: ThreadList,
    ui: slint::Weak<AppWindow>,
    rows: Rc<VecModel<ChatRow>>,
    /// The `MessageState` each row shows; `None` forces a presentation-only
    /// rebuild. A weak reference keeps the allocation's address reserved, so a
    /// recycled address is never mistaken for an unchanged row, while the row's
    /// contents are freed as soon as the view replaces it. Holding strong
    /// references kept a second copy of every row alive across full
    /// reconciliations (about 0.9 KB per message of peak RSS).
    shown: RefCell<Vec<Option<ArcWeak<MessageState>>>>,
    /// The message each row shows.
    shown_ids: RefCell<Vec<MessageId>>,
    /// Independent cursor: other consumers may read the view between syncs.
    view_cursor: RefCell<Option<ViewCursor>>,
    /// Locate formatter completions without walking the historical rows.
    row_indices: RefCell<HashMap<String, usize>>,
    /// Presentation-only changes (Markdown completion and page navigation).
    dirty_rows: RefCell<Vec<usize>>,
    markdown: RefCell<Markdown>,
    /// Long reasoning texts as chunks, by (message id, part index).
    chunks: RefCell<HashMap<(String, usize), Chunks>>,
    /// Only a bounded page of a large user message reaches Slint's layout.
    pages: RefCell<Pages<ai_ui_core::ThreadMessage>>,
    markdown_timer: Timer,
    frame_timer: Timer,
    /// Runs a focus change after the event that asked for it.
    focus_timer: Timer,
    epoch: Instant,
    clipboard: Option<ai_ui_clipboard::Clipboard>,
    conversation: Cell<i32>,
    /// The sidebar's search text; the visible threads are filtered by it.
    thread_query: RefCell<String>,
    /// The thread whose rows are in the model.
    shown_thread: RefCell<String>,
    /// Whether the last sync saw a run in progress, to focus the composer once per run.
    was_running: Cell<bool>,
    /// The sidebar rows, updated in place: the thread list notifies on every
    /// streamed token, and replacing the model made Slint rebuild every row.
    thread_items: Rc<VecModel<AssistantThreadListItem>>,
    /// Message counts the host knows for stored threads, by remote id.
    known_counts: RefCell<Option<KnownCounts>>,
    /// The message count and running state the summary was last made for.
    seen: Cell<(usize, bool)>,
    /// The last message count of each thread that was open, so the sidebar keeps it after switching away.
    open_counts: RefCell<HashMap<String, usize>>,
    /// The thread a top notice offers to go back to.
    toast_back: RefCell<Option<String>>,
}

/// Messages (newest first) whose Markdown is formatted; older ones show plain text.
const MARKDOWN_WINDOW: usize = 1200;

/// Tell the formatter about a message's assistant text parts.
fn observe_message(markdown: &mut Markdown, message: &MessageState) {
    if message.role != ai_ui_core::MessageRole::Assistant {
        return;
    }
    for part in &message.parts {
        if let PartKind::Text { text, streaming } = &part.kind {
            markdown.observe_one((message.id.0.clone(), part.index), text, *streaming);
        }
    }
}

/// What sizes a user bubble: its longest line, capped. Any line longer than
/// the cap already reaches the bubble's maximum width, so measuring the whole
/// text (the previous hint) only cost time on long prompts.
fn width_hint(text: &str) -> &str {
    const CAP: usize = 400;
    let longest = text.lines().max_by_key(|line| line.len()).unwrap_or("");
    if longest.len() <= CAP {
        return longest;
    }
    let mut end = CAP;
    while !longest.is_char_boundary(end) {
        end -= 1;
    }
    &longest[..end]
}

fn spawn(run: RunFuture) {
    // The message records failures; nothing else to report here.
    let _ = slint::spawn_local(async move {
        let _ = run.await;
    });
}

fn pretty(value: &serde_json::Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

thread_local! {
    /// The UI-thread app, for callbacks that arrive through the event loop.
    static APP: RefCell<Option<Weak<App>>> = const { RefCell::new(None) };
}

impl App {
    pub fn new(ui: &AppWindow, list: ThreadList, view: ThreadView) -> Rc<Self> {
        let rows = Rc::new(VecModel::default());
        ui.set_messages(ModelRc::from(rows.clone()));
        ui.set_suggestions(ModelRc::new(VecModel::from(
            SUGGESTIONS
                .iter()
                .enumerate()
                .map(|(i, s)| AssistantSuggestion {
                    id: i.to_string().into(),
                    title: (*s).into(),
                })
                .collect::<Vec<_>>(),
        )));
        let app = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            view,
            list,
            ui: ui.as_weak(),
            rows,
            shown: RefCell::new(Vec::new()),
            shown_ids: RefCell::new(Vec::new()),
            view_cursor: RefCell::new(None),
            row_indices: RefCell::new(HashMap::new()),
            dirty_rows: RefCell::new(Vec::new()),
            markdown: RefCell::new(Markdown::new()),
            chunks: RefCell::new(HashMap::new()),
            pages: RefCell::new(Pages::new(1)),
            markdown_timer: Timer::default(),
            frame_timer: Timer::default(),
            focus_timer: Timer::default(),
            epoch: Instant::now(),
            clipboard: ai_ui_clipboard::Clipboard::new().ok(),
            conversation: Cell::new(1),
            thread_query: RefCell::new(String::new()),
            shown_thread: RefCell::new(String::new()),
            was_running: Cell::new(false),
            thread_items: Rc::new(VecModel::default()),
            known_counts: RefCell::new(None),
            seen: Cell::new((usize::MAX, false)),
            open_counts: RefCell::new(HashMap::new()),
            toast_back: RefCell::new(None),
        });
        APP.with(|a| *a.borrow_mut() = Some(Rc::downgrade(&app)));
        let weak = Rc::downgrade(&app);
        app.view
            .subscribe(move || {
                if let Some(app) = weak.upgrade() {
                    app.sync();
                }
            })
            .detach();
        let weak = Rc::downgrade(&app);
        app.list
            .subscribe(move || {
                if let Some(app) = weak.upgrade() {
                    app.sync_threads();
                }
            })
            .detach();
        ui.set_thread_items(ModelRc::from(app.thread_items.clone()));
        app.bind(ui);
        app.sync_threads();
        app.sync();
        app
    }

    fn now_ms(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64() * 1000.0
    }

    // -------------------------------------------------------------- rows

    /// Bring the Slint rows up to date. A usable view cursor visits only dirty
    /// rows; structural/global changes and expired cursors reconcile all rows.
    pub fn sync(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let changes = self
            .view
            .state_changes_since(self.view_cursor.borrow().as_ref());
        let state = changes.state;
        *self.view_cursor.borrow_mut() = Some(changes.cursor);
        let len = state.messages.len();
        // The sidebar counts and the summary follow the open thread's length and run state.
        let seen = (len, state.is_running);
        if self.seen.get() != seen {
            self.seen.set(seen);
            self.open_counts
                .borrow_mut()
                .insert(self.list.main_id(), len);
            self.sync_threads();
        }
        {
            let mut shown = self.shown.borrow_mut();
            let mut ids = self.shown_ids.borrow_mut();
            let mut structure = shown.len() != len;
            let mut changed = match changes.indices {
                Some(indices) if !structure => indices,
                _ => (0..len).collect(),
            };
            changed.append(&mut self.dirty_rows.borrow_mut());
            changed.sort_unstable();
            changed.dedup();
            changed.retain(|&i| {
                i < len
                    && !shown.get(i).and_then(Option::as_ref).is_some_and(|row| {
                        std::ptr::eq(row.as_ptr(), Arc::as_ptr(&state.messages[i]))
                    })
            });
            shown.truncate(len);
            ids.truncate(len);
            structure |= changed
                .iter()
                .any(|&i| ids.get(i).is_some_and(|id| *id != state.messages[i].id));
            if structure {
                let mut indices = self.row_indices.borrow_mut();
                indices.clear();
                indices.extend(
                    state
                        .messages
                        .iter()
                        .enumerate()
                        .map(|(i, message)| (message.id.0.clone(), i)),
                );
            }
            {
                // Newest text first, so the visible end of the thread formats first.
                let mut markdown = self.markdown.borrow_mut();
                for &i in changed.iter().rev() {
                    if i + MARKDOWN_WINDOW >= len {
                        observe_message(&mut markdown, &state.messages[i]);
                    }
                }
                if structure {
                    // Messages left the thread, or it grew: keep formatting only
                    // for the newest ones.
                    let live: HashSet<&str> = state
                        .messages
                        .iter()
                        .rev()
                        .take(MARKDOWN_WINDOW)
                        .map(|m| m.id.0.as_str())
                        .collect();
                    markdown.retain(|(id, _)| live.contains(id.as_str()));
                    let all: HashSet<&str> =
                        state.messages.iter().map(|m| m.id.0.as_str()).collect();
                    self.chunks
                        .borrow_mut()
                        .retain(|(id, _), _| all.contains(id.as_str()));
                    self.pages.borrow_mut().retain(|id| all.contains(id));
                }
            }
            for &index in &changed {
                let message = &state.messages[index];
                let row = self.row(message);
                if index < self.rows.row_count() {
                    self.update_row(index, row);
                    shown[index] = Some(Arc::downgrade(message));
                    ids[index] = message.id.clone();
                } else {
                    self.rows.push(row);
                    shown.push(Some(Arc::downgrade(message)));
                    ids.push(message.id.clone());
                }
            }
        }
        while self.rows.row_count() > len {
            self.rows.remove(self.rows.row_count() - 1);
        }
        ui.set_running(state.is_running);
        // Any run start (send, suggestion, regenerate, approval, edit) focuses the composer.
        let started = state.is_running && !self.was_running.get();
        self.was_running.set(state.is_running);
        if started {
            self.after_event(|ui| ui.invoke_focus_composer_for_run());
        }
        ui.set_thread_revision(self.conversation.get());
        if self.markdown.borrow().pending() && !self.markdown_timer.running() {
            self.poll_markdown_soon();
        }
        if self.view.needs_frame() {
            self.animate_soon();
        }
    }

    /// A host-only change does not alter the view's message identity or cursor.
    fn invalidate_row(&self, index: usize) {
        if let Some(shown) = self.shown.borrow_mut().get_mut(index) {
            *shown = None;
            self.dirty_rows.borrow_mut().push(index);
        }
    }

    /// Replace a row, keeping its parts model when it already has one and updating
    /// that model in place: Slint then re-lays out only the parts that changed
    /// instead of rebuilding every part of a message on each streamed update.
    fn update_row(&self, index: usize, mut row: ChatRow) {
        if let Some(old) = self.rows.row_data(index) {
            // Only for the same message: Slint keeps local state in each part (a code block's
            // scroll position), and another message must not inherit it.
            let reusable = old.id == row.id
                && old
                    .parts
                    .as_any()
                    .downcast_ref::<VecModel<ChatPart>>()
                    .is_some();
            if reusable {
                let parts: Vec<ChatPart> = row.parts.iter().collect();
                let model = old
                    .parts
                    .as_any()
                    .downcast_ref::<VecModel<ChatPart>>()
                    .expect("checked above");
                for (i, part) in parts.iter().enumerate() {
                    match model.row_data(i) {
                        Some(existing) if existing == *part => {}
                        Some(_) => model.set_row_data(i, part.clone()),
                        None => model.push(part.clone()),
                    }
                }
                while model.row_count() > parts.len() {
                    model.remove(model.row_count() - 1);
                }
                row.parts = old.parts.clone();
            }
            if old == row {
                return;
            }
        }
        self.rows.set_row_data(index, row);
    }

    /// No formatting is pending and nothing is animating.
    pub fn idle(&self) -> bool {
        !self.markdown.borrow().pending() && !self.view.needs_frame()
    }

    fn poll_markdown_soon(&self) {
        let weak = self.me.clone();
        self.markdown_timer
            .start(TimerMode::Repeated, Duration::from_millis(1), move || {
                let Some(app) = weak.upgrade() else { return };
                let (ready, pending) = {
                    let mut markdown = app.markdown.borrow_mut();
                    (markdown.poll(), markdown.pending())
                };
                if ready {
                    // Rebuild only the rows whose text finished formatting.
                    let finished = app.markdown.borrow_mut().take_changed();
                    {
                        let indices = app.row_indices.borrow();
                        for (id, _) in finished {
                            if let Some(&index) = indices.get(&id) {
                                app.invalidate_row(index);
                            }
                        }
                    }
                    app.sync();
                }
                if !pending {
                    app.markdown_timer.stop();
                }
            });
    }

    /// Advance smooth streaming and the copy confirmation once per frame.
    fn animate_soon(&self) {
        if self.frame_timer.running() {
            return;
        }
        let weak = self.me.clone();
        self.frame_timer
            .start(TimerMode::Repeated, Duration::from_millis(16), move || {
                let Some(app) = weak.upgrade() else { return };
                // The view notifies, and sync() runs, when something changed.
                app.view.tick(app.now_ms());
                if !app.view.needs_frame() {
                    app.frame_timer.stop();
                }
            });
    }

    fn row(&self, message: &Arc<MessageState>) -> ChatRow {
        let id = message.id.0.clone();
        self.pages
            .borrow_mut()
            .reconcile_source(&id, &message.message);
        let user = message.role == ai_ui_core::MessageRole::User;
        let mut row = ChatRow {
            id: id.clone().into(),
            branch_token: id.clone().into(),
            role: if user {
                MessageRole::User
            } else {
                MessageRole::Assistant
            },
            branch_number: message.branch_number as i32,
            branch_count: message.branch_count as i32,
            can_edit: message.can_edit,
            can_regenerate: message.can_reload,
            can_copy: message.can_copy,
            can_feedback: message.can_feedback,
            copied: message.is_copied,
            feedback: match message.feedback {
                Some(FeedbackKind::Positive) => MessageFeedback::Positive,
                Some(FeedbackKind::Negative) => MessageFeedback::Negative,
                None => MessageFeedback::None,
            },
            ..Default::default()
        };
        if user {
            row.width_hint = width_hint(&message.text).into();
        }
        let mut parts = Vec::new();
        for part in &message.parts {
            let part_id: SharedString = part.index.to_string().into();
            match &part.kind {
                PartKind::Text { text, streaming } => {
                    let mut chat = ChatPart {
                        id: part_id,
                        kind: ChatPartKind::Text,
                        ..Default::default()
                    };
                    if user {
                        // Full text remains authoritative in the runtime for
                        // copy/edit/model input. Large messages expose only a
                        // bounded, verbatim page to Slint's text layout.
                        if let Some(page) =
                            self.pages
                                .borrow_mut()
                                .project(&id, part.index, &message.message, text)
                        {
                            chat.text_paged = true;
                            chat.text_page = page.text.into();
                            chat.text_page_number = page.number as i32;
                            chat.text_page_count = page.count as i32;
                            chat.text_page_token = page.token.to_string().into();
                        } else {
                            chat.body = text.clone().into();
                        }
                    } else {
                        let blocks = self.markdown.borrow_mut().view(
                            &(id.clone(), part.index),
                            text,
                            *streaming,
                        );
                        chat.markdown_enabled = blocks.is_some();
                        // The raw text is shown only until the first formatting exists. Converting
                        // a long reply to a Slint string costs about 4 ns per byte, which would
                        // be paid on every streamed update for text that is never displayed.
                        if blocks.is_none() {
                            chat.body = text.clone().into();
                        }
                        chat.markdown_blocks = blocks.unwrap_or_default();
                    }
                    parts.push(chat);
                }
                PartKind::Reasoning {
                    text,
                    streaming,
                    expanded,
                } => {
                    let mut chat = ChatPart {
                        id: part_id,
                        kind: ChatPartKind::Reasoning,
                        icon: PartIcon::Brain,
                        label: "Reasoning".into(),
                        streaming: *streaming,
                        expanded: *expanded,
                        ..Default::default()
                    };
                    // Collapsed reasoning shows no text, so none is converted.
                    if *expanded && text.len() >= MIN_CHUNKED {
                        chat.body_chunks = self
                            .chunks
                            .borrow_mut()
                            .entry((id.clone(), part.index))
                            .or_insert_with(Chunks::new)
                            .update(text);
                    } else if *expanded {
                        chat.body = text.clone().into();
                    }
                    parts.push(chat);
                }
                PartKind::ToolCall(call) => parts.push(tool_part(part_id, &id, call)),
                PartKind::Source { title, url } => parts.push(ChatPart {
                    id: part_id,
                    kind: ChatPartKind::Source,
                    label: title.clone().into(),
                    body: url.clone().unwrap_or_default().into(),
                    ..Default::default()
                }),
                _ => {}
            }
        }
        if let Some(error) = &message.error {
            let failed = matches!(
                message.status,
                Some(MessageStatus::Incomplete {
                    reason: ai_ui_core::IncompleteReason::Error,
                    ..
                })
            );
            row.status = if failed {
                format!("Error: {error}")
            } else {
                error.clone()
            }
            .into();
            row.status_tone = if failed {
                MessageStatusTone::Error
            } else {
                MessageStatusTone::Neutral
            };
        }
        row.parts = ModelRc::new(VecModel::from(parts));
        row
    }

    // ----------------------------------------------------------- threads

    /// The threads the sidebar shows, in order and filtered by its search.
    fn visible_threads(&self) -> Vec<ThreadItem> {
        let query = self.thread_query.borrow().trim().to_lowercase();
        self.list
            .items()
            .into_iter()
            .filter(|i| i.status != ThreadStatus::Archived)
            .filter(|i| query.is_empty() || thread_title(i).to_lowercase().contains(&query))
            .collect()
    }

    /// The message count the sidebar shows for a thread: the open one from its
    /// own state, a stored one from the host's knowledge of it (without opening it).
    fn message_count(&self, item: &ThreadItem, main: &str) -> Option<usize> {
        if item.id == main {
            return Some(self.view.state().messages.len());
        }
        if let Some(remote) = item.remote_id.as_deref() {
            let known = self
                .known_counts
                .borrow()
                .as_ref()
                .and_then(|count| count(remote));
            if known.is_some() {
                return known;
            }
        }
        self.open_counts.borrow().get(&item.id).copied()
    }

    /// Give the sidebar the message counts of stored threads.
    pub fn set_known_counts(&self, counts: impl Fn(&str) -> Option<usize> + 'static) {
        *self.known_counts.borrow_mut() = Some(Box::new(counts));
        self.sync_threads();
    }

    /// Explain a switch away from the long thread, and offer to go back to it.
    fn notify_leaving(&self, ui: &AppWindow, previous: &str, next: &str) {
        if previous.is_empty() || previous == next {
            return;
        }
        let count = self
            .open_counts
            .borrow()
            .get(previous)
            .copied()
            .unwrap_or(0);
        if count < LARGE_THREAD {
            return;
        }
        ui.set_toast_title(format!("You left the {}-message thread", grouped(count)).into());
        ui.set_toast_body(
            format!(
                "That was the real test: {} messages in one thread, and it stayed smooth. Only the rows on screen are laid out, and finished Markdown blocks are cached. Go back to see it again.",
                grouped(count)
            )
            .into(),
        );
        *self.toast_back.borrow_mut() = Some(previous.to_owned());
        ui.set_toast_open(true);
    }

    fn hide_toast(&self) {
        if let Some(ui) = self.ui.upgrade() {
            ui.set_toast_open(false);
        }
    }

    fn sync_threads(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let main = self.list.main_id();
        let running = self.view.state().is_running;
        let items = self.visible_threads();
        let selected = items
            .iter()
            .position(|i| i.id == main)
            .map_or(-1, |p| p as i32);
        let rows: Vec<AssistantThreadListItem> = items
            .iter()
            .map(|i| AssistantThreadListItem {
                id: i.id.clone().into(),
                title: thread_title(i).into(),
                running: running && i.id == main,
                detail: self
                    .message_count(i, &main)
                    .filter(|n| *n > 0)
                    .map(grouped)
                    .unwrap_or_default()
                    .into(),
            })
            .collect();
        let summary = {
            let messages = self.list.main().messages();
            let bytes: usize = messages.iter().map(|m| m.text().len()).sum();
            stats_line(messages.len(), bytes)
        };
        ui.set_thread_stats(summary.into());
        let model = &self.thread_items;
        if model.row_count() == rows.len() {
            for (index, row) in rows.into_iter().enumerate() {
                if model.row_data(index).as_ref() != Some(&row) {
                    model.set_row_data(index, row);
                }
            }
        } else {
            model.set_vec(rows);
        }
        if ui.get_selected_thread() != selected {
            ui.set_selected_thread(selected);
        }
        // A different main thread is a different conversation: restart scrolling.
        if *self.shown_thread.borrow() != main {
            let previous = self.shown_thread.borrow().clone();
            self.notify_leaving(&ui, &previous, &main);
            *self.shown.borrow_mut() = Vec::new();
            self.shown_ids.borrow_mut().clear();
            *self.view_cursor.borrow_mut() = None;
            self.row_indices.borrow_mut().clear();
            self.dirty_rows.borrow_mut().clear();
            self.markdown.borrow_mut().retain(|_| false);
            self.chunks.borrow_mut().clear();
            self.rows.set_vec(Vec::new());
            self.conversation.set(self.conversation.get() + 1);
            self.pages.borrow_mut().reset(self.conversation.get());
            *self.shown_thread.borrow_mut() = main;
            self.sync();
        }
    }

    // --------------------------------------------------------- callbacks

    fn bind(self: &Rc<Self>, ui: &AppWindow) {
        let app = Rc::downgrade(self);
        macro_rules! on {
            ($setter:ident, |$app:ident $(, $arg:ident)*| $body:expr) => {{
                let app = app.clone();
                ui.$setter(move |$($arg),*| {
                    let Some($app) = app.upgrade() else { return Default::default() };
                    $body
                });
            }};
        }
        on!(on_send, |app, text| app.send(text.as_str()));
        on!(on_suggestion_selected, |app, id| {
            if let Some(prompt) = id.parse::<usize>().ok().and_then(|i| SUGGESTIONS.get(i)) {
                app.send(prompt);
            }
        });
        on!(on_stop, |app| app.view.stop());
        on!(on_toggle_part, |app, message, part| {
            if let Ok(index) = part.parse::<usize>() {
                app.view.toggle_part(&MessageId(message.to_string()), index);
            }
        });
        on!(on_page_requested, |app,
                                revision,
                                id,
                                part,
                                token,
                                page,
                                delta| {
            let Ok(part) = part.parse::<usize>() else {
                return;
            };
            let state = app.view.state();
            let Some(index) = app.row_indices.borrow().get(id.as_str()).copied() else {
                return;
            };
            let Some(message) = state.messages.get(index).filter(|m| m.id.0 == id.as_str()) else {
                return;
            };
            if app.pages.borrow_mut().advance(
                revision,
                (id.as_str(), part),
                &message.message,
                token.as_str(),
                page,
                delta,
            ) {
                // A presentation-only change: rebuild this row without
                // notifying or changing the model/runtime.
                app.invalidate_row(index);
                app.sync();
            }
        });
        on!(on_resolve_approval, |app, request, choice| {
            let approved = matches!(choice, ApprovalChoice::Approve);
            let id = MessageId(request.message.to_string());
            if let Ok(Some(run)) =
                app.view
                    .respond_to_approval(&id, request.approval.as_str(), approved)
            {
                spawn(run);
            }
        });
        on!(on_switch_branch, |app, token, next| {
            let _ = app.view.switch_branch(&MessageId(token.to_string()), next);
        });
        on!(on_regenerate, |app, token| {
            if let Ok(run) = app.view.reload(&MessageId(token.to_string())) {
                spawn(run);
            }
        });
        on!(on_begin_edit, |app, token| {
            let Some(ui) = app.ui.upgrade() else { return };
            if let Ok(composer) = app.view.begin_edit(&MessageId(token.to_string())) {
                ui.set_edit_draft(composer.text().into());
                ui.set_edit_token(token);
            }
        });
        on!(on_save_edit, |app, token, text| {
            let Some(ui) = app.ui.upgrade() else { return };
            if let Some(composer) = app.view.edit_composer(&MessageId(token.to_string())) {
                composer.set_text(text.as_str());
                if let Ok(Some(run)) = composer.send() {
                    spawn(run);
                }
            }
            ui.set_edit_token("".into());
            app.focus_composer();
        });
        on!(on_cancel_edit, |app, token| {
            let Some(ui) = app.ui.upgrade() else { return };
            if let Some(composer) = app.view.edit_composer(&MessageId(token.to_string())) {
                composer.cancel();
            }
            ui.set_edit_token("".into());
            app.focus_composer();
        });
        on!(on_copy_message, |app, token| {
            let id = MessageId(token.to_string());
            if let Some(text) = app.view.copy_text(&id) {
                app.copy(text, Some(id));
            }
        });
        on!(on_copy_code, |app, code| app.copy(code.to_string(), None));
        on!(on_submit_feedback, |app, token, choice| {
            let kind = match choice {
                MessageFeedback::Positive => FeedbackKind::Positive,
                MessageFeedback::Negative => FeedbackKind::Negative,
                MessageFeedback::None => return,
            };
            let _ = app
                .view
                .submit_feedback(&MessageId(token.to_string()), kind);
        });
        ui.on_link_activated(|url| open_url(url.as_str()));

        // Thread list. Indices refer to the rows the sidebar was given.
        on!(on_toast_back, |app| {
            let back = app.toast_back.borrow_mut().take();
            if let Some(id) = back {
                app.list.switch_to(&id);
            }
            app.hide_toast();
        });
        on!(on_toast_dismiss, |app| app.hide_toast());
        on!(on_select_thread, |app, index| {
            if let Some(item) = app.thread_at(index) {
                app.list.switch_to(&item.id);
                app.focus_composer();
            }
        });
        on!(on_new_thread, |app| {
            app.list.switch_to_new();
            app.focus_composer();
        });
        on!(on_rename_thread, |app, index, text| {
            // The host trims the name; a blank one cancels the rename.
            let title = text.as_str().trim();
            if let (Some(item), false) = (app.thread_at(index), title.is_empty()) {
                app.list.rename(&item.id, title);
            }
        });
        on!(on_archive_thread, |app, index| {
            if let Some(item) = app.thread_at(index) {
                app.list.archive(&item.id);
            }
        });
        on!(on_delete_thread, |app, index| {
            if let Some(item) = app.thread_at(index) {
                app.list.delete(&item.id);
            }
        });
        on!(on_search_threads, |app, query| {
            *app.thread_query.borrow_mut() = query.to_string();
            app.sync_threads();
        });
    }

    /// Move keyboard focus to the main composer. The composer must be enabled.
    /// Focus taken during an event is lost if that event removes the focused
    /// item (an edit composer closing), so it is taken again after the event.
    pub fn focus_composer(&self) {
        if let Some(ui) = self.ui.upgrade() {
            ui.invoke_focus_composer();
        }
        self.after_event(|ui| ui.invoke_focus_composer());
    }

    fn after_event(&self, change: fn(&AppWindow)) {
        let weak = self.ui.clone();
        self.focus_timer
            .start(TimerMode::SingleShot, Duration::ZERO, move || {
                if let Some(ui) = weak.upgrade() {
                    change(&ui);
                }
            });
    }

    /// The visible thread at a sidebar row, if the index is still valid.
    fn thread_at(&self, index: i32) -> Option<ThreadItem> {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.visible_threads().into_iter().nth(i))
    }

    pub fn send(&self, text: &str) -> bool {
        if text.trim().is_empty() {
            return false;
        }
        match self.view.send(text) {
            Ok(Some(run)) => {
                spawn(run);
                true
            }
            _ => false,
        }
    }

    /// Send a message and return its run, for offscreen scenarios.
    pub fn send_for_test(&self, text: &str) -> Option<RunFuture> {
        self.view.send(text).ok().flatten()
    }

    /// Expand every reasoning and tool-call part, for offscreen scenarios.
    pub fn expand_all_for_test(&self) {
        let state = self.view.state();
        for message in &state.messages {
            for (index, part) in message.parts.iter().enumerate() {
                let collapsed = match &part.kind {
                    PartKind::Reasoning { expanded, .. } => !*expanded,
                    PartKind::ToolCall(call) => !call.expanded,
                    _ => false,
                };
                if collapsed {
                    self.view.toggle_part(&message.id, index);
                }
            }
        }
    }

    /// Answer the newest pending approval, for offscreen scenarios.
    pub fn decide_pending(&self, approved: bool) -> Option<RunFuture> {
        let state = self.view.state();
        let message = state.messages.last()?;
        let approval = message.parts.iter().find_map(|p| match &p.kind {
            PartKind::ToolCall(call) => call
                .approval
                .clone()
                .filter(|a| a.decision == ApprovalDecision::Pending),
            _ => None,
        })?;
        self.view
            .respond_to_approval(&message.id, &approval.id, approved)
            .ok()
            .flatten()
    }

    fn copy(&self, text: String, message: Option<MessageId>) {
        let Some(clipboard) = &self.clipboard else {
            return;
        };
        let _ = clipboard.copy(
            text,
            Box::new(move |result| {
                if result.is_ok() {
                    let _ = slint::invoke_from_event_loop(move || {
                        let Some(app) = APP.with(|a| a.borrow().as_ref().and_then(|w| w.upgrade()))
                        else {
                            return;
                        };
                        if let Some(id) = message {
                            app.view.mark_copied(&id, app.now_ms());
                        }
                    });
                }
            }),
        );
    }
}

/// A thread's sidebar title: its own title, or a placeholder until it has one.
/// `1204` as `1,204`, so a count reads at a glance.
fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The summary above an open thread: its size and what keeps it smooth.
fn stats_line(messages: usize, bytes: usize) -> String {
    if messages == 0 {
        return String::new();
    }
    let noun = if messages == 1 { "message" } else { "messages" };
    let text = if bytes >= 1 << 20 {
        format!("{:.1} MB", bytes as f64 / (1 << 20) as f64)
    } else {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    };
    format!(
        "{} {noun} · {text} of text · only on-screen rows are laid out · finished Markdown blocks are cached",
        grouped(messages)
    )
}

fn thread_title(item: &ThreadItem) -> String {
    item.title.clone().unwrap_or_else(|| "New chat".into())
}

fn tool_part(part_id: SharedString, message: &str, call: &ToolCallState) -> ChatPart {
    let failed = matches!(call.status, ToolStatus::Error);
    let (approval_state, approval_prompt, approval_request) = match &call.approval {
        Some(a) => (
            match a.decision {
                ApprovalDecision::Pending => ApprovalState::Requested,
                ApprovalDecision::Approved => ApprovalState::Approved,
                ApprovalDecision::Denied => ApprovalState::Denied,
                ApprovalDecision::Unavailable => ApprovalState::Unavailable,
            },
            a.prompt.clone(),
            ApprovalRequest {
                message: message.into(),
                part: part_id.clone(),
                approval: a.id.clone().into(),
                ..Default::default()
            },
        ),
        None => (ApprovalState::None, String::new(), Default::default()),
    };
    let result = call
        .presentation
        .result_text
        .clone()
        .or_else(|| call.result.as_ref().map(pretty))
        .unwrap_or_default();
    ChatPart {
        id: part_id,
        kind: ChatPartKind::Tool,
        icon: match call.status {
            ToolStatus::Error => PartIcon::Attention,
            ToolStatus::Complete => PartIcon::Complete,
            ToolStatus::Running | ToolStatus::AwaitingApproval => PartIcon::Running,
        },
        label: call
            .presentation
            .title
            .clone()
            .unwrap_or_else(|| call.name.clone())
            .into(),
        approval_state,
        approval_prompt: approval_prompt.into(),
        approval_request,
        tool_structured: true,
        tool_arguments: call.args_text.clone().into(),
        tool_detail: call.presentation.detail.clone().unwrap_or_default().into(),
        tool_has_result: call.result.is_some(),
        tool_result_label: if failed {
            "Error".into()
        } else {
            "Result".into()
        },
        tool_result: result.into(),
        expanded: call.expanded,
        ..Default::default()
    }
}

/// Open an http(s) link in the system browser.
fn open_url(url: &str) {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return;
    }
    #[cfg(target_os = "linux")]
    let command = std::process::Command::new("xdg-open").arg(url).spawn();
    #[cfg(target_os = "macos")]
    let command = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let command = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn();
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    let _ = command;
}

#[cfg(test)]
mod tests {
    use super::*;
    use ai_ui_core::{
        adapter, ChatModelRunResult, ExportedRepository, LocalRuntimeOptions, LocalThreadRuntime,
        ThreadMessage,
    };
    use ai_ui_state::ViewOptions;

    fn user_message(text: &str) -> ThreadMessage {
        serde_json::from_value(serde_json::json!({
            "id": "large", "role": "user", "createdAt": 0,
            "content": [{ "type": "text", "text": text }]
        }))
        .unwrap()
    }

    fn first_part(app: &App) -> ChatPart {
        app.rows.row_data(0).unwrap().parts.row_data(0).unwrap()
    }

    #[test]
    fn large_user_pages_render_navigate_and_keep_full_source_for_copy_and_edit() {
        let _window = crate::shot::install(900, 760);
        let ui = AppWindow::new().unwrap();
        let list = ThreadList::new(
            |_| {
                Rc::new(LocalThreadRuntime::new(
                    |_| adapter::from_results([Ok(ChatModelRunResult::text("answer"))]),
                    LocalRuntimeOptions::default(),
                ))
            },
            Rc::new(|_| {}),
            None,
        );
        let view = ThreadView::for_list(&list, ViewOptions::default());
        let app = App::new(&ui, list.clone(), view.clone());
        let text = (0..20_000)
            .map(|line| format!("Line {line:05}: a complete user message.\n"))
            .collect::<String>();
        list.main()
            .import(ExportedRepository::from_linear([user_message(&text)]))
            .unwrap();
        ui.show().unwrap();
        let started = Instant::now();
        ui.window().take_snapshot().unwrap();
        let rendered = started.elapsed();
        if let Ok(path) = std::env::var("AI_UI_PAGED_TEXT_SCREENSHOT") {
            crate::shot::save_now(ui.window(), &path).unwrap();
        }
        eprintln!(
            "large user message: {} bytes, first bounded-page layout + paint {:?}",
            text.len(),
            rendered
        );
        let first = first_part(&app);
        assert!(first.text_paged);
        assert!(
            first.body.is_empty(),
            "do not duplicate the full source in Slint"
        );
        assert!(first.text_page.len() <= 16 * 1024);
        assert_eq!(first.text_page_number, 1);
        assert!(first.text_page_count > 1);
        assert_eq!(view.copy_text(&MessageId("large".into())).unwrap(), text);
        let edit = view.begin_edit(&MessageId("large".into())).unwrap();
        assert_eq!(edit.text(), text);
        edit.cancel();

        let revision = ui.get_thread_revision();
        ui.invoke_page_requested(
            revision,
            "large".into(),
            "0".into(),
            first.text_page_token.clone(),
            1,
            1,
        );
        let second = first_part(&app);
        assert_eq!(second.text_page_number, 2);
        assert_ne!(second.text_page, first.text_page);
        ui.window().take_snapshot().unwrap();
        // A queued duplicate click cannot skip a page.
        ui.invoke_page_requested(
            revision,
            "large".into(),
            "0".into(),
            first.text_page_token.clone(),
            1,
            1,
        );
        assert_eq!(first_part(&app).text_page_number, 2);

        // Same ID, new source: return to the first page and reject the old token.
        list.main()
            .import(ExportedRepository::from_linear([user_message(
                &text.replace("Line", "Edit"),
            )]))
            .unwrap();
        assert_eq!(first_part(&app).text_page_number, 1);
        ui.invoke_page_requested(
            revision,
            "large".into(),
            "0".into(),
            second.text_page_token,
            2,
            1,
        );
        assert_eq!(first_part(&app).text_page_number, 1);

        list.switch_to_new();
        list.main()
            .import(ExportedRepository::from_linear([user_message(&text)]))
            .unwrap();
        assert_ne!(ui.get_thread_revision(), revision);
        ui.invoke_page_requested(
            revision,
            "large".into(),
            "0".into(),
            first.text_page_token,
            1,
            1,
        );
        assert_eq!(first_part(&app).text_page_number, 1);

        list.main()
            .import(ExportedRepository::from_linear([user_message(
                "ordinary message",
            )]))
            .unwrap();
        let ordinary = first_part(&app);
        assert!(!ordinary.text_paged);
        assert_eq!(ordinary.body, "ordinary message");
        ui.window().take_snapshot().unwrap();
        ui.hide().unwrap();
    }

    /// A shown app with one thread holding a user message, and the composer focused.
    fn focused_app() -> (AppWindow, Rc<App>, ThreadList, ThreadView) {
        let _window = crate::shot::install(900, 760);
        let ui = AppWindow::new().unwrap();
        let list = ThreadList::new(
            |_| {
                Rc::new(LocalThreadRuntime::new(
                    |_| adapter::from_results([Ok(ChatModelRunResult::text("answer"))]),
                    LocalRuntimeOptions::default(),
                ))
            },
            Rc::new(|_| {}),
            None,
        );
        let view = ThreadView::for_list(&list, ViewOptions::default());
        let app = App::new(&ui, list.clone(), view.clone());
        list.main()
            .import(ExportedRepository::from_linear([user_message("hello")]))
            .unwrap();
        ui.show().unwrap();
        ui.window().take_snapshot().unwrap();
        app.focus_composer();
        settle();
        (ui, app, list, view)
    }

    /// Run the zero-delay focus timers, as the event loop would after the event.
    fn settle() {
        for _ in 0..10 {
            slint::platform::update_timers_and_animations();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn press(ui: &AppWindow, key: slint::platform::Key) {
        let text: slint::SharedString = key.into();
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
    }

    #[test]
    fn markdown_completions_and_copy_confirmation_reach_the_rows() {
        let (ui, app, list, view) = focused_app();
        list.main()
            .import(ExportedRepository::from_linear(crate::bench::history(4)))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !app.idle() && Instant::now() < deadline {
            slint::platform::update_timers_and_animations();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(
            app.idle(),
            "Markdown did not finish within the test deadline"
        );
        for index in [1, 3] {
            let part = app.rows.row_data(index).unwrap().parts.row_data(0).unwrap();
            assert!(
                part.markdown_enabled,
                "completed formatting must reach Slint"
            );
            assert!(part.markdown_blocks.row_count() > 0);
        }

        // These changes have no new message content, and the copied row is
        // not the last row in the thread.
        view.mark_copied(&MessageId("h0".into()), 0.0);
        assert!(app.rows.row_data(0).unwrap().copied);
        assert!(!app.rows.row_data(2).unwrap().copied);
        view.mark_copied(&MessageId("h2".into()), 100.0);
        assert!(!app.rows.row_data(0).unwrap().copied);
        assert!(app.rows.row_data(2).unwrap().copied);
        view.tick(2200.0);
        assert!(!app.rows.row_data(2).unwrap().copied);
        ui.hide().unwrap();
    }

    #[test]
    fn switching_threads_with_reused_message_ids_resets_host_changes() {
        let (ui, app, list, _view) = focused_app();
        let first = list.main_id();
        list.switch_to_new();
        list.main()
            .import(ExportedRepository::from_linear([user_message(
                "second thread",
            )]))
            .unwrap();
        let second = list.main_id();
        assert_eq!(first_part(&app).body, "second thread");

        // A queued presentation refresh belongs to the old conversation.
        app.invalidate_row(0);
        list.switch_to(&first);
        assert_eq!(first_part(&app).body, "hello");
        assert!(app.dirty_rows.borrow().is_empty());
        list.switch_to(&second);
        assert_eq!(first_part(&app).body, "second thread");
        assert_eq!(app.row_indices.borrow().get("large"), Some(&0));
        ui.hide().unwrap();
    }

    #[test]
    fn settings_drive_the_text_scale_global_the_column_width_and_the_keyboard_shortcuts() {
        use crate::AssistantTheme;
        use slint::platform::{Key, WindowEvent};
        let (ui, _app, _list, _view) = focused_app();
        let theme = ui.global::<AssistantTheme>();
        let _state = crate::settings::install(&ui, crate::settings::Settings::default(), None);
        let default_scale = crate::settings::SCALES[crate::settings::DEFAULT_STEP];
        assert_eq!(theme.get_text_scale(), default_scale);
        assert_eq!(ui.get_column_width(), 704.0 * default_scale);

        // Ctrl+= reaches the app through the focused composer.
        let key = |text: slint::SharedString| {
            ui.window()
                .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
            ui.window()
                .dispatch_event(WindowEvent::KeyReleased { text });
        };
        ui.window().dispatch_event(WindowEvent::KeyPressed {
            text: Key::Control.into(),
        });
        // The default is the large step (1.15): one step up is 1.3, two down is 1.0.
        key("=".into());
        assert_eq!(theme.get_text_scale(), 1.3);
        assert_eq!(ui.get_text_size_index(), 3);
        key("-".into());
        key("-".into());
        assert_eq!(theme.get_text_scale(), 1.0);
        key("0".into());
        assert_eq!(
            theme.get_text_scale(),
            crate::settings::SCALES[crate::settings::DEFAULT_STEP]
        );
        ui.window().dispatch_event(WindowEvent::KeyReleased {
            text: Key::Control.into(),
        });
        // The shortcut did not type into the composer.
        assert_eq!(ui.get_draft(), "");

        ui.invoke_text_size_selected(3);
        ui.invoke_reading_width_selected(2);
        ui.invoke_theme_selected(1);
        assert_eq!(theme.get_text_scale(), 1.3);
        // Wide, times the text scale of the large step.
        assert!((ui.get_column_width() - 880.0 * 1.3).abs() < 0.01);
        assert!(!theme.get_dark());
        ui.invoke_theme_selected(2);
        assert!(theme.get_dark());
        ui.hide().unwrap();
    }

    #[test]
    fn escape_in_edit_composer_cancels_and_returns_focus_to_the_composer() {
        let (ui, _app, _list, _view) = focused_app();
        assert!(
            ui.get_composer_focused(),
            "the composer is focused at start"
        );
        ui.invoke_begin_edit("large".into());
        ui.window().take_snapshot().unwrap();
        assert_eq!(ui.get_edit_token(), "large");
        assert!(!ui.get_composer_focused(), "the edit composer takes focus");
        press(&ui, slint::platform::Key::Escape);
        settle();
        assert_eq!(ui.get_edit_token(), "", "Escape cancels the edit");
        assert!(
            ui.get_composer_focused(),
            "focus returns to the main composer"
        );
        ui.hide().unwrap();
    }

    #[test]
    fn selecting_a_thread_returns_focus_to_the_composer() {
        let (ui, _app, list, _view) = focused_app();
        list.switch_to_new();
        ui.window().take_snapshot().unwrap();
        // Shift+Tab walks backwards from the composer into the sidebar.
        press(&ui, slint::platform::Key::Backtab);
        assert!(!ui.get_composer_focused(), "focus left the composer");
        ui.invoke_select_thread(0);
        settle();
        assert!(
            ui.get_composer_focused(),
            "selecting a thread focuses the composer"
        );
        ui.hide().unwrap();
    }

    #[test]
    fn new_thread_returns_focus_to_the_composer() {
        let (ui, _app, _list, _view) = focused_app();
        press(&ui, slint::platform::Key::Backtab);
        assert!(!ui.get_composer_focused(), "focus left the composer");
        ui.invoke_new_thread();
        settle();
        assert!(
            ui.get_composer_focused(),
            "a new thread focuses the composer"
        );
        ui.hide().unwrap();
    }

    /// Same source, renderer and window; only the user part's layout policy
    /// differs. Run alone so another software-platform test cannot interfere:
    /// `cargo test -p ai-ui-example-thread bench_large_user_layout -- --ignored --nocapture`
    #[test]
    #[ignore = "manual interleaved layout benchmark"]
    fn bench_large_user_layout() {
        let _window = crate::shot::install(900, 760);
        // A 512 KiB raw Text overflows the software renderer's coordinates.
        // Use a size both policies render correctly for a meaningful ratio;
        // the integration test above exercises a much larger paged source.
        let text = crate::bench::long_prompt(32);
        let source = Arc::new(());
        let mut raw_times = Vec::new();
        let mut paged_times = Vec::new();
        // Warm both paths, then alternate five pairs to reduce order effects.
        for iteration in 0..6 {
            for paged in if iteration % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            } {
                let ui = AppWindow::new().unwrap();
                let mut part = ChatPart {
                    id: "0".into(),
                    kind: ChatPartKind::Text,
                    ..Default::default()
                };
                let mut pages = Pages::new(1);
                if paged {
                    let page = pages.project("large", 0, &source, &text).unwrap();
                    part.text_paged = true;
                    part.text_page = page.text.into();
                    part.text_page_number = page.number as i32;
                    part.text_page_count = page.count as i32;
                } else {
                    part.body = text.as_str().into();
                }
                ui.set_messages(ModelRc::new(VecModel::from(vec![ChatRow {
                    id: "large".into(),
                    role: MessageRole::User,
                    width_hint: width_hint(&text).into(),
                    parts: ModelRc::new(VecModel::from(vec![part])),
                    ..Default::default()
                }])));
                ui.show().unwrap();
                let started = Instant::now();
                std::hint::black_box(ui.window().take_snapshot().unwrap());
                let elapsed = started.elapsed();
                if iteration > 0 {
                    if paged {
                        paged_times.push(elapsed);
                    } else {
                        raw_times.push(elapsed);
                    }
                }
                ui.hide().unwrap();
            }
        }
        raw_times.sort();
        paged_times.sort();
        eprintln!(
            "{} byte user message; 900x760 software renderer; median of 5 interleaved first layouts: raw {:?}, paged {:?}, ratio {:.2}x",
            text.len(), raw_times[2], paged_times[2], raw_times[2].as_secs_f64() / paged_times[2].as_secs_f64()
        );
    }
}
