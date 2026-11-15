use crate::{Completion, CopyError, ReadCompletion, ReadError, MAX_COPY_BYTES, MAX_READ_BYTES};
use js_sys::{Function, Promise, Reflect};
use std::cell::Cell;
use std::rc::Rc;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{spawn_local, JsFuture};

/// Browser clipboard adapter; no browser capability is assumed until each call.
pub struct Clipboard {
    active: Cell<bool>,
    pending: Rc<Cell<bool>>,
}

impl Clipboard {
    /// Create the adapter; always succeeds in the browser.
    pub fn new() -> Result<Self, CopyError> {
        Ok(Self {
            active: Cell::new(true),
            pending: Rc::new(Cell::new(false)),
        })
    }

    /// Start the browser operation synchronously in the caller's user gesture.
    /// Returning `Ok` only means the browser accepted the request; completion
    /// reports whether its promise resolved. Immediate failures do not call it.
    pub fn copy(&self, text: String, complete: Completion) -> Result<(), CopyError> {
        if !self.active.get() {
            return Err(CopyError::Cancelled);
        }
        if self.pending.get() {
            return Err(CopyError::Busy);
        }
        if text.len() > MAX_COPY_BYTES {
            return Err(CopyError::TooLarge);
        }
        let window = web_sys::window().ok_or(CopyError::Unavailable)?;
        if !window.is_secure_context() {
            return Err(CopyError::Unavailable);
        }
        let clipboard = Reflect::get(window.navigator().as_ref(), &JsValue::from_str("clipboard"))
            .map_err(|_| CopyError::Unavailable)?;
        if clipboard.is_null() || clipboard.is_undefined() {
            return Err(CopyError::Unavailable);
        }
        let write = Reflect::get(&clipboard, &JsValue::from_str("writeText"))
            .ok()
            .and_then(|value| value.dyn_into::<Function>().ok())
            .ok_or(CopyError::Unavailable)?;
        // Reserve before calling into JavaScript: a page-provided wrapper can
        // synchronously re-enter this adapter from writeText.
        self.pending.set(true);
        let promise = match write.call1(&clipboard, &JsValue::from_str(&text)) {
            Ok(value) => match value.dyn_into::<Promise>() {
                Ok(promise) => promise,
                Err(_) => {
                    self.pending.set(false);
                    return Err(CopyError::Unavailable);
                }
            },
            Err(_) => {
                self.pending.set(false);
                return Err(CopyError::Rejected);
            }
        };
        let pending = Rc::clone(&self.pending);
        spawn_local(async move {
            let result = JsFuture::from(promise)
                .await
                .map(|_| ())
                .map_err(|_| CopyError::Rejected);
            pending.set(false);
            complete(result);
        });
        Ok(())
    }
    /// Start a clipboard read synchronously in the user's paste gesture.
    /// Permission, activation and browser support may still reject the promise.
    /// An already-started read cannot be revoked by shutdown; fence the result
    /// to the initiating editor revision and selection.
    pub fn read_text(&self, complete: ReadCompletion) -> Result<(), ReadError> {
        if !self.active.get() {
            return Err(ReadError::Cancelled);
        }
        if self.pending.get() {
            return Err(ReadError::Busy);
        }
        let window = web_sys::window().ok_or(ReadError::Unavailable)?;
        if !window.is_secure_context() {
            return Err(ReadError::Unavailable);
        }
        let clipboard = Reflect::get(window.navigator().as_ref(), &JsValue::from_str("clipboard"))
            .map_err(|_| ReadError::Unavailable)?;
        if clipboard.is_null() || clipboard.is_undefined() {
            return Err(ReadError::Unavailable);
        }
        let read = Reflect::get(&clipboard, &JsValue::from_str("readText"))
            .ok()
            .and_then(|value| value.dyn_into::<Function>().ok())
            .ok_or(ReadError::Unavailable)?;
        self.pending.set(true);
        let promise = match read.call0(&clipboard) {
            Ok(value) => match value.dyn_into::<Promise>() {
                Ok(promise) => promise,
                Err(_) => {
                    self.pending.set(false);
                    return Err(ReadError::Unavailable);
                }
            },
            Err(_) => {
                self.pending.set(false);
                return Err(ReadError::Rejected);
            }
        };
        let pending = Rc::clone(&self.pending);
        spawn_local(async move {
            let result = JsFuture::from(promise)
                .await
                .map_err(|_| ReadError::Rejected)
                .and_then(|value| value.as_string().ok_or(ReadError::Rejected))
                .and_then(|text| {
                    if text.len() > MAX_READ_BYTES {
                        Err(ReadError::TooLarge)
                    } else {
                        Ok(text)
                    }
                });
            pending.set(false);
            complete(result);
        });
        Ok(())
    }
    /// Prevent subsequent operations. A browser operation already started in a
    /// user gesture cannot be revoked; its result still reaches the completion.
    pub fn shutdown(&self) {
        self.active.set(false);
    }
}

impl Drop for Clipboard {
    fn drop(&mut self) {
        self.shutdown();
    }
}
