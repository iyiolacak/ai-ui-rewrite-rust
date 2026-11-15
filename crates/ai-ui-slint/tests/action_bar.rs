//! Upstream autohide "not-last": the action row of an older message appears
//! only while the pointer is over the message, while the newest one keeps it.
use slint::platform::{
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    Platform, WindowAdapter, WindowEvent,
};
use slint::{ComponentHandle, LogicalPosition, ModelRc, VecModel};
use std::rc::Rc;
use std::time::Duration;

slint::slint! {
    import { AssistantMessage } from "../ui/components/message.slint";
    import { ChatRow, ChatPart, ChatPartKind, MessageRole } from "../ui/models.slint";
    export component ActionBarFixture inherits Window {
        width: 400px; height: 240px;
        in property <ChatRow> message;
        in property <bool> last: false;
        out property <length> message-height: row.height;
        row := AssistantMessage { width: 380px; x: 10px; y: 10px; message: root.message; last: root.last; }
    }
}

struct SoftwarePlatform;
impl Platform for SoftwarePlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer))
    }
}

fn assistant_row() -> ChatRow {
    ChatRow {
        id: "message".into(),
        role: MessageRole::Assistant,
        branch_token: "branch".into(),
        can_copy: true,
        parts: ModelRc::new(VecModel::from(vec![ChatPart {
            id: "part".into(),
            kind: ChatPartKind::Text,
            body: "A short visible message".into(),
            ..Default::default()
        }])),
        ..Default::default()
    }
}

// The opacity animates over 120 ms; let it finish before comparing frames.
fn settle(ui: &ActionBarFixture) -> slint::SharedPixelBuffer<slint::Rgba8Pixel> {
    std::thread::sleep(Duration::from_millis(200));
    slint::platform::update_timers_and_animations();
    ui.window().take_snapshot().unwrap()
}

fn move_to(ui: &ActionBarFixture, x: f32, y: f32) {
    ui.window().dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(x, y),
    });
}

#[test]
fn older_message_actions_show_only_on_hover_and_last_always_shows_them() {
    slint::platform::set_platform(Box::new(SoftwarePlatform)).unwrap();
    let ui = ActionBarFixture::new().unwrap();
    ui.window().set_size(slint::PhysicalSize::new(400, 240));
    ui.set_message(assistant_row());
    ui.show().unwrap();

    // Not last, pointer away: the row is transparent.
    let hidden = settle(&ui);
    // The newest message keeps its actions without any pointer.
    ui.set_last(true);
    let shown = settle(&ui);
    assert_ne!(
        hidden.as_slice(),
        shown.as_slice(),
        "last message shows actions"
    );

    // Back to an older message: the actions go away again.
    ui.set_last(false);
    assert_eq!(
        settle(&ui).as_slice(),
        hidden.as_slice(),
        "older message hides actions"
    );

    // Pointer over the message body reveals the row.
    move_to(&ui, 100.0, 20.0);
    assert_eq!(
        settle(&ui).as_slice(),
        shown.as_slice(),
        "hover over message shows actions"
    );

    // Pointer over the first action (the copy button) keeps the row shown.
    // The row sits below the bubble: its top is the message height minus 56 px
    // (32 px row plus the 24 px gap at the foot of the message).
    let row_top = ui.get_message_height() - 56.0;
    move_to(&ui, 30.0, 10.0 + row_top + 16.0);
    assert_ne!(
        settle(&ui).as_slice(),
        hidden.as_slice(),
        "hover over an action keeps it shown"
    );

    // Pointer leaves the message: the row fades out again.
    move_to(&ui, 300.0, 200.0);
    assert_eq!(
        settle(&ui).as_slice(),
        hidden.as_slice(),
        "leaving the message hides actions"
    );
    ui.hide().unwrap();
}

fn user_row() -> ChatRow {
    ChatRow {
        id: "user-message".into(),
        role: MessageRole::User,
        branch_token: "branch".into(),
        can_copy: true,
        can_edit: true,
        width_hint: "A short question".into(),
        parts: ModelRc::new(VecModel::from(vec![ChatPart {
            id: "part".into(),
            kind: ChatPartKind::Text,
            body: "A short question".into(),
            ..Default::default()
        }])),
        ..Default::default()
    }
}

fn shot(ui: &ActionBarFixture, name: &str) -> slint::SharedPixelBuffer<slint::Rgba8Pixel> {
    let buffer = settle(ui);
    if let Some(dir) = std::env::var_os("AI_UI_ACTION_BAR_SHOTS") {
        let dir = std::path::PathBuf::from(dir);
        let _ = std::fs::create_dir_all(&dir);
        image_dump(&dir.join(format!("{name}.ppm")), &buffer);
    }
    buffer
}

fn image_dump(path: &std::path::Path, buffer: &slint::SharedPixelBuffer<slint::Rgba8Pixel>) {
    let mut out = format!("P6\n{} {}\n255\n", buffer.width(), buffer.height()).into_bytes();
    for p in buffer.as_slice() {
        out.extend_from_slice(&[p.r, p.g, p.b]);
    }
    let _ = std::fs::write(path, out);
}

// Moving the pointer from the bubble onto each action button of an older message
// must keep the whole row shown (a button is a TouchArea above the message's
// hover area), for user and assistant rows alike.
#[test]
fn actions_stay_shown_while_the_pointer_is_on_each_button() {
    slint::platform::set_platform(Box::new(SoftwarePlatform)).ok();
    for (name, row, buttons) in [
        ("user", user_row(), 2usize),
        (
            "assistant",
            ChatRow {
                can_regenerate: true,
                can_edit: true,
                ..assistant_row()
            },
            3,
        ),
    ] {
        let ui = ActionBarFixture::new().unwrap();
        ui.window().set_size(slint::PhysicalSize::new(400, 240));
        ui.set_message(row);
        ui.show().unwrap();
        let hidden = shot(&ui, &format!("{name}-0-away"));
        let bubble_point = if name == "user" {
            (340.0, 24.0)
        } else {
            (100.0, 20.0)
        };
        move_to(&ui, bubble_point.0, bubble_point.1);
        let shown = shot(&ui, &format!("{name}-1-bubble"));
        assert_ne!(
            shown.as_slice(),
            hidden.as_slice(),
            "{name}: bubble hover shows actions"
        );
        let row_y = 10.0 + ui.get_message_height() - 56.0 + 16.0;
        let row_width = 28.0 * buttons as f32;
        let row_x = if name == "user" {
            10.0 + 380.0 - row_width - 6.0
        } else {
            10.0 + 6.0
        };
        for i in 0..buttons {
            move_to(&ui, row_x + 28.0 * i as f32 + 14.0, row_y);
            let now = shot(&ui, &format!("{name}-2-button{i}"));
            // The hovered button draws its own highlight; compare the row's
            // icons, not pixels: the row must not be transparent.
            assert_ne!(
                now.as_slice(),
                hidden.as_slice(),
                "{name}: row visible over button {i}"
            );
            let opacity_probe = now
                .as_slice()
                .iter()
                .zip(hidden.as_slice())
                .filter(|(a, b)| a != b)
                .count();
            assert!(
                opacity_probe > 40,
                "{name}: button {i} row drawn ({opacity_probe} px differ)"
            );
        }
        move_to(&ui, 300.0, 230.0);
        assert_eq!(
            settle(&ui).as_slice(),
            hidden.as_slice(),
            "{name}: leaving hides"
        );
        ui.hide().unwrap();
    }
}
