//! The external-store runtime: the app owns the messages.

use ai_ui_core::*;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Default)]
struct Log {
    new: RefCell<Vec<AppendMessage>>,
    edits: RefCell<Vec<AppendMessage>>,
    branch_changes: RefCell<Vec<Option<String>>>,
    cancels: RefCell<u32>,
}

struct App {
    log: Rc<Log>,
    can_edit: bool,
}

impl ExternalStore for App {
    fn on_new(&self, message: AppendMessage) -> StoreFuture {
        self.log.new.borrow_mut().push(message);
        Box::pin(async { Ok(()) })
    }
    fn on_edit(&self, message: AppendMessage) -> Option<StoreFuture> {
        self.can_edit.then(|| {
            self.log.edits.borrow_mut().push(message);
            Box::pin(async { Ok(()) }) as StoreFuture
        })
    }
    fn on_cancel(&self) {
        *self.log.cancels.borrow_mut() += 1;
    }
    fn on_branch_change(&self, head: Option<&MessageId>) {
        self.log
            .branch_changes
            .borrow_mut()
            .push(head.map(|h| h.0.clone()));
    }
    fn capabilities(&self) -> RuntimeCapabilities {
        RuntimeCapabilities {
            edit: self.can_edit,
            cancel: true,
            ..ExternalStore::capabilities(&Basic)
        }
    }
}

struct Basic;
impl ExternalStore for Basic {
    fn on_new(&self, _: AppendMessage) -> StoreFuture {
        Box::pin(async { Ok(()) })
    }
}

fn message(role: &str, id: &str, text: &str) -> ThreadMessage {
    let mut value = serde_json::json!({ "role": role, "id": id, "createdAt": 0, "content": [{ "type": "text", "text": text }] });
    if role == "assistant" {
        value["status"] = serde_json::json!({ "type": "complete", "reason": "stop" });
    }
    serde_json::from_value(value).unwrap()
}

fn runtime(can_edit: bool) -> (ExternalStoreRuntime, Rc<Log>) {
    let log = Rc::new(Log::default());
    (
        ExternalStoreRuntime::new(App {
            log: log.clone(),
            can_edit,
        }),
        log,
    )
}

fn run(future: RunFuture) -> Result<(), RunError> {
    futures_executor::block_on(future)
}

#[test]
fn shows_the_apps_messages_and_keeps_unchanged_ones_identical() {
    let (runtime, _) = runtime(false);
    let heard = Rc::new(RefCell::new(0));
    let _sub = {
        let heard = heard.clone();
        runtime.subscribe(move || *heard.borrow_mut() += 1)
    };
    runtime.set_messages([
        message("user", "u1", "Hi"),
        message("assistant", "a1", "Hello"),
    ]);
    let first = runtime.messages();
    assert_eq!(
        first.iter().map(|m| m.text()).collect::<Vec<_>>(),
        ["Hi", "Hello"]
    );

    // The app streams a longer answer: only the last message changes.
    runtime.set_running(true);
    runtime.set_messages([
        message("user", "u1", "Hi"),
        message("assistant", "a1", "Hello there"),
    ]);
    let second = runtime.messages();
    assert!(
        Arc::ptr_eq(&first[0], &second[0]),
        "an unchanged message keeps its Arc"
    );
    assert!(!Arc::ptr_eq(&first[1], &second[1]));
    assert_eq!(second[1].text(), "Hello there");
    assert!(runtime.is_running());
    assert!(*heard.borrow() >= 3);
}

use std::sync::Arc;

#[test]
fn the_composer_calls_on_new_with_the_right_parent_and_inserts_nothing() {
    let (runtime, log) = runtime(false);
    runtime.set_messages([
        message("user", "u1", "Hi"),
        message("assistant", "a1", "Hello"),
    ]);
    let composer = runtime.composer();
    composer.set_text("And you?");
    run(composer.send().unwrap().unwrap()).unwrap();
    let sent = log.new.borrow();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].parent_id, Some("a1".into()));
    assert!(sent[0].starts_run());
    assert_eq!(
        runtime.messages().len(),
        2,
        "the app decides what the thread shows"
    );
}

#[test]
fn edits_use_on_edit_when_available_and_on_new_otherwise() {
    for can_edit in [true, false] {
        let (runtime, log) = runtime(can_edit);
        runtime.set_messages([
            message("user", "u1", "Hi"),
            message("assistant", "a1", "Hello"),
        ]);
        let edit = runtime.begin_edit(&"u1".into()).unwrap();
        edit.set_text("Hello again");
        run(edit.send().unwrap().unwrap()).unwrap();
        assert_eq!(log.edits.borrow().len(), usize::from(can_edit));
        assert_eq!(log.new.borrow().len(), usize::from(!can_edit));
        let sent = if can_edit {
            log.edits.borrow()[0].clone()
        } else {
            log.new.borrow()[0].clone()
        };
        assert_eq!((sent.parent_id, sent.source_id), (None, Some("u1".into())));
        assert!(
            runtime.edit_composer(&"u1".into()).is_none(),
            "sending ends the edit"
        );
    }
}

#[test]
fn branches_switch_locally_and_the_app_is_told() {
    let (runtime, log) = runtime(false);
    let messages = ["u1", "a1", "a2"]
        .iter()
        .enumerate()
        .map(|(i, id)| ExportedMessage {
            message: Arc::new(message(if i == 0 { "user" } else { "assistant" }, id, id)),
            parent_id: (i > 0).then(|| "u1".into()),
            run_config: None,
        })
        .collect();
    runtime.set_repository(ExportedRepository {
        head_id: Some(Some("a2".into())),
        messages,
    });
    assert_eq!(runtime.branches(&"a1".into()).unwrap().len(), 2);
    assert_eq!(runtime.messages().last().unwrap().id().0, "a2");
    runtime.switch_to_branch(&"a1".into()).unwrap();
    assert_eq!(runtime.messages().last().unwrap().id().0, "a1");
    assert_eq!(*log.branch_changes.borrow(), [Some("a1".to_owned())]);
}

#[test]
fn unsupported_actions_are_reported_and_cancel_reaches_the_app() {
    let (runtime, log) = runtime(false);
    runtime.set_messages([
        message("user", "u1", "Hi"),
        message("assistant", "a1", "Hello"),
    ]);
    assert_eq!(
        runtime.reload(StartRunConfig::default()).err(),
        Some(RuntimeError::Unsupported("reload"))
    );
    assert_eq!(
        runtime.submit_feedback(&"a1".into(), None).err(),
        Some(RuntimeError::Unsupported("feedback"))
    );
    runtime.cancel_run();
    runtime.composer().cancel();
    assert_eq!(*log.cancels.borrow(), 2);
    assert!(runtime.capabilities().cancel && !runtime.capabilities().edit);
    runtime.set_disabled(true);
    runtime.composer().set_text("x");
    assert!(
        runtime.composer().send().is_err(),
        "a disabled thread does not send"
    );
    assert_eq!(runtime.composer().text(), "x", "the draft is kept");
}
