//! Streaming cost measurements (run with `--release -- --ignored --nocapture`).
//! They print timings instead of asserting, to find scaling problems.

use ai_ui_core::*;
use ai_ui_stream::*;
use std::time::Instant;

fn text_delta_body(total_bytes: usize, token_bytes: usize) -> Vec<u8> {
    let mut body = String::from("data: {\"type\":\"text-start\",\"id\":\"t\"}\n\n");
    let token = "w".repeat(token_bytes - 1) + " ";
    for _ in 0..total_bytes / token_bytes {
        body.push_str(&format!(
            "data: {{\"type\":\"text-delta\",\"id\":\"t\",\"delta\":\"{token}\"}}\n\n"
        ));
    }
    body.push_str("data: {\"type\":\"finish\",\"finishReason\":\"stop\"}\n\ndata: [DONE]\n\n");
    body.into_bytes()
}

#[test]
#[ignore]
fn accumulator_and_result_cost_by_message_length() {
    for total in [10_000usize, 100_000, 400_000] {
        let body = text_delta_body(total, 6);
        let mut decoder = UiMessageStreamDecoder::default();
        let mut accumulator = MessageAccumulator::default();
        let (mut push, mut results) = (0.0, 0.0);
        let mut slices = 0;
        for slice in body.chunks(256) {
            let t = Instant::now();
            let chunks = decoder.push(slice).unwrap();
            chunks.into_iter().for_each(|c| accumulator.push(c));
            push += t.elapsed().as_secs_f64();
            // The runtime asks for the cumulative result after every slice.
            let t = Instant::now();
            std::hint::black_box(accumulator.result());
            results += t.elapsed().as_secs_f64();
            slices += 1;
        }
        println!("{total:>7} B in {slices:>5} slices: decode+fold {:>7.1} ms, result() {:>8.1} ms ({:.1} us each)", push * 1e3, results * 1e3, results * 1e6 / slices as f64);
    }
}

#[test]
#[ignore]
fn runtime_update_cost_by_thread_length() {
    for messages in [10usize, 100, 1000] {
        let updates = 2000;
        let runtime = LocalThreadRuntime::new(
            move |_o: ChatModelRunOptions| -> RunStream {
                // A reply growing one token per cumulative result.
                let mut text = String::new();
                ai_ui_core::adapter::from_results((0..updates).map(move |_| {
                    text.push_str("token ");
                    Ok(ChatModelRunResult::text(text.clone()))
                }))
            },
            LocalRuntimeOptions::default(),
        );
        let history: Vec<ThreadMessage> = (0..messages)
            .map(|i| {
                serde_json::from_value(serde_json::json!({
                    "role": if i % 2 == 0 { "user" } else { "assistant" }, "id": format!("m{i}"), "createdAt": 0,
                    "content": [{ "type": "text", "text": "history message" }],
                    "status": { "type": "complete", "reason": "stop" }
                }))
                .unwrap()
            })
            .collect();
        runtime
            .import(ExportedRepository::from_linear(history))
            .unwrap();
        // A UI subscriber that reads the selected path on every change.
        let reads = std::rc::Rc::new(std::cell::Cell::new(0usize));
        let (r, rt) = (reads.clone(), runtime.clone());
        let _sub = runtime.subscribe(move || r.set(r.get() + rt.messages().len()));
        let parent = runtime.messages().last().map(|m| m.id().clone());
        let t = Instant::now();
        futures_executor::block_on(
            runtime
                .append(AppendMessage::user_text(parent, "go"))
                .unwrap(),
        )
        .unwrap();
        println!(
            "{messages:>5} messages: {:>7.1} us per streamed update",
            t.elapsed().as_secs_f64() * 1e6 / updates as f64
        );
    }
}

