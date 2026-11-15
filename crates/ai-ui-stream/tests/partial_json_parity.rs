use ai_ui_stream::partial_json::{Error, Limits, Parser, State};
use serde_json::{json, Value};

fn parser() -> Parser {
    Parser::new(Limits::default()).unwrap()
}

// JS serializes integral floating-point values without `.0`; serde_json
// preserves the spelling's number category. Compare their numeric value.
fn equal_json(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => a.as_f64() == b.as_f64(),
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| equal_json(x, y))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(k, x)| b.get(k).is_some_and(|y| equal_json(x, y)))
        }
        _ => left == right,
    }
}

#[test]
fn pinned_upstream_prefix_fixture_matches_for_each_fragment() {
    let cases: Value =
        serde_json::from_str(include_str!("fixtures/upstream-prefixes.json")).unwrap();
    for case in cases.as_array().unwrap() {
        let mut parser = parser();
        let mut prefix = String::new();
        for step in case["steps"].as_array().unwrap() {
            let delta = step["delta"].as_str().unwrap();
            prefix.push_str(delta);
            parser
                .append(delta)
                .unwrap_or_else(|err| panic!("{prefix:?}: {err:?}"));
            let snapshot = parser.snapshot();
            assert!(
                equal_json(&snapshot.value, &step["value"]),
                "value at {prefix:?}: {:?} vs {:?}",
                snapshot.value,
                step["value"]
            );
            assert_eq!(
                format!("{:?}", snapshot.state),
                step["state"].as_str().unwrap(),
                "state at {prefix:?}"
            );
            assert_eq!(
                snapshot.partial_path,
                serde_json::from_value::<Vec<String>>(step["partial_path"].clone()).unwrap(),
                "path at {prefix:?}"
            );
        }
    }
}

#[test]
fn nested_fields_and_literals_match_upstream_completeness() {
    let mut p = parser();
    p.append("{\"foo\":[{\"bar\":\"abc").unwrap();
    assert_eq!(p.snapshot().value, json!({"foo":[{"bar":"abc"}]}));
    assert_eq!(
        p.snapshot().field_state(&["foo", "0", "bar"]),
        State::Partial
    );
    assert_eq!(
        p.snapshot().field_state(&["foo", "length"]),
        State::Complete
    );
    p.append("\",\"ok\":tr").unwrap();
    assert_eq!(
        p.snapshot().field_state(&["foo", "0", "bar"]),
        State::Complete
    );
    assert_eq!(
        p.snapshot().field_state(&["foo", "0", "ok"]),
        State::Complete
    );
    assert_eq!(
        p.snapshot().field_state(&["foo", "0", "absent"]),
        State::Partial
    );
    p.append("ue}]}").unwrap();
    assert_eq!(p.snapshot().state, State::Complete);
    assert_eq!(p.snapshot().field_state(&["missing"]), State::Complete);
}

#[test]
fn chunk_boundaries_do_not_lose_last_complete_number() {
    let mut p = parser();
    p.append("{\"n\":12e+").unwrap();
    assert_eq!(p.snapshot().value, json!({"n":12}));
    assert_eq!(p.snapshot().partial_path, vec!["n"]);
    p.append("2}").unwrap();
    assert_eq!(p.snapshot().value["n"].as_f64(), Some(1200.0));
    assert_eq!(p.snapshot().state, State::Complete);
}

#[test]
fn unicode_surrogates_are_withheld_until_the_pair_completes() {
    let mut p = parser();
    p.append("{\"a\":\"x\\uD83D").unwrap();
    assert_eq!(p.snapshot().value, json!({"a":"x"}));
    assert_eq!(p.snapshot().partial_path, vec!["a"]);
    p.append("\\uDE00y\"}").unwrap();
    assert_eq!(p.snapshot().value, json!({"a":"x😀y"}));
    assert_eq!(p.snapshot().state, State::Complete);
}

