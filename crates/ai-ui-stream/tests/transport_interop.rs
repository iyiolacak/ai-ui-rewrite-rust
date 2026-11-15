use ai_ui_stream::transport::{Chunk, Decoder, Error, Limits};

fn decode(bytes: &[u8], chunk_size: usize) -> (Vec<Chunk>, bool) {
    let mut decoder = Decoder::new(Limits::default()).unwrap();
    let mut chunks = Vec::new();
    let mut done = false;
    for piece in bytes.chunks(chunk_size) {
        let result = decoder.push(piece);
        assert_eq!(result.error, None);
        chunks.extend(result.chunks);
        done |= result.done;
        if done {
            break;
        }
    }
    assert_eq!(decoder.finish(), Ok(()));
    (chunks, done)
}

#[test]
fn pinned_upstream_python_fixture_survives_every_byte_boundary() {
    let fixture = include_bytes!("fixtures/python-encoder.sse");
    let (whole, done) = decode(fixture, fixture.len());
    assert!(done);
    assert_eq!(whole.len(), 27);
    assert_eq!(whole[0].kind(), "step-start");
    assert_eq!(whole[0].path(), Vec::<u64>::new());
    assert_eq!(whole[12].value()["isPreliminary"], true);
    assert_eq!(
        whole[14].value()["messages"][0]["content"][0]["text"],
        "Sunny"
    );
    assert_eq!(whole[23].value()["part"]["type"], "file");
    assert_eq!(whole[26].value()["severity"], "warning");
    assert_eq!(decode(fixture, 1).0, whole);
    assert_eq!(decode(fixture, 5).0, whole);
}

#[test]
fn sse_line_endings_bom_multiline_and_unicode_are_incremental() {
    let stream = concat!(
        "\u{feff}:comment\r",
        "event: message\r\n",
        "data: {\"type\":\"text-delta\",\r",
        "data: \"path\":[0],\"textDelta\":\"🍀\"}\n\n",
        "data: [DONE]\r\r",
        "data: ignored\n\n"
    );
    let (chunks, done) = decode(stream.as_bytes(), 1);
    assert!(done);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].value()["textDelta"], "🍀");
}

#[test]
fn strict_truncation_does_not_infer_success() {
    for input in [
        "",
        "data: [DONE]",
        "data: [DONE]\n",
        "data: {\"type\":\"step-start\"}\n\n",
    ] {
        let mut decoder = Decoder::new(Limits::default()).unwrap();
        assert_eq!(decoder.push(input.as_bytes()).error, None);
        assert_eq!(decoder.finish(), Err(Error::MissingDone));
        assert_eq!(decoder.push(b"data: [DONE]\n\n").error, Some(Error::Closed));
    }
}

/// A frame upstream drops with a warning (protocol vector AT05): no error, no chunk.
fn is_dropped(frame: &[u8]) -> bool {
    let mut decoder = Decoder::new(Limits::default()).unwrap();
    let result = decoder.push(frame);
    result.error.is_none() && result.chunks.is_empty() && result.dropped.len() == 1
}

#[test]
fn malformed_chunks_are_dropped_and_the_stream_continues() {
    let mut decoder = Decoder::new(Limits::default()).unwrap();
    let input = b"data: {\"type\":\"text-delta\",\"path\":[0],\"textDelta\":\"good\"}\n\ndata: {\"type\":\"text-delta\",\"textDelta\":\"missing path\"}\n\ndata: [DONE]\n\n";
    let result = decoder.push(input);
    assert_eq!(result.chunks.len(), 1);
    assert_eq!(result.chunks[0].value()["textDelta"], "good");
    assert_eq!(result.dropped.len(), 1);
    assert_eq!(result.error, None);
    assert!(result.done);
    assert_eq!(decoder.finish(), Ok(()));
}

#[test]
fn malformed_and_unknown_frames_are_explicit() {
    // Chunk-level problems are dropped, as upstream does.
    for input in [
        "data: x\n\n",
        "data: {\"type\":\"future\"}\n\n",
        "data: {\"type\":\"part-finish\",\"path\":[-1]}\n\n",
        "data: {\"type\":\"part-finish\",\"path\":[1.5]}\n\n",
    ] {
        assert!(is_dropped(input.as_bytes()), "{input}");
    }
    // Framing problems stay fatal.
    let mut decoder = Decoder::new(Limits::default()).unwrap();
    assert_eq!(
        decoder.push(b"event: error\ndata: {}\n\n").error,
        Some(Error::UnknownSseEvent)
    );
    let mut decoder = Decoder::new(Limits::default()).unwrap();
    assert_eq!(
        decoder.push(b"data: \xff\n\n").error,
        Some(Error::InvalidUtf8)
    );
}

