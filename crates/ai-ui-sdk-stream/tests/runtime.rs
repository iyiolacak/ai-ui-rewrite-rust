//! Runtime policy and ownership invariants, distinct from upstream wire fixtures.

use ai_ui_sdk_stream::{
    Config, FeedOutcome, Limits, RunState, Runtime, ServerErrorPolicy, StreamError, UnknownPolicy,
    Update,
};
use serde_json::{json, Value};

#[allow(
    clippy::unwrap_used,
    reason = "The fixed valid test configuration must construct"
)]
fn runtime() -> Runtime {
    Runtime::new("m", Config::default()).unwrap()
}

#[allow(
    clippy::unwrap_used,
    reason = "These fixtures expect successful delivery; failure fails the test"
)]
fn feed(runtime: &mut Runtime, events: &[Value]) {
    let ticket = runtime.ticket();
    for event in events {
        let wire = format!("data: {event}\n\n");
        runtime.push(ticket, wire.as_bytes(), |_| {}).unwrap();
    }
}

#[test]
fn assistant_runtime_stale_network_and_eof_cannot_publish_under_a_new_run() {
    let mut stream = runtime();
    let stale = stream.ticket();
    stream
        .restart(json!({"id":"next","role":"assistant","parts":[]}))
        .unwrap();
    let mut calls = 0;
    assert_eq!(
        stream
            .push(stale, b"data: {broken}\n\n", |_| calls += 1)
            .unwrap(),
        FeedOutcome::IgnoredStale
    );
    assert_eq!(stream.end(stale).unwrap(), FeedOutcome::IgnoredStale);
    assert!(!stream.cancel(stale));
    assert_eq!(calls, 0);
    assert_eq!(stream.stats().wire_bytes, 0);
    assert_eq!(stream.state(), &RunState::Open);
    assert_eq!(
        stream.view().snapshot(),
        json!({"id":"next","role":"assistant","parts":[]})
    );
}

#[test]
fn assistant_runtime_reset_invalidates_queued_part_handles_even_if_index_is_reused() {
    let mut stream = runtime();
    feed(&mut stream, &[json!({"type":"text-start","id":"a"})]);
    let old = stream.view().handle(0).unwrap();
    feed(
        &mut stream,
        &[
            json!({"type":"reset-step"}),
            json!({"type":"text-start","id":"b"}),
        ],
    );
    assert!(stream.view().part(old).is_none());
    let new = stream.view().handle(0).unwrap();
    assert_ne!(old, new);
    assert!(stream.view().part(new).is_some());
}

#[test]
fn assistant_runtime_finish_abort_and_done_do_not_invent_text_end_or_transport_eof() {
    let mut stream = runtime();
    feed(
        &mut stream,
        &[
            json!({"type":"text-start","id":"t"}),
            json!({"type":"text-delta","id":"t","delta":"a"}),
            json!({"type":"finish","finishReason":"stop"}),
            json!({"type":"abort","reason":"host"}),
        ],
    );
    let ticket = stream.ticket();
    stream.push(ticket, b"data: [DONE]\n\n", |_| {}).unwrap();
    feed(
        &mut stream,
        &[json!({"type":"text-delta","id":"t","delta":"b"})],
    );
    assert_eq!(stream.state(), &RunState::Open);
    assert_eq!(stream.stats().done_markers, 1);
    stream.end(ticket).unwrap();
    assert_eq!(stream.state(), &RunState::Eof);
    assert_eq!(
        stream.view().snapshot()["parts"][0],
        json!({"type":"text","text":"ab","state":"streaming"})
    );
}

#[test]
fn assistant_runtime_malformed_later_event_preserves_content_and_earlier_notifications() {
    let mut stream = runtime();
    let ticket = stream.ticket();
    let wire = b"data: {\"type\":\"text-start\",\"id\":\"t\"}\n\ndata: {\"type\":\"text-delta\",\"id\":\"t\",\"delta\":\"kept\"}\n\ndata: {bad}\n\ndata: {\"type\":\"text-delta\",\"id\":\"t\",\"delta\":\"lost\"}\n\n";
    let mut published = 0;
    let result = stream.push(ticket, wire, |delivery| {
        published += usize::from(delivery.applied.published)
    });
    assert!(matches!(result, Err(StreamError::Json(_))));
    assert_eq!(published, 2);
    assert_eq!(stream.view().snapshot()["parts"][0]["text"], "kept");
    assert!(matches!(
        stream.state(),
        RunState::Failed(StreamError::Json(_))
    ));
    assert_eq!(
        stream.push(ticket, b"", |_| {}).unwrap_err(),
        StreamError::Closed
    );
}

#[test]
fn assistant_runtime_invalid_known_event_is_atomic() {
    let mut stream = runtime();
    feed(&mut stream, &[json!({"type":"text-start","id":"t"})]);
    let before = stream.view().snapshot();
    let ticket = stream.ticket();
    assert!(stream
        .push(
            ticket,
            b"data: {\"type\":\"text-delta\",\"id\":\"t\",\"delta\":5}\n\n",
            |_| {}
        )
        .is_err());
    assert_eq!(stream.view().snapshot(), before);
}