#[test]
fn failures_are_atomic_and_clones_are_independent() {
    let mut p = parser();
    p.append("{\"safe\":1,").unwrap();
    let original = p.snapshot().clone();
    let len = p.text_len();
    assert_eq!(p.append("\"__proto__\":1}"), Err(Error::UnsafeKey));
    assert_eq!(p.snapshot(), &original);
    assert_eq!(p.text_len(), len);
    assert!(matches!(
        p.clone().appended("\"__proto__\":1}"),
        Err(Error::UnsafeKey)
    ));
    assert_eq!(p.snapshot(), &original);
    let mut fork = p.clone();
    p.append("\"other\":2}").unwrap();
    fork.append("\"other\":3}").unwrap();
    assert_eq!(p.snapshot().value, json!({"safe":1,"other":2}));
    assert_eq!(fork.snapshot().value, json!({"safe":1,"other":3}));
    let mut bad = parser();
    assert_eq!(
        bad.append("{\"constructor\":{\"prototype\":1}}"),
        Err(Error::UnsafeKey)
    );
    assert_eq!(bad.append("{\"a\":01}"), Err(Error::InvalidSyntax));
    assert_eq!(bad.append("{\"a\":\"\\uZZ"), Err(Error::InvalidSyntax));
    assert_eq!(
        bad.append("{\"a\":\"\\uD800\"}"),
        Err(Error::UnsupportedUnicode)
    );
    assert_eq!(bad.text_len(), 0);
}

#[test]
fn plain_string_fragments_match_whole_prefix_and_reject_atomically() {
    let fragments = [
        "{\"nested\":[{\"value\":\"",
        "plain",
        " 😀",
        "\\n",
        "tail",
        "\"}],\"done\":true}",
    ];
    let mut incremental = parser();
    let mut staged = parser();
    let mut prefix = String::new();
    for fragment in fragments {
        prefix.push_str(fragment);
        incremental.append(fragment).unwrap();
        staged = staged.clone().appended(fragment).unwrap();
        let mut whole = parser();
        whole.append(&prefix).unwrap();
        assert_eq!(incremental.snapshot(), whole.snapshot());
        assert_eq!(staged.snapshot(), whole.snapshot());
        assert_eq!(incremental.text_len(), prefix.len());
    }

    let mut p = parser();
    p.append("{\"value\":\"before").unwrap();
    let before = p.snapshot().clone();
    let len = p.text_len();
    assert_eq!(p.append("after\u{0}"), Err(Error::InvalidSyntax));
    assert_eq!(p.snapshot(), &before);
    assert_eq!(p.text_len(), len);
    p.append("after").unwrap();
    assert_eq!(p.snapshot().value, json!({"value":"beforeafter"}));
}

#[test]
fn admission_limits_cover_source_nodes_and_depth() {
    let mut p = Parser::new(Limits {
        source_bytes: 7,
        nodes: 2,
        depth: 1,
    })
    .unwrap();
    p.append("{\"a\":1}").unwrap();
    assert_eq!(p.append(" "), Err(Error::SourceTooLarge));
    let mut p = Parser::new(Limits {
        source_bytes: 64,
        nodes: 2,
        depth: 1,
    })
    .unwrap();
    assert_eq!(p.append("{\"a\":1,\"b\":2}"), Err(Error::TooManyNodes));
    assert_eq!(p.append("{\"a\":{}}"), Err(Error::TooDeep));
    assert_eq!(p.text_len(), 0);
}

