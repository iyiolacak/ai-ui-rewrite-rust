//! Incremental SSE framing. Network read boundaries never become event boundaries.

use crate::StreamError;

/// A bounded SSE event assembler, independent of the event's JSON schema.
#[derive(Debug)]
pub(crate) struct SseFramer {
    line: Vec<u8>,
    data: Vec<u8>,
    has_data: bool,
    first_line: bool,
    skip_lf: bool,
    limit: usize,
}

impl SseFramer {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            line: Vec::new(),
            data: Vec::new(),
            has_data: false,
            first_line: true,
            skip_lf: false,
            limit,
        }
    }

    /// Each input byte is visited once. Completed events are delivered before
    /// later events in the same network read, including when a later event fails.
    pub(crate) fn feed(
        &mut self,
        bytes: &[u8],
        mut on_event: impl FnMut(&[u8]) -> Result<(), StreamError>,
    ) -> Result<(), StreamError> {
        for &byte in bytes {
            if self.skip_lf {
                self.skip_lf = false;
                if byte == b'\n' {
                    continue;
                }
            }
            match byte {
                b'\r' => {
                    self.finish_line(&mut on_event)?;
                    self.skip_lf = true;
                }
                b'\n' => self.finish_line(&mut on_event)?,
                _ => {
                    // Bound the current physical line AND the assembled event.
                    // Comments/unknown fields cannot bypass the line bound.
                    if self.line.len().saturating_add(self.data.len()) >= self.limit {
                        return Err(self.limit_error());
                    }
                    self.line.push(byte);
                }
            }
        }
        Ok(())
    }

    fn finish_line(
        &mut self,
        on_event: &mut impl FnMut(&[u8]) -> Result<(), StreamError>,
    ) -> Result<(), StreamError> {
        let line = if self.first_line {
            self.first_line = false;
            self.line
                .strip_prefix(&[0xef, 0xbb, 0xbf])
                .unwrap_or(&self.line)
        } else {
            &self.line
        };

        if line.is_empty() {
            if self.has_data {
                // The event separator, not EOF, dispatches the accumulated data.
                // A `data:` line with an empty value is still an event.
                on_event(&self.data)?;
            }
            self.data.clear();
            self.has_data = false;
        } else if !line.starts_with(b":") {
            let (field, value) = match line.iter().position(|&byte| byte == b':') {
                Some(index) => {
                    let (field, rest) = line.split_at(index);
                    let after_colon = rest.get(1..).unwrap_or_default();
                    (field, after_colon.strip_prefix(b" ").unwrap_or(after_colon))
                }
                None => (line, &[][..]),
            };
            if field == b"data" {
                let separator = usize::from(self.has_data);
                if self
                    .data
                    .len()
                    .saturating_add(separator)
                    .saturating_add(value.len())
                    > self.limit
                {
                    return Err(self.limit_error());
                }
                if self.has_data {
                    self.data.push(b'\n');
                }
                self.data.extend_from_slice(value);
                self.has_data = true;
            }
            // event/id/retry and unknown SSE fields are intentionally ignored:
            // the UI-message protocol takes the type from JSON, not SSE event:.
        }
        self.line.clear();
        Ok(())
    }

    fn limit_error(&self) -> StreamError {
        StreamError::Limit {
            resource: "event bytes",
            limit: self.limit,
        }
    }

    /// WHATWG SSE discards an unfinished event at EOF. It does not synthesize a
    /// blank line or a protocol finish event. Release retained framing buffers.
    pub(crate) fn discard(&mut self) {
        self.line = Vec::new();
        self.data = Vec::new();
        self.has_data = false;
    }

    #[cfg(test)]
    fn buffered_bytes(&self) -> usize {
        self.line.len() + self.data.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(bytes: &[u8], widths: &[usize]) -> Vec<Vec<u8>> {
        let mut framer = SseFramer::new(1024);
        let mut events = Vec::new();
        let mut offset = 0;
        let mut i = 0;
        while offset < bytes.len() {
            let width = widths[i % widths.len()].max(1);
            let end = offset.saturating_add(width).min(bytes.len());
            framer
                .feed(&bytes[offset..end], |data| {
                    events.push(data.to_vec());
                    Ok(())
                })
                .unwrap();
            offset = end;
            i += 1;
        }
        framer.discard();
        events
    }

    #[test]
    fn assistant_sse_line_ending_and_unicode_partitions_do_not_change_events() {
        let wire = "\u{feff}: ping\r\nevent: ignored\r\nid: 9\r\ndata: {\"text\":\r\ndata: \"é🦀\"}\r\n\r\ndata: [DONE]\r\r";
        let expected = vec![
            "{\"text\":\n\"é🦀\"}".as_bytes().to_vec(),
            b"[DONE]".to_vec(),
        ];
        for width in 1..=wire.len() {
            assert_eq!(
                collect(wire.as_bytes(), &[width]),
                expected,
                "width={width}"
            );
        }
    }

    #[test]
    fn assistant_sse_comments_and_empty_data_have_distinct_semantics() {
        assert_eq!(
            collect(
                b": keep-alive\n\ndata:\n\ndata\n\ndata: x\ndata: y\n\n",
                &[1]
            ),
            vec![b"".to_vec(), b"".to_vec(), b"x\ny".to_vec()]
        );
    }

    #[test]
    fn assistant_sse_eof_never_dispatches_a_partial_event() {
        assert!(collect(b"data: x\n", &[3]).is_empty());
        assert!(collect(b"data: x", &[1]).is_empty());
        assert_eq!(collect(b"data: x\n\n", &[2]), vec![b"x".to_vec()]);
    }

    #[test]
    fn assistant_sse_bounded_buffers_fail_before_retaining_an_excess_byte() {
        let mut framer = SseFramer::new(32);
        let error = framer.feed(&[b'x'; 33], |_| Ok(())).unwrap_err();
        assert_eq!(
            error,
            StreamError::Limit {
                resource: "event bytes",
                limit: 32
            }
        );
        assert_eq!(framer.buffered_bytes(), 32);
        framer.discard();
        assert_eq!(framer.buffered_bytes(), 0);
    }

    #[test]
    fn assistant_sse_multi_line_data_cannot_bypass_event_budget() {
        let mut framer = SseFramer::new(40);
        let wire = b"data: 01234567890123456789\ndata: 01234567890123456789\n\n";
        assert!(matches!(
            framer.feed(wire, |_| Ok(())),
            Err(StreamError::Limit { .. })
        ));
        assert!(framer.buffered_bytes() <= 40);
    }

    #[test]
    fn assistant_sse_stop_on_callback_error_keeps_earlier_deliveries() {
        let mut framer = SseFramer::new(128);
        let mut delivered = Vec::new();
        let result = framer.feed(b"data: ok\n\ndata: bad\n\ndata: later\n\n", |data| {
            if data == b"bad" {
                return Err(StreamError::Json("test failure".into()));
            }
            delivered.push(data.to_vec());
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(delivered, vec![b"ok".to_vec()]);
    }

    #[test]
    fn assistant_sse_random_chunking_preserves_wire_events() {
        let wire = b"data: 1\n\ndata: 2\r\rdata: 3\r\n\r\n: no data\r\ndata: 4\r\n\r\n";
        let expected = collect(wire, &[wire.len()]);
        for seed in 0..256_u64 {
            let mut state = seed + 1;
            let widths: Vec<usize> = (0..32)
                .map(|_| {
                    state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                    usize::try_from((state >> 32) % 17 + 1).unwrap()
                })
                .collect();
            assert_eq!(collect(wire, &widths), expected, "seed={seed}");
        }
    }
}
