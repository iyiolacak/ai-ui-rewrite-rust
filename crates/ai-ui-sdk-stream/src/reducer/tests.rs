use serde_json::{json, Value};

use super::{ReducerOptions, UiReducer, UnknownPolicy};
use crate::{error::StreamError, update::Update};

fn reducer() -> UiReducer {
    UiReducer::new("m".to_owned(), ReducerOptions::default())
}

fn apply(reducer: &mut UiReducer, events: &[Value]) {
    for event in events {
        let _result = reducer.apply(event.clone()).expect("valid test event");
    }
}

#[test]
fn assistant_reducer_text_ids_are_scoped_by_kind_and_duplicate_starts_append() {
    let mut reducer = reducer();
    apply(
        &mut reducer,
        &[
            json!({"type":"text-start","id":"same"}),
            json!({"type":"text-delta","id":"same","delta":"old"}),
            json!({"type":"reasoning-start","id":"same"}),
            json!({"type":"reasoning-delta","id":"same","delta":"thought"}),
            json!({"type":"text-start","id":"same"}),
            json!({"type":"text-delta","id":"same","delta":"new"}),
            json!({"type":"text-end","id":"same"}),
        ],
    );
    assert_eq!(
        reducer.snapshot(),
        json!({"id":"m","role":"assistant","parts":[
            {"type":"text","text":"old","state":"streaming"},
            {"type":"reasoning","id":"same","text":"thought","state":"streaming"},
            {"type":"text","text":"new","state":"done"}
        ]})
    );
}

#[test]
fn assistant_reducer_schema_failure_is_atomic_and_known_extras_are_ignored() {
    let mut reducer = reducer();
    apply(
        &mut reducer,
        &[json!({"type":"text-start","id":"t","extra":true})],
    );
    let before = reducer.snapshot();
    assert!(matches!(
        reducer.apply(json!({"type":"text-delta","id":"t","delta":"bad","providerMetadata":null})),
        Err(StreamError::Validation(_))
    ));
    assert_eq!(reducer.snapshot(), before);
    assert!(matches!(
        reducer.apply(json!({"type":"text-delta","id":"missing","delta":"bad"})),
        Err(StreamError::Lifecycle { .. })
    ));
    assert_eq!(reducer.snapshot(), before);
}

#[test]
fn assistant_reducer_tool_delta_defers_previews_and_completion_clears_wire_preview_fields() {
    let mut reducer = reducer();
    apply(
        &mut reducer,
        &[
            json!({"type":"tool-input-start","toolCallId":"c","toolName":"lookup"}),
            json!({"type":"tool-input-delta","toolCallId":"c","inputTextDelta":"{\"city\":\"Par"}),
        ],
    );
    assert!(reducer.part(0).expect("part").get("input").is_none());
    assert_eq!(reducer.raw_tool_input(0), Some("{\"city\":\"Par"));
    assert_eq!(
        reducer.part_snapshot(0),
        Some(
            json!({"type":"tool-lookup","toolCallId":"c","state":"input-streaming","input":{"city":"Par"},"rawInput":"{\"city\":\"Par"})
        )
    );
    apply(
        &mut reducer,
        &[
            json!({"type":"tool-input-available","toolCallId":"c","toolName":"lookup","input":{"city":"Paris"}}),
        ],
    );
    assert_eq!(
        reducer.part_snapshot(0),
        Some(
            json!({"type":"tool-lookup","toolCallId":"c","state":"input-available","input":{"city":"Paris"}})
        )
    );
    assert_eq!(reducer.raw_tool_input(0), Some("{\"city\":\"Par"));
}

#[test]
fn assistant_reducer_late_preliminary_result_matches_upstream_overwrite() {
    let mut reducer = reducer();
    apply(
        &mut reducer,
        &[
            json!({"type":"tool-input-available","toolCallId":"c","toolName":"x","input":null}),
            json!({"type":"tool-output-available","toolCallId":"c","output":"final"}),
            json!({"type":"tool-output-available","toolCallId":"c","output":null,"preliminary":true}),
        ],
    );
    assert_eq!(
        reducer.part_snapshot(0),
        Some(
            json!({"type":"tool-x","toolCallId":"c","state":"output-available","input":null,"output":null,"preliminary":true})
        )
    );
}

