//! A bounded Server-Sent Events reader yielding each event's `data` payload
//! (`createSSEJsonDecoder`): `data:` lines are joined, comments and other fields
//! are ignored, and `data: [DONE]` ends the stream.

/// Size bounds for the SSE framing of the UI message stream.
#[derive(Clone, Copy, Debug)]
pub struct SseLimits {
    /// Longest accepted line, in bytes.
    pub line_bytes: usize,
    /// Largest accepted event payload, in bytes.
    pub event_bytes: usize,
}

impl Default for SseLimits {
    fn default() -> Self {
        Self {
            line_bytes: 1024 * 1024,
            event_bytes: 4 * 1024 * 1024,
        }
    }
}

#[derive(Default)]
pub(crate) struct SseEvents {
    limits: SseLimits,
    line: Vec<u8>,
    data: Option<String>,
    pub(crate) done: bool,
}

impl SseEvents {
    pub(crate) fn new(limits: SseLimits) -> Self {
        Self {
            limits,
            ..Self::default()
        }
    }

    /// Complete event payloads in this input. Bytes after `[DONE]` are ignored.
    pub(crate) fn push(&mut self, input: &[u8]) -> Result<Vec<String>, String> {
        let mut events = Vec::new();
        for segment in input.split_inclusive(|&byte| byte == b'\n') {
            if self.done {
                break;
            }
            let (bytes, complete) = match segment.strip_suffix(b"\n") {
                Some(line) => (line, true),
                None => (segment, false),
            };
            if bytes.len() > self.limits.line_bytes.saturating_sub(self.line.len()) {
                return Err("SSE line exceeds the limit".into());
            }
            if self.line.is_empty() && complete {
                // The usual network read contains complete lines. Borrow those
                // bytes directly; only fragmented lines need a scratch buffer.
                self.end_line(bytes, &mut events)?;
            } else {
                self.line.extend_from_slice(bytes);
                if complete {
                    let mut line = std::mem::take(&mut self.line);
                    let result = self.end_line(&line, &mut events);
                    line.clear();
                    self.line = line;
                    result?;
                }
            }
        }
        Ok(events)
    }

    fn end_line(&mut self, bytes: &[u8], events: &mut Vec<String>) -> Result<(), String> {
        let bytes = bytes.strip_suffix(b"\r").unwrap_or(bytes);
        let line = std::str::from_utf8(bytes).map_err(|_| "SSE line is not UTF-8".to_owned())?;
        if line.is_empty() {
            if let Some(data) = self.data.take() {
                if data == "[DONE]" {
                    self.done = true;
                } else {
                    events.push(data);
                }
            }
        } else if let Some(value) = line.strip_prefix("data:") {
            let value = value.strip_prefix(' ').unwrap_or(value);
            let data = self.data.get_or_insert_with(String::new);
            let separator = usize::from(!data.is_empty());
            if value.len().saturating_add(separator)
                > self.limits.event_bytes.saturating_sub(data.len())
            {
                return Err("SSE event exceeds the limit".into());
            }
            if separator != 0 {
                data.push('\n');
            }
            data.push_str(value);
        }
        // Comments (`:`) and `event:`/`id:`/`retry:` fields carry nothing here.
        Ok(())
    }

    /// A final event without its blank-line terminator still counts.
    pub(crate) fn finish(&mut self) -> Result<Vec<String>, String> {
        if self.done {
            return Ok(Vec::new());
        }
        if !self.line.is_empty() {
            self.push(b"\n")?;
        }
        self.push(b"\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragmented_and_complete_lines_preserve_events_and_unicode() {
        let wire = "data: {\"text\":\r\ndata: \"é🦀\"}\r\n\r\n: keep-alive\n\nevent: ignored\ndata: second\n\ndata: [DONE]\n\ninvalid bytes after done";
        for width in 1..=wire.len() {
            let mut decoder = SseEvents::default();
            let mut events = Vec::new();
            for slice in wire.as_bytes().chunks(width) {
                events.extend(decoder.push(slice).unwrap());
            }
            events.extend(decoder.finish().unwrap());
            assert_eq!(events, ["{\"text\":\n\"é🦀\"}", "second"], "width={width}");
        }
    }

    #[test]
    fn physical_line_limit_includes_cr_and_applies_across_reads() {
        for width in [1, 2, 32] {
            let mut decoder = SseEvents::new(SseLimits {
                line_bytes: 8,
                event_bytes: 8,
            });
            let mut events = Vec::new();
            for slice in b"data: x\r\n\r\n".chunks(width) {
                events.extend(decoder.push(slice).unwrap());
            }
            assert_eq!(events, ["x"]);

            let mut error = None;
            for slice in b"data: xx\r\n\r\n".chunks(width) {
                if let Err(failure) = decoder.push(slice) {
                    error = Some(failure);
                    break;
                }
            }
            assert_eq!(error.as_deref(), Some("SSE line exceeds the limit"));
        }
    }

    #[test]
    fn multiline_event_limit_counts_inserted_newlines() {
        for width in [1, 7, 64] {
            let mut decoder = SseEvents::new(SseLimits {
                line_bytes: 32,
                event_bytes: 3,
            });
            let mut events = Vec::new();
            for slice in b"data: a\ndata: b\n\n".chunks(width) {
                events.extend(decoder.push(slice).unwrap());
            }
            assert_eq!(events, ["a\nb"]);
            let mut error = None;
            for slice in b"data: ab\ndata: c\n\n".chunks(width) {
                if let Err(failure) = decoder.push(slice) {
                    error = Some(failure);
                    break;
                }
            }
            assert_eq!(error.as_deref(), Some("SSE event exceeds the limit"));
        }
    }

    #[test]
    fn final_unterminated_event_and_utf8_validation_keep_their_semantics() {
        let mut decoder = SseEvents::default();
        assert!(decoder.push(b"data: last").unwrap().is_empty());
        assert_eq!(decoder.finish().unwrap(), ["last"]);

        for width in [1, 32] {
            let mut decoder = SseEvents::default();
            let mut error = None;
            for slice in b"data: \xff\n\n".chunks(width) {
                if let Err(failure) = decoder.push(slice) {
                    error = Some(failure);
                    break;
                }
            }
            assert_eq!(error.as_deref(), Some("SSE line is not UTF-8"));
        }
    }
}
