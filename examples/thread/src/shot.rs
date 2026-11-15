//! Offscreen rendering to PNG with Slint's software renderer, for visual
//! review and comparison without opening a window.

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use std::rc::Rc;

struct Offscreen(Rc<MinimalSoftwareWindow>);

impl Platform for Offscreen {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

/// Install the offscreen platform; call before creating any window.
pub fn install(width: u32, height: u32) -> Rc<MinimalSoftwareWindow> {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    window.set_size(slint::PhysicalSize::new(width, height));
    slint::platform::set_platform(Box::new(Offscreen(window.clone())))
        .expect("no platform installed yet");
    window
}

/// Let timers and animations settle, render, and write `path`.
pub fn save(window: &slint::Window, path: &str) -> Result<(), String> {
    for _ in 0..30 {
        slint::platform::update_timers_and_animations();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    save_now(window, path)
}

/// Write what the window shows now. For a running event loop, where timers
/// must not be pumped by hand.
pub fn save_now(window: &slint::Window, path: &str) -> Result<(), String> {
    let buffer = window.take_snapshot().map_err(|e| e.to_string())?;
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut encoder = png::Encoder::new(
        std::io::BufWriter::new(file),
        buffer.width(),
        buffer.height(),
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer
        .write_image_data(buffer.as_bytes())
        .map_err(|e| e.to_string())
}