#[test]
fn input_line_and_event_limits_stop_unbounded_retention() {
    assert!(matches!(
        Decoder::new(Limits {
            input_bytes: 0,
            ..Limits::default()
        }),
        Err(Error::InvalidLimits)
    ));
    let mut decoder = Decoder::new(Limits {
        input_bytes: 4,
        line_bytes: 10,
        event_bytes: 10,
    })
    .unwrap();
    assert_eq!(decoder.push(b"12345").error, Some(Error::InputTooLarge));
    let mut decoder = Decoder::new(Limits {
        input_bytes: 10,
        line_bytes: 4,
        event_bytes: 10,
    })
    .unwrap();
    assert_eq!(decoder.push(b"12345").error, Some(Error::LineTooLarge));
    let mut decoder = Decoder::new(Limits {
        input_bytes: 10,
        line_bytes: 10,
        event_bytes: 4,
    })
    .unwrap();
    assert_eq!(decoder.push(b"data: abc\n").error, None);
    assert_eq!(decoder.push(b"data: d\n").error, Some(Error::EventTooLarge));
}

#[test]
fn path_indices_accept_json_numbers_like_javascript_with_a_safe_integer_limit() {
    for (literal, expected) in [
        ("0", 0),
        ("-0", 0),
        ("-0.0", 0),
        ("1.0", 1),
        ("1e0", 1),
        ("9007199254740991", 9_007_199_254_740_991),
        ("9007199254740991.0", 9_007_199_254_740_991),
    ] {
        let frame = format!("data: {{\"type\":\"part-finish\",\"path\":[{literal}]}}\n\n");
        let mut decoder = Decoder::new(Limits::default()).unwrap();
        let result = decoder.push(frame.as_bytes());
        assert_eq!(result.error, None, "{literal}");
        assert_eq!(result.chunks[0].path(), vec![expected], "{literal}");
    }
    for literal in [
        "-1",
        "-0.5",
        "1.5",
        "9007199254740992",
        "9007199254740992.0",
    ] {
        let frame = format!("data: {{\"type\":\"part-finish\",\"path\":[{literal}]}}\n\n");
        assert!(is_dropped(frame.as_bytes()), "{literal}");
    }
}

#[test]
fn rejects_prototype_pollution_keys_at_any_depth_even_when_escaped() {
    for payload in [
        r#"{"type":"data","data":[{"__proto__":1}]}"#,
        r#"{"type":"data","data":[{"\u005f\u005fproto__":{}}]}"#,
        r#"{"type":"data","data":[[{"constructor":{"prototype":null}}]]}"#,
        r#"{"type":"data","data":[{"\u0063onstructor":{"\u0070rototype":{}}}]}"#,
    ] {
        let mut decoder = Decoder::new(Limits::default()).unwrap();
        let frame = format!("data: {payload}\n\n");
        assert_eq!(
            decoder.push(frame.as_bytes()).error,
            Some(Error::UnsafeJsonKey),
            "{payload}"
        );
    }
    // Benign keys by themselves must remain available to the application.
    let mut decoder = Decoder::new(Limits::default()).unwrap();
    let result = decoder.push(
        br#"data: {"type":"data","data":[{"constructor":{"other":1},"prototype":true}]}

"#,
    );
    assert_eq!(result.error, None);
    assert_eq!(result.chunks.len(), 1);
}

#[test]
fn unpaired_utf16_surrogate_escapes_are_dropped() {
    for payload in [
        r#"{"type":"text-delta","path":[0],"textDelta":"\ud800"}"#,
        r#"{"type":"text-delta","path":[0],"textDelta":"\udc00"}"#,
    ] {
        let frame = format!("data: {payload}\n\n");
        assert!(is_dropped(frame.as_bytes()), "{payload}");
    }
    let mut decoder = Decoder::new(Limits::default()).unwrap();
    let result = decoder.push(
        br#"data: {"type":"text-delta","path":[0],"textDelta":"\ud83c\udf40"}

"#,
    );
    assert_eq!(result.error, None);
    assert_eq!(result.chunks[0].value()["textDelta"], "🍀");
}
#[test]
fn debug_redacts_decoded_private_content() {
    use ai_ui_stream::transport::{Decoder, Limits};

    let mut decoder = Decoder::new(Limits::default()).unwrap();
    let batch = decoder.push(concat!(
        "data: {\"type\":\"part-start\",\"part\":{\"type\":\"source\",\"sourceType\":\"url\",\"id\":\"source-secret\",\"url\":\"https://example.test/?token=url-secret\"}}\n\n",
        "data: {\"type\":\"part-start\",\"part\":{\"type\":\"file\",\"data\":\"file-secret\",\"mimeType\":\"text/plain\"}}\n\n",
        "data: {\"type\":\"error\",\"error\":\"error-secret\"}\n\n",
    ).as_bytes());
    assert_eq!(batch.chunks.len(), 3);
    let debug = format!("{batch:?} {:?}", batch.chunks);
    for secret in ["source-secret", "url-secret", "file-secret", "error-secret"] {
        assert!(!debug.contains(secret));
    }
    // Redaction does not alter the data available to an explicitly inspecting host.
    assert_eq!(batch.chunks[2].value()["error"], "error-secret");
}
