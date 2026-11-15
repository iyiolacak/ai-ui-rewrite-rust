//! The native client and the data-stream adapter against a real AI SDK 7 backend: `streamText` over a mock model,
//! served by `benchmarks/ai-sdk-stream/interop-server.mjs`. Skipped (with a message) when Node or the harness's
//! packages are missing; set `AI_UI_REQUIRE_INTEROP=1` to make that a failure, as CI does.
#![cfg(not(target_arch = "wasm32"))]

use ai_ui_core::*;
use ai_ui_data_stream::{DataStreamAdapter, DataStreamOptions};
use ai_ui_http::NativeClient;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

struct Server {
    child: Child,
    port: u16,
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn harness() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/ai-sdk-stream")
}

/// Start the server, or explain why it cannot be started.
fn start() -> Result<Server, String> {
    let dir = harness();
    if !dir.join("node_modules/ai").exists() {
        return Err("run `npm ci --ignore-scripts` in benchmarks/ai-sdk-stream".into());
    }
    let mut child = Command::new("node")
        .arg("interop-server.mjs")
        .current_dir(&dir)
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run node: {e}"))?;
    let mut line = String::new();
    BufReader::new(child.stdout.take().expect("piped"))
        .read_line(&mut line)
        .map_err(|e| format!("server did not start: {e}"))?;
    let port = line
        .trim()
        .strip_prefix("LISTENING ")
        .and_then(|p| p.parse().ok())
        .ok_or_else(|| format!("unexpected server output {line:?}"))?;
    Ok(Server { child, port })
}

fn get(port: u16, path: &str) -> Value {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(stream, "GET {path} HTTP/1.0\r\nHost: localhost\r\n\r\n").unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    serde_json::from_str(response.split("\r\n\r\n").nth(1).unwrap()).unwrap()
}

#[test]
fn a_real_ai_sdk_backend_streams_reasoning_a_server_tool_and_text() {
    let server = match start() {
        Ok(server) => server,
        Err(reason) => {
            assert!(
                std::env::var_os("AI_UI_REQUIRE_INTEROP").is_none(),
                "interop test cannot run: {reason}"
            );
            eprintln!("skipped: {reason}");
            return;
        }
    };
    let url = format!("http://127.0.0.1:{}/api/chat", server.port);
    let runtime = LocalThreadRuntime::new(
        DataStreamAdapter::new(NativeClient::new(), DataStreamOptions::new(url)),
        LocalRuntimeOptions::default(),
    );

    let run = runtime
        .append(AppendMessage::user_text(
            None,
            "What is the weather in Paris?",
        ))
        .unwrap();
    futures_executor::block_on(run).expect("the run succeeds against a real AI SDK server");

    let messages = runtime.messages();
    let answer = messages[1].as_assistant().unwrap();
    assert_eq!(
        answer.status,
        MessageStatus::Complete {
            reason: CompleteReason::Stop
        }
    );
    let parts = serde_json::to_value(&answer.content).unwrap();
    let kinds: Vec<&str> = parts
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["type"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["reasoning", "tool-call", "text"], "{parts}");
    assert_eq!(parts[0]["text"], "Thinking it over.");
    assert_eq!(parts[1]["toolName"], "weather");
    assert_eq!(parts[1]["args"], json!({ "city": "Paris" }));
    assert_eq!(parts[1]["result"], json!({ "city": "Paris", "temp_c": 21 }));
    assert_eq!(parts[2]["text"], "It is 21 degrees.");
    assert_eq!(answer.metadata.steps.len(), 2, "two model steps");

    // What the server received is what upstream's client sends (model-style messages, no UI parts).
    let request = get(server.port, "/last-request");
    assert_eq!(request["messages"][0]["role"], "user");
    assert_eq!(
        request["messages"][0]["content"][0]["text"],
        "What is the weather in Paris?"
    );

    // A follow-up replays the tool call and its result; the AI SDK must accept that history and convert it to a prompt.
    let parent = runtime.messages().last().map(|m| m.id().clone());
    let run = runtime
        .append(AppendMessage::user_text(parent, "And tomorrow?"))
        .unwrap();
    futures_executor::block_on(run).expect("the SDK accepts the replayed tool history");
    let prompt = get(server.port, "/last-prompt");
    let roles: Vec<&str> = prompt
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["role"].as_str().unwrap())
        .collect();
    assert_eq!(
        roles,
        ["user", "assistant", "tool", "assistant", "user"],
        "{prompt}"
    );
    assert_eq!(prompt[1]["content"][0]["input"], json!({ "city": "Paris" }));
    assert_eq!(
        prompt[2]["content"][0]["output"],
        json!({ "type": "json", "value": { "city": "Paris", "temp_c": 21 } })
    );
    assert_eq!(prompt[3]["content"][0]["text"], "It is 21 degrees.");
}
