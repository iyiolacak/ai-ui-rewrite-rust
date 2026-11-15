//! The thread list reports the index of the row acted on: a click selects it,
//! and its "more" menu renames, archives or deletes it. Rename commits on Enter
//! and cancels on Escape.
use slint::platform::{
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    Key, Platform, PointerEventButton, WindowAdapter, WindowEvent,
};
use slint::{ComponentHandle, LogicalPosition, ModelRc, PhysicalSize, VecModel};
use std::{cell::RefCell, rc::Rc};

slint::slint! {
    import { AssistantThreadList } from "../ui/lib.slint";
    import { AssistantThreadListItem } from "../ui/components/thread-list.slint";
    export { AssistantThreadListItem }
    export component ThreadListFixture inherits Window {
        width: 256px; height: 320px;
        in property <[AssistantThreadListItem]> items;
        in property <int> selected-index: -1;
        callback select(int);
        callback rename(int, string);
        callback archive(int);
        callback delete(int);
        AssistantThreadList {
            width: parent.width; height: parent.height;
            items: root.items;
            selected-index: root.selected-index;
            select(index) => { root.select(index); }
            rename(index, text) => { root.rename(index, text); }
            archive(index) => { root.archive(index); }
            delete(index) => { root.delete(index); }
        }
    }
}
struct Software;
impl Platform for Software {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer))
    }
}
fn items(titles: &[&str]) -> ModelRc<AssistantThreadListItem> {
    ModelRc::new(VecModel::from(
        titles
            .iter()
            .enumerate()
            .map(|(i, title)| AssistantThreadListItem {
                id: format!("thread-{i}").into(),
                title: (*title).into(),
                running: false,
                detail: "".into(),
            })
            .collect::<Vec<_>>(),
    ))
}
/// A fixture with three threads, the middle one selected, and a log of callbacks.
fn fixture() -> (ThreadListFixture, Rc<RefCell<Vec<String>>>) {
    slint::platform::set_platform(Box::new(Software)).unwrap();
    let ui = ThreadListFixture::new().unwrap();
    ui.window().set_size(PhysicalSize::new(256, 320));
    ui.set_items(items(&["Alpha", "Beta", "Gamma"]));
    ui.set_selected_index(1);
    ui.show().unwrap();
    let log = Rc::new(RefCell::new(Vec::new()));
    let (a, b, c, d) = (log.clone(), log.clone(), log.clone(), log.clone());
    ui.on_select(move |index| a.borrow_mut().push(format!("select {index}")));
    ui.on_rename(move |index, text| b.borrow_mut().push(format!("rename {index} {text}")));
    ui.on_archive(move |index| c.borrow_mut().push(format!("archive {index}")));
    ui.on_delete(move |index| d.borrow_mut().push(format!("delete {index}")));
    (ui, log)
}
fn click(ui: &ThreadListFixture, x: f32, y: f32) {
    let position = LogicalPosition::new(x, y);
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
}
fn key(ui: &ThreadListFixture, text: impl Into<slint::SharedString>) {
    let text = text.into();
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}
// Geometry of the fixture (window 256x320): the second row (index 1) spans
// y 122..154 and its "more" button sits at x 218..242. The popup opens below
// that row, and its items are 32px tall starting 4px into the popup.
const MORE: (f32, f32) = (230.0, 138.0);
const MENU_RENAME: (f32, f32) = (172.0, 174.0);
const MENU_ARCHIVE: (f32, f32) = (172.0, 206.0);
const MENU_DELETE: (f32, f32) = (172.0, 238.0);

#[test]
fn clicking_a_row_selects_that_thread() {
    let (ui, log) = fixture();
    // The middle row (index 1) sits at y 122..154.
    click(&ui, 100.0, 138.0);
    assert_eq!(*log.borrow(), vec!["select 1".to_string()]);
}

#[test]
fn more_menu_archive_reports_the_row_index() {
    let (ui, log) = fixture();
    click(&ui, MORE.0, MORE.1);
    click(&ui, MENU_ARCHIVE.0, MENU_ARCHIVE.1);
    assert_eq!(*log.borrow(), vec!["archive 1".to_string()]);
}

#[test]
fn more_menu_delete_reports_the_row_index() {
    let (ui, log) = fixture();
    click(&ui, MORE.0, MORE.1);
    click(&ui, MENU_DELETE.0, MENU_DELETE.1);
    assert_eq!(*log.borrow(), vec!["delete 1".to_string()]);
}

#[test]
fn more_menu_rename_commits_the_edited_title_on_enter() {
    let (ui, log) = fixture();
    click(&ui, MORE.0, MORE.1);
    click(&ui, MENU_RENAME.0, MENU_RENAME.1);
    // The draft starts as "Beta"; replace it with "Renamed".
    // The caret starts at the front of the draft; move to the end first.
    key(&ui, Key::End);
    for _ in 0..4 {
        key(&ui, Key::Backspace);
    }
    for c in "Renamed".chars() {
        key(&ui, c.to_string());
    }
    key(&ui, Key::Return);
    assert_eq!(*log.borrow(), vec!["rename 1 Renamed".to_string()]);
}

#[test]
fn escape_cancels_a_rename() {
    let (ui, log) = fixture();
    click(&ui, MORE.0, MORE.1);
    click(&ui, MENU_RENAME.0, MENU_RENAME.1);
    for c in "xyz".chars() {
        key(&ui, c.to_string());
    }
    key(&ui, Key::Escape);
    assert!(
        log.borrow().is_empty(),
        "Escape must not rename: {:?}",
        log.borrow()
    );
}
