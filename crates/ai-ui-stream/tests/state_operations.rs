use ai_ui_stream::state::{decode_operations, Error, Limits, Operation, State};
use serde_json::{json, Value};

fn path(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| (*s).into()).collect()
}

#[test]
fn set_and_append_preserve_unrelated_state_and_array_order() {
    let mut state = State::new(json!({"items": ["a"], "keep": 7}), Limits::default()).unwrap();
    state
        .apply(&[
            Operation::Set {
                path: path(&["nested", "text"]),
                value: json!("hel"),
            },
            Operation::AppendText {
                path: path(&["nested", "text"]),
                value: "lo".into(),
            },
            Operation::Set {
                path: path(&["items", "1"]),
                value: json!("b"),
            },
        ])
        .unwrap();
    assert_eq!(
        state.value(),
        &json!({"items": ["a", "b"], "keep": 7, "nested": {"text": "hello"}})
    );
}

#[test]
fn a_failed_operation_rolls_back_the_entire_batch() {
    let mut state = State::new(json!({"a": "before", "items": [1]}), Limits::default()).unwrap();
    let before = state.value().clone();
    assert_eq!(
        state.apply(&[
            Operation::AppendText {
                path: path(&["a"]),
                value: " after".into()
            },
            Operation::Set {
                path: path(&["items", "3"]),
                value: json!(4)
            },
        ]),
        Err(Error::ArrayIndexOutOfBounds)
    );
    assert_eq!(state.value(), &before);
}

#[test]
fn strict_paths_reject_unsafe_noncanonical_and_wrong_types() {
    for key in ["__proto__", "constructor", "prototype"] {
        let mut state = State::new(json!({}), Limits::default()).unwrap();
        assert_eq!(
            state.apply(&[Operation::Set {
                path: path(&["outer", key]),
                value: json!(1)
            }]),
            Err(Error::UnsafeKey)
        );
        assert_eq!(state.value(), &json!({}));
    }
    for key in ["", "01", "1e0", "0.5", "-1"] {
        let mut state = State::new(json!(["a", "b"]), Limits::default()).unwrap();
        assert_eq!(
            state.apply(&[Operation::Set {
                path: path(&[key]),
                value: json!(1)
            }]),
            Err(Error::InvalidArrayIndex)
        );
    }
    let mut state = State::new(json!({"a": 2}), Limits::default()).unwrap();
    assert_eq!(
        state.apply(&[Operation::AppendText {
            path: path(&["a"]),
            value: "x".into()
        }]),
        Err(Error::ExpectedString)
    );
    assert_eq!(
        state.apply(&[Operation::Set {
            path: path(&["a", "b"]),
            value: json!(1)
        }]),
        Err(Error::InvalidPath)
    );
}

#[test]
fn limits_reject_input_and_result_without_publishing_partial_state() {
    let limits = Limits {
        nodes: 8,
        value_bytes: 32,
        depth: 3,
        ..Limits::default()
    };
    let mut state = State::new(json!({"text": "x"}), limits).unwrap();
    let original = state.value().clone();
    assert_eq!(
        state.apply(&[Operation::AppendText {
            path: path(&["text"]),
            value: "x".repeat(32)
        }]),
        Err(Error::ValueTooLarge)
    );
    assert_eq!(state.value(), &original);
    assert_eq!(
        state.apply(&[Operation::Set {
            path: path(&["a", "b", "c", "d"]),
            value: json!(1)
        }]),
        Err(Error::PathTooLong)
    );
    assert_eq!(
        state.apply(&[Operation::Set {
            path: path(&["x"]),
            value: json!([1, 2, 3, 4, 5, 6, 7, 8, 9])
        }]),
        Err(Error::TooManyNodes)
    );
    assert_eq!(state.value(), &original);
}

#[test]
fn aggregate_batch_input_is_bounded_before_mutation() {
    let limits = Limits {
        value_bytes: 16,
        ..Limits::default()
    };
    let mut state = State::new(json!("initial"), limits).unwrap();
    let operations = vec![
        Operation::Set {
            path: vec![],
            value: json!("123456789"),
        },
        Operation::Set {
            path: vec![],
            value: json!("abcdefghi"),
        },
    ];
    assert_eq!(state.apply(&operations), Err(Error::ValueTooLarge));
    assert_eq!(state.value(), &json!("initial"));
}

#[test]
fn decodes_pinned_fixture_shape_and_numeric_wire_indices() {
    let wire = json!([
        {"type":"set", "path":["status"], "value":"running"},
        {"type":"set", "path":["values"], "value":["a"]},
        {"type":"set", "path":["values", 1.0], "value":"b"},
    ]);
    let mut state = State::new(Value::Null, Limits::default()).unwrap();
    state
        .apply(&decode_operations(&wire, Limits::default()).unwrap())
        .unwrap();
    assert_eq!(
        state.value(),
        &json!({"status":"running", "values":["a","b"]})
    );
    assert_eq!(
        decode_operations(
            &json!([{"type":"set", "path":[0.5], "value":1}]),
            Limits::default()
        ),
        Err(Error::InvalidOperation)
    );
}

#[test]
fn dangerous_nested_json_and_invalid_operations_are_rejected() {
    assert!(matches!(
        State::new(json!({"outer":{"__proto__":{"x":1}}}), Limits::default()),
        Err(Error::UnsafeKey)
    ));
    assert!(matches!(
        State::new(json!({"constructor":{"prototype":{}}}), Limits::default()),
        Err(Error::UnsafeKey)
    ));
    assert_eq!(
        decode_operations(
            &json!([{"type":"append-text", "path": [], "value": 9}]),
            Limits::default()
        ),
        Err(Error::InvalidOperation)
    );
    assert_eq!(
        decode_operations(
            &json!([{"type":"unknown", "path": [], "value": 9}]),
            Limits::default()
        ),
        Err(Error::InvalidOperation)
    );
}

#[test]
fn rejects_excessive_config_and_deep_owned_input_without_recursive_drop() {
    let too_deep = Limits {
        depth: 20_000,
        ..Limits::default()
    };
    assert!(matches!(
        State::new(Value::Null, too_deep),
        Err(Error::InvalidLimits)
    ));
    let mut nested = Value::Null;
    for _ in 0..20_000 {
        nested = Value::Array(vec![nested]);
    }
    assert!(matches!(
        State::new(nested, Limits::default()),
        Err(Error::TooDeep)
    ));
}

#[test]
fn preserves_json_integers_outside_javascript_safe_range_as_data() {
    let wire: Value =
        serde_json::from_str(r#"[{"type":"set","path":["id"],"value":9007199254740993}]"#).unwrap();
    let mut state = State::new(Value::Null, Limits::default()).unwrap();
    state
        .apply(&decode_operations(&wire, Limits::default()).unwrap())
        .unwrap();
    assert_eq!(state.value()["id"].as_u64(), Some(9_007_199_254_740_993));
}
