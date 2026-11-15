//! The thread list, after upstream's `ThreadListRuntimeCore` and its remote
//! variant: many threads, one of them the main thread.
//!
//! Every thread is any [`ThreadRuntime`] the `factory` creates, so a list can
//! hold local and external-store threads alike. Changes apply immediately
//! (optimistically) and, if a [`ThreadListAdapter`] is given, are persisted by
//! background tasks handed to the host's [`Spawner`]; a failed task undoes
//! its change and is reported through [`ThreadList::last_error`].
//!
//! A new thread becomes a regular thread (and is registered with the adapter)
//! when its first message appears. After the first answer completes, the
//! adapter may generate a title.

use crate::repository::ExportedRepository;
use crate::runtime::ThreadRuntime;
use crate::subscribe::{Subscribers, Subscription};
use crate::types::ThreadMessage;
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::rc::{Rc, Weak};
use std::sync::Arc;

/// Runs background work on the host's executor (for example
/// `|f| { slint::spawn_local(f).ok(); }`).
pub type Spawner = Rc<dyn Fn(Pin<Box<dyn Future<Output = ()>>>)>;

/// The outcome of an adapter call; an error is a message for the user.
pub type AdapterFuture<T> = Pin<Box<dyn Future<Output = Result<T, String>>>>;

/// Where a thread is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadStatus {
    /// Not yet registered: no message has been sent.
    New,
    /// A stored thread in the main list.
    Regular,
    /// A stored thread that is hidden from the main list.
    Archived,
}

/// One entry of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadItem {
    /// Stable local id.
    pub id: String,
    /// The adapter's id, once known.
    pub remote_id: Option<String>,
    /// Whether the thread is new, regular or archived.
    pub status: ThreadStatus,
    /// The thread's title, once it has one.
    pub title: Option<String>,
}

/// A thread the adapter knows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteThread {
    /// The adapter's id for the thread.
    pub remote_id: String,
    /// The thread's title, if stored.
    pub title: Option<String>,
    /// Whether the thread is archived.
    pub archived: bool,
}

/// Persistence for the list (`RemoteThreadListAdapter`). Every method has an
/// inert default, so an adapter implements only what it stores.
pub trait ThreadListAdapter {
    /// The stored threads, newest first.
    fn list(&self) -> AdapterFuture<Vec<RemoteThread>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    /// Register a new thread; returns its remote id.
    fn initialize(&self, local_id: &str) -> AdapterFuture<String> {
        let id = local_id.to_owned();
        Box::pin(async move { Ok(id) })
    }
    /// Store a new title. The default does nothing.
    fn rename(&self, _remote_id: &str, _title: &str) -> AdapterFuture<()> {
        Box::pin(async { Ok(()) })
    }
    /// Mark the thread archived. The default does nothing.
    fn archive(&self, _remote_id: &str) -> AdapterFuture<()> {
        Box::pin(async { Ok(()) })
    }
    /// Restore an archived thread. The default does nothing.
    fn unarchive(&self, _remote_id: &str) -> AdapterFuture<()> {
        Box::pin(async { Ok(()) })
    }
    /// Delete the thread. The default does nothing.
    fn delete(&self, _remote_id: &str) -> AdapterFuture<()> {
        Box::pin(async { Ok(()) })
    }
    /// A title for a thread, from its messages.
    fn generate_title(
        &self,
        _remote_id: &str,
        _messages: Vec<Arc<ThreadMessage>>,
    ) -> AdapterFuture<Option<String>> {
        Box::pin(async { Ok(None) })
    }
    /// The stored history of a thread, loaded when it is first opened.
    fn load(&self, _remote_id: &str) -> AdapterFuture<ExportedRepository> {
        Box::pin(async { Ok(ExportedRepository::default()) })
    }
}

type Factory = dyn Fn(&str) -> Rc<dyn ThreadRuntime>;

struct Entry {
    item: ThreadItem,
    runtime: Option<Rc<dyn ThreadRuntime>>,
    /// Keeps the thread's change subscription alive.
    _subscription: Option<Subscription>,
    loaded: bool,
    initializing: bool,
    /// A title was already requested; asked at most once per thread.
    titled: bool,
}