#[test]
fn assistant_reducer_step_finish_does_not_close_interleaved_text_and_reset_keeps_boundary() {
    let mut reducer = reducer();
    apply(
        &mut reducer,
        &[
            json!({"type":"start-step"}),
            json!({"type":"text-start","id":"a"}),
            json!({"type":"finish-step"}),
            json!({"type":"text-delta","id":"a","delta":"retained"}),
            json!({"type":"start-step"}),
            json!({"type":"text-start","id":"b"}),
            json!({"type":"reset-step"}),
        ],
    );
    assert_eq!(reducer.part_count(), 3);
    assert_eq!(reducer.part_snapshot(2), Some(json!({"type":"step-start"})));
    assert!(matches!(
        reducer.apply(json!({"type":"text-delta","id":"a","delta":"obsolete"})),
        Err(StreamError::Lifecycle { .. })
    ));
}

#[test]
fn assistant_reducer_output_can_target_prior_step_and_approval_uses_approval_id() {
    let mut reducer = reducer();
    apply(
        &mut reducer,
        &[
            json!({"type":"start-step"}),
            json!({"type":"tool-input-available","toolCallId":"c","toolName":"delete","input":{}}),
            json!({"type":"tool-approval-request","toolCallId":"c","approvalId":"a","inputSchemaInput":null,"isAutomatic":false,"reason":"ask"}),
            json!({"type":"start-step"}),
            json!({"type":"tool-approval-response","approvalId":"a","approved":false,"reason":"no"}),
            json!({"type":"tool-output-denied","toolCallId":"c"}),
        ],
    );
    assert_eq!(
        reducer.part_snapshot(1),
        Some(
            json!({"type":"tool-delete","toolCallId":"c","state":"output-denied","input":{},"approval":{"id":"a","inputSchemaInput":null,"requestReason":"ask","approved":false,"reason":"no"}})
        )
    );
}

#[test]
fn assistant_reducer_data_keys_include_type_and_transient_events_are_not_parts() {
    let mut reducer = reducer();
    apply(
        &mut reducer,
        &[
            json!({"type":"data-a","id":"same","data":1,"extension":"retained"}),
            json!({"type":"data-b","id":"same","data":2}),
            json!({"type":"data-a","id":"same","data":3,"extension":"not replaced"}),
        ],
    );
    let notification = reducer
        .apply(json!({"type":"data-a","id":"same","data":4,"transient":true}))
        .expect("event");
    assert!(!notification.published);
    assert!(matches!(
        notification.updates.as_slice(),
        [Update::Data {
            transient: true,
            ..
        }]
    ));
    assert_eq!(reducer.part_count(), 2);
    assert_eq!(
        reducer.part_snapshot(0),
        Some(json!({"type":"data-a","id":"same","data":3,"extension":"retained"}))
    );
}

#[test]
fn assistant_reducer_unknown_policy_is_explicit_and_validates_known_types_in_both_modes() {
    let mut strict = reducer();
    assert!(strict
        .apply(json!({"type":"future-chunk","opaque":1}))
        .is_err());
    let mut preserving = UiReducer::new(
        "m".to_owned(),
        ReducerOptions {
            unknown_policy: UnknownPolicy::Preserve,
            ..ReducerOptions::default()
        },
    );
    let event = json!({"type":"future-chunk","opaque":1});
    let result = preserving.apply(event.clone()).expect("preserved");
    assert_eq!(result.updates, vec![Update::Unknown { chunk: event }]);
    assert_eq!(preserving.part_count(), 0);
    assert!(preserving
        .apply(json!({"type":"text-start","id":false}))
        .is_err());
}

#[test]
fn assistant_reducer_part_limit_failure_does_not_rebind_a_duplicate_id() {
    let mut reducer = UiReducer::new(
        "m".to_owned(),
        ReducerOptions {
            max_parts: 1,
            ..ReducerOptions::default()
        },
    );
    apply(&mut reducer, &[json!({"type":"text-start","id":"a"})]);
    assert!(matches!(
        reducer.apply(json!({"type":"text-start","id":"a"})),
        Err(StreamError::Limit {
            resource: "parts",
            limit: 1
        })
    ));
    apply(
        &mut reducer,
        &[json!({"type":"text-delta","id":"a","delta":"original"})],
    );
    assert_eq!(
        reducer.part_snapshot(0).expect("part").get("text"),
        Some(&json!("original"))
    );
}

