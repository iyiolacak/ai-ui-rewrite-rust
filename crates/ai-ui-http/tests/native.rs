//! The native client against a loopback HTTP server, through the adapter and
//! the local runtime.
#![cfg(not(target_arch = "wasm32"))]

use ai_ui_core::*;
use ai_ui_data_stream::{DataStreamAdapter, DataStreamOptions};
use ai_ui_http::NativeClient;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// Serve one request: record its body, then write `head` and the `body`
/// pieces with a pause between them. `hold` keeps the connection open after.
fn serve(
    head: &'static str,
    body: Vec<&'static str>,
    hold: bool,
) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/api/chat", listener.local_addr().unwrap());
    let (sent, received) = mpsc::channel();
    thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse().unwrap();
            }
            if line == "\r\n" {
                break;
            }
        }
        let mut request = vec![0; length];
        reader.read_exact(&mut request).unwrap();
        sent.send(String::from_utf8(request).unwrap()).unwrap();
        let mut stream = stream;
        stream.write_all(head.as_bytes()).unwrap();
        for piece in body {
            stream.write_all(piece.as_bytes()).unwrap();
            stream.flush().unwrap();
            thread::sleep(Duration::from_millis(20));
        }
        if hold {
            thread::sleep(Duration::from_secs(5));
        }
    });
    (url, received)
}

fn runtime(url: String) -> LocalThreadRuntime {
    LocalThreadRuntime::new(
        DataStreamAdapter::new(NativeClient::new(), DataStreamOptions::new(url)),
        LocalRuntimeOptions::default(),
    )
}

const OK_HEAD: &str = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nx-vercel-ai-ui-message-stream: v1\r\nConnection: close\r\n\r\n";

#[test]
fn streams_an_ai_sdk_response_from_a_real_socket() {
    let (url, request) = serve(
        OK_HEAD,
        vec![
            "data: {\"type\":\"text-start\",\"id\":\"t\"}\n\n",
            "data: {\"type\":\"text-delta\",\"id\":\"t\",\"delta\":\"Hello \"}\n\n",
            "data: {\"type\":\"text-delta\",\"id\":\"t\",\"delta\":\"from the network\"}\n\n",
            "data: {\"type\":\"finish\",\"finishReason\":\"stop\"}\n\ndata: [DONE]\n\n",
        ],
        false,
    );
    let runtime = runtime(url);
    futures_executor::block_on(
        runtime
            .append(AppendMessage::user_text(None, "Hi"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(runtime.messages()[1].text(), "Hello from the network");
    let body: serde_json::Value = serde_json::from_str(&request.recv().unwrap()).unwrap();
    assert_eq!(body["messages"][0]["content"][0]["text"], "Hi");
}

#[test]
fn an_error_status_is_reported() {
    let (url, _request) = serve(
        "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 4\r\nConnection: close\r\n\r\nbusy",
        vec![],
        false,
    );
    let runtime = runtime(url);
    let result = futures_executor::block_on(
        runtime
            .append(AppendMessage::user_text(None, "Hi"))
            .unwrap(),
    );
    assert!(matches!(result, Err(RunError::Failed { message, .. }) if message == "Status 503"));
}

#[test]
fn cancelling_stops_a_stream_the_server_keeps_open() {
    let (url, _request) = serve(
        OK_HEAD,
        vec!["data: {\"type\":\"text-delta\",\"delta\":\"partial\"}\n\n"],
        true,
    );
    let runtime = runtime(url);
    let run = runtime
        .append(AppendMessage::user_text(None, "Hi"))
        .unwrap();
    let canceller = runtime.clone();
    let started = std::time::Instant::now();
    // Cancel from a subscription once the partial text has arrived.
    let _subscription = runtime.subscribe(move || {
        if canceller
            .messages()
            .get(1)
            .is_some_and(|m| m.text() == "partial")
        {
            canceller.cancel_run();
        }
    });
    futures_executor::block_on(run).unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "the run ends without waiting for the server"
    );
    assert_eq!(
        runtime.messages()[1].status(),
        Some(&MessageStatus::cancelled())
    );
    assert_eq!(runtime.messages()[1].text(), "partial");
}
