//! Software-window interaction check for the reusable copy tooltip.
use slint::platform::{
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    Platform, PointerEventButton, WindowAdapter, WindowEvent,
};
use slint::{ComponentHandle, LogicalPosition};
use std::rc::Rc;
use std::time::Duration;

slint::slint! {
    import { AssistantCopyButton } from "../ui/components/copy-button.slint";
    export component TooltipFixture inherits Window {
        width: 200px; height: 140px; background: #0f0f0e;
        in property <bool> enabled: true;
        out property <int> activations: 0;
        // A clipping ancestor verifies that the tooltip is an overlay.
        Rectangle {
            x: 40px; y: 30px; width: 24px; height: 24px; clip: true;
            AssistantCopyButton {
                enabled: root.enabled;
                activated => { root.activations += 1; }
            }
        }
    }
}
struct SoftwarePlatform;
impl Platform for SoftwarePlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer))
    }
}

#[test]
fn hover_tooltip_does_not_block_copy() {
    slint::platform::set_platform(Box::new(SoftwarePlatform)).unwrap();
    let ui = TooltipFixture::new().unwrap();
    ui.window().set_size(slint::PhysicalSize::new(200, 140));
    ui.show().unwrap();
    slint::platform::update_timers_and_animations();
    let initial = ui.window().take_snapshot().unwrap();
    let position = LogicalPosition::new(52.0, 42.0);
    ui.window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    slint::platform::update_timers_and_animations();
    let hover = ui.window().take_snapshot().unwrap();
    std::thread::sleep(Duration::from_millis(600));
    slint::platform::update_timers_and_animations();
    let shown = ui.window().take_snapshot().unwrap();
    assert_ne!(
        hover.as_slice(),
        shown.as_slice(),
        "tooltip should appear after hover"
    );
    let changed_below_clip = (54..110).any(|y| {
        (35..155).any(|x| {
            let index = y * 200 + x;
            hover.as_slice()[index] != shown.as_slice()[index]
        })
    });
    assert!(
        changed_below_clip,
        "tooltip should escape the clipped parent"
    );
    if let Ok(path) = std::env::var("AI_UI_TOOLTIP_SNAPSHOT") {
        use std::io::Write;
        let mut file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
        write!(file, "P6\n{} {}\n255\n", shown.width(), shown.height()).unwrap();
        for pixel in shown.as_slice() {
            file.write_all(&[pixel.r, pixel.g, pixel.b]).unwrap();
        }
    }
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
    assert_eq!(
        ui.get_activations(),
        1,
        "tooltip must not intercept a click"
    );
    ui.window().dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(150.0, 110.0),
    });
    slint::platform::update_timers_and_animations();
    let exited = ui.window().take_snapshot().unwrap();
    // The click may give the button focus, changing its border. Compare only
    // the region below the clipping ancestor where the tooltip was shown.
    for y in 54..110 {
        for x in 35..155 {
            let index = y * 200 + x;
            assert_eq!(
                initial.as_slice()[index],
                exited.as_slice()[index],
                "tooltip should close after exit"
            );
        }
    }
    // Disabling an already-hovered control must also remove its tooltip.
    ui.window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    slint::platform::update_timers_and_animations();
    std::thread::sleep(Duration::from_millis(600));
    slint::platform::update_timers_and_animations();
    let reopened = ui.window().take_snapshot().unwrap();
    assert_ne!(
        exited.as_slice(),
        reopened.as_slice(),
        "tooltip should reopen on a new hover"
    );
    ui.set_enabled(false);
    slint::platform::update_timers_and_animations();
    let disabled = ui.window().take_snapshot().unwrap();
    for y in 54..110 {
        for x in 35..155 {
            let index = y * 200 + x;
            assert_eq!(
                initial.as_slice()[index],
                disabled.as_slice()[index],
                "disabled control must not leave a tooltip open"
            );
        }
    }
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
    assert_eq!(ui.get_activations(), 1, "disabled copy must not activate");
    ui.hide().unwrap();
}
