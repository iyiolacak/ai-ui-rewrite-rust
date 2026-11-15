//! The assistant's unused width hint must not be measured by a hidden Text.
use slint::platform::{
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    Platform, WindowAdapter,
};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::rc::Rc;

slint::slint! {
    import { AssistantMessage } from "../ui/components/message.slint";
    import { ChatRow } from "../ui/models.slint";
    export { ChatRow, ChatPart, ChatPartKind, MessageRole } from "../ui/models.slint";
    export component MessageSizingFixture inherits Window {
        width: 400px; height: 240px;
        in property <ChatRow> message;
        AssistantMessage { width: 380px; x: 10px; y: 10px; message: root.message; }
    }
}
struct Software;
impl Platform for Software {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer))
    }
}
fn row(role: MessageRole, hint: &str) -> ChatRow {
    ChatRow {
        id: "message".into(),
        role,
        width_hint: hint.into(),
        parts: ModelRc::new(VecModel::from(vec![ChatPart {
            id: "part".into(),
            kind: ChatPartKind::Text,
            body: "A short visible message".into(),
            ..Default::default()
        }])),
        ..Default::default()
    }
}
#[test]
fn assistant_ignores_unused_multiline_hint_and_user_bubble_still_uses_it() {
    slint::platform::set_platform(Box::new(Software)).unwrap();
    let ui = MessageSizingFixture::new().unwrap();
    ui.window().set_size(slint::PhysicalSize::new(400, 240));
    ui.set_message(row(MessageRole::Assistant, ""));
    ui.show().unwrap();
    let baseline = ui.window().take_snapshot().unwrap();
    // This many lines previously overflowed i16 in the software renderer's
    // invisible sizing measurement despite the short visible response.
    ui.set_message(row(MessageRole::Assistant, &"hint\n".repeat(3_000)));
    let long = ui.window().take_snapshot().unwrap();
    assert_eq!(baseline.as_slice(), long.as_slice());

    // The same hint remains meaningful for the user bubble, which is its only
    // consumer. Visible text stays identical in the two screenshots.
    ui.set_message(row(MessageRole::User, "x"));
    let narrow = ui.window().take_snapshot().unwrap();
    ui.set_message(row(MessageRole::User, "A reasonably long user sizing hint"));
    let wide = ui.window().take_snapshot().unwrap();
    assert_ne!(narrow.as_slice(), wide.as_slice());
    ui.hide().unwrap();
}
