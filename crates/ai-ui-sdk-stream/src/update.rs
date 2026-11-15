//! Small ordered notifications; full snapshots remain an explicit read.

use serde_json::Value;

/// A semantic event or a dirty-part notification produced by the reducer.
///
/// Part indices address the current message only. A [`Self::Reset`] invalidates
/// previously queued part indices. A host must also scope work to its run ticket.
#[derive(Debug, Clone, PartialEq)]
pub enum Update {
    /// A part was appended to the ordered message.
    PartAdded {
        /// Position of the new part.
        index: usize,
    },
    /// An existing part changed; read only this part when presenting it.
    PartChanged {
        /// Position of the changed part.
        index: usize,
    },
    /// Message or step contents changed structurally; refresh current handles.
    Reset,
    /// Message metadata changed.
    MetadataChanged,
    /// A start event supplied the current message identifier.
    MessageIdChanged,
    /// A data event was observed; transient events are never stored as parts.
    Data {
        /// The data part as the wire carried it (`type`, `data` and optional `id`).
        part: Value,
        /// Transient parts are reported here only, never stored in the message.
        transient: bool,
    },
    /// A finish event was observed; it does not close active content parts.
    Finish {
        /// The finish reason, if the event had one.
        reason: Option<String>,
    },
    /// An abort event was observed; it does not erase partial content.
    Abort {
        /// Why the server aborted, if it said.
        reason: Option<String>,
    },
    /// A server error event was observed; the host chooses whether to stop.
    Error {
        /// The server's error text.
        error_text: String,
    },
    /// A step boundary was appended.
    StepStarted,
    /// A step finished; other interleaved parts may remain active.
    StepFinished,
    /// An unrecognized event was preserved under an explicit tolerant policy.
    Unknown {
        /// The event as received.
        chunk: Value,
    },
}

/// The effects of applying one complete, validated event.
#[derive(Debug, Default, Clone, PartialEq)]
#[must_use = "semantic events and dirty-part notifications must be consumed"]
pub struct ApplyResult {
    /// Notifications in the order in which the event affected state.
    pub updates: Vec<Update>,
    /// Whether the AI SDK 7 read helper would emit a message snapshot here.
    ///
    /// Structural step-start changes are visible through direct reads even
    /// though the upstream helper waits for a later publishing event.
    pub published: bool,
}
