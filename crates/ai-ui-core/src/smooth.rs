//! Smooth streaming text, ported from `@assistant-ui/react`'s `useSmooth`
//! (`TextStreamAnimator`).
//!
//! Model output arrives in bursts. The animator reveals the target text at a
//! steady pace instead. Each character takes `min(max_char_interval_ms,
//! drain_ms / remaining)`: a large backlog is revealed faster (it would take
//! `drain_ms` at a constant rate), while the last characters, and any slow
//! trickle, appear at most `max_char_interval_ms` apart. The caller supplies
//! the clock and drives [`SmoothText::tick`] once per frame.

/// Pacing options (`SmoothOptions`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SmoothOptions {
    /// Reveal rate for a backlog: `drain_ms / remaining` per character (default 250).
    pub drain_ms: f64,
    /// Slowest reveal per character, in milliseconds (default 5).
    pub max_char_interval_ms: f64,
    /// Cap on characters revealed per tick (default unlimited).
    pub max_chars_per_frame: usize,
}

impl Default for SmoothOptions {
    fn default() -> Self {
        Self {
            drain_ms: 250.0,
            max_char_interval_ms: 5.0,
            max_chars_per_frame: usize::MAX,
        }
    }
}

/// The revealed prefix of one streaming text part.
#[derive(Clone, Debug)]
pub struct SmoothText {
    options: SmoothOptions,
    target: String,
    /// Revealed length in bytes, always on a character boundary.
    shown_bytes: usize,
    last_update_ms: Option<f64>,
}

impl SmoothText {
    /// Start with nothing revealed if the part is still streaming, else all of it.
    pub fn new(text: &str, streaming: bool, options: SmoothOptions) -> Self {
        let mut smooth = Self {
            options,
            target: String::new(),
            shown_bytes: 0,
            last_update_ms: None,
        };
        smooth.reset(text, streaming);
        smooth
    }

    fn reset(&mut self, text: &str, streaming: bool) {
        self.target = text.to_owned();
        self.last_update_ms = None;
        self.shown_bytes = if streaming { 0 } else { text.len() };
    }

    /// Update the latest text. Text that no longer extends what is shown
    /// (for example after a regeneration) starts over, as upstream.
    pub fn set_target(&mut self, text: &str, streaming: bool) {
        if !text.starts_with(self.text()) {
            self.reset(text, streaming);
        } else if text.len() != self.target.len() {
            self.target.truncate(self.shown_bytes);
            self.target.push_str(&text[self.shown_bytes..]);
        }
    }

    /// The text to display now.
    pub fn text(&self) -> &str {
        &self.target[..self.shown_bytes]
    }

    /// Characters still to reveal.
    pub fn is_animating(&self) -> bool {
        self.shown_bytes < self.target.len()
    }

    /// Advance to `now_ms`; returns whether the revealed text changed.
    pub fn tick(&mut self, now_ms: f64) -> bool {
        let remaining = self.target[self.shown_bytes..].chars().count();
        if remaining == 0 {
            self.last_update_ms = None;
            return false;
        }
        let Some(last) = self.last_update_ms else {
            // The first frame only starts the clock.
            self.last_update_ms = Some(now_ms);
            return false;
        };
        let per_char = self
            .options
            .max_char_interval_ms
            .min(self.options.drain_ms / remaining as f64);
        let limit = remaining.min(self.options.max_chars_per_frame);
        let mut budget = now_ms - last;
        let mut add = 0;
        while budget >= per_char && add < limit {
            add += 1;
            budget -= per_char;
        }
        if add == limit && limit == self.options.max_chars_per_frame {
            budget = 0.0;
        }
        if add == 0 {
            return false;
        }
        let bytes: usize = self.target[self.shown_bytes..]
            .chars()
            .take(add)
            .map(char::len_utf8)
            .sum();
        self.shown_bytes += bytes;
        self.last_update_ms = Some(now_ms - budget);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(smooth: &mut SmoothText, from: f64, to: f64) -> f64 {
        let mut now = from;
        while now <= to {
            smooth.tick(now);
            now += 16.0;
        }
        now
    }

    #[test]
    fn a_burst_is_revealed_gradually_and_faster_than_the_slowest_rate() {
        let text = "x".repeat(400);
        let mut smooth = SmoothText::new("", true, SmoothOptions::default());
        smooth.set_target(&text, true);
        smooth.tick(0.0);
        smooth.tick(16.0);
        let first = smooth.text().len();
        assert!(
            first > 3,
            "a big backlog beats 5 ms per character, got {first}"
        );
        assert!(first < 400, "but it is not shown at once");
        let mut now = 32.0;
        while smooth.is_animating() && now < 5_000.0 {
            smooth.tick(now);
            now += 16.0;
        }
        assert_eq!(smooth.text(), text);
        // The tail runs at 5 ms per character, so a 400-character burst takes
        // a little over half a second, far less than 400 x 5 ms = 2 s.
        assert!(now < 1_000.0, "drained after {now} ms");
        assert!(now > 250.0, "not instant: {now} ms");
    }

    #[test]
    fn a_trickle_is_revealed_at_the_slowest_rate() {
        let mut smooth = SmoothText::new("", true, SmoothOptions::default());
        smooth.set_target("abcdef", true);
        smooth.tick(0.0);
        smooth.tick(10.0);
        assert_eq!(
            smooth.text(),
            "ab",
            "5 ms per character when the backlog is small"
        );
    }

    #[test]
    fn finished_parts_show_at_once_and_unrelated_text_starts_over() {
        let mut smooth = SmoothText::new("done already", false, SmoothOptions::default());
        assert_eq!(smooth.text(), "done already");
        assert!(!smooth.is_animating());
        smooth.set_target("a different answer", true);
        assert_eq!(smooth.text(), "", "a regeneration starts from nothing");
    }

    #[test]
    fn growing_text_keeps_the_revealed_prefix_and_respects_char_boundaries() {
        let mut smooth = SmoothText::new(
            "",
            true,
            SmoothOptions {
                max_chars_per_frame: 1,
                ..Default::default()
            },
        );
        smooth.set_target("雪だ", true);
        smooth.tick(0.0);
        assert!(smooth.tick(100.0));
        assert_eq!(smooth.text(), "雪");
        smooth.set_target("雪だるま", true);
        assert_eq!(smooth.text(), "雪");
        run(&mut smooth, 116.0, 400.0);
        assert_eq!(smooth.text(), "雪だるま");
    }
}
