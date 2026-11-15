//! Run scoping, resource bounds and the ordered byte-to-state pipeline.

use serde_json::{json, Value};

use crate::framing::SseFramer;
use crate::{ApplyResult, ReducerOptions, StreamError, UiReducer, UnknownPolicy, Update};

/// Opaque generation captured when the host starts network work.
///
/// Tickets belong to one [`Runtime`]. Do not transfer them between runtimes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RunTicket(u64);

/// An indexed part reference valid only for its run and structural generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PartHandle {
    ticket: RunTicket,
    generation: u64,
    index: usize,
}

/// Explicit application resource policy, separate from upstream wire semantics.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Maximum combined assembled data/current physical line bytes, including
    /// the current line's SSE prefix. Also bounds comments and ignored fields.
    pub event_bytes: usize,
    /// Maximum initial-message JSON bytes plus received wire bytes per run.
    /// This conservative bound is not an exact accounting of allocator overhead.
    pub run_bytes: usize,
    /// Maximum stored parts in the active message.
    pub parts: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            event_bytes: 2 * 1024 * 1024,
            run_bytes: 64 * 1024 * 1024,
            parts: 16_384,
        }
    }
}

/// Whether an explicit server `error` chunk stops the run after being delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerErrorPolicy {
    /// Match the upstream read helper's default: report the event and continue.
    Continue,
    /// Report the event, preserve content, then reject subsequent input.
    Stop,
}

/// Compatibility and application policy for one stream runtime.
#[derive(Debug, Clone)]
pub struct Config {
    /// Configurable resource limits; oversized input fails explicitly.
    pub limits: Limits,
    /// Unknown event handling. Reject is source-faithful; Preserve is opt-in.
    pub unknown_policy: UnknownPolicy,
    /// Explicit server-error policy; malformed wire/lifecycle errors always stop.
    pub server_error_policy: ServerErrorPolicy,
    /// Match `readUIMessageStream`: a changed message ID starts a new message.
    pub reset_on_message_id_change: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            limits: Limits::default(),
            unknown_policy: UnknownPolicy::Reject,
            server_error_policy: ServerErrorPolicy::Continue,
            reset_on_message_id_change: true,
        }
    }
}

impl Config {
    fn reducer_options(&self) -> ReducerOptions {
        ReducerOptions {
            max_parts: self.limits.parts,
            unknown_policy: self.unknown_policy,
            reset_on_message_id_change: self.reset_on_message_id_change,
        }
    }
}

/// Transport state. A protocol `finish`/`abort`/`[DONE]` does not imply EOF.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunState {
    /// More bytes can be accepted.
    Open,
    /// The host explicitly reported transport EOF.
    Eof,
    /// The host cancelled this run locally; partial content remains readable.
    Cancelled,
    /// A malformed event, invalid lifecycle, limit or stopping server error.
    Failed(StreamError),
}

/// Bounded scalar observations; the runtime does not retain an event log.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RunStats {
    /// Wire bytes accepted within the configured run budget.
    pub wire_bytes: usize,
    /// Successfully applied JSON events, including non-publishing control events.
    pub events: usize,
    /// Observed `[DONE]` markers, which the compatibility profile ignores.
    pub done_markers: usize,
}

/// Whether a host completion belonged to the active run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedOutcome {
    /// The current run accepted the operation.
    Applied,
    /// Old work was ignored before parsing or modifying any state.
    IgnoredStale,
}

/// A read-only view into the current message; no copy is made to create it.
#[derive(Clone, Copy)]
pub struct MessageView<'a> {
    reducer: &'a UiReducer,
    ticket: RunTicket,
    generation: u64,
}

impl<'a> MessageView<'a> {
    /// Return a complete owned AI SDK UI message. Streaming tool previews are
    /// materialized here; this operation scales with the requested message.
    #[must_use]
    pub fn snapshot(self) -> Value {
        self.reducer.snapshot()
    }

    /// Number of currently retained parts; independent of conversation history.
    #[must_use]
    pub fn part_count(self) -> usize {
        self.reducer.part_count()
    }

    /// Make a scoped handle for an existing part index from an [`Update`].
    #[must_use]
    pub fn handle(self, index: usize) -> Option<PartHandle> {
        self.reducer.part(index).map(|_| PartHandle {
            ticket: self.ticket,
            generation: self.generation,
            index,
        })
    }

