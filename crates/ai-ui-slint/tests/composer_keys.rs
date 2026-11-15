//! The main composer's keyboard contract: Enter sends, Shift+Enter inserts a
//! newline, Enter does nothing while a run is in progress, and Escape stops a
//! running run. Keys are dispatched offscreen to the focused composer.
use slint::platform::{
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    Key, Platform, WindowAdapter, WindowEvent,
};
use slint::{ComponentHandle, PhysicalSize};
use std::{cell::RefCell, rc::Rc};

slint::slint! {
    import { AssistantComposer } from "../ui/lib.slint";
    import { RunReference } from "../ui/models.slint";
    export { RunReference }
    export component ComposerFixture inherits Window {
        width: 480px; height: 160px;
        in property <bool> busy;
        in-out property <string> draft;
        callback submit(string) -> bool;
        callback cancel(RunReference);
        out property <bool> focused: composer.has-focus;
        public function focus-composer() { composer.focus-editor(); }
        composer := AssistantComposer {
            width: parent.width; height: parent.height;
            busy: root.busy;
            draft <=> root.draft;
            submit(text) => { return root.submit(text); }
            cancel(run) => { root.cancel(run); }
        }
    }
}

struct Software;
impl Platform for Software {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer))
    }
}

/// A shown fixture with its composer focused, and a log of submit and cancel calls.
fn fixture(draft: &str, busy: bool) -> (ComposerFixture, Rc<RefCell<Vec<String>>>) {
    let _ = slint::platform::set_platform(Box::new(Software));
    let ui = ComposerFixture::new().unwrap();
    ui.window().set_size(PhysicalSize::new(480, 160));
    ui.set_draft(draft.into());
    ui.set_busy(busy);
    ui.show().unwrap();
    let log = Rc::new(RefCell::new(Vec::new()));
    let (submitted, cancelled) = (log.clone(), log.clone());
    ui.on_submit(move |text| {
        submitted.borrow_mut().push(format!("submit {text}"));
        true
    });
    ui.on_cancel(move |_| cancelled.borrow_mut().push("cancel".to_string()));
    ui.invoke_focus_composer();
    (ui, log)
}

fn key(ui: &ComposerFixture, text: impl Into<slint::SharedString>) {
    let text = text.into();
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn type_text(ui: &ComposerFixture, text: &str) {
    for c in text.chars() {
        key(ui, c.to_string());
    }
}

#[test]
fn enter_sends_the_draft() {
    let (ui, log) = fixture("", false);
    type_text(&ui, "hello");
    key(&ui, Key::Return);
    assert_eq!(*log.borrow(), vec!["submit hello".to_string()]);
    assert_eq!(ui.get_draft(), "", "a sent draft is cleared");
}

#[test]
fn shift_enter_inserts_a_newline_and_does_not_send() {
    let (ui, log) = fixture("", false);
    type_text(&ui, "a");
    ui.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Shift.into(),
    });
    key(&ui, Key::Return);
    ui.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Shift.into(),
    });
    type_text(&ui, "b");
    assert!(
        log.borrow().is_empty(),
        "Shift+Enter must not send: {:?}",
        log.borrow()
    );
    assert_eq!(ui.get_draft(), "a\nb");
}

#[test]
fn enter_while_running_does_not_send_or_insert_a_newline() {
    let (ui, log) = fixture("pending", true);
    key(&ui, Key::Return);
    assert!(log.borrow().is_empty(), "Enter during a run must not send");
    assert_eq!(
        ui.get_draft(),
        "pending",
        "Enter during a run must not add a newline"
    );
}

#[test]
fn typing_while_running_edits_the_draft() {
    let (ui, log) = fixture("", true);
    type_text(&ui, "hi");
    ui.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Shift.into(),
    });
    key(&ui, Key::Return);
    ui.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Shift.into(),
    });
    type_text(&ui, "yo");
    assert!(log.borrow().is_empty(), "typing during a run must not send");
    assert_eq!(
        ui.get_draft(),
        "hi
yo"
    );
}

#[test]
fn escape_while_running_cancels_the_run() {
    let (ui, log) = fixture("", true);
    key(&ui, Key::Escape);
    assert_eq!(*log.borrow(), vec!["cancel".to_string()]);
}

#[test]
fn escape_when_idle_does_nothing() {
    let (ui, log) = fixture("draft", false);
    key(&ui, Key::Escape);
    assert!(
        log.borrow().is_empty(),
        "Escape with no run must not cancel"
    );
    assert_eq!(ui.get_draft(), "draft");
}

#[test]
fn composer_keeps_focus_while_running_so_escape_reaches_it() {
    let (ui, _log) = fixture("", true);
    assert!(ui.get_focused(), "the scope keeps focus during a run");
}
