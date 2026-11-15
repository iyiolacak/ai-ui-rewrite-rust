//! Focus rings are for keyboard focus only: a pointer click focuses a button
//! for keyboard handling but shows no ring, while Tab shows one.
use slint::platform::{
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    Key, Platform, PointerEventButton, WindowAdapter, WindowEvent,
};
use slint::{ComponentHandle, LogicalPosition, PhysicalSize};
use std::rc::Rc;

slint::slint! {
    import { AssistantButton, AssistantIconButton } from "../ui/lib.slint";
    export component FocusFixture inherits Window {
        width: 300px; height: 80px;
        out property <bool> button-ring: first.focus-visible;
        out property <bool> icon-ring: second.focus-visible;
        first := AssistantButton { x: 10px; y: 10px; label: "First"; }
        second := AssistantIconButton {
            x: 200px; y: 10px;
            icon: @image-url("../ui/assets/plus.svg");
            label: "Second";
        }
    }
}

struct Software;
impl Platform for Software {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer))
    }
}

fn fixture() -> FocusFixture {
    let _ = slint::platform::set_platform(Box::new(Software));
    let ui = FocusFixture::new().unwrap();
    ui.window().set_size(PhysicalSize::new(300, 80));
    ui.show().unwrap();
    ui
}

fn click(ui: &FocusFixture, position: LogicalPosition) {
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
}

fn tab(ui: &FocusFixture) {
    let text: slint::SharedString = Key::Tab.into();
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

#[test]
fn clicking_a_button_shows_no_focus_ring() {
    let ui = fixture();
    // The first button is 10..~110 px wide and 10..46 px tall.
    click(&ui, LogicalPosition::new(40.0, 28.0));
    assert!(
        !ui.get_button_ring(),
        "a pointer click must not show the ring"
    );
}

#[test]
fn tab_to_a_button_shows_the_focus_ring() {
    let ui = fixture();
    tab(&ui);
    assert!(ui.get_button_ring(), "Tab focus shows the ring");
    assert!(!ui.get_icon_ring());
}

#[test]
fn tab_to_an_icon_button_shows_the_focus_ring() {
    let ui = fixture();
    tab(&ui);
    tab(&ui);
    assert!(!ui.get_button_ring(), "the ring moves with focus");
    assert!(
        ui.get_icon_ring(),
        "Tab focus on an icon button shows its ring"
    );
}

#[test]
fn clicking_after_keyboard_focus_hides_the_ring() {
    let ui = fixture();
    tab(&ui);
    assert!(ui.get_button_ring());
    click(&ui, LogicalPosition::new(40.0, 28.0));
    assert!(!ui.get_button_ring(), "a click ends keyboard focus display");
}
