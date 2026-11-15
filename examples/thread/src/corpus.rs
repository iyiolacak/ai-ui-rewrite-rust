//! Mock conversations for the demo: a monkeytype-style word generator, so a
//! long thread reads as different text instead of one message repeated. Every
//! message and every thread title comes from a seeded generator, so the same
//! seed always gives the same text and runs stay comparable.
//!
//! Shape of the text: most user messages are one question. Some are several
//! paragraphs, and about one in four hundred is long enough to take the paged
//! path in `paging.rs`. Assistant replies are written like a classic LLM
//! answer — an opening, a few long paragraphs, then markdown: **bold**,
//! _italics_, `inline code`, headings, numbered and bulleted lists, a quote,
//! a fenced code block, a table, a horizontal rule. Sizes vary widely,
//! from two lines to a few hundred words, so scrolling feels real.
//!
//! The word generator stays monkeytype: lower-case common words joined into
//! sentences. It is the only source of variety — no template sentences — so
//! 10,000 messages are still all distinct, and no markdown snippet repeats.

use crate::titles::FirstMessageTitles;
use ai_ui_core::{
    AdapterFuture, ExportedRepository, RemoteThread, ThreadListAdapter, ThreadMessage,
};
use serde_json::json;
use std::sync::Arc;

/// Common English words, lower case, no punctuation.
const WORDS: &[&str] = &[
    "the", "of", "and", "to", "a", "in", "is", "it", "you", "that", "he", "was", "for", "on",
    "are", "with", "as", "his", "they", "be", "at", "one", "have", "this", "from", "or", "had",
    "by", "not", "word", "but", "what", "some", "we", "can", "out", "other", "were", "all",
    "there", "when", "up", "use", "your", "how", "said", "an", "each", "she", "which", "do",
    "their", "time", "if", "will", "way", "about", "many", "then", "them", "write", "would",
    "like", "so", "these", "her", "long", "make", "thing", "see", "him", "two", "has", "look",
    "more", "day", "could", "go", "come", "did", "number", "sound", "no", "most", "people", "my",
    "over", "know", "water", "than", "call", "first", "who", "may", "down", "side", "been", "now",
    "find", "any", "new", "work", "part", "take", "get", "place", "made", "live", "where", "after",
    "back", "little", "only", "round", "man", "year", "came", "show", "every", "good", "me",
    "give", "our", "under", "name", "very", "through", "just", "form", "sentence", "great",
    "think", "say", "help", "low", "line", "differ", "turn", "cause", "much", "mean", "before",
    "move", "right", "boy", "old", "too", "same", "tell", "does", "set", "three", "want", "air",
    "well", "also", "play", "small", "end", "put", "home", "read", "hand", "port", "large",
    "spell", "add", "even", "land", "here", "must", "big", "high", "such", "follow", "act", "why",
    "ask", "men", "change", "went", "light", "kind", "off", "need", "house", "picture", "try",
    "us", "again", "animal", "point", "mother", "world", "near", "build", "self", "earth",
    "father", "head", "stand", "own", "page", "should", "country", "found", "answer", "school",
    "grow", "study", "still", "learn", "plant", "cover", "food", "sun", "four", "between", "state",
    "keep", "eye", "never", "last", "let", "thought", "city", "tree", "cross", "farm", "hard",
    "start", "might", "story", "saw", "far", "sea", "draw", "left", "late", "run", "while",
    "press", "close", "night", "real", "life", "few", "north", "open", "seem", "together", "next",
    "white", "children", "begin", "got", "walk", "example", "ease", "paper", "group", "always",
    "music", "those", "both", "mark", "often", "letter", "until", "mile", "river", "car", "feet",
    "care", "second", "book", "carry", "took", "science", "eat", "room", "friend", "began", "idea",
    "fish", "mountain", "stop", "once", "base", "hear", "horse", "cut", "sure", "watch", "color",
    "face", "wood", "main", "enough", "plain", "girl", "usual", "young", "ready", "above", "ever",
    "red", "list", "though", "feel", "talk", "bird", "soon", "body", "dog", "family", "direct",
    "pose", "leave", "song", "measure", "door", "product", "black", "short", "numeral", "class",
    "wind", "question", "happen", "complete", "ship", "area", "half", "rock", "order", "fire",
    "south", "problem", "piece", "told", "knew", "pass", "since", "top", "whole", "king", "space",
    "heard", "best", "hour", "better", "true", "during", "hundred", "five", "remember", "step",
    "early", "hold", "west", "ground", "interest", "reach", "fast", "verb", "sing", "listen",
    "six", "table", "travel", "less", "morning", "ten", "simple", "several", "vowel", "toward",
    "war", "lay", "against", "pattern", "slow", "center", "love", "person", "money", "serve",
    "appear", "road", "map", "rain", "rule", "govern", "pull", "cold", "notice", "voice", "unit",
    "power", "town", "fine", "certain", "fly", "fall", "lead", "cry", "dark", "machine", "note",
    "wait", "plan", "figure", "star", "box", "noun", "field", "rest", "correct", "able", "pound",
    "done", "beauty", "drive", "stood", "contain", "front", "teach", "week", "final", "gave",
    "green", "oh", "quick", "develop", "ocean", "warm", "free", "minute", "strong", "special",
    "mind", "behind", "clear", "tail", "produce", "fact", "street", "inch", "multiply", "nothing",
    "course", "stay", "wheel", "full", "force", "blue", "object", "decide", "surface", "deep",
    "moon", "island", "foot", "system", "busy", "test", "record", "boat", "common", "gold",
    "possible", "plane", "stead", "dry", "wonder", "laugh", "thousand", "ago", "ran", "check",
    "game", "shape", "equate", "hot", "miss", "brought", "heat", "snow", "tire", "bring", "yes",
    "distant", "fill", "east", "paint", "language", "among", "cache", "buffer", "stream", "render",
    "layout", "thread", "message",
];

