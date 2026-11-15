//! A built-in model, so the example runs with no server. It streams a
//! reasoning part and then either a Markdown answer or a tool call, and
//! answers from the tool's result on the next step, exactly the sequence a
//! real model produces:
//!
//! - "weather" → calls `get_weather` (runs without asking)
//! - "email" → calls `send_email` (the user must approve it)
//! - anything else → a Markdown answer

use crate::delay::Delay;
use ai_ui_core::adapter::{ChatModelRunOptions, RunError, RunStream};
use ai_ui_core::{
    AbortSignal, AssistantMessagePart, ChatModelAdapter, ChatModelRunResult, CompleteReason,
    MessageStatus, PartStatus, ReasoningPart, RequiresActionReason, TextPart, ToolCallPart,
};
use futures_core::Stream;
use serde_json::{json, Value};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

pub struct DemoModel {
    /// Pause between tokens; zero produces the whole answer at once.
    pub token_delay: Duration,
}

/// What a step ends with.
enum Outcome {
    Answer(String),
    Call(Box<ToolCallPart>),
}

struct Plan {
    reasoning: String,
    outcome: Outcome,
}

fn city(prompt: &str) -> String {
    let words: Vec<&str> = prompt
        .trim_end_matches(['?', '.', '!'])
        .split_whitespace()
        .collect();
    words
        .iter()
        .position(|w| w.eq_ignore_ascii_case("in"))
        .and_then(|i| words.get(i + 1..))
        .map(|rest| rest.join(" "))
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| "Tokyo".into())
}

fn call(id: &str, name: &str, args: Value) -> ToolCallPart {
    ToolCallPart {
        tool_call_id: id.into(),
        tool_name: name.into(),
        args_text: args.to_string(),
        args: args.as_object().cloned().unwrap_or_default(),
        ..Default::default()
    }
}

fn plan(options: &ChatModelRunOptions) -> Plan {
    let prompt = options
        .messages
        .last()
        .map(|m| m.text())
        .unwrap_or_default();
    let topic = prompt.trim().trim_end_matches('?').to_owned();
    // A step after a tool ran: answer from its result.
    if let Some(result) = options.message.tool_calls().find_map(|c| c.result.clone()) {
        return Plan {
            reasoning: String::new(),
            outcome: Outcome::Answer(after_tool(&result)),
        };
    }
    let lower = prompt.to_lowercase();
    if lower.contains("weather") {
        let city = city(&prompt);
        Plan {
            reasoning: format!("The user wants the weather in {city}. I'll call get_weather."),
            outcome: Outcome::Call(Box::new(call(
                "call-weather",
                "get_weather",
                json!({ "city": city }),
            ))),
        }
    } else if lower.contains("email") {
        Plan {
            reasoning: "Sending an email changes things outside the chat, so I'll call send_email and let the user confirm.".into(),
            outcome: Outcome::Call(Box::new(call(
                "call-email",
                "send_email",
                json!({ "to": "team@example.com", "subject": "Quick update" }),
            ))),
        }
    } else {
        Plan {
            reasoning: format!("The user asked about '{topic}'. A short answer with a list, code and a table shows the renderer."),
            outcome: Outcome::Answer(answer(&topic)),
        }
    }
}

fn after_tool(result: &Value) -> String {
    if let Some(error) = result.get("error").and_then(Value::as_str) {
        return format!("That didn't go through: **{error}**. Nothing was changed.");
    }
    if let (Some(city), Some(temp)) = (result.get("city"), result.get("temp_c")) {
        return format!(
            "It's currently **{} C** and *{}* in **{}**.\n\nThat came from the `get_weather` tool, which ran inside the app and fed its result back to the model.",
            temp,
            result["sky"].as_str().unwrap_or("clear"),
            city.as_str().unwrap_or("there"),
        );
    }
    if result.get("sent").and_then(Value::as_bool) == Some(true) {
        return format!(
            "Done: the email to **{}** is sent.",
            result["to"].as_str().unwrap_or("the team")
        );
    }
    format!("The tool returned `{result}`.")
}

