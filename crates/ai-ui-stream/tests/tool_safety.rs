//! A tool must never run on arguments that did not finish arriving, however a parser could repair them.

use ai_ui_core::adapter::RunStream;
use ai_ui_core::*;
use ai_ui_stream::{
    run_stream, DataStreamDecoder, MessageAccumulator, SseDecoder, SseLimits,
    UiMessageStreamDecoder,
};
use futures_core::Stream;
use serde_json::json;
use std::cell::Cell;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

struct Reads(Vec<Vec<u8>>);
impl Stream for Reads {
    type Item = Result<Vec<u8>, String>;
    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready((!self.0.is_empty()).then(|| Ok(self.0.remove(0))))
    }
}

/// Run `body` through `decoder` with a frontend tool `f` that counts how often it runs.
fn calls_made(body: &'static [u8], decoder: fn(Reads) -> RunStream) -> usize {
    let calls = Rc::new(Cell::new(0));
    let counter = calls.clone();
    let runtime = LocalThreadRuntime::new(
        move |_options: ChatModelRunOptions| decoder(Reads(vec![body.to_vec()])),
        LocalRuntimeOptions::default(),
    );
    runtime.set_tools(ToolRegistry::new().with(
        "f",
        Tool::new(json!({ "type": "object" })).execute(move |_args, _context| {
            counter.set(counter.get() + 1);
            async { Ok(json!("ran")) }
        }),
    ));
    let run = runtime
        .append(AppendMessage::user_text(None, "go"))
        .unwrap();
    let _ = futures_executor::block_on(run);
    calls.get()
}

fn ui(reads: Reads) -> RunStream {
    run_stream(
        reads,
        UiMessageStreamDecoder::new(SseLimits::default()),
        MessageAccumulator::default(),
    )
}
fn data(reads: Reads) -> RunStream {
    run_stream(
        reads,
        DataStreamDecoder::new(1 << 20),
        MessageAccumulator::default(),
    )
}
fn transport(reads: Reads) -> RunStream {
    run_stream(reads, SseDecoder::default(), MessageAccumulator::default())
}

#[test]
fn ui_stream_cut_off_inside_tool_arguments_runs_nothing() {
    let body = b"data: {\"type\":\"tool-input-start\",\"toolCallId\":\"c\",\"toolName\":\"f\"}\n\ndata: {\"type\":\"tool-input-delta\",\"toolCallId\":\"c\",\"inputTextDelta\":\"{\\\"x\\\":\"}\n\n";
    assert_eq!(calls_made(body, ui), 0);
}

#[test]
fn data_stream_cut_off_inside_tool_arguments_runs_nothing() {
    let body = b"b:{\"toolCallId\":\"c\",\"toolName\":\"f\"}\nc:{\"toolCallId\":\"c\",\"argsTextDelta\":\"{\\\"x\\\":\"}\n";
    assert_eq!(calls_made(body, data), 0);
}

#[test]
fn transport_cut_off_inside_tool_arguments_runs_nothing() {
    let body = b"data: {\"type\":\"part-start\",\"part\":{\"type\":\"tool-call\",\"toolCallId\":\"c\",\"toolName\":\"f\"}}\n\ndata: {\"type\":\"text-delta\",\"path\":[0],\"textDelta\":\"{\\\"x\\\":\"}\n\n";
    assert_eq!(calls_made(body, transport), 0);
}

#[test]
fn complete_arguments_do_run_the_tool() {
    // The control: the same flow with finished arguments does execute, so the tests above mean something.
    let body = b"data: {\"type\":\"tool-input-available\",\"toolCallId\":\"c\",\"toolName\":\"f\",\"input\":{\"x\":1}}\n\ndata: {\"type\":\"finish\",\"finishReason\":\"tool-calls\"}\n\ndata: [DONE]\n\n";
    assert_eq!(calls_made(body, ui), 1);
}