#[test]
fn assistant_reducer_resume_restores_tool_text_but_not_wire_text_ids() {
    let initial = json!({"id":"m","role":"assistant","parts":[{"type":"tool-code","toolCallId":"c","state":"input-streaming","inputEncoding":"text","input":"Hello","rawInput":"Hello"}]});
    let mut reducer = UiReducer::with_message(initial, ReducerOptions::default()).expect("initial");
    apply(
        &mut reducer,
        &[json!({"type":"tool-input-delta","toolCallId":"c","inputTextDelta":" world"})],
    );
    assert_eq!(
        reducer.part_snapshot(0),
        Some(
            json!({"type":"tool-code","toolCallId":"c","state":"input-streaming","inputEncoding":"text","input":"Hello world","rawInput":"Hello world"})
        )
    );
}

#[test]
fn assistant_reducer_message_id_change_invalidates_previous_parts_and_metadata() {
    let mut reducer = reducer();
    apply(
        &mut reducer,
        &[
            json!({"type":"start","messageMetadata":{"old":1}}),
            json!({"type":"text-start","id":"a"}),
            json!({"type":"start","messageId":"new","messageMetadata":{"new":2}}),
        ],
    );
    assert_eq!(
        reducer.snapshot(),
        json!({"id":"new","role":"assistant","parts":[],"metadata":{"new":2}})
    );
    assert!(reducer
        .apply(json!({"type":"text-delta","id":"a","delta":"stale"}))
        .is_err());
}

#[test]
fn assistant_reducer_missing_unknown_payloads_are_not_conflated_with_explicit_null() {
    let mut reducer = reducer();
    let empty = reducer.snapshot();
    for event in [
        json!({"type":"tool-input-available","toolCallId":"c","toolName":"x"}),
        json!({"type":"tool-input-error","toolCallId":"c","toolName":"x","errorText":"bad"}),
        json!({"type":"tool-output-available","toolCallId":"c"}),
        json!({"type":"data-missing"}),
        json!({"type":"message-metadata"}),
    ] {
        assert!(matches!(
            reducer.apply(event),
            Err(StreamError::Validation(_))
        ));
        assert_eq!(reducer.snapshot(), empty);
    }
    apply(
        &mut reducer,
        &[
            json!({"type":"tool-input-available","toolCallId":"c","toolName":"x","input":null}),
            json!({"type":"tool-output-available","toolCallId":"c","output":null}),
            json!({"type":"data-null","data":null}),
            json!({"type":"message-metadata","messageMetadata":null}),
        ],
    );
    assert_eq!(
        reducer.part(0).expect("part").get("input"),
        Some(&Value::Null)
    );
    assert_eq!(
        reducer.part(0).expect("part").get("output"),
        Some(&Value::Null)
    );
    assert_eq!(
        reducer.part(1).expect("part").get("data"),
        Some(&Value::Null)
    );
}

#[test]
fn assistant_reducer_dynamic_new_step_delta_omits_raw_input_once_but_retains_bytes() {
    let mut reducer = reducer();
    apply(
        &mut reducer,
        &[
            json!({"type":"tool-input-start","toolCallId":"c","toolName":"code","dynamic":true}),
            json!({"type":"tool-input-delta","toolCallId":"c","inputTextDelta":"{\"x\":1"}),
            json!({"type":"start-step"}),
            json!({"type":"tool-input-delta","toolCallId":"c","inputTextDelta":",\"y\":2}"}),
        ],
    );
    let part = reducer.part_snapshot(2).expect("new dynamic part");
    assert_eq!(part.get("input"), Some(&json!({"x":1,"y":2})));
    assert!(part.get("rawInput").is_none());
    assert_eq!(reducer.raw_tool_input(2), Some("{\"x\":1,\"y\":2}"));
    assert_eq!(
        reducer
            .part_snapshot(0)
            .expect("prior part")
            .get("rawInput"),
        Some(&json!("{\"x\":1"))
    );
    apply(
        &mut reducer,
        &[json!({"type":"tool-input-delta","toolCallId":"c","inputTextDelta":" "})],
    );
    assert_eq!(
        reducer
            .part_snapshot(2)
            .expect("updated dynamic part")
            .get("rawInput"),
        Some(&json!("{\"x\":1,\"y\":2} "))
    );
}