struct State {
    entries: Vec<Entry>,
    main: String,
    error: Option<String>,
    loading: bool,
}

struct Inner {
    me: Weak<Inner>,
    factory: Box<Factory>,
    adapter: Option<Rc<dyn ThreadListAdapter>>,
    spawner: Spawner,
    state: RefCell<State>,
    subscribers: Rc<Subscribers>,
    next_id: Cell<u64>,
}

/// The user's threads, with one main thread shown at a time.
#[derive(Clone)]
pub struct ThreadList {
    inner: Rc<Inner>,
}

impl ThreadList {
    /// A list with one empty new thread as the main thread. With an adapter,
    /// the stored threads are fetched in the background.
    pub fn new(
        factory: impl Fn(&str) -> Rc<dyn ThreadRuntime> + 'static,
        spawner: Spawner,
        adapter: Option<Rc<dyn ThreadListAdapter>>,
    ) -> Self {
        let list = Self {
            inner: Rc::new_cyclic(|me| Inner {
                me: me.clone(),
                factory: Box::new(factory),
                adapter,
                spawner,
                state: RefCell::new(State {
                    entries: Vec::new(),
                    main: String::new(),
                    error: None,
                    loading: false,
                }),
                subscribers: Rc::default(),
                next_id: Cell::new(1),
            }),
        };
        let id = list.inner.add_new_entry();
        list.inner.state.borrow_mut().main = id;
        if list.inner.adapter.is_some() {
            list.refresh();
        }
        list
    }

    /// Called after every change. Drop the guard to unsubscribe.
    pub fn subscribe(&self, callback: impl Fn() + 'static) -> Subscription {
        self.inner.subscribers.subscribe(callback)
    }

    /// Every thread, in list order (new first, then as fetched or created).
    pub fn items(&self) -> Vec<ThreadItem> {
        self.inner
            .state
            .borrow()
            .entries
            .iter()
            .map(|e| e.item.clone())
            .collect()
    }
    /// The threads in the main list.
    pub fn regular(&self) -> Vec<ThreadItem> {
        self.with_status(ThreadStatus::Regular)
    }
    /// The archived threads.
    pub fn archived(&self) -> Vec<ThreadItem> {
        self.with_status(ThreadStatus::Archived)
    }
    fn with_status(&self, status: ThreadStatus) -> Vec<ThreadItem> {
        self.items()
            .into_iter()
            .filter(|i| i.status == status)
            .collect()
    }
    /// One thread, by local id.
    pub fn item(&self, id: &str) -> Option<ThreadItem> {
        self.items().into_iter().find(|i| i.id == id)
    }
    /// The local id of the thread being shown.
    pub fn main_id(&self) -> String {
        self.inner.state.borrow().main.clone()
    }
    /// The main thread's runtime, created on first use.
    pub fn main(&self) -> Rc<dyn ThreadRuntime> {
        let id = self.main_id();
        self.inner.runtime(&id).expect("the main thread exists")
    }
    /// The runtime of any thread, created on first use.
    pub fn runtime(&self, id: &str) -> Option<Rc<dyn ThreadRuntime>> {
        self.inner.runtime(id)
    }
    /// The adapter is fetching the stored threads.
    pub fn is_loading(&self) -> bool {
        self.inner.state.borrow().loading
    }
    /// The last background failure, until the next successful change.
    pub fn last_error(&self) -> Option<String> {
        self.inner.state.borrow().error.clone()
    }