    /// Read stored structural fields without cloning. Lazy streaming-tool input
    /// fields are available through [`Self::raw_tool_input`] or [`Self::part_snapshot`].
    #[must_use]
    pub fn part(self, handle: PartHandle) -> Option<&'a Value> {
        if handle.ticket != self.ticket || handle.generation != self.generation {
            return None;
        }
        self.reducer.part(handle.index)
    }

    /// Materialize only the requested part, including its partial JSON preview.
    #[must_use]
    pub fn part_snapshot(self, handle: PartHandle) -> Option<Value> {
        self.part(handle)?;
        self.reducer.part_snapshot(handle.index)
    }

    /// Borrow accumulated tool argument text. The renderer may read only the new
    /// suffix using its previous byte length; no full-prefix clone is required.
    #[must_use]
    pub fn raw_tool_input(self, handle: PartHandle) -> Option<&'a str> {
        self.part(handle)?;
        self.reducer.raw_tool_input(handle.index)
    }
}

/// One fully applied event. Inspect or collect its small updates synchronously.
///
/// All semantic events are delivered in order; a host may coalesce subsequent
/// paint work, but must not discard transient data, approvals, errors or aborts.
pub struct Delivery<'a> {
    /// The event's ordered notifications and upstream publication flag.
    pub applied: &'a ApplyResult,
    /// Borrowed current state, valid for the duration of this callback.
    pub view: MessageView<'a>,
}

/// Single-owner incremental AI SDK 7 UI-message stream runtime.
pub struct Runtime {
    config: Config,
    framer: SseFramer,
    reducer: UiReducer,
    ticket: RunTicket,
    generation: u64,
    initial_bytes: usize,
    state: RunState,
    stats: RunStats,
}

impl Runtime {
    /// Create an empty assistant message with a caller-supplied ID.
    ///
    /// # Errors
    /// Returns a validation/limit error if the initial message exceeds policy.
    pub fn new(message_id: impl Into<String>, config: Config) -> Result<Self, StreamError> {
        Self::with_message(
            json!({"id":message_id.into(),"role":"assistant","parts":[]}),
            config,
        )
    }

    /// Resume an existing assistant message. Active text IDs are not inferred;
    /// current-step input-streaming tools are reconstructed as upstream does.
    ///
    /// # Errors
    /// Returns a validation/limit error without starting a partially valid run.
    pub fn with_message(message: Value, config: Config) -> Result<Self, StreamError> {
        let initial_bytes = initial_size(&message, config.limits.run_bytes)?;
        let reducer = UiReducer::with_message(message, config.reducer_options())?;
        Ok(Self {
            framer: SseFramer::new(config.limits.event_bytes),
            config,
            reducer,
            ticket: RunTicket(1),
            generation: 0,
            initial_bytes,
            state: RunState::Open,
            stats: RunStats::default(),
        })
    }

    /// Capture this ticket before starting an asynchronous read.
    #[must_use]
    pub fn ticket(&self) -> RunTicket {
        self.ticket
    }

    /// Inspect transport completion separately from protocol finish/abort events.
    #[must_use]
    pub fn state(&self) -> &RunState {
        &self.state
    }

    /// Scalar counters only; no event/message snapshots are retained for metrics.
    #[must_use]
    pub fn stats(&self) -> RunStats {
        self.stats
    }

