//! Standalone software-window check for the shared Markdown code viewport.
use slint::platform::{
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    Platform, PointerEventButton, WindowAdapter, WindowEvent,
};
use slint::{ComponentHandle, LogicalPosition};
use std::{cell::Cell, rc::Rc, time::Duration};

slint::slint! {
    import { AssistantMarkdown, MarkdownBlockKind } from "../ui/components/markdown.slint";
    import { AssistantHorizontalScrollbar } from "../ui/lib.slint";
    export component CodeScrollFixture inherits Window {
        width: 400px; height: 200px;
        out property <length> thread-y: thread.content-y;
        thread := Flickable {
            x: 20px; y: 20px; width: 360px; height: 150px;
            content-width: self.width;
            content-height: 500px;
            AssistantMarkdown {
                x: 0px; y: 0px; width: parent.width;
                blocks: [{kind: MarkdownBlockKind.code, language: "rust", code: "let long_line = call(first_argument, second_argument, third_argument, fourth_argument, fifth_argument, sixth_argument);"}];
            }
        }
    }
    export component ScrollbarFixture inherits Window {
        width: 400px; height: 80px;
        in-out property <length> offset: 0px;
        out property <int> scroll-count: 0;
        AssistantHorizontalScrollbar {
            x: 20px; y: 20px; width: 300px;
            content-width: 600px;
            visible-width: 300px;
            content-x <=> root.offset;
            scrolled => { root.scroll-count += 1; }
        }
    }
}

struct SoftwarePlatform(Rc<Cell<u64>>);
impl Platform for SoftwarePlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer))
    }
    fn duration_since_start(&self) -> Duration {
        Duration::from_millis(self.0.get())
    }
}

#[test]
fn code_scrollbar_moves_text_and_vertical_wheel_reaches_thread() {
    let clock = Rc::new(Cell::new(0));
    slint::platform::set_platform(Box::new(SoftwarePlatform(clock.clone()))).unwrap();
    let ui = CodeScrollFixture::new().unwrap();
    ui.window().set_size(slint::PhysicalSize::new(400, 200));
    ui.show().unwrap();
    slint::platform::update_timers_and_animations();
    let before = ui.window().take_snapshot().unwrap();
    let on_track = LogicalPosition::new(350.0, 90.0);
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position: on_track,
        button: PointerEventButton::Left,
    });
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position: on_track,
        button: PointerEventButton::Left,
    });
    slint::platform::update_timers_and_animations();
    let after = ui.window().take_snapshot().unwrap();
    let text_region_changed = (70..86).any(|y| {
        (34..366).any(|x| {
            let index = y * 400 + x;
            before.as_slice()[index] != after.as_slice()[index]
        })
    });
    assert!(
        text_region_changed,
        "track click must scroll the actual code, not only the thumb"
    );
    assert_eq!(ui.get_thread_y(), 0.0);
    let position = LogicalPosition::new(180.0, 75.0);
    ui.window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    ui.window().dispatch_event(WindowEvent::PointerScrolled {
        position,
        delta_x: 100.0,
        delta_y: 0.0,
    });
    slint::platform::update_timers_and_animations();
    clock.set(200);
    slint::platform::update_timers_and_animations();
    let after_wheel = ui.window().take_snapshot().unwrap();
    assert!(
        (70..86).any(|y| (34..366).any(|x| {
            let index = y * 400 + x;
            after.as_slice()[index] != after_wheel.as_slice()[index]
        })),
        "horizontal wheel must scroll the actual code"
    );
    assert_eq!(ui.get_thread_y(), 0.0);
    clock.set(500);
    slint::platform::update_timers_and_animations();
    ui.window().dispatch_event(WindowEvent::PointerScrolled {
        position,
        delta_x: 0.0,
        delta_y: -100.0,
    });
    slint::platform::update_timers_and_animations();
    clock.set(700);
    slint::platform::update_timers_and_animations();
    ui.window().take_snapshot().unwrap();
    assert!(
        ui.get_thread_y() < -1.0,
        "vertical wheel over code must reach the thread"
    );
    ui.hide().unwrap();

    let scrollbar = ScrollbarFixture::new().unwrap();
    scrollbar
        .window()
        .set_size(slint::PhysicalSize::new(400, 80));
    scrollbar.show().unwrap();
    scrollbar.window().take_snapshot().unwrap();
    let start = LogicalPosition::new(80.0, 26.0);
    let end = LogicalPosition::new(130.0, 26.0);
    scrollbar
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position: start });
    scrollbar
        .window()
        .dispatch_event(WindowEvent::PointerPressed {
            position: start,
            button: PointerEventButton::Left,
        });
    scrollbar
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position: end });
    scrollbar
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position: end,
            button: PointerEventButton::Left,
        });
    scrollbar.window().take_snapshot().unwrap();
    assert!(
        (scrollbar.get_offset() + 100.0).abs() < 1.0,
        "thumb drag must scale with the content extent"
    );
    assert!(scrollbar.get_scroll_count() > 0);
    scrollbar.hide().unwrap();
}