#[test]
fn array_length_and_nested_field_queries_follow_upstream() {
    let mut p = parser();
    p.append("{\"items\":[{\"name\":\"unfin").unwrap();
    let snapshot = p.snapshot();
    assert_eq!(snapshot.field_state(&["items"]), State::Partial);
    assert_eq!(snapshot.field_state(&["items", "length"]), State::Complete);
    assert_eq!(
        snapshot.field_state(&["items", "length", "anything"]),
        State::Complete
    );
    assert_eq!(
        snapshot.field_state(&["items", "0", "name"]),
        State::Partial
    );
    assert_eq!(
        snapshot.field_state(&["items", "0", "missing"]),
        State::Partial
    );
    assert_eq!(snapshot.field_state(&["items", "00"]), State::Partial);
    assert_eq!(snapshot.field_state(&["items", "1"]), State::Partial);
    p.append("ished\"},2").unwrap();
    assert_eq!(
        p.snapshot().field_state(&["items", "0", "name"]),
        State::Complete
    );
    assert_eq!(p.snapshot().field_state(&["items", "1"]), State::Partial);
}

#[test]
fn utf8_budget_and_duplicate_replacement_are_atomic() {
    let mut p = Parser::new(Limits {
        source_bytes: 10,
        nodes: 4,
        depth: 2,
    })
    .unwrap();
    p.append("{\"x\":\"é").unwrap();
    let before = p.snapshot().clone();
    assert_eq!(p.text_len(), 8); // é occupies two UTF-8 bytes
    assert_eq!(p.append("😀"), Err(Error::SourceTooLarge));
    assert_eq!(p.snapshot(), &before);
    assert_eq!(p.text_len(), 8);
    p.append("\"}").unwrap();
    assert_eq!(p.snapshot().value, json!({"x":"é"}));

    let mut p = Parser::new(Limits {
        source_bytes: 64,
        nodes: 2,
        depth: 2,
    })
    .unwrap();
    p.append("{\"x\":1,").unwrap();
    let before = p.snapshot().clone();
    assert_eq!(p.append("\"x\":2}"), Err(Error::TooManyNodes));
    assert_eq!(p.snapshot(), &before);
    p.append("\"ignored\"").unwrap(); // no value started yet
    assert_eq!(p.snapshot().value, json!({"x":1}));
}

#[test]
fn invalid_fragments_do_not_advance_open_unicode_or_container_state() {
    let mut p = parser();
    p.append("{\"safe\":\"\\uD83D").unwrap();
    let before = p.snapshot().clone();
    let len = p.text_len();
    assert_eq!(p.append("x"), Err(Error::UnsupportedUnicode));
    assert_eq!(p.snapshot(), &before);
    assert_eq!(p.text_len(), len);
    p.append("\\uDE00\",\"next\":[1").unwrap();
    let before = p.snapshot().clone();
    assert_eq!(p.append(",]}"), Err(Error::InvalidSyntax));
    assert_eq!(p.snapshot(), &before);
    p.append(",2]}").unwrap();
    assert_eq!(p.snapshot().value, json!({"safe":"😀","next":[1,2]}));
}

#[test]
fn escaped_unsafe_keys_and_depth_limit_are_rejected_atomically() {
    let mut p = parser();
    p.append("{\"safe\":1,").unwrap();
    let before = p.snapshot().clone();
    assert_eq!(
        p.append("\"\\u005f\\u005fproto__\":2}"),
        Err(Error::UnsafeKey)
    );
    assert_eq!(p.snapshot(), &before);
    assert_eq!(
        p.append("\"constructor\":{\"\\u0070rototype\":{}}}"),
        Err(Error::UnsafeKey)
    );
    assert_eq!(p.snapshot(), &before);
    p.append("\"good\":2}").unwrap();
    assert_eq!(p.snapshot().value, json!({"safe":1,"good":2}));

    let mut p = Parser::new(Limits {
        source_bytes: 4096,
        nodes: 256,
        depth: 128,
    })
    .unwrap();
    let prefix = format!("{{\"a\":{}", "[".repeat(127));
    p.append(&prefix).unwrap();
    let before = p.snapshot().clone();
    assert_eq!(p.append("["), Err(Error::TooDeep));
    assert_eq!(p.snapshot(), &before);
    p.append(&format!("0{} }}", "]".repeat(127))).unwrap();
    assert_eq!(p.snapshot().state, State::Complete);
}