/// A small deterministic generator (splitmix64). Same seed, same text.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x2545_F491_4F6C_DD1D) ^ 0x9E37_79B9_7F4A_7C15)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `lo..=hi`.
    fn range(&mut self, lo: usize, hi: usize) -> usize {
        lo + (self.next_u64() % (hi - lo + 1) as u64) as usize
    }

    /// True with probability 1 in `one_in`.
    fn chance(&mut self, one_in: usize) -> bool {
        self.range(1, one_in) == 1
    }

    fn word(&mut self) -> &'static str {
        WORDS[self.range(0, WORDS.len() - 1)]
    }
}

fn sentence(rng: &mut Rng, out: &mut String) {
    let words = rng.range(6, 16);
    for i in 0..words {
        if i > 0 {
            out.push(' ');
        }
        let word = rng.word();
        if i == 0 {
            let mut chars = word.chars();
            if let Some(first) = chars.next() {
                out.extend(first.to_uppercase());
                out.push_str(chars.as_str());
            }
        } else {
            out.push_str(word);
        }
        if i + 1 < words && i > 2 && rng.chance(9) {
            out.push(',');
        }
    }
    out.push(match rng.range(0, 11) {
        0 => '!',
        1 => '?',
        _ => '.',
    });
}

/// A sentence with one inline span — **bold**, _italic_ or `code` — the way an
/// LLM answer sprinkles emphasis through a paragraph. The words stay
/// monkeytype; only the markers are added.
fn styled_sentence(rng: &mut Rng, out: &mut String) {
    let words = rng.range(7, 18);
    let span_at = rng.range(1, 3);
    let span_len = rng.range(1, 3);
    let kind = rng.range(0, 2);
    for i in 0..words {
        if i > 0 {
            out.push(' ');
        }
        if i == span_at {
            out.push_str(match kind { 0 => "**", 1 => "_", _ => "`" });
        }
        let word = rng.word();
        if i == 0 {
            let mut chars = word.chars();
            if let Some(first) = chars.next() {
                out.extend(first.to_uppercase());
                out.push_str(chars.as_str());
            }
        } else {
            out.push_str(word);
        }
        if i + 1 < words && i > 2 && rng.chance(9) {
            out.push(',');
        }
        if i == span_at + span_len && kind != 2 {
            out.push_str(if kind == 0 { "**" } else { "_" });
        }
        if i == span_at + span_len && kind == 2 {
            out.push('`');
        }
    }
    out.push(match rng.range(0, 11) {
        0 => '!',
        1 => '?',
        _ => '.',
    });
}

fn paragraphs(rng: &mut Rng, count: usize) -> String {
    let mut out = String::new();
    for p in 0..count {
        if p > 0 {
            out.push_str("\n\n");
        }
        let sentences = rng.range(3, 7);
        for s in 0..sentences {
            if s > 0 {
                out.push(' ');
            }
            styled_sentence(rng, &mut out);
        }
    }
    out
}

