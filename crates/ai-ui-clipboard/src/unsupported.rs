use crate::{Completion, CopyError, ReadCompletion, ReadError};

/// Clipboard is not implemented on this target.
pub struct Clipboard;

impl Clipboard {
    pub fn new() -> Result<Self, CopyError> {
        Err(CopyError::Unavailable)
    }

    pub fn copy(&self, _text: String, _complete: Completion) -> Result<(), CopyError> {
        Err(CopyError::Unavailable)
    }

    pub fn read_text(&self, _complete: ReadCompletion) -> Result<(), ReadError> {
        Err(ReadError::Unavailable)
    }

    pub fn shutdown(&self) {}
}