impl ChatModelAdapter for DemoModel {
    fn run(&self, options: ChatModelRunOptions) -> RunStream {
        let plan = plan(&options);
        let (body, call) = match plan.outcome {
            Outcome::Answer(text) => (tokens(&text), None),
            // One step to emit the call itself.
            Outcome::Call(call) => (vec![String::new()], Some(*call)),
        };
        Box::pin(DemoStream {
            reasoning: tokens(&plan.reasoning),
            body,
            call,
            emitted: 0,
            delay: Delay::new(Duration::ZERO),
            token_delay: self.token_delay,
            abort: options.abort_signal,
            done: false,
        })
    }
}

fn answer(topic: &str) -> String {
    format!(
        "Here's a quick answer about **{topic}**.\n\n\
         This reply comes from the example's local demo model, streamed token by token through \
         the Rust port of assistant-ui's runtime.\n\n\
         - Edit a message or regenerate an answer to create **branches**\n\
         - Every update is a cumulative `ChatModelRunResult`, as upstream\n\
         - Stop the stream at any time\n\n\
         Try asking for the **weather**, or to send an **email**, to see tools and approvals.\n\n\
         ```rust\n\
         let runtime = LocalThreadRuntime::new(adapter, LocalRuntimeOptions::default());\n\
         slint::spawn_local(runtime.append(AppendMessage::user_text(None, \"{topic}\"))?)?;\n\
         ```\n\n\
         | Layer | Crate |\n|---|---|\n| Runtime | `ai-ui-core` |\n| Protocols | `ai-ui-stream` |\n| UI | `ai-ui-slint` |\n"
    )
}

/// Words with their trailing whitespace.
fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for c in text.chars() {
        current.push(c);
        if c.is_whitespace() {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

struct DemoStream {
    reasoning: Vec<String>,
    body: Vec<String>,
    call: Option<ToolCallPart>,
    emitted: usize,
    delay: Delay,
    token_delay: Duration,
    abort: AbortSignal,
    done: bool,
}

impl DemoStream {
    fn snapshot(&self) -> ChatModelRunResult {
        let reasoned = self.emitted.min(self.reasoning.len());
        let revealed = self
            .emitted
            .saturating_sub(self.reasoning.len())
            .min(self.body.len());
        let reasoning_done = reasoned == self.reasoning.len();
        let body_done = revealed == self.body.len();
        let status = |done: bool| {
            Some(if done {
                PartStatus::Complete
            } else {
                PartStatus::Running
            })
        };
        let mut content = Vec::new();
        if !self.reasoning.is_empty() {
            content.push(AssistantMessagePart::Reasoning(ReasoningPart {
                text: self.reasoning[..reasoned].concat(),
                status: status(reasoning_done),
                ..Default::default()
            }));
        }
        let mut finish = None;
        if reasoning_done {
            match (&self.call, body_done) {
                (Some(call), true) => {
                    content.push(AssistantMessagePart::ToolCall(call.clone()));
                    finish = Some(MessageStatus::RequiresAction {
                        reason: RequiresActionReason::ToolCalls,
                    });
                }
                (Some(_), false) => {}
                (None, _) => {
                    content.push(AssistantMessagePart::Text(TextPart {
                        text: self.body[..revealed].concat(),
                        status: status(body_done),
                        ..Default::default()
                    }));
                    if body_done {
                        finish = Some(MessageStatus::Complete {
                            reason: CompleteReason::Stop,
                        });
                    }
                }
            }
        }
        let result = ChatModelRunResult::content(content);
        match finish {
            Some(status) => result.with_status(status),
            None => result,
        }
    }
}

impl Stream for DemoStream {
    type Item = Result<ChatModelRunResult, RunError>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.done || self.abort.is_aborted() {
            return Poll::Ready(None);
        }
        if Pin::new(&mut self.delay).poll(cx).is_pending() {
            return Poll::Pending;
        }
        let total = self.reasoning.len() + self.body.len();
        self.emitted = if self.token_delay.is_zero() {
            total
        } else {
            (self.emitted + 1).min(total)
        };
        self.done = self.emitted == total;
        self.delay = Delay::new(self.token_delay);
        Poll::Ready(Some(Ok(self.snapshot())))
    }
}
