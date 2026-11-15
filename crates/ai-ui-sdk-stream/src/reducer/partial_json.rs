//! On-demand partial JSON preview. This is not called on the delta hot path.
//!
//! The repair state machine is adapted from AI SDK 7.0.166 `src/util/fix-json.ts`.
//! Copyright 2023 Vercel, Inc. Licensed under Apache-2.0; see LICENSE-APACHE.txt.
//! Rust adaptation: byte-safe Unicode traversal, typed states, and no panics.

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Root,
    Finish,
    ObjectStart,
    ObjectKey,
    ObjectAfterKey,
    ObjectBeforeValue,
    ObjectAfterValue,
    ObjectAfterComma,
    String,
    StringEscape,
    StringUnicodeEscape,
    ArrayStart,
    ArrayAfterValue,
    ArrayAfterComma,
    Number,
    Literal,
}

struct Repair {
    stack: Vec<State>,
    last_valid: usize,
    literal_start: Option<usize>,
    unicode_digits: u8,
}

impl Repair {
    fn replace_top(&mut self, next: State) {
        self.stack.pop();
        self.stack.push(next);
    }

    fn value_start(&mut self, character: char, start: usize, end: usize, after: State) {
        let state = match character {
            '"' => State::String,
            'f' | 't' | 'n' => {
                self.literal_start = Some(start);
                State::Literal
            }
            '-' | '0'..='9' => State::Number,
            '{' => State::ObjectStart,
            '[' => State::ArrayStart,
            _ => return,
        };
        if character != '-' {
            self.last_valid = end;
        }
        self.replace_top(after);
        self.stack.push(state);
    }

    fn after_value(&mut self, character: char, end: usize) {
        match (self.stack.last(), character) {
            (Some(State::ObjectAfterValue), ',') => self.replace_top(State::ObjectAfterComma),
            (Some(State::ArrayAfterValue), ',') => self.replace_top(State::ArrayAfterComma),
            (Some(State::ObjectAfterValue), '}') | (Some(State::ArrayAfterValue), ']') => {
                self.last_valid = end;
                self.stack.pop();
            }
            _ => {}
        }
    }

    fn accept(&mut self, input: &str, character: char, start: usize) {
        let end = start + character.len_utf8();
        match self.stack.last().copied() {
            Some(State::Root) => self.value_start(character, start, end, State::Finish),
            Some(State::ObjectStart) => match character {
                '"' => self.replace_top(State::ObjectKey),
                '}' => {
                    self.last_valid = end;
                    self.stack.pop();
                }
                _ => {}
            },
            Some(State::ObjectAfterComma) => {
                if character == '"' {
                    self.replace_top(State::ObjectKey);
                }
            }
            Some(State::ObjectKey) => {
                if character == '"' {
                    self.replace_top(State::ObjectAfterKey);
                }
            }
            Some(State::ObjectAfterKey) => {
                if character == ':' {
                    self.replace_top(State::ObjectBeforeValue);
                }
            }
            Some(State::ObjectBeforeValue) => {
                self.value_start(character, start, end, State::ObjectAfterValue);
            }
            Some(State::ObjectAfterValue) => self.after_value(character, end),
            Some(State::String) => match character {
                '"' => {
                    self.stack.pop();
                    self.last_valid = end;
                }
                '\\' => self.stack.push(State::StringEscape),
                _ => self.last_valid = end,
            },
            Some(State::ArrayStart) => {
                if character == ']' {
                    self.last_valid = end;
                    self.stack.pop();
                } else {
                    self.value_start(character, start, end, State::ArrayAfterValue);
                }
            }
            Some(State::ArrayAfterValue) => {
                if matches!(character, ',' | ']') {
                    self.after_value(character, end);
                } else {
                    self.last_valid = end;
                }
            }
            Some(State::ArrayAfterComma) => {
                self.value_start(character, start, end, State::ArrayAfterValue);
            }
            Some(State::StringEscape) => {
                self.stack.pop();
                if character == 'u' {
                    self.unicode_digits = 0;
                    self.stack.push(State::StringUnicodeEscape);
                } else {
                    self.last_valid = end;
                }
            }
            Some(State::StringUnicodeEscape) => {
                if character.is_ascii_hexdigit() {
                    self.unicode_digits += 1;
                    if self.unicode_digits == 4 {
                        self.stack.pop();
                        self.last_valid = end;
                    }
                }
            }
            Some(State::Number) => match character {
                '0'..='9' => self.last_valid = end,
                'e' | 'E' | '-' | '.' => {}
                ',' | '}' | ']' => {
                    self.stack.pop();
                    self.after_value(character, end);
                }
                _ => {
                    self.stack.pop();
                }
            },
            Some(State::Literal) => {
                let partial = self.literal_start.and_then(|begin| input.get(begin..end));
                if partial.is_some_and(|literal| {
                    ["false", "true", "null"]
                        .iter()
                        .any(|whole| whole.starts_with(literal))
                }) {
                    self.last_valid = end;
                } else {
                    self.stack.pop();
                    self.after_value(character, end);
                }
            }
            Some(State::Finish) | None => {}
        }
    }

