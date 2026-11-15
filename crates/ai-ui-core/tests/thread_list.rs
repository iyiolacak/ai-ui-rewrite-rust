//! The thread list: switching, optimistic persistence with rollback, history
//! loading and title generation.

use ai_ui_core::adapter::{self, RunStream};
use ai_ui_core::*;
use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Waker};

/// Background tasks, run to completion on demand.
type Task = Pin<Box<dyn Future<Output = ()>>>;

#[derive(Clone, Default)]
struct Tasks(Rc<RefCell<Vec<Task>>>);

impl Tasks {
    fn spawner(&self) -> Spawner {
        let tasks = self.0.clone();
        Rc::new(move |f| tasks.borrow_mut().push(f))
    }
    fn run(&self) {
        let mut cx = Context::from_waker(Waker::noop());
        loop {
            let next = {
                let mut tasks = self.0.borrow_mut();
                (!tasks.is_empty()).then(|| tasks.remove(0))
            };
            let Some(mut task) = next else { return };
            if task.as_mut().poll(&mut cx).is_pending() {
                self.0.borrow_mut().push(task);
            }
        }
    }
}

#[derive(Default)]
struct Store {
    calls: RefCell<Vec<String>>,
    stored: RefCell<Vec<RemoteThread>>,
    fail: RefCell<bool>,
    title: RefCell<Option<String>>,
}

struct Adapter(Rc<Store>);

impl Adapter {
    fn record<T: 'static>(&self, call: String, ok: T) -> AdapterFuture<T> {
        self.0.calls.borrow_mut().push(call);
        let fail = *self.0.fail.borrow();
        Box::pin(async move {
            if fail {
                Err("offline".into())
            } else {
                Ok(ok)
            }
        })
    }
}

impl ThreadListAdapter for Adapter {
    fn list(&self) -> AdapterFuture<Vec<RemoteThread>> {
        self.record("list".into(), self.0.stored.borrow().clone())
    }
    fn initialize(&self, local_id: &str) -> AdapterFuture<String> {
        self.record(
            format!("initialize {local_id}"),
            format!("remote-{local_id}"),
        )
    }
    fn rename(&self, remote_id: &str, title: &str) -> AdapterFuture<()> {
        self.record(format!("rename {remote_id} {title}"), ())
    }
    fn archive(&self, remote_id: &str) -> AdapterFuture<()> {
        self.record(format!("archive {remote_id}"), ())
    }
    fn delete(&self, remote_id: &str) -> AdapterFuture<()> {
        self.record(format!("delete {remote_id}"), ())
    }
    fn generate_title(
        &self,
        remote_id: &str,
        _: Vec<std::sync::Arc<ThreadMessage>>,
    ) -> AdapterFuture<Option<String>> {
        self.record(format!("title {remote_id}"), self.0.title.borrow().clone())
    }
    fn load(&self, remote_id: &str) -> AdapterFuture<ExportedRepository> {
        let message = |role: &str, id: &str, text: &str| -> ThreadMessage {
            let mut v = serde_json::json!({ "role": role, "id": id, "createdAt": 0, "content": [{ "type": "text", "text": text }] });
            if role == "assistant" {
                v["status"] = serde_json::json!({ "type": "complete", "reason": "stop" });
            }
            serde_json::from_value(v).unwrap()
        };
        self.record(
            format!("load {remote_id}"),
            ExportedRepository::from_linear([
                message("user", "h1", "stored question"),
                message("assistant", "h2", "stored answer"),
            ]),
        )
    }
}

fn setup(adapter: bool) -> (ThreadList, Tasks, Rc<Store>) {
    let tasks = Tasks::default();
    let store = Rc::new(Store::default());
    let list = ThreadList::new(
        |_id| -> Rc<dyn ThreadRuntime> {
            Rc::new(LocalThreadRuntime::new(
                |_o: ChatModelRunOptions| -> RunStream {
                    adapter::from_results([Ok(ChatModelRunResult::text("answer"))])
                },
                LocalRuntimeOptions::default(),
            ))
        },
        tasks.spawner(),
        adapter.then(|| Rc::new(Adapter(store.clone())) as Rc<dyn ThreadListAdapter>),
    );
    (list, tasks, store)
}

fn send(list: &ThreadList, tasks: &Tasks, text: &str) {
    let composer = list.main().composer();
    composer.set_text(text);
    futures_executor::block_on(composer.send().unwrap().unwrap()).unwrap();
    tasks.run();
}

fn calls(store: &Store) -> Vec<String> {
    store.calls.borrow().clone()
}

