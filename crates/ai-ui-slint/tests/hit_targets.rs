//! Pointer targets: an icon button's 32 px hit area reaches past its 24 px
//! visual, and only opted-in actions ignore a fast double click.
use slint::platform::{
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    Platform, PointerEventButton, WindowAdapter, WindowEvent,
};
use slint::{ComponentHandle, LogicalPosition};
use std::rc::Rc;
use std::time::Duration;

slint::slint! {
    import { AssistantIconButton } from "../ui/components/icon-button.slint";
    import { AssistantButton } from "../ui/components/button.slint";
    export component HitFixture inherits Window {
        width: 240px; height: 120px; background: #0f0f0e;
        out property <int> icon-activations: 0;
        out property <int> guarded-icon-activations: 0;
        out property <int> button-activations: 0;
        out property <int> guarded-button-activations: 0;
        // Visual 24 px square at (44, 24); its 32 px target spans (40, 20)-(72, 52).
        AssistantIconButton {
            x: 40px; y: 20px;
            control-size: 24px;
            label: "Probe";
            activated => { root.icon-activations += 1; }
        }
        AssistantIconButton {
            x: 140px; y: 20px;
            control-size: 24px;
            label: "Guarded probe";
            guard-repeat: true;
            activated => { root.guarded-icon-activations += 1; }
        }
        AssistantButton {
            x: 40px; y: 70px;
            label: "Probe";
            activated => { root.button-activations += 1; }
        }
        AssistantButton {
            x: 140px; y: 70px;
            label: "Guarded";
            guard-repeat: true;
            activated => { root.guarded-button-activations += 1; }
        }
    }
}

struct SoftwarePlatform;
impl Platform for SoftwarePlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer))
    }
}

// The cooldown of guarded controls is a real 300 ms timer, so tests must let it lapse.
fn settle() {
    std::thread::sleep(Duration::from_millis(350));
    slint::platform::update_timers_and_animations();
}

fn click(ui: &HitFixture, x: f32, y: f32) {
    let position = LogicalPosition::new(x, y);
    ui.window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
}

fn fixture() -> HitFixture {
    slint::platform::set_platform(Box::new(SoftwarePlatform)).unwrap();
    let ui = HitFixture::new().unwrap();
    ui.window().set_size(slint::PhysicalSize::new(240, 120));
    ui.show().unwrap();
    slint::platform::update_timers_and_animations();
    ui
}

#[test]
fn icon_button_activates_inside_its_enlarged_target_only() {
    let ui = fixture();
    // Three pixels inside each edge of the 32 px target, outside the 24 px visual.
    let inside = [(43.0, 36.0), (69.0, 36.0), (56.0, 23.0), (56.0, 49.0)];
    for (count, (x, y)) in inside.into_iter().enumerate() {
        click(&ui, x, y);
        settle();
        assert_eq!(
            ui.get_icon_activations(),
            count as i32 + 1,
            "click at ({x}, {y}) inside the target should activate"
        );
    }
    // One pixel outside the target on the left and right does not activate.
    click(&ui, 39.0, 36.0);
    click(&ui, 72.5, 36.0);
    assert_eq!(ui.get_icon_activations(), inside.len() as i32);
    ui.hide().unwrap();
}

#[test]
fn double_click_starts_guarded_actions_once_and_unguarded_ones_twice() {
    let ui = fixture();
    // Two clicks inside one cooldown window, as a fast double click delivers.
    click(&ui, 56.0, 36.0);
    click(&ui, 56.0, 36.0);
    assert_eq!(
        ui.get_icon_activations(),
        2,
        "unguarded icon button fires twice"
    );
    click(&ui, 156.0, 36.0);
    click(&ui, 156.0, 36.0);
    assert_eq!(
        ui.get_guarded_icon_activations(),
        1,
        "guarded icon button fires once"
    );
    click(&ui, 60.0, 88.0);
    click(&ui, 60.0, 88.0);
    assert_eq!(
        ui.get_button_activations(),
        2,
        "unguarded button fires twice"
    );
    click(&ui, 150.0, 88.0);
    click(&ui, 150.0, 88.0);
    assert_eq!(
        ui.get_guarded_button_activations(),
        1,
        "guarded button fires once"
    );

    // After the window lapses, a deliberate click activates a guarded control again.
    settle();
    click(&ui, 156.0, 36.0);
    assert_eq!(ui.get_guarded_icon_activations(), 2);
    ui.hide().unwrap();
}