#[test]
fn assistant_runtime_resource_limits_are_explicit_and_stale_input_uses_no_budget() {
    let config = Config {
        limits: Limits {
            event_bytes: 80,
            run_bytes: 200,
            parts: 1,
        },
        ..Config::default()
    };
    let mut stream = Runtime::new("m", config).unwrap();
    feed(&mut stream, &[json!({"type":"text-start","id":"one"})]);
    let ticket = stream.ticket();
    let result = stream.push(
        ticket,
        b"data: {\"type\":\"text-start\",\"id\":\"two\"}\n\n",
        |_| {},
    );
    assert!(matches!(
        result,
        Err(StreamError::Limit {
            resource: "parts",
            ..
        })
    ));
    assert_eq!(stream.view().part_count(), 1);

    let mut stream = Runtime::new(
        "m",
        Config {
            limits: Limits {
                event_bytes: 1000,
                run_bytes: 100,
                parts: 10,
            },
            ..Config::default()
        },
    )
    .unwrap();
    let ticket = stream.ticket();
    assert!(matches!(
        stream.push(ticket, &[b' '; 101], |_| {}),
        Err(StreamError::Limit {
            resource: "run bytes",
            ..
        })
    ));
    assert_eq!(stream.stats().wire_bytes, 0);
}

#[test]
fn assistant_runtime_server_error_policy_is_explicit_and_not_a_wire_parse_error() {
    let wire = b"data: {\"type\":\"error\",\"errorText\":\"offline\"}\n\ndata: {\"type\":\"text-start\",\"id\":\"t\"}\n\n";
    for policy in [ServerErrorPolicy::Continue, ServerErrorPolicy::Stop] {
        let mut stream = Runtime::new(
            "m",
            Config {
                server_error_policy: policy,
                ..Config::default()
            },
        )
        .unwrap();
        let ticket = stream.ticket();
        let mut errors = 0;
        let result = stream.push(ticket, wire, |delivery| {
            errors += delivery
                .applied
                .updates
                .iter()
                .filter(|update| matches!(update, Update::Error { .. }))
                .count();
        });
        assert_eq!(errors, 1);
        match policy {
            ServerErrorPolicy::Continue => {
                assert!(result.is_ok());
                assert_eq!(stream.view().part_count(), 1);
            }
            ServerErrorPolicy::Stop => {
                assert_eq!(result.unwrap_err(), StreamError::Server("offline".into()));
                assert_eq!(stream.view().part_count(), 0);
            }
        }
    }
}

#[test]
fn assistant_runtime_transient_events_are_ordered_without_growing_message_history() {
    let mut stream = runtime();
    let ticket = stream.ticket();
    let mut observed = 0;
    for expected in 0..1000 {
        let event = format!(
            "data: {{\"type\":\"data-progress\",\"data\":{expected},\"transient\":true}}\n\n"
        );
        stream
            .push(ticket, event.as_bytes(), |delivery| {
                for update in &delivery.applied.updates {
                    if let Update::Data {
                        part,
                        transient: true,
                    } = update
                    {
                        assert_eq!(part["data"], expected);
                        observed += 1;
                    }
                }
            })
            .unwrap();
    }
    assert_eq!(observed, 1000);
    assert_eq!(stream.view().part_count(), 0);
}

#[test]
fn assistant_runtime_unknown_chunks_require_an_explicit_preserving_policy() {
    let mut stream = Runtime::new(
        "m",
        Config {
            unknown_policy: UnknownPolicy::Preserve,
            ..Config::default()
        },
    )
    .unwrap();
    let ticket = stream.ticket();
    let mut preserved = None;
    stream
        .push(
            ticket,
            b"data: {\"type\":\"future-part\",\"payload\":17}\n\n",
            |delivery| {
                for update in &delivery.applied.updates {
                    if let Update::Unknown { chunk } = update {
                        preserved = Some(chunk.clone());
                    }
                }
            },
        )
        .unwrap();
    assert_eq!(preserved, Some(json!({"type":"future-part","payload":17})));
    assert_eq!(stream.view().part_count(), 0);
}

#[test]
fn assistant_runtime_host_cancellation_preserves_partial_content_and_stops_publication() {
    let mut stream = runtime();
    feed(
        &mut stream,
        &[
            json!({"type":"text-start","id":"t"}),
            json!({"type":"text-delta","id":"t","delta":"partial"}),
        ],
    );
    let ticket = stream.ticket();
    assert!(stream.cancel(ticket));
    assert!(!stream.cancel(ticket));
    assert_eq!(stream.state(), &RunState::Cancelled);
    assert_eq!(stream.view().snapshot()["parts"][0]["text"], "partial");
    assert_eq!(
        stream
            .push(ticket, b"data: [DONE]\n\n", |_| {})
            .unwrap_err(),
        StreamError::Closed
    );
}

#[test]
fn assistant_runtime_failed_restart_does_not_discard_the_previous_message_or_ticket() {
    let mut stream = runtime();
    feed(&mut stream, &[json!({"type":"text-start","id":"t"})]);
    let ticket = stream.ticket();
    let before = stream.view().snapshot();
    assert!(stream.restart(json!({"role":"user","parts":[]})).is_err());
    assert_eq!(stream.ticket(), ticket);
    assert_eq!(stream.view().snapshot(), before);
    assert_eq!(stream.state(), &RunState::Open);
}

#[test]
fn assistant_runtime_raw_tool_text_is_borrowable_and_previews_are_explicit() {
    let mut stream = runtime();
    feed(
        &mut stream,
        &[
            json!({"type":"tool-input-start","toolCallId":"c","toolName":"find"}),
            json!({"type":"tool-input-delta","toolCallId":"c","inputTextDelta":"{\"city\":\"Par"}),
        ],
    );
    let view = stream.view();
    let handle = view.handle(0).unwrap();
    assert_eq!(view.raw_tool_input(handle), Some("{\"city\":\"Par"));
    assert_eq!(
        view.part_snapshot(handle).unwrap()["input"],
        json!({"city":"Par"})
    );
}