/// Heading words: pick a token out of the word list so headings feel varied.
fn heading(rng: &mut Rng) -> String {
    let n = rng.range(1, 3);
    let mut out = String::new();
    for i in 0..n {
        if i > 0 { out.push(' '); }
        let word = rng.word();
        if i == 0 {
            let mut chars = word.chars();
            if let Some(first) = chars.next() {
                out.extend(first.to_uppercase());
                out.push_str(chars.as_str());
            }
        } else {
            out.push_str(word);
        }
    }
    out
}

fn code_block(rng: &mut Rng) -> String {
    // A handful of rust-ish lines from the word generator, so the demo
    // exercises code rendering without a fixed snippet that repeats.
    let lang = match rng.range(0, 4) {
        0 => "rust",
        1 => "python",
        2 => "bash",
        3 => "json",
        _ => "rust",
    };
    let lines = rng.range(3, 7);
    let mut out = format!("\n```{lang}\n");
    for _ in 0..lines {
        let (a, b) = (rng.word(), rng.word());
        let n = rng.range(0, 9999);
        let shape = rng.range(0, 2);
        match shape {
            0 => out.push_str(&format!("let {a}_{b} = {n};\n")),
            1 => out.push_str(&format!("fn {a}({b}: {n}) -> bool {{ {b} > 0 }}\n")),
            _ => out.push_str(&format!("// {a} {b} {n}\n")),
        }
    }
    out.push_str("```\n");
    out
}

fn table(rng: &mut Rng) -> String {
    let mut out = String::from("\n| Thread | Messages | Text |\n|---|---|---|\n");
    for _ in 0..rng.range(3, 5) {
        let (word, n) = (rng.word(), rng.range(2, 9999));
        let mb = (n as f32 / 100.0).max(0.1);
        out.push_str(&format!("| {word} | {n} | {mb:.1} MB |\n"));
    }
    out
}

