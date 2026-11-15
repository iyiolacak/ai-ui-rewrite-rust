//! Software-window checks for host-controlled Markdown code copy actions.
use slint::platform::{
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    Platform, PointerEventButton, WindowAdapter, WindowEvent,
};
use slint::{ComponentHandle, LogicalPosition};
use std::rc::Rc;

slint::slint! {
    import { AssistantMarkdown, MarkdownBlockKind } from "../ui/components/markdown.slint";
    export component CopyFixture inherits Window {
        width: 400px; height: 240px;
        in property <bool> copied: false;
        in property <bool> enabled: true;
        out property <int> modern-count: 0;
        out property <int> legacy-count: 0;
        out property <string> token;
        out property <string> code;
        AssistantMarkdown {
            x: 20px; y: 20px; width: 360px;
            can-copy-code: root.enabled;
            blocks: [
                {kind: MarkdownBlockKind.code, language: "rust", code: "let one = 1;", copy-token: "scope-a", copied: root.copied},
                {kind: MarkdownBlockKind.code, language: "rs", code: "let two = 2;"},
            ];
            copy-code-block(token, code) => { root.modern-count += 1; root.token = token; root.code = code; }
            copy-code(code) => { root.legacy-count += 1; root.code = code; }
        }
    }
}
struct SoftwarePlatform;
impl Platform for SoftwarePlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer))
    }
}
fn click(ui: &CopyFixture, x: f32, y: f32) {
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
#[test]
fn controlled_copy_feedback_and_legacy_callback() {
    slint::platform::set_platform(Box::new(SoftwarePlatform)).unwrap();
    let ui = CopyFixture::new().unwrap();
    ui.window().set_size(slint::PhysicalSize::new(400, 240));
    ui.show().unwrap();
    slint::platform::update_timers_and_animations();
    let initial = ui.window().take_snapshot().unwrap();
    click(&ui, 354.0, 38.0);
    assert_eq!(ui.get_modern_count(), 1);
    assert_eq!(ui.get_token().as_str(), "scope-a");
    assert_eq!(ui.get_code().as_str(), "let one = 1;");
    assert_eq!(ui.get_legacy_count(), 0);
    // Only the host's successful completion changes the indicator.
    ui.set_copied(true);
    slint::platform::update_timers_and_animations();
    let confirmed = ui.window().take_snapshot().unwrap();
    assert_ne!(initial.as_slice(), confirmed.as_slice());
    click(&ui, 354.0, 38.0);
    assert_eq!(
        ui.get_modern_count(),
        1,
        "a confirmed block must not invoke copy again"
    );
    if let Ok(path) = std::env::var("AI_UI_MARKDOWN_COPY_SNAPSHOT") {
        use std::io::Write;
        let mut file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
        write!(
            file,
            "P6\n{} {}\n255\n",
            confirmed.width(),
            confirmed.height()
        )
        .unwrap();
        for pixel in confirmed.as_slice() {
            file.write_all(&[pixel.r, pixel.g, pixel.b]).unwrap();
        }
    }
    ui.set_copied(false);
    ui.set_enabled(false);
    slint::platform::update_timers_and_animations();
    click(&ui, 354.0, 38.0);
    assert_eq!(
        ui.get_modern_count(),
        1,
        "disabled copy must not invoke the host"
    );
    ui.set_enabled(true);
    slint::platform::update_timers_and_animations();
    click(&ui, 354.0, 128.0);
    assert_eq!(ui.get_legacy_count(), 1);
    assert_eq!(ui.get_code().as_str(), "let two = 2;");
    ui.hide().unwrap();
}