    /// Borrow current state without notifying the renderer or cloning the message.
    #[must_use]
    pub fn view(&self) -> MessageView<'_> {
        MessageView {
            reducer: &self.reducer,
            ticket: self.ticket,
            generation: self.generation,
        }
    }

    /// Start a new run atomically, invalidating old callbacks and all part handles.
    ///
    /// # Errors
    /// Invalid initial state or exhausted generation leaves the previous run intact.
    pub fn restart(&mut self, message: Value) -> Result<RunTicket, StreamError> {
        let next = self.ticket.0.checked_add(1).ok_or(StreamError::Limit {
            resource: "run generations",
            limit: usize::MAX,
        })?;
        let initial_bytes = initial_size(&message, self.config.limits.run_bytes)?;
        let reducer = UiReducer::with_message(message, self.config.reducer_options())?;
        self.reducer = reducer;
        self.framer = SseFramer::new(self.config.limits.event_bytes);
        self.initial_bytes = initial_bytes;
        self.ticket = RunTicket(next);
        self.generation = 0;
        self.state = RunState::Open;
        self.stats = RunStats::default();
        Ok(self.ticket)
    }

    /// Consume arbitrary network bytes and synchronously deliver each applied event.
    /// A stale ticket is ignored before examining the bytes. Network I/O and UI
    /// scheduling remain outside this function; callers should use bounded reads.
    ///
    /// # Errors
    /// JSON, schema, lifecycle and resource failures poison the run and preserve
    /// earlier committed content. Later calls return [`StreamError::Closed`].
    pub fn push(
        &mut self,
        ticket: RunTicket,
        bytes: &[u8],
        mut observe: impl FnMut(Delivery<'_>),
    ) -> Result<FeedOutcome, StreamError> {
        if ticket != self.ticket {
            return Ok(FeedOutcome::IgnoredStale);
        }
        if self.state != RunState::Open {
            return Err(StreamError::Closed);
        }
        let new_size = self
            .initial_bytes
            .checked_add(self.stats.wire_bytes)
            .and_then(|size| size.checked_add(bytes.len()));
        if new_size.is_none_or(|size| size > self.config.limits.run_bytes) {
            return self.fail(StreamError::Limit {
                resource: "run bytes",
                limit: self.config.limits.run_bytes,
            });
        }
        self.stats.wire_bytes += bytes.len();
        let reducer = &mut self.reducer;
        let generation = &mut self.generation;
        let stats = &mut self.stats;
        let stop_on_error = self.config.server_error_policy == ServerErrorPolicy::Stop;
        let result = self.framer.feed(bytes, |data| {
            if data == b"[DONE]" {
                stats.done_markers = stats.done_markers.saturating_add(1);
                return Ok(());
            }
            // Browser TextDecoder is non-fatal by default. Decode completed SSE
            // event bytes, never individual network fragments, so split UTF-8 is safe.
            let text = String::from_utf8_lossy(data);
            let chunk =
                serde_json::from_str(&text).map_err(|e| StreamError::Json(e.to_string()))?;
            let applied = reducer.apply(chunk)?;
            if applied
                .updates
                .iter()
                .any(|update| matches!(update, Update::Reset))
            {
                *generation = generation.checked_add(1).ok_or(StreamError::Limit {
                    resource: "part generations",
                    limit: usize::MAX,
                })?;
            }
            stats.events = stats.events.saturating_add(1);
            observe(Delivery {
                applied: &applied,
                view: MessageView {
                    reducer,
                    ticket,
                    generation: *generation,
                },
            });
            if stop_on_error {
                if let Some(text) = applied.updates.iter().find_map(|update| {
                    if let Update::Error { error_text } = update {
                        Some(error_text)
                    } else {
                        None
                    }
                }) {
                    return Err(StreamError::Server(text.clone()));
                }
            }
            Ok(())
        });
        if let Err(error) = result {
            return self.fail(error);
        }
        Ok(FeedOutcome::Applied)
    }

    /// Report transport EOF. Missing `[DONE]` is accepted, and an unfinished SSE
    /// event is discarded. This does not synthesize text-end or finish events.
    ///
    /// # Errors
    /// Returns [`StreamError::Closed`] for an already ended current run.
    pub fn end(&mut self, ticket: RunTicket) -> Result<FeedOutcome, StreamError> {
        if ticket != self.ticket {
            return Ok(FeedOutcome::IgnoredStale);
        }
        if self.state != RunState::Open {
            return Err(StreamError::Closed);
        }
        self.framer.discard();
        self.state = RunState::Eof;
        Ok(FeedOutcome::Applied)
    }

    /// Cancel locally without inventing a backend abort or losing partial content.
    /// Returns false for stale tickets or an already ended run.
    pub fn cancel(&mut self, ticket: RunTicket) -> bool {
        if ticket != self.ticket || self.state != RunState::Open {
            return false;
        }
        self.framer.discard();
        self.state = RunState::Cancelled;
        true
    }

    fn fail<T>(&mut self, error: StreamError) -> Result<T, StreamError> {
        self.framer.discard();
        self.state = RunState::Failed(error.clone());
        Err(error)
    }
}

fn initial_size(message: &Value, limit: usize) -> Result<usize, StreamError> {
    let size = serde_json::to_vec(message)
        .map_err(|e| StreamError::Validation(e.to_string()))?
        .len();
    if size > limit {
        return Err(StreamError::Limit {
            resource: "initial message bytes",
            limit,
        });
    }
    Ok(size)
}