    /// Fetch the stored threads (again) and add the ones not yet listed.
    pub fn refresh(&self) {
        let Some(adapter) = self.inner.adapter.clone() else {
            return;
        };
        self.inner.state.borrow_mut().loading = true;
        self.inner.notify();
        let weak = self.inner.me.clone();
        self.inner.spawn(async move {
            let result = adapter.list().await;
            let Some(inner) = weak.upgrade() else { return };
            {
                let mut state = inner.state.borrow_mut();
                state.loading = false;
                match result {
                    Ok(threads) => {
                        let mut added = Vec::new();
                        for thread in threads {
                            if state
                                .entries
                                .iter()
                                .any(|e| e.item.remote_id.as_deref() == Some(&thread.remote_id))
                            {
                                continue;
                            }
                            added.push(thread);
                        }
                        for thread in added {
                            let id = format!("thread-{}", inner.next_id.get());
                            inner.next_id.set(inner.next_id.get() + 1);
                            state.entries.push(Entry {
                                item: ThreadItem {
                                    id,
                                    remote_id: Some(thread.remote_id),
                                    status: if thread.archived {
                                        ThreadStatus::Archived
                                    } else {
                                        ThreadStatus::Regular
                                    },
                                    title: thread.title,
                                },
                                runtime: None,
                                _subscription: None,
                                loaded: false,
                                initializing: false,
                                titled: false,
                            });
                        }
                    }
                    Err(error) => state.error = Some(error),
                }
            }
            inner.notify();
        });
    }

    /// Start a fresh thread, or return to the empty one already there.
    pub fn switch_to_new(&self) {
        let existing = {
            let state = self.inner.state.borrow();
            state
                .entries
                .iter()
                .find(|e| e.item.status == ThreadStatus::New)
                .map(|e| e.item.id.clone())
        };
        let id = match existing {
            Some(id) => id,
            None => self.inner.add_new_entry(),
        };
        self.switch_to(&id);
    }

    /// Make a thread the main thread; a stored thread loads its history.
    pub fn switch_to(&self, id: &str) {
        let known = self
            .inner
            .state
            .borrow()
            .entries
            .iter()
            .any(|e| e.item.id == id);
        if !known {
            return;
        }
        self.inner.state.borrow_mut().main = id.to_owned();
        self.inner.load_history(id);
        self.inner.notify();
    }

    /// Rename a thread. The change shows at once and is undone if the adapter fails.
    pub fn rename(&self, id: &str, title: &str) {
        let title = title.trim().to_owned();
        let (previous, remote) = {
            let mut state = self.inner.state.borrow_mut();
            let Some(entry) = state.entries.iter_mut().find(|e| e.item.id == id) else {
                return;
            };
            let previous = std::mem::replace(
                &mut entry.item.title,
                (!title.is_empty()).then(|| title.clone()),
            );
            (previous, entry.item.remote_id.clone())
        };
        self.inner.notify();
        if let (Some(adapter), Some(remote)) = (self.inner.adapter.clone(), remote) {
            let id = id.to_owned();
            self.inner
                .persist(adapter.rename(&remote, &title), move |inner| {
                    inner.with_entry(&id, |e| e.item.title = previous);
                });
        }
    }

    /// Archive a thread. The change shows at once and is undone if the adapter fails.
    pub fn archive(&self, id: &str) {
        self.set_archived(id, true);
    }
    /// Unarchive a thread. The change shows at once and is undone if the adapter fails.
    pub fn unarchive(&self, id: &str) {
        self.set_archived(id, false);
    }
    fn set_archived(&self, id: &str, archived: bool) {
        let (previous, remote) = {
            let mut state = self.inner.state.borrow_mut();
            let Some(entry) = state.entries.iter_mut().find(|e| e.item.id == id) else {
                return;
            };
            let wanted = if archived {
                ThreadStatus::Archived
            } else {
                ThreadStatus::Regular
            };
            if entry.item.status != ThreadStatus::Regular
                && entry.item.status != ThreadStatus::Archived
            {
                return;
            }
            let previous = std::mem::replace(&mut entry.item.status, wanted);
            (previous, entry.item.remote_id.clone())
        };
        // Archiving the open thread moves on to a new one.
        if archived && self.main_id() == id {
            self.switch_to_new();
        }
        self.inner.notify();
        if let (Some(adapter), Some(remote)) = (self.inner.adapter.clone(), remote) {
            let id = id.to_owned();
            let future = if archived {
                adapter.archive(&remote)
            } else {
                adapter.unarchive(&remote)
            };
            self.inner.persist(future, move |inner| {
                inner.with_entry(&id, |e| e.item.status = previous);
            });
        }
    }