/// The `benchmarks/ai-sdk-stream` text workload (10,000 deltas of 24 bytes in one text part) through this crate's
/// UI-stream path, for a same-machine comparison with `ai-ui-sdk-stream`. Results arrive once per read.
#[test]
#[ignore]
fn ui_stream_text_workload_like_the_ai_sdk_harness() {
    use ai_ui_stream::{run_stream, MessageAccumulator, SseLimits, UiMessageStreamDecoder};
    use futures_core::Stream;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    struct Reads(std::vec::IntoIter<Vec<u8>>);
    impl Stream for Reads {
        type Item = Result<Vec<u8>, String>;
        fn poll_next(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            Poll::Ready(self.0.next().map(Ok))
        }
    }
    let mut wire = String::from("data: {\"type\":\"text-start\",\"id\":\"t\"}\n\n");
    for _ in 0..10_000 {
        wire.push_str("data: {\"type\":\"text-delta\",\"id\":\"t\",\"delta\":\"abcdefghijklmnopqrstuvwx\"}\n\n");
    }
    wire.push_str("data: {\"type\":\"text-end\",\"id\":\"t\"}\n\ndata: [DONE]\n\n");
    let events: Vec<Vec<u8>> = wire
        .split_inclusive("\n\n")
        .map(|e| e.as_bytes().to_vec())
        .collect();
    let chunked: Vec<Vec<u8>> = wire.as_bytes().chunks(4096).map(<[u8]>::to_vec).collect();
    let mut results = Vec::new();
    for (name, reads) in [("4 KB reads", chunked), ("one read per event", events)] {
        let mut times = Vec::new();
        let mut count = 0;
        for _ in 0..9 {
            let t = Instant::now();
            let mut stream = run_stream(
                Reads(reads.clone().into_iter()),
                UiMessageStreamDecoder::new(SseLimits::default()),
                MessageAccumulator::default(),
            );
            count = 0;
            futures_executor::block_on(async {
                while let Some(item) =
                    std::future::poll_fn(|cx| stream.as_mut().poll_next(cx)).await
                {
                    std::hint::black_box(item.unwrap());
                    count += 1;
                }
            });
            times.push(t.elapsed().as_secs_f64() * 1e3);
        }
        times.sort_by(f64::total_cmp);
        results.push(format!(
            "{name}: median {:.2} ms over 9 runs, {count} results yielded",
            times[4]
        ));
    }
    println!("{}", results.join("\n"));
}

/// Isolate decoder + accumulator work from transport polling and cumulative
/// snapshots. Build each wire once, then feed borrowed slices in release mode.
#[test]
#[ignore]
fn protocol_decode_and_fold_throughput() {
    fn median<D: StreamDecoder>(new_decoder: impl Fn() -> D, wire: &[u8], width: usize) -> f64 {
        let mut samples = Vec::new();
        for _ in 0..9 {
            let mut decoder = new_decoder();
            let mut accumulator = MessageAccumulator::default();
            let start = Instant::now();
            for slice in wire.chunks(width) {
                for chunk in decoder.push(slice).unwrap() {
                    accumulator.push(chunk);
                }
            }
            for chunk in decoder.finish().unwrap() {
                accumulator.push(chunk);
            }
            std::hint::black_box(accumulator.parts());
            samples.push(start.elapsed().as_secs_f64() * 1e3);
            assert!(
                matches!(&accumulator.parts()[0], AssistantMessagePart::Text(text) if text.text.len() == 240_000)
            );
        }
        samples.sort_by(f64::total_cmp);
        samples[4]
    }

    let ui = text_delta_body(240_000, 24);
    let mut transport = String::from(
        "data: {\"type\":\"part-start\",\"part\":{\"type\":\"text\"},\"path\":[]}\n\n",
    );
    for _ in 0..10_000 {
        transport.push_str("data: {\"type\":\"text-delta\",\"textDelta\":\"abcdefghijklmnopqrstuvwx\",\"path\":[0]}\n\n");
    }
    transport.push_str("data: {\"type\":\"part-finish\",\"path\":[0]}\n\ndata: [DONE]\n\n");
    for width in [1, 64, 4096] {
        println!(
            "{width:>4} byte reads, 10,000 deltas: UI {:.3} ms, assistant-transport {:.3} ms (median of 9)",
            median(UiMessageStreamDecoder::default, &ui, width),
            median(SseDecoder::default, transport.as_bytes(), width),
        );
    }
}