fn title(rng: &mut Rng) -> String {
    let words = rng.range(3, 6);
    (0..words)
        .map(|_| {
            let mut chars = rng.word().chars();
            chars
                .next()
                .map(|f| f.to_uppercase().chain(chars).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn user_text(rng: &mut Rng) -> String {
    if rng.chance(400) {
        // Past the paging threshold: about 24 KB of verbatim paragraphs.
        {
            let n = rng.range(48, 60);
            paragraphs(rng, n)
        }
    } else if rng.chance(6) {
        {
            let n = rng.range(2, 4);
            paragraphs(rng, n)
        }
    } else {
        let mut out = String::new();
        sentence(rng, &mut out);
        out
    }
}

fn assistant_text(rng: &mut Rng) -> String {
    let n = rng.range(2, 5);
    let mut out = paragraphs(rng, n);

    // A heading between sections, like a classic LLM answer.
    if rng.chance(3) {
        out.push_str("\n\n## ");
        out.push_str(&heading(rng));
        out.push('\n');
    }

    // Long paragraphs: 1 in 3 answers run past the paging threshold.
    if rng.chance(3) {
        out.push_str("\n\n");
        let n = rng.range(5, 9);
        for p in 0..n {
            if p > 0 { out.push_str("\n\n"); }
            let sentences = rng.range(3, 6);
            for s in 0..sentences {
                if s > 0 { out.push(' '); }
                styled_sentence(rng, &mut out);
            }
        }
    }

    // Numbered steps, then a bulleted recap.
    if rng.chance(4) {
        out.push_str("\n\n");
        let items = rng.range(3, 6);
        for i in 1..=items {
            out.push_str(&format!("{i}. "));
            styled_sentence(rng, &mut out);
            out.push('\n');
        }
    }
    if rng.chance(4) {
        out.push_str("\n");
        let items = rng.range(3, 5);
        for _ in 0..items {
            out.push_str("- ");
            styled_sentence(rng, &mut out);
            out.push('\n');
        }
    }

    // A block quote.
    if rng.chance(6) {
        out.push_str("\n> ");
        styled_sentence(rng, &mut out);
        out.push('\n');
    }

    // A fenced code block (generated, never a fixed snippet).
    if rng.chance(4) {
        out.push_str("\n");
        out.push_str(&code_block(rng));
    }

    // A table of the thread's stats.
    if rng.chance(4) {
        out.push_str("\n");
        out.push_str(&table(rng));
    }

    // A closing horizontal rule, 1 in 8.
    if rng.chance(8) {
        out.push_str("\n---\n");
    }

    out
}

/// `messages` messages alternating user and assistant, starting with the user.
/// The ids are stable for a seed.
pub fn history(seed: u64, messages: usize) -> Vec<ThreadMessage> {
    let mut rng = Rng::new(seed);
    (0..messages)
        .map(|i| {
            let user = i % 2 == 0;
            let text = if user {
                user_text(&mut rng)
            } else {
                assistant_text(&mut rng)
            };
            serde_json::from_value(json!({
                "role": if user { "user" } else { "assistant" },
                "id": format!("m{seed}-{i}"),
                "createdAt": 0,
                "content": [{ "type": "text", "text": text }],
                "status": { "type": "complete", "reason": "stop" }
            }))
            .expect("valid message")
        })
        .collect()
}

/// One stored thread of the demo's sidebar. Its history is generated when it is
/// first opened, so a long list costs nothing until a thread is used.
#[derive(Clone)]
pub struct StoredThread {
    pub remote_id: String,
    pub title: String,
    pub seed: u64,
    pub messages: usize,
}

/// `count` stored threads: mostly short, with some long ones to flex with.
pub fn catalog(count: usize) -> Vec<StoredThread> {
    (0..count)
        .map(|i| {
            let seed = i as u64 + 1;
            let mut rng = Rng::new(seed ^ 0xA5A5);
            let title = title(&mut rng);
            let messages = if rng.chance(8) {
                rng.range(300, 1200) * 2
            } else {
                rng.range(2, 60) * 2
            };
            StoredThread {
                remote_id: format!("demo-{i}"),
                title,
                seed,
                messages,
            }
        })
        .collect()
}

/// The sidebar's threads: a list of stored demo threads whose histories are
/// generated on load. Titles come from the first user message, as before.
pub struct DemoThreads {
    threads: Vec<StoredThread>,
    titles: FirstMessageTitles,
}

impl DemoThreads {
    pub fn new(count: usize) -> Self {
        Self {
            threads: catalog(count),
            titles: FirstMessageTitles,
        }
    }
}

impl ThreadListAdapter for DemoThreads {
    fn list(&self) -> AdapterFuture<Vec<RemoteThread>> {
        let threads = self
            .threads
            .iter()
            .map(|t| RemoteThread {
                remote_id: t.remote_id.clone(),
                title: Some(t.title.clone()),
                archived: false,
            })
            .collect();
        Box::pin(async move { Ok(threads) })
    }

    fn load(&self, remote_id: &str) -> AdapterFuture<ExportedRepository> {
        let found = self
            .threads
            .iter()
            .find(|t| t.remote_id == remote_id)
            .cloned();
        Box::pin(async move {
            let thread = found.ok_or_else(|| "no such demo thread".to_owned())?;
            Ok(ExportedRepository::from_linear(history(
                thread.seed,
                thread.messages,
            )))
        })
    }

    fn generate_title(
        &self,
        remote_id: &str,
        messages: Vec<Arc<ai_ui_core::ThreadMessage>>,
    ) -> AdapterFuture<Option<String>> {
        self.titles.generate_title(remote_id, messages)
    }
}

impl DemoThreads {
    /// How many messages a stored thread has, known without loading it.
    pub fn message_count(&self, remote_id: &str) -> Option<usize> {
        self.threads
            .iter()
            .find(|t| t.remote_id == remote_id)
            .map(|t| t.messages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_text_and_different_threads_differ() {
        let a = history(7, 6);
        let b = history(7, 6);
        let c = history(8, 6);
        let text = |m: &[ThreadMessage]| m.iter().map(|x| x.text()).collect::<Vec<_>>();
        assert_eq!(text(&a), text(&b));
        assert_ne!(text(&a), text(&c));
        assert_ne!(text(&a)[0], text(&a)[2], "messages do not repeat");
    }

    #[test]
    fn catalog_is_stable_and_has_long_threads() {
        let a = catalog(40);
        assert_eq!(a.len(), 40);
        assert_eq!(a[3].title, catalog(40)[3].title);
        assert!(a.iter().all(|t| t.messages % 2 == 0));
    }

    #[test]
    fn ten_thousand_messages_are_varied_and_some_take_the_paged_path() {
        let all = history(0, 10_000);
        let texts: Vec<String> = all.iter().map(|m| m.text()).collect();
        let distinct: std::collections::HashSet<&String> = texts.iter().collect();
        assert_eq!(distinct.len(), texts.len(), "no two messages are the same");
        let bytes: usize = texts.iter().map(String::len).sum();
        assert!(
            bytes > 4 * 1024 * 1024,
            "the history is long: {bytes} bytes"
        );
        assert!(
            texts.iter().step_by(2).any(|t| t.len() > 16 * 1024),
            "a user message is past the paging threshold"
        );
    }
}
