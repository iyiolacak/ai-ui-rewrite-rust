//! A byte-stream adapter driving the local runtime end to end.

use ai_ui_core::adapter::RunStream;
use ai_ui_core::*;
use ai_ui_stream::{run_stream_from_sse, MessageAccumulator};
use futures_core::Stream;
use std::pin::Pin;
use std::task::{Context, Poll};

/// Response bytes delivered in fixed slices, like network reads.
struct Bytes(Vec<Vec<u8>>);
impl Stream for Bytes {
    type Item = Result<Vec<u8>, String>;
    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready((!self.0.is_empty()).then(|| Ok(self.0.remove(0))))
    }
}

fn adapter(body: &'static [u8]) -> impl ChatModelAdapter {
    move |_options: ChatModelRunOptions| -> RunStream {
        run_stream_from_sse(
            Bytes(body.chunks(64).map(<[u8]>::to_vec).collect()),
            MessageAccumulator::default(),
        )
    }
}

#[test]
fn an_sse_response_streams_into_the_thread() {
    let body = include_bytes!("fixtures/python-encoder.sse");
    let runtime = LocalThreadRuntime::new(adapter(body), LocalRuntimeOptions::default());
    let run = runtime
        .append(AppendMessage::user_text(None, "Weather in NYC?"))
        .unwrap();
    futures_executor::block_on(run).unwrap();
    let answer = runtime.messages()[1].as_assistant().unwrap().clone();
    assert_eq!(answer.content.len(), 6);
    assert_eq!(answer.metadata.steps.len(), 1);
    assert_eq!(
        answer.metadata.state,
        serde_json::json!({ "status": "running" })
    );
    assert!(matches!(
        answer.status,
        MessageStatus::Incomplete {
            reason: IncompleteReason::Error,
            ..
        }
    ));
}

#[test]
fn a_truncated_response_fails_the_run_and_keeps_partial_content() {
    let body = b"data: {\"type\": \"part-start\", \"part\": {\"type\": \"text\"}, \"path\": []}\n\ndata: {\"type\": \"text-delta\", \"textDelta\": \"Hal\", \"path\": [0]}\n\n";
    let runtime = LocalThreadRuntime::new(adapter(body), LocalRuntimeOptions::default());
    let run = runtime
        .append(AppendMessage::user_text(None, "Hi"))
        .unwrap();
    let error = futures_executor::block_on(run).unwrap_err();
    assert!(matches!(error, RunError::Failed { .. }));
    let answer = runtime.messages()[1].as_assistant().unwrap().clone();
    assert_eq!(
        answer.content,
        [AssistantMessagePart::Text(TextPart {
            text: "Hal".into(),
            status: Some(PartStatus::Running),
            ..Default::default()
        })]
    );
    assert!(matches!(
        answer.status,
        MessageStatus::Incomplete {
            reason: IncompleteReason::Error,
            ..
        }
    ));
}

/// Count the results a body yields when every slice is immediately ready.
fn results_of(body: &'static [u8], slice: usize) -> Vec<Result<ChatModelRunResult, RunError>> {
    let mut stream = run_stream_from_sse(
        Bytes(body.chunks(slice).map(<[u8]>::to_vec).collect()),
        MessageAccumulator::default(),
    );
    let mut results = Vec::new();
    futures_executor::block_on(async {
        while let Some(item) = std::future::poll_fn(|cx| stream.as_mut().poll_next(cx)).await {
            results.push(item);
        }
    });
    results
}

#[test]
fn ready_slices_are_folded_into_one_result() {
    let body = include_bytes!("fixtures/python-encoder.sse");
    // Many tiny slices, all ready at once: one result, not one per slice.
    let tiny = results_of(body, 7);
    let whole = results_of(body, body.len());
    assert!(tiny.len() < body.len() / 7 / 4, "{} results", tiny.len());
    assert_eq!(
        tiny.last().unwrap().as_ref().unwrap(),
        whole.last().unwrap().as_ref().unwrap(),
    );
}

#[test]
fn content_before_a_failure_is_delivered_first() {
    let body = b"data: {\"type\": \"part-start\", \"part\": {\"type\": \"text\"}, \"path\": []}\n\ndata: {\"type\": \"text-delta\", \"textDelta\": \"Hal\", \"path\": [0]}\n\n";
    let results = results_of(body, 16);
    let (last, content) = results.split_last().unwrap();
    assert!(last.is_err());
    assert!(!content.is_empty() && content.iter().all(Result::is_ok));
}

#[test]
fn ready_heartbeats_and_partial_frames_yield_without_losing_progress() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::task::{Wake, Waker};

    #[derive(Default)]
    struct Wakes(AtomicUsize);
    impl Wake for Wakes {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    let partial_comment = format!(": {}\n\n", "x".repeat(1024));
    for mut reads in [
        vec![b": keep-alive\n\n".to_vec(); 1024],
        vec![Vec::new(); 1024],
        partial_comment.bytes().map(|byte| vec![byte]).collect(),
    ] {
        reads.push(b"data: [DONE]\n\n".to_vec());
        let mut stream = run_stream_from_sse(Bytes(reads), MessageAccumulator::default());
        let wakes = Arc::new(Wakes::default());
        let waker = Waker::from(wakes.clone());
        let mut context = Context::from_waker(&waker);

        // A finite but busy ready source must hand control back before EOF,
        // even when none of its reads changes the visible message.
        assert!(stream.as_mut().poll_next(&mut context).is_pending());
        assert!(wakes.0.load(Ordering::Relaxed) > 0, "schedule another poll");

        let results = futures_executor::block_on(async {
            let mut results = Vec::new();
            while let Some(result) = std::future::poll_fn(|cx| stream.as_mut().poll_next(cx)).await
            {
                results.push(result);
            }
            results
        });
        assert_eq!(results.len(), 1, "no synthetic heartbeat results");
        assert!(results[0].is_ok(), "resume through the stream terminator");
    }
}

#[test]
fn ready_content_bursts_yield_between_cumulative_results() {
    let mut reads = vec![
        b"data: {\"type\":\"part-start\",\"part\":{\"type\":\"text\"},\"path\":[]}\n\n".to_vec(),
    ];
    reads.extend(std::iter::repeat_n(
        b"data: {\"type\":\"text-delta\",\"textDelta\":\"x\",\"path\":[0]}\n\n".to_vec(),
        1024,
    ));
    reads.push(b"data: [DONE]\n\n".to_vec());
    let mut stream = run_stream_from_sse(Bytes(reads), MessageAccumulator::default());
    let mut context = Context::from_waker(std::task::Waker::noop());
    assert!(matches!(
        stream.as_mut().poll_next(&mut context),
        Poll::Ready(Some(Ok(_)))
    ));
    // Returning Ready alone is insufficient: an async consumer's next().await
    // can keep running synchronously. Its next poll must return control.
    assert!(stream.as_mut().poll_next(&mut context).is_pending());
    let last = futures_executor::block_on(async {
        let mut last = None;
        while let Some(result) = std::future::poll_fn(|cx| stream.as_mut().poll_next(cx)).await {
            last = Some(result.unwrap());
        }
        last.unwrap()
    });
    assert!(
        matches!(&last.content.unwrap()[0], AssistantMessagePart::Text(text) if text.text == "x".repeat(1024))
    );
}
