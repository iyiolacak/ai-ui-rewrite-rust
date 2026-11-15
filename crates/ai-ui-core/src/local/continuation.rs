//! Rules for continuing a run after tool calls, ported from
//! `runtimes/local/should-continue.ts` and `withContinuation`.

use crate::types::{
    AssistantMessage, AssistantMessagePart, MessageStatus, RequiresActionReason, ToolCallPart,
};
use std::collections::HashMap;

/// Whether the run should take another model step: the message asks for tool
/// results, none awaits an approval, and every tool call the frontend must
/// answer has an answer. Calls to tools not in `human_tools` are expected to be
/// answered elsewhere and do not hold the run; with no list, all must be answered.
pub fn should_continue(message: &AssistantMessage, human_tools: Option<&[String]>) -> bool {
    if message.status
        != (MessageStatus::RequiresAction {
            reason: RequiresActionReason::ToolCalls,
        })
    {
        return false;
    }
    let mut calls = message.tool_calls();
    if message
        .tool_calls()
        .any(|c| c.result.is_none() && c.approval.as_ref().is_some_and(|a| a.is_pending()))
    {
        return false;
    }
    calls.all(|c| {
        c.result.is_some()
            || c.approval.is_some()
            || human_tools.is_some_and(|names| !names.contains(&c.tool_name))
    })
}

/// Combine the content a step started with and the step's cumulative content.
/// Open tool calls in `initial` (no final result) are settled in place by a
/// result for the same call in `incoming`; everything else is appended.
pub fn with_continuation(
    initial: &[AssistantMessagePart],
    incoming: Vec<AssistantMessagePart>,
) -> Vec<AssistantMessagePart> {
    let open: HashMap<&str, usize> = initial
        .iter()
        .enumerate()
        .filter_map(|(i, p)| {
            p.as_tool_call()
                .filter(|c| c.is_open())
                .map(|c| (c.tool_call_id.as_str(), i))
        })
        .collect();
    if open.is_empty() {
        let mut content = initial.to_vec();
        content.extend(incoming);
        return content;
    }
    let mut content = initial.to_vec();
    let mut appended = Vec::new();
    for part in incoming {
        let target = part
            .as_tool_call()
            .and_then(|c| open.get(c.tool_call_id.as_str()).copied());
        match (target, &part) {
            (Some(index), AssistantMessagePart::ToolCall(call)) => {
                if call.result.is_some() {
                    if let AssistantMessagePart::ToolCall(open_call) = &mut content[index] {
                        settle(open_call, call);
                    }
                }
                // A repeated call without a result adds nothing new.
            }
            _ => appended.push(part),
        }
    }
    content.extend(appended);
    content
}

fn settle(target: &mut ToolCallPart, from: &ToolCallPart) {
    target.result = from.result.clone();
    target.is_error = from.is_error;
    target.is_preliminary = from.is_preliminary.filter(|p| *p);
    target.artifact = from.artifact.clone().or(target.artifact.take());
}

