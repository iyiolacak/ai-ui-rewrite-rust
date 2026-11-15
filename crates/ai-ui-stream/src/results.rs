//! Turn a response byte stream into the runtime's [`RunStream`].

use crate::accumulator::MessageAccumulator;
use crate::decoder::StreamDecoder;
use crate::sse::SseDecoder;
use ai_ui_core::adapter::{RunError, RunStream};
use ai_ui_core::ChatModelRunResult;
use futures_core::Stream;
use std::fmt::Display;
use std::pin::Pin;
use std::task::{Context, Poll};

/// Decode a response with `decoder` and yield the cumulative message after
/// each wake-up that changed it: slices that are already available are folded
/// into one result. A transport or decoding error, or a stream the
/// decoder considers cut short, fails the run; accumulated content stays on
/// the message.
pub fn run_stream<S, E, D>(bytes: S, decoder: D, accumulator: MessageAccumulator) -> RunStream
where
    S: Stream<Item = Result<Vec<u8>, E>> + 'static,
    E: Display + 'static,
    D: StreamDecoder + 'static,
{
    Box::pin(Results {
        bytes: Box::pin(bytes),
        decoder: Box::new(decoder),
        accumulator,
        dirty: false,
        stopped: false,
        error: None,
        yield_pending: false,
    })
}

/// [`run_stream`] for assistant-stream's `assistant-transport` SSE format.
pub fn run_stream_from_sse<S, E>(bytes: S, accumulator: MessageAccumulator) -> RunStream
where
    S: Stream<Item = Result<Vec<u8>, E>> + 'static,
    E: Display + 'static,
{
    run_stream(bytes, SseDecoder::default(), accumulator)
}

/// Slices folded in one poll before yielding, so an endless ready source
/// cannot starve the consumer.
const MAX_SLICES_PER_POLL: usize = 64;

struct Results<E, D> {
    bytes: Pin<Box<dyn Stream<Item = Result<Vec<u8>, E>>>>,
    decoder: Box<D>,
    accumulator: MessageAccumulator,
    /// Chunks were folded since the last result was yielded.
    dirty: bool,
    /// No more input will be read.
    stopped: bool,
    /// A failure to report after the content received before it.
    error: Option<RunError>,
    /// A full batch was published; yield before reading another ready batch.
    yield_pending: bool,
}

impl<E: Display, D: StreamDecoder> Results<E, D> {
    /// Fold a decoder's output; a failure stops the stream.
    fn absorb(&mut self, decoded: Result<Vec<crate::chunk::Chunk>, String>) {
        match decoded {
            Err(error) => {
                self.stopped = true;
                self.error = Some(RunError::failed(error));
            }
            Ok(chunks) => {
                self.dirty |= !chunks.is_empty();
                chunks
                    .into_iter()
                    .for_each(|chunk| self.accumulator.push(chunk));
            }
        }
    }

    fn take_result(&mut self) -> Option<Result<ChatModelRunResult, RunError>> {
        self.dirty.then(|| {
            self.dirty = false;
            Ok(self.accumulator.result())
        })
    }
}

/// Yields the cumulative message once per wake-up: every slice that is already
/// available is folded first, so a burst costs one result, not one per slice.
impl<E: Display, D: StreamDecoder> Stream for Results<E, D> {
    type Item = Result<ChatModelRunResult, RunError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = &mut *self;
        if std::mem::take(&mut this.yield_pending) {
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        let mut folded = 0;
        loop {
            if this.stopped {
                // Content received before a failure is delivered first.
                return Poll::Ready(this.take_result().or_else(|| this.error.take().map(Err)));
            }
            if folded >= MAX_SLICES_PER_POLL {
                if let Some(result) = this.take_result() {
                    // An eager `.next().await` consumer can immediately poll
                    // again in the same task. Make that next poll cooperate too.
                    this.yield_pending = true;
                    return Poll::Ready(Some(result));
                }
                // Heartbeats, ignored events, and incomplete frames do not make
                // the accumulator dirty. They must still yield to the executor
                // so a ready source cannot starve input, painting, or cancel.
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
            match this.bytes.as_mut().poll_next(cx) {
                Poll::Pending => {
                    return match this.take_result() {
                        Some(result) => Poll::Ready(Some(result)),
                        None => Poll::Pending,
                    };
                }
                Poll::Ready(Some(Err(error))) => {
                    this.stopped = true;
                    this.error = Some(RunError::failed(error.to_string()));
                }
                Poll::Ready(None) => {
                    this.stopped = true;
                    let decoded = this.decoder.finish();
                    this.absorb(decoded);
                    if this.error.is_none() {
                        this.accumulator.finish();
                        this.dirty = true;
                    }
                }
                Poll::Ready(Some(Ok(bytes))) => {
                    folded += 1;
                    let decoded = this.decoder.push(&bytes);
                    this.absorb(decoded);
                }
            }
        }
    }
}