#[test]
fn starts_with_one_empty_thread_that_registers_on_its_first_message() {
    let (list, tasks, store) = setup(true);
    tasks.run();
    let first = list.main_id();
    assert_eq!(list.items().len(), 1);
    assert_eq!(list.item(&first).unwrap().status, ThreadStatus::New);
    assert!(list.regular().is_empty());

    send(&list, &tasks, "Hello");
    let item = list.item(&first).unwrap();
    assert_eq!(item.status, ThreadStatus::Regular);
    assert_eq!(item.remote_id, Some(format!("remote-{first}")));
    assert_eq!(
        calls(&store)
            .iter()
            .filter(|c| c.starts_with("initialize"))
            .count(),
        1
    );
}

#[test]
fn a_new_thread_is_reused_while_empty_and_old_threads_keep_their_messages() {
    let (list, tasks, _) = setup(false);
    let first = list.main_id();
    send(&list, &tasks, "one");
    list.switch_to_new();
    let second = list.main_id();
    assert_ne!(first, second);
    list.switch_to_new();
    assert_eq!(list.main_id(), second, "no pile of empty threads");
    assert_eq!(list.items().len(), 2);
    list.switch_to(&first);
    assert_eq!(list.main().messages()[0].text(), "one");
}

#[test]
fn rename_is_immediate_and_rolled_back_when_persisting_fails() {
    let (list, tasks, store) = setup(true);
    send(&list, &tasks, "Hello");
    let id = list.main_id();
    list.rename(&id, "Plans");
    assert_eq!(
        list.item(&id).unwrap().title.as_deref(),
        Some("Plans"),
        "applied before the adapter answers"
    );
    tasks.run();
    assert!(calls(&store).contains(&format!("rename remote-{id} Plans")));
    assert_eq!(list.last_error(), None);

    *store.fail.borrow_mut() = true;
    list.rename(&id, "Other");
    tasks.run();
    assert_eq!(
        list.item(&id).unwrap().title.as_deref(),
        Some("Plans"),
        "undone"
    );
    assert_eq!(list.last_error().as_deref(), Some("offline"));
}

#[test]
fn archiving_the_open_thread_moves_on_and_delete_can_be_undone() {
    let (list, tasks, store) = setup(true);
    send(&list, &tasks, "Hello");
    let id = list.main_id();
    list.archive(&id);
    assert_eq!(list.item(&id).unwrap().status, ThreadStatus::Archived);
    assert_ne!(
        list.main_id(),
        id,
        "the open thread was archived, so a new one opens"
    );
    assert_eq!(list.archived().len(), 1);
    list.unarchive(&id);
    assert_eq!(list.regular().len(), 1);
    tasks.run();

    *store.fail.borrow_mut() = true;
    list.delete(&id);
    assert!(list.item(&id).is_none());
    tasks.run();
    assert!(
        list.item(&id).is_some(),
        "a failed delete restores the thread"
    );
    *store.fail.borrow_mut() = false;
    list.delete(&id);
    tasks.run();
    assert!(list.item(&id).is_none());
}

#[test]
fn stored_threads_are_listed_and_load_their_history_when_opened() {
    let (list, tasks, store) = setup(true);
    store.stored.borrow_mut().extend([
        RemoteThread {
            remote_id: "r1".into(),
            title: Some("Old chat".into()),
            archived: false,
        },
        RemoteThread {
            remote_id: "r2".into(),
            title: None,
            archived: true,
        },
    ]);
    list.refresh();
    assert!(list.is_loading());
    tasks.run();
    assert!(!list.is_loading());
    assert_eq!(list.regular().len(), 1);
    assert_eq!(list.archived().len(), 1);
    list.refresh();
    tasks.run();
    assert_eq!(
        list.items().len(),
        3,
        "refreshing does not duplicate threads"
    );

    let old = list.regular()[0].id.clone();
    assert!(list.runtime(&old).unwrap().messages().is_empty());
    list.switch_to(&old);
    tasks.run();
    assert_eq!(
        list.main()
            .messages()
            .iter()
            .map(|m| m.text())
            .collect::<Vec<_>>(),
        ["stored question", "stored answer"]
    );
    list.switch_to(&list.items()[0].id.clone());
    list.switch_to(&old);
    tasks.run();
    assert_eq!(
        calls(&store)
            .iter()
            .filter(|c| c.starts_with("load"))
            .count(),
        1,
        "history loads once"
    );
}

#[test]
fn a_title_is_requested_once_after_the_first_answer() {
    let (list, tasks, store) = setup(true);
    *store.title.borrow_mut() = Some("Greetings".into());
    send(&list, &tasks, "Hello");
    tasks.run();
    let id = list.main_id();
    assert_eq!(list.item(&id).unwrap().title.as_deref(), Some("Greetings"));
    assert!(
        calls(&store).contains(&format!("rename remote-{id} Greetings")),
        "the title is stored"
    );
    send(&list, &tasks, "More");
    assert_eq!(
        calls(&store)
            .iter()
            .filter(|c| c.starts_with("title"))
            .count(),
        1
    );
}