/// Keep results the app added while the step streamed (`addToolResult`) when
/// the adapter's newer snapshot still shows those calls open.
pub fn with_external_results(
    stored: &[AssistantMessagePart],
    incoming: Vec<AssistantMessagePart>,
) -> Vec<AssistantMessagePart> {
    let settled: HashMap<&str, &ToolCallPart> = stored
        .iter()
        .filter_map(AssistantMessagePart::as_tool_call)
        .filter(|c| !c.is_open())
        .map(|c| (c.tool_call_id.as_str(), c))
        .collect();
    if settled.is_empty() {
        return incoming;
    }
    incoming
        .into_iter()
        .map(|part| match part {
            AssistantMessagePart::ToolCall(mut call) if call.is_open() => {
                if let Some(done) = settled.get(call.tool_call_id.as_str()) {
                    settle(&mut call, done);
                }
                AssistantMessagePart::ToolCall(call)
            }
            other => other,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AssistantMetadata, CompleteReason, Timestamp, ToolApproval};
    use serde_json::json;

    fn call(id: &str, name: &str, result: Option<serde_json::Value>) -> AssistantMessagePart {
        AssistantMessagePart::ToolCall(ToolCallPart {
            tool_call_id: id.into(),
            tool_name: name.into(),
            result,
            ..ToolCallPart::default()
        })
    }
    fn message(status: MessageStatus, content: Vec<AssistantMessagePart>) -> AssistantMessage {
        AssistantMessage {
            id: "a".into(),
            created_at: Timestamp(0),
            content,
            status,
            metadata: AssistantMetadata::default(),
        }
    }
    const TOOLS: MessageStatus = MessageStatus::RequiresAction {
        reason: RequiresActionReason::ToolCalls,
    };

    #[test]
    fn continues_only_when_every_call_is_answered() {
        assert!(should_continue(
            &message(TOOLS, vec![call("1", "a", Some(json!(1)))]),
            None
        ));
        assert!(!should_continue(
            &message(TOOLS, vec![call("1", "a", None)]),
            None
        ));
        assert!(!should_continue(
            &message(
                MessageStatus::Complete {
                    reason: CompleteReason::Stop
                },
                vec![]
            ),
            None
        ));
        let interrupt = MessageStatus::RequiresAction {
            reason: RequiresActionReason::Interrupt,
        };
        assert!(!should_continue(
            &message(interrupt, vec![call("1", "a", Some(json!(1)))]),
            None
        ));
    }

    #[test]
    fn human_tool_list_limits_which_calls_hold_the_run() {
        let content = vec![call("1", "ask_user", None), call("2", "server", None)];
        let humans = ["ask_user".to_owned()];
        assert!(!should_continue(
            &message(TOOLS, content.clone()),
            Some(&humans)
        ));
        let content = vec![
            call("1", "ask_user", Some(json!("yes"))),
            call("2", "server", None),
        ];
        assert!(should_continue(&message(TOOLS, content), Some(&humans)));
    }

    #[test]
    fn a_pending_approval_pauses_the_run() {
        let mut part = ToolCallPart {
            tool_call_id: "1".into(),
            tool_name: "a".into(),
            ..Default::default()
        };
        part.approval = Some(ToolApproval {
            id: "ap".into(),
            ..Default::default()
        });
        assert!(!should_continue(
            &message(TOOLS, vec![AssistantMessagePart::ToolCall(part.clone())]),
            None
        ));
        part.approval.as_mut().unwrap().approved = Some(false);
        assert!(should_continue(
            &message(TOOLS, vec![AssistantMessagePart::ToolCall(part)]),
            None
        ));
    }

    #[test]
    fn continuation_settles_open_calls_in_place_and_appends_new_parts() {
        let initial = vec![AssistantMessagePart::text("before"), call("1", "a", None)];
        let incoming = vec![
            call("1", "a", Some(json!(42))),
            AssistantMessagePart::text("after"),
        ];
        let content = with_continuation(&initial, incoming);
        assert_eq!(content.len(), 3);
        assert_eq!(content[1].as_tool_call().unwrap().result, Some(json!(42)));
        assert_eq!(content[2], AssistantMessagePart::text("after"));
    }

    #[test]
    fn without_open_calls_continuation_appends() {
        let initial = vec![call("1", "a", Some(json!(1)))];
        let content = with_continuation(&initial, vec![AssistantMessagePart::text("next")]);
        assert_eq!(content.len(), 2);
    }

    #[test]
    fn external_results_survive_a_stale_snapshot() {
        let stored = vec![call("1", "a", Some(json!("done")))];
        let merged = with_external_results(
            &stored,
            vec![call("1", "a", None), AssistantMessagePart::text("t")],
        );
        assert_eq!(
            merged[0].as_tool_call().unwrap().result,
            Some(json!("done"))
        );
    }
}
