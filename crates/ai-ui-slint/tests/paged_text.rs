//! Controlled paged text must not lay out the retained source and must forward
//! the captured page identity through the shared thread components.
use slint::platform::{
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    Platform, PointerEventButton, WindowAdapter, WindowEvent,
};
use slint::{ComponentHandle, LogicalPosition, Model, ModelRc, VecModel};
use std::{cell::RefCell, rc::Rc};

slint::slint! {
    import { AssistantThread } from "../ui/components/thread.slint";
    import { ChatRow } from "../ui/models.slint";
    export { ChatRow, ChatPart, ChatPartKind, MessageRole } from "../ui/models.slint";
    export component PagedTextFixture inherits Window {
        width: 400px; height: 240px;
        in property <[ChatRow]> messages;
        callback page-requested(int, string, string, string, int, int);
        AssistantThread {
            width: parent.width; height: parent.height;
            messages: root.messages;
            conversation-revision: 17;
            page-requested(revision, message-id, part-id, token, page-number, delta) => {
                root.page-requested(revision, message-id, part-id, token, page-number, delta);
            }
        }
    }
}
struct Software;
impl Platform for Software {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer))
    }
}
fn model(source: String, page_number: i32, token: &str, error: &str) -> ModelRc<ChatRow> {
    ModelRc::new(VecModel::from(vec![ChatRow {
        id: "message".into(),
        role: MessageRole::Assistant,
        parts: ModelRc::new(VecModel::from(vec![ChatPart {
            id: "part".into(),
            kind: ChatPartKind::Text,
            body: source.into(),
            // Paging takes precedence even when an old Markdown projection is present.
            markdown_enabled: true,
            text_paged: true,
            text_page: "First page".into(),
            text_page_number: page_number,
            text_page_count: 2,
            text_page_token: token.into(),
            text_page_error: error.into(),
            ..Default::default()
        }])),
        ..Default::default()
    }]))
}
fn click(ui: &PagedTextFixture, x: f32, y: f32) {
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
fn page_controls_forward_captured_identity_without_laying_out_source() {
    slint::platform::set_platform(Box::new(Software)).unwrap();
    let ui = PagedTextFixture::new().unwrap();
    ui.window().set_size(slint::PhysicalSize::new(400, 240));
    ui.set_messages(model("short secret".into(), 1, "token-1", ""));
    ui.show().unwrap();
    let baseline = ui.window().take_snapshot().unwrap();
    // The source remains in the model, but cannot enter a Text element while paged.
    ui.set_messages(model("hidden\n".repeat(3_000), 1, "token-1", ""));
    let long = ui.window().take_snapshot().unwrap();
    assert_eq!(baseline.as_slice(), long.as_slice());
    let captured = Rc::new(RefCell::new(Vec::new()));
    let events = captured.clone();
    ui.on_page_requested(move |revision, message, part, token, page, delta| {
        events.borrow_mut().push((
            revision,
            message.to_string(),
            part.to_string(),
            token.to_string(),
            page,
            delta,
        ));
    });
    click(&ui, 20., 43.); // First page: Previous is disabled.
    assert!(captured.borrow().is_empty());
    click(&ui, 113., 43.);
    assert_eq!(
        &*captured.borrow(),
        &[(17, "message".into(), "part".into(), "token-1".into(), 1, 1)]
    );
    captured.borrow_mut().clear();
    // Replacing a source between pointer-down and release must return the
    // original page identity, so a host can reject the stale gesture.
    let position = LogicalPosition::new(113., 43.);
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    let row = ui.get_messages().row_data(0).unwrap();
    let parts = row
        .parts
        .as_any()
        .downcast_ref::<VecModel<ChatPart>>()
        .unwrap();
    let mut replacement = parts.row_data(0).unwrap();
    replacement.text_page_token = "replacement-token".into();
    parts.set_row_data(0, replacement);
    ui.window().take_snapshot().unwrap();
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
    assert_eq!(
        &*captured.borrow(),
        &[(17, "message".into(), "part".into(), "token-1".into(), 1, 1)]
    );
    captured.borrow_mut().clear();
    ui.set_messages(model("hidden".into(), 2, "token-2", ""));
    let _ = ui.window().take_snapshot().unwrap();
    click(&ui, 113., 43.); // Last page: Next is disabled.
    assert!(captured.borrow().is_empty());
    click(&ui, 20., 43.);
    assert_eq!(
        &*captured.borrow(),
        &[(17, "message".into(), "part".into(), "token-2".into(), 2, -1)]
    );
    captured.borrow_mut().clear();
    ui.set_messages(model(
        "hidden\n".repeat(3_000),
        1,
        "token-3",
        "Page unavailable",
    ));
    let _ = ui.window().take_snapshot().unwrap();
    click(&ui, 113., 43.);
    assert!(captured.borrow().is_empty());
    ui.hide().unwrap();
}
