//! Browser glue for the web demo (`docs/WEB.md`): panic messages in the console, a frame
//! counter and milestones the page can read (`window.__aui`), and an optional
//! mock thread fetched as JSON.

use crate::AppWindow;
use ai_ui_core::{ExportedRepository, ThreadMessage};
use slint::ComponentHandle;
use std::cell::Cell;
use std::rc::Rc;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

fn metrics() -> Option<js_sys::Object> {
    let window = web_sys::window()?;
    let key = js_sys::JsString::from("__aui");
    let existing = js_sys::Reflect::get(&window, &key).ok()?;
    if existing.is_object() {
        return existing.dyn_into().ok();
    }
    let object = js_sys::Object::new();
    js_sys::Reflect::set(&window, &key, &object).ok()?;
    Some(object)
}

fn set(name: &str, value: f64) {
    if let Some(object) = metrics() {
        let _ = js_sys::Reflect::set(&object, &name.into(), &value.into());
    }
}

fn now() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map_or(0.0, |p| p.now())
}

/// Tell the page that `window.__aui` changed. The demo's scroll counter listens for
/// this instead of polling every frame: the value is ready on the scroll event itself,
/// and it still arrives when the browser throttles animation frames.
fn announce() {
    if let Some(window) = web_sys::window() {
        if let Ok(event) = web_sys::Event::new("aui:index") {
            let _ = window.dispatch_event(&event);
        }
    }
}

pub fn start(
    ui: &AppWindow,
    list: &ai_ui_core::ThreadList,
    thread: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    std::panic::set_hook(Box::new(|info| {
        web_sys::console::error_1(&info.to_string().into());
    }));
    set("frames", 0.0);
    set("started", now());
    // The landing page draws a scroll counter over the canvas. The thread only
    // tests its visible rows while this is on; `firstVisible` is 0-based. The
    // callback fires on the scroll event itself, not once per painted frame, so
    // the counter keeps up even when the browser throttles rendering.
    ui.set_track_visible_index(true);
    set("firstVisible", 0.0);
    set("messages", f64::from(ui.get_message_count()));
    announce();
    let weak = ui.as_weak();
    ui.on_visible_index_changed(move |index, scrolling| {
        set("firstVisible", f64::from(index));
        set("userScrolling", if scrolling { 1.0 } else { 0.0 });
        if let Some(ui) = weak.upgrade() {
            // Streaming a reply, or switching threads, can change the total.
            set("messages", f64::from(ui.get_message_count()));
        }
        announce();
    });
    let mut frames = 0u32;
    // The page requests a position with an `aui:scroll` event after writing
    // `__aui.scrollTo` (0-1). Each request is applied once; nothing is polled or
    // re-applied, so user gestures are never undone. `__aui.settled` is set on the
    // first painted frame after a request for the bottom.
    let pinned = Rc::new(Cell::new(false));
    if let Some(window) = web_sys::window() {
        let weak = ui.as_weak();
        let pinned = pinned.clone();
        let handler = Closure::<dyn FnMut()>::new(move || {
            let Some(ui) = weak.upgrade() else { return };
            let Some(object) = metrics() else { return };
            let Ok(value) = js_sys::Reflect::get(&object, &"scrollTo".into()) else { return };
            let Some(f) = value.as_f64().filter(|f| f.is_finite() && *f >= 0.0) else { return };
            let f = f.min(1.0);
            if f >= 1.0 {
                // The same value does not refire `changed`; nudge so a repeated 1 re-pins.
                if ui.get_scroll_fraction() >= 1.0 {
                    ui.set_scroll_fraction(-1.0);
                }
                pinned.set(true);
            }
            ui.set_scroll_fraction(f as f32);
        });
        let _ = window.add_event_listener_with_callback("aui:scroll", handler.as_ref().unchecked_ref());
        handler.forget();
    }
    ui.window()
        .set_rendering_notifier(move |state, _| {
            if matches!(state, slint::RenderingState::AfterRendering) {
                frames += 1;
                set("frames", f64::from(frames));
                if frames == 1 {
                    set("firstFrame", now());
                }
                set("lastFrame", now());
                if pinned.replace(false) {
                    set("settled", 1.0);
                }
            }
        })
        .map_err(|e| format!("rendering notifier: {e:?}"))?;
    if let Some(url) = thread {
        let list = list.clone();
        slint::spawn_local(async move {
            match load(&url).await {
                Ok(messages) => {
                    let count = messages.len();
                    let begin = now();
                    match list
                        .main()
                        .import(ExportedRepository::from_linear(messages))
                    {
                        Ok(_) => {
                            set("importMs", now() - begin);
                            set("messages", count as f64);
                        }
                        Err(e) => web_sys::console::error_1(&format!("import: {e}").into()),
                    }
                }
                Err(e) => web_sys::console::error_1(&format!("thread: {e}").into()),
            }
        })?;
    }
    Ok(())
}

async fn load(url: &str) -> Result<Vec<ThreadMessage>, String> {
    let window = web_sys::window().ok_or("no window")?;
    let response = JsFuture::from(window.fetch_with_str(url))
        .await
        .map_err(|e| format!("{e:?}"))?;
    let response: web_sys::Response = response.dyn_into().map_err(|_| "not a response")?;
    let text = JsFuture::from(response.text().map_err(|e| format!("{e:?}"))?)
        .await
        .map_err(|e| format!("{e:?}"))?
        .as_string()
        .ok_or("not text")?;
    set("fetched", now());
    // The mock-data format: {"headId", "messages": [{"parentId", "message"}]}, linear.
    let file: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    file["messages"]
        .as_array()
        .ok_or("no messages")?
        .iter()
        .map(|item| serde_json::from_value(item["message"].clone()).map_err(|e| e.to_string()))
        .collect()
}