#[test]
fn assistant_reducer_primitive_metadata_merge_matches_reference_type_error() {
    for base in [json!(false), json!(0), json!(""), json!("ab")] {
        for overrides in [json!({"x":1}), json!([1]), json!("x")] {
            let mut reducer = reducer();
            apply(
                &mut reducer,
                &[json!({"type":"start","messageMetadata":base})],
            );
            let before = reducer.snapshot();
            assert!(matches!(
                reducer.apply(json!({"type":"message-metadata","messageMetadata":overrides})),
                Err(StreamError::Metadata(_))
            ));
            assert_eq!(reducer.snapshot(), before);
        }
    }
    let mut reducer = reducer();
    apply(
        &mut reducer,
        &[
            json!({"type":"start","messageMetadata":false}),
            json!({"type":"message-metadata","messageMetadata":{}}),
        ],
    );
    assert_eq!(reducer.snapshot().get("metadata"), Some(&json!({})));
    apply(
        &mut reducer,
        &[
            json!({"type":"start","messageId":"next","messageMetadata":false}),
            json!({"type":"start","messageId":"new","messageMetadata":{"x":1}}),
        ],
    );
    assert_eq!(reducer.snapshot().get("metadata"), Some(&json!({"x":1})));
}

#[test]
fn assistant_reducer_prototype_guard_precedes_schema_even_for_unknown_extras() {
    let mut reducer = reducer();
    for event in [
        json!({"type":"data-x","data":{"__proto__":null}}),
        json!({"type":"data-x","data":[{"constructor":{"prototype":false}}]}),
        json!({"type":"future-event","extra":{"__proto__":{}}}),
        json!({"type":"text-start","id":"t","extra":{"constructor":{"prototype":{}}}}),
    ] {
        assert!(matches!(reducer.apply(event), Err(StreamError::Json(_))));
        assert_eq!(reducer.part_count(), 0);
    }
    apply(
        &mut reducer,
        &[
            json!({"type":"data-x","data":{"constructor":1,"prototype":false}}),
            json!({"type":"data-x","data":{"constructor":null}}),
        ],
    );
    assert_eq!(reducer.part_count(), 2);
    let initial =
        json!({"id":"m","role":"assistant","parts":[{"type":"data-x","data":{"__proto__":{}}}]});
    let resumed = UiReducer::with_message(initial.clone(), ReducerOptions::default())
        .expect("initial state is not wire JSON");
    assert_eq!(resumed.snapshot(), initial);
}

#[test]
fn assistant_reducer_metadata_merges_objects_replaces_arrays_and_ignores_null() {
    let mut reducer = reducer();
    apply(
        &mut reducer,
        &[
            json!({"type":"message-metadata","messageMetadata":{"x":{"a":1},"array":[1]}}),
            json!({"type":"message-metadata","messageMetadata":{"x":{"b":2},"array":[2]}}),
            json!({"type":"message-metadata","messageMetadata":null}),
        ],
    );
    assert_eq!(
        reducer.snapshot().get("metadata"),
        Some(&json!({"x":{"a":1,"b":2},"array":[2]}))
    );
}

#[test]
fn assistant_reducer_citations_validate_then_strip_unknown_nested_fields() {
    let mut reducer = reducer();
    apply(&mut reducer, &[json!({"type":"text-start","id":"t"})]);
    assert!(reducer.apply(json!({"type":"text-end","id":"t","citations":[{"source":{"type":"source","sourceType":"url","id":"s","url":"x"},"startIndex":-1}]})).is_err());
    apply(
        &mut reducer,
        &[
            json!({"type":"text-end","id":"t","citations":[{"source":{"type":"source","sourceType":"url","id":"s","url":"x","extra":1},"startIndex":0,"extra":2}]}),
        ],
    );
    assert_eq!(
        reducer.part(0).expect("part").get("citations"),
        Some(
            &json!([{"source":{"type":"source","sourceType":"url","id":"s","url":"x"},"startIndex":0}])
        )
    );
}

#[test]
fn assistant_reducer_step_start_mutates_without_emitting_upstream_snapshot() {
    let mut reducer = reducer();
    let result = reducer.apply(json!({"type":"start-step"})).expect("step");
    assert!(!result.published);
    assert_eq!(reducer.part_count(), 1);
    let abort = reducer
        .apply(json!({"type":"abort","reason":"cancel"}))
        .expect("abort");
    assert!(!abort.published);
    assert_eq!(
        abort.updates,
        vec![Update::Abort {
            reason: Some("cancel".to_owned())
        }]
    );
}