    /// Delete a thread. The change shows at once and is undone if the adapter fails.
    pub fn delete(&self, id: &str) {
        let removed = {
            let mut state = self.inner.state.borrow_mut();
            let Some(index) = state.entries.iter().position(|e| e.item.id == id) else {
                return;
            };
            (index, state.entries.remove(index))
        };
        if self.main_id() == id {
            self.switch_to_new();
        } else {
            self.inner.notify();
        }
        if let (Some(adapter), Some(remote)) =
            (self.inner.adapter.clone(), removed.1.item.remote_id.clone())
        {
            let (index, mut entry) = removed;
            // A failed delete puts the thread back (without its live runtime).
            entry.runtime = None;
            entry._subscription = None;
            entry.loaded = false;
            self.inner.persist(adapter.delete(&remote), move |inner| {
                let mut state = inner.state.borrow_mut();
                let index = index.min(state.entries.len());
                state.entries.insert(index, entry);
            });
        }
    }
}

impl Inner {
    fn notify(&self) {
        self.subscribers.notify();
    }

    fn spawn(&self, future: impl Future<Output = ()> + 'static) {
        (self.spawner)(Box::pin(future));
    }

    fn with_entry(&self, id: &str, change: impl FnOnce(&mut Entry)) {
        if let Some(entry) = self
            .state
            .borrow_mut()
            .entries
            .iter_mut()
            .find(|e| e.item.id == id)
        {
            change(entry);
        }
    }

    fn add_new_entry(&self) -> String {
        let id = format!("thread-{}", self.next_id.get());
        self.next_id.set(self.next_id.get() + 1);
        self.state.borrow_mut().entries.insert(
            0,
            Entry {
                item: ThreadItem {
                    id: id.clone(),
                    remote_id: None,
                    status: ThreadStatus::New,
                    title: None,
                },
                runtime: None,
                _subscription: None,
                loaded: true,
                initializing: false,
                titled: false,
            },
        );
        id
    }

    /// Run an adapter task in the background; `undo` reverts the change if it fails.
    fn persist(&self, task: AdapterFuture<()>, undo: impl FnOnce(&Inner) + 'static) {
        let weak = self.me.clone();
        self.spawn(async move {
            let result = task.await;
            let Some(inner) = weak.upgrade() else { return };
            match result {
                Ok(()) => inner.state.borrow_mut().error = None,
                Err(error) => {
                    undo(&inner);
                    inner.state.borrow_mut().error = Some(error);
                }
            }
            inner.notify();
        });
    }

    /// The thread's runtime, created (and observed) on first use.
    fn runtime(&self, id: &str) -> Option<Rc<dyn ThreadRuntime>> {
        if let Some(runtime) = self
            .state
            .borrow()
            .entries
            .iter()
            .find(|e| e.item.id == id)
            .map(|e| e.runtime.clone())?
        {
            return Some(runtime);
        }
        let runtime = (self.factory)(id);
        let weak = self.me.clone();
        let thread = id.to_owned();
        let subscription = runtime.subscribe_dyn(Box::new(move || {
            if let Some(inner) = weak.upgrade() {
                inner.thread_changed(&thread);
            }
        }));
        let mut state = self.state.borrow_mut();
        let entry = state.entries.iter_mut().find(|e| e.item.id == id)?;
        entry.runtime = Some(runtime.clone());
        entry._subscription = Some(subscription);
        Some(runtime)
    }

