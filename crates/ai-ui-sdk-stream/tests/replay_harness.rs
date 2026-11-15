//! Benchmark-request validation and the deliberately distinct replay contracts.

#[path = "../examples/support/mod.rs"]
mod support;

use serde_json::{json, Value};

const TEXT_WIRE: &str = concat!(
    "data: {\"type\":\"text-start\",\"id\":\"t\"}\n\n",
    "data: {\"type\":\"text-delta\",\"id\":\"t\",\"delta\":\"Hello\"}\n\n",
    "data: {\"type\":\"text-end\",\"id\":\"t\"}\n\n",
);

#[test]
fn replay_rejects_unknown_or_non_string_modes() {
    for mode in [
        json!("snapshots"),
        json!(""),
        json!("PATCH"),
        Value::Null,
        json!(true),
        json!(0),
        json!({}),
        json!([]),
    ] {
        assert!(support::replay(&json!({"raw_utf8":"", "mode":mode})).is_err());
    }
}

#[test]
fn replay_accepts_only_positive_safe_integer_chunk_widths() {
    for width in [
        json!(0),
        json!(-1),
        json!(1.5),
        Value::Null,
        json!("4"),
        json!(false),
        json!(9_007_199_254_740_992_u64),
    ] {
        assert!(support::replay(&json!({"raw_utf8":"", "chunk_bytes":width})).is_err());
        assert!(support::replay(&json!({"network_chunks_utf8":[], "chunk_bytes":width})).is_err());
    }
    let baseline = support::replay(&json!({"raw_utf8":TEXT_WIRE})).unwrap();
    for width in [
        json!(1),
        json!(1.0),
        json!(7),
        json!(9_007_199_254_740_991_u64),
    ] {
        let replay = support::replay(&json!({"raw_utf8":TEXT_WIRE, "chunk_bytes":width})).unwrap();
        assert_eq!(replay["foldedMessage"], baseline["foldedMessage"]);
        assert_eq!(replay["sidecars"], baseline["sidecars"]);
        assert_eq!(replay["wire_bytes"], baseline["wire_bytes"]);
    }
}

#[test]
fn replay_rejects_noncanonical_base64() {
    for encoded in [
        "Zg", "Zh==", "Zm9=", "Zm8", "Zg=", "Zg===", "====", "A===", "AA=A", "Zg==AAAA", "Zg==\n",
        "Zg--", "🙂",
    ] {
        assert!(support::replay(&json!({"network_chunks_base64":[encoded]})).is_err());
    }
}

#[test]
fn replay_accepts_canonical_base64_and_preserves_exact_byte_counts() {
    for (encoded, length) in [
        ("", 0),
        ("Zg==", 1),
        ("Zm8=", 2),
        ("Zm9v", 3),
        ("AA==", 1),
        ("AAA=", 2),
        ("AAAA", 3),
    ] {
        let replay = support::replay(&json!({"network_chunks_base64":[encoded]})).unwrap();
        assert_eq!(replay["wire_bytes"], length);
        assert_eq!(replay["sidecars"]["errors"], json!([]));
    }
    let encoded =
        support::replay(&json!({"network_chunks_base64":["ZGF0YTogW0RPTkVdCgo="]})).unwrap();
    let literal = support::replay(&json!({"raw_utf8":"data: [DONE]\n\n"})).unwrap();
    assert_eq!(encoded["wire_bytes"], literal["wire_bytes"]);
    assert_eq!(encoded["foldedMessage"], literal["foldedMessage"]);
    assert_eq!(encoded["sidecars"], literal["sidecars"]);
}

#[test]
fn replay_rejects_ambiguous_or_mistyped_explicit_network_chunks() {
    assert!(
        support::replay(&json!({"network_chunks_base64":[], "network_chunks_utf8":[]})).is_err()
    );
    for field in ["network_chunks_base64", "network_chunks_utf8"] {
        for chunks in [Value::Null, json!(""), json!({}), json!([1]), json!([null])] {
            let mut request = json!({"raw_utf8":""});
            request[field] = chunks;
            assert!(support::replay(&request).is_err());
        }
    }
}

#[test]
fn replay_explicit_network_reads_override_raw_and_preserve_publications() {
    let replay = support::replay(&json!({
        "raw_utf8":"not the network input",
        "chunk_bytes":1,
        "network_chunks_utf8":["data: {\"type\":\"text-start\",\"id\":\"t\"}", "\n", "\n"]
    }))
    .unwrap();
    assert_eq!(replay["emissions"], 1);
    assert_eq!(
        replay["foldedMessage"]["parts"],
        json!([{"type":"text","text":"","state":"streaming"}])
    );
}

#[test]
fn replay_patch_is_current_state_not_last_published_snapshot() {
    let raw = "data: {\"type\":\"start-step\"}\n\n";
    let snapshot = support::replay(&json!({"raw_utf8":raw})).unwrap();
    let patch = support::replay(&json!({"raw_utf8":raw, "mode":"patch"})).unwrap();
    assert_eq!(snapshot["mode"], "snapshot");
    assert_eq!(patch["mode"], "patch");
    assert_eq!(snapshot["foldedMessage"]["parts"], json!([]));
    assert_eq!(
        patch["foldedMessage"]["parts"],
        json!([{"type":"step-start"}])
    );
    assert_eq!(snapshot["emissions"], 0);
    assert_eq!(patch["emissions"], 0);
    assert_eq!(snapshot["snapshots_materialized"], 0);
    assert_eq!(patch["snapshots_materialized"], 1);
}

#[test]
fn replay_trailing_unpublished_step_also_distinguishes_contracts() {
    let raw = format!("{TEXT_WIRE}data: {{\"type\":\"start-step\"}}\n\n");
    let snapshot = support::replay(&json!({"raw_utf8":raw, "mode":"snapshot"})).unwrap();
    let patch = support::replay(&json!({"raw_utf8":raw, "mode":"patch"})).unwrap();
    assert_eq!(
        snapshot["foldedMessage"]["parts"].as_array().unwrap().len(),
        1
    );
    assert_eq!(patch["foldedMessage"]["parts"].as_array().unwrap().len(), 2);
    assert_eq!(snapshot["emissions"], 3);
    assert_eq!(patch["emissions"], 3);
    assert_eq!(snapshot["sidecars"], patch["sidecars"]);
}

#[test]
fn replay_modes_agree_on_equal_fold_workloads_but_not_snapshot_work() {
    let snapshot = support::replay(&json!({"raw_utf8":TEXT_WIRE, "mode":"snapshot"})).unwrap();
    let patch = support::replay(&json!({"raw_utf8":TEXT_WIRE, "mode":"patch"})).unwrap();
    for field in [
        "foldedMessage",
        "sidecars",
        "emissions",
        "events",
        "wire_bytes",
    ] {
        assert_eq!(snapshot[field], patch[field]);
    }
    assert_eq!(snapshot["snapshots_materialized"], 3);
    assert_eq!(patch["snapshots_materialized"], 1);
}

#[test]
fn replay_metadata_merge_error_uses_upstream_type_error_projection() {
    let replay = support::replay(&json!({"raw_utf8":concat!(
        "data: {\"type\":\"start\",\"messageMetadata\":false}\n\n",
        "data: {\"type\":\"message-metadata\",\"messageMetadata\":{\"x\":1}}\n\n"
    )}))
    .unwrap();
    assert_eq!(replay["sidecars"]["errors"], json!([{"name":"TypeError"}]));
    assert_eq!(replay["foldedMessage"]["metadata"], false);
}
