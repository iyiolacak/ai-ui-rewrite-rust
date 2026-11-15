//! Published AI SDK 7.0.166 expected folds, plus transport partition invariance.

#[path = "../examples/support/mod.rs"]
mod support;

use serde_json::{json, Value};

#[allow(
    clippy::unwrap_used,
    reason = "A malformed checked-in fixture must fail the test"
)]
fn vectors() -> Vec<Value> {
    serde_json::from_str::<Value>(include_str!("fixtures/ui7.json")).unwrap()["vectors"]
        .as_array()
        .unwrap()
        .clone()
}

#[allow(
    clippy::unwrap_used,
    reason = "A failed fixture replay must fail this test helper"
)]
fn assert_vector(vector: &Value, request: &Value, partition: &str) {
    let result = support::replay(request).unwrap();
    assert_eq!(
        result["foldedMessage"], vector["expected"]["foldedMessage"],
        "{} message ({partition})",
        vector["id"]
    );
    assert_eq!(
        result["sidecars"], vector["expected"]["sidecars"],
        "{} sidecars ({partition})",
        vector["id"]
    );
}

#[test]
fn assistant_ui7_all_45_published_upstream_fixtures_match() {
    let fixtures = vectors();
    assert_eq!(fixtures.len(), 45);
    for vector in &fixtures {
        assert_vector(vector, vector, "recorded network reads");
    }
}

#[test]
fn assistant_ui7_every_byte_and_deterministic_partitionings_preserve_folds() {
    for vector in vectors() {
        for width in [1, 2, 7, 31, 257] {
            let mut request = vector.clone();
            let object = request.as_object_mut().unwrap();
            object.remove("network_chunks_base64");
            object.remove("network_chunks_utf8");
            object.insert("chunk_bytes".into(), json!(width));
            assert_vector(&vector, &request, &format!("{width} bytes"));
        }
    }
}

#[test]
fn assistant_ui7_transport_blank_line_controls_publication_not_eof() {
    let result =
        support::replay(&json!({"raw_utf8":"data: {\"type\":\"start-step\"}\n\n"})).unwrap();
    assert_eq!(result["emissions"], 0);
    assert_eq!(
        result["foldedMessage"],
        json!({"id":"m","role":"assistant","parts":[]})
    );
}