    /// Load a stored thread's history the first time it is opened.
    fn load_history(&self, id: &str) {
        let Some(adapter) = self.adapter.clone() else {
            let _ = self.runtime(id);
            return;
        };
        let remote = {
            let mut state = self.state.borrow_mut();
            let Some(entry) = state.entries.iter_mut().find(|e| e.item.id == id) else {
                return;
            };
            match (&entry.item.remote_id, entry.loaded) {
                (Some(remote), false) => {
                    entry.loaded = true;
                    Some(remote.clone())
                }
                _ => None,
            }
        };
        let Some(runtime) = self.runtime(id) else {
            return;
        };
        let Some(remote) = remote else { return };
        let (weak, id) = (self.me.clone(), id.to_owned());
        self.spawn(async move {
            let result = adapter.load(&remote).await;
            let Some(inner) = weak.upgrade() else { return };
            match result {
                Ok(history) => {
                    // A thread the user already started is not overwritten.
                    if runtime.messages().is_empty() && runtime.import(history).is_err() {
                        inner.state.borrow_mut().error = Some("could not load the thread".into());
                    }
                }
                Err(error) => {
                    inner.with_entry(&id, |e| e.loaded = false);
                    inner.state.borrow_mut().error = Some(error);
                }
            }
            inner.notify();
        });
    }

    /// A thread's runtime changed: register it on its first message and ask
    /// for a title after its first answer.
    fn thread_changed(&self, id: &str) {
        let Some(runtime) = self
            .state
            .borrow()
            .entries
            .iter()
            .find(|e| e.item.id == id)
            .and_then(|e| e.runtime.clone())
        else {
            return;
        };
        let messages = runtime.messages();
        let running = runtime.is_running();
        let (register, title_for) = {
            let mut state = self.state.borrow_mut();
            let Some(entry) = state.entries.iter_mut().find(|e| e.item.id == id) else {
                return;
            };
            let mut register = false;
            if entry.item.status == ThreadStatus::New && !messages.is_empty() {
                entry.item.status = ThreadStatus::Regular;
                register = !entry.initializing && entry.item.remote_id.is_none();
                entry.initializing |= register;
            }
            let answered = !running
                && messages
                    .iter()
                    .any(|m| m.as_assistant().is_some_and(|a| a.status.is_terminal()));
            let title_for =
                (answered && entry.item.title.is_none() && !entry.titled && !entry.initializing)
                    .then(|| entry.item.remote_id.clone())
                    .flatten();
            if title_for.is_some() {
                entry.titled = true;
            }
            (register, title_for)
        };
        if register {
            self.initialize(id);
        }
        if let Some(remote) = title_for {
            self.generate_title(id, &remote, messages.iter().cloned().collect());
        }
        self.notify();
    }

    fn initialize(&self, id: &str) {
        let Some(adapter) = self.adapter.clone() else {
            self.with_entry(id, |e| e.initializing = false);
            return;
        };
        let (weak, id) = (self.me.clone(), id.to_owned());
        self.spawn(async move {
            let result = adapter.initialize(&id).await;
            let Some(inner) = weak.upgrade() else { return };
            match result {
                Ok(remote) => inner.with_entry(&id, |e| {
                    e.item.remote_id = Some(remote);
                    e.initializing = false;
                }),
                Err(error) => {
                    inner.with_entry(&id, |e| e.initializing = false);
                    inner.state.borrow_mut().error = Some(error);
                }
            }
            // The thread may already have been answered while registering.
            inner.thread_changed(&id);
            inner.notify();
        });
    }

    fn generate_title(&self, id: &str, remote: &str, messages: Vec<Arc<ThreadMessage>>) {
        let Some(adapter) = self.adapter.clone() else {
            return;
        };
        let (weak, id, remote) = (self.me.clone(), id.to_owned(), remote.to_owned());
        self.spawn(async move {
            let result = adapter.generate_title(&remote, messages).await;
            let Some(inner) = weak.upgrade() else { return };
            match result {
                Ok(Some(title)) => {
                    inner.with_entry(&id, |e| {
                        if e.item.title.is_none() {
                            e.item.title = Some(title.clone());
                        }
                    });
                    let persist = adapter.rename(&remote, &title);
                    inner.persist(persist, |_| {});
                }
                Ok(None) => {}
                Err(error) => inner.state.borrow_mut().error = Some(error),
            }
            inner.notify();
        });
    }
}
