use ai_ui_clipboard::Clipboard;
use std::sync::{Arc, Mutex};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct Fixture {
    clipboard: Clipboard,
    results: Arc<Mutex<Vec<String>>>,
}

impl Default for Fixture {
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen]
impl Fixture {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            clipboard: Clipboard::new().unwrap(),
            results: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn copy(&self, text: String) -> String {
        let results = Arc::clone(&self.results);
        match self.clipboard.copy(
            text,
            Box::new(move |result| {
                results.lock().unwrap().push(match result {
                    Ok(()) => "ok".into(),
                    Err(error) => error.to_string(),
                });
            }),
        ) {
            Ok(()) => "accepted".into(),
            Err(error) => error.to_string(),
        }
    }

    pub fn results(&self) -> String {
        self.results.lock().unwrap().join(",")
    }

    pub fn read_text(&self) -> String {
        let results = Arc::clone(&self.results);
        match self.clipboard.read_text(Box::new(move |result| {
            results.lock().unwrap().push(match result {
                Ok(text) => format!("read:{text}"),
                Err(error) => error.to_string(),
            });
        })) {
            Ok(()) => "accepted".into(),
            Err(error) => error.to_string(),
        }
    }

    pub fn shutdown(&self) {
        self.clipboard.shutdown();
    }
}