    fn finish(self, input: &str) -> String {
        let mut result = input.get(..self.last_valid).unwrap_or_default().to_owned();
        for state in self.stack.iter().rev() {
            match state {
                State::String => result.push('"'),
                State::ObjectStart
                | State::ObjectKey
                | State::ObjectAfterKey
                | State::ObjectBeforeValue
                | State::ObjectAfterValue
                | State::ObjectAfterComma => result.push('}'),
                State::ArrayStart | State::ArrayAfterValue | State::ArrayAfterComma => {
                    result.push(']')
                }
                State::Literal => {
                    if let Some(partial) = self.literal_start.and_then(|begin| input.get(begin..)) {
                        if let Some(whole) = ["true", "false", "null"]
                            .iter()
                            .find(|whole| whole.starts_with(partial))
                        {
                            if let Some(suffix) = whole.get(partial.len()..) {
                                result.push_str(suffix);
                            }
                        }
                    }
                }
                State::Root
                | State::Finish
                | State::StringEscape
                | State::StringUnicodeEscape
                | State::Number => {}
            }
        }
        result
    }
}

pub(super) fn preview(input: &str) -> Option<Value> {
    if let Some(value) = parse(input) {
        return Some(value);
    }
    let mut repair = Repair {
        stack: vec![State::Root],
        last_valid: 0,
        literal_start: None,
        unicode_digits: 0,
    };
    for (index, character) in input.char_indices() {
        repair.accept(input, character, index);
    }
    parse(&repair.finish(input))
}

fn parse(input: &str) -> Option<Value> {
    serde_json::from_str(input)
        .ok()
        .filter(|value| super::schema::validate_json_keys(value).is_ok())
}

#[cfg(test)]
mod tests {
    use super::preview;
    use serde_json::json;

    #[test]
    fn assistant_partial_json_previews_without_claiming_completion() {
        assert_eq!(preview(r#"{"city":"Par"#), Some(json!({"city":"Par"})));
        assert_eq!(preview(r#"{"a":[1,tru"#), Some(json!({"a":[1,true]})));
        assert_eq!(preview(r#"{"a":"é🙂"#), Some(json!({"a":"é🙂"})));
        assert_eq!(preview(""), None);
    }

    #[test]
    fn assistant_partial_json_drops_incomplete_escape_and_object_key() {
        assert_eq!(preview(r#"{"a":"abc\u12"#), Some(json!({"a":"abc"})));
        assert_eq!(preview(r#"{"a":1,"unfinished"#), Some(json!({"a":1})));
        assert_eq!(preview("[1,"), Some(json!([1])));
    }

    #[test]
    fn assistant_partial_json_rejects_prototype_keys_without_discarding_raw_input() {
        assert_eq!(preview(r#"{"__proto__":{"polluted":true}}"#), None);
        assert_eq!(preview(r#"{"constructor":{"prototype":false}}"#), None);
        assert_eq!(preview(r#"{"nested":[{"__proto__":null"#), None);
        assert_eq!(
            preview(r#"{"constructor":1}"#),
            Some(json!({"constructor":1}))
        );
    }
}
