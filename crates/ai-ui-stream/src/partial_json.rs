//! Bounded incremental projection of streaming tool argument objects.
//!
//! This parser has no transport, execution or session authority. A host must
//! scope each parser to its captured tool call, retain raw input as needed and
//! treat syntactic completion separately from protocol argument completion.
#![forbid(unsafe_code)]

use serde_json::{Map, Value};
use std::fmt;

/// Hard recursion bound, independent of caller configuration.
pub const MAX_DEPTH: usize = 128;

/// Limits count source UTF-8 bytes, value starts and nested containers. They do
/// not include allocator overhead, caller copies or transactional snapshots.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Most bytes of JSON text accepted.
    pub source_bytes: usize,
    /// Most values (including containers) accepted.
    pub nodes: usize,
    /// Deepest nesting accepted.
    pub depth: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            source_bytes: 256 * 1024,
            nodes: 16_384,
            depth: 64,
        }
    }
}

/// A message's state tree, updated by operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// The JSON is incomplete so far; the value is the best reading of what has arrived.
    Partial,
    /// The JSON is complete.
    Complete,
}

/// A snapshot is owned by its parser and changes only on an accepted append.
#[derive(Clone, PartialEq)]
pub struct Snapshot {
    /// The value read so far.
    pub value: Value,
    /// Whether the JSON is complete.
    pub state: State,
    /// Path to the value that is still being written; empty when complete.
    pub partial_path: Vec<String>,
}

// Values and property names may contain private tool arguments.
impl fmt::Debug for Snapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Snapshot")
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}
impl Snapshot {
    /// Mirrors upstream's field-state traversal, including missing children.
    pub fn field_state(&self, path: &[&str]) -> State {
        let mut parent = &self.value;
        let mut partial = self.partial_path.as_slice();
        for segment in path {
            if self.state == State::Complete {
                return State::Complete;
            }
            // JavaScript arrays have an own `length` property even though it
            // is not a JSON array element. No streaming value path targets it.
            if matches!(parent, Value::Array(_)) && *segment == "length" {
                return State::Complete;
            }
            let child = match parent {
                Value::Object(object) => object.get(*segment),
                Value::Array(array) => segment
                    .parse::<usize>()
                    .ok()
                    .filter(|i| i.to_string() == *segment)
                    .and_then(|i| array.get(i)),
                _ => return State::Partial,
            };
            let Some(child) = child else {
                return State::Partial;
            };
            if partial.first().map(String::as_str) != Some(*segment) {
                return State::Complete;
            }
            partial = &partial[1..];
            parent = child;
        }
        self.state
    }
}

/// Why incremental JSON parsing failed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// A limit was zero or beyond the hard bound.
    InvalidLimits,
    /// The source exceeds the byte limit.
    SourceTooLarge,
    /// The value has more nodes than allowed.
    TooManyNodes,
    /// The value nests deeper than allowed.
    TooDeep,
    /// The text is not valid JSON.
    InvalidSyntax,
    /// An object key such as `__proto__` is refused.
    UnsafeKey,
    /// A string contains an escape the parser does not accept.
    UnsupportedUnicode,
    /// A number cannot be represented exactly.
    UnsupportedNumber,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ObjectState {
    First,
    Key,
    Colon,
    Value,
    After,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ArrayState {
    First,
    Value,
    After,
}
#[derive(Clone, Debug)]
enum Frame {
    Object {
        path: Vec<String>,
        state: ObjectState,
        key: Option<String>,
    },
    Array {
        path: Vec<String>,
        state: ArrayState,
        index: usize,
    },
}
impl Frame {
    fn path(&self) -> &Vec<String> {
        match self {
            Self::Object { path, .. } | Self::Array { path, .. } => path,
        }
    }
}
#[derive(Clone, Debug)]
enum Role {
    Key,
    Value(Vec<String>),
}
#[derive(Clone, Debug)]
enum Escape {
    None,
    Single,
    Unicode { digits: u8, value: u16 },
}
#[derive(Clone, Copy, Debug)]
enum NumberPhase {
    Minus,
    Zero,
    Integer,
    Dot,
    Fraction,
    Exponent,
    ExponentSign,
    ExponentDigits,
}
impl NumberPhase {
    fn next(self, ch: char) -> Result<Self, Error> {
        use NumberPhase::*;
        match (self, ch) {
            (Minus, '0') => Ok(Zero),
            (Minus, '1'..='9') => Ok(Integer),
            (Integer, '0'..='9') => Ok(Integer),
            (Zero | Integer, '.') => Ok(Dot),
            (Dot | Fraction, '0'..='9') => Ok(Fraction),
            (Zero | Integer | Fraction, 'e' | 'E') => Ok(Exponent),
            (Exponent, '+' | '-') => Ok(ExponentSign),
            (Exponent | ExponentSign | ExponentDigits, '0'..='9') => Ok(ExponentDigits),
            _ => Err(Error::InvalidSyntax),
        }
    }
    fn complete(self) -> bool {
        matches!(
            self,
            Self::Zero | Self::Integer | Self::Fraction | Self::ExponentDigits
        )
    }
}
#[derive(Clone, Debug)]
enum Token {
    String {
        role: Role,
        text: String,
        escape: Escape,
        high: Option<u16>,
    },
    Number {
        path: Vec<String>,
        text: String,
        complete_len: usize,
        phase: NumberPhase,
    },
    Literal {
        expected: &'static str,
        offset: usize,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Start,
    Parsing,
    Complete,
}

/// Cloneable for host-side transaction staging. Plain text appended to an open
/// value string avoids cloning the retained tree within `append`.
#[derive(Clone)]
pub struct Parser {
    limits: Limits,
    source_bytes: usize,
    nodes: usize,
    mode: Mode,
    frames: Vec<Frame>,
    token: Option<Token>,
    snapshot: Snapshot,
}

impl fmt::Debug for Parser {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Parser").finish_non_exhaustive()
    }
}

impl Parser {
    /// A parser with the given limits.
    pub fn new(limits: Limits) -> Result<Self, Error> {
        if limits.source_bytes == 0
            || limits.nodes == 0
            || limits.depth == 0
            || limits.depth > MAX_DEPTH
        {
            return Err(Error::InvalidLimits);
        }
        Ok(Self {
            limits,
            source_bytes: 0,
            nodes: 1,
            mode: Mode::Start,
            frames: Vec::new(),
            token: None,
            snapshot: Snapshot {
                value: Value::Object(Map::new()),
                state: State::Partial,
                partial_path: Vec::new(),
            },
        })
    }
    /// The value read so far.
    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }
    /// Bytes of text accepted so far.
    pub fn text_len(&self) -> usize {
        self.source_bytes
    }

    /// Atomically consume one UTF-8 fragment. On error no parser state changes.
    pub fn append(&mut self, delta: &str) -> Result<(), Error> {
        let size = self
            .source_bytes
            .checked_add(delta.len())
            .filter(|n| *n <= self.limits.source_bytes)
            .ok_or(Error::SourceTooLarge)?;
        if delta.is_empty() {
            return Ok(());
        }
        if self.append_plain_string(delta, size) {
            return Ok(());
        }
        *self = self.clone().appended(delta)?;
        Ok(())
    }

    // A plain value-string fragment cannot change syntax or violate a node or
    // depth limit. All checks precede mutation, so no rollback is needed.
    fn append_plain_string(&mut self, delta: &str, size: usize) -> bool {
        if delta
            .chars()
            .all(|ch| ch >= '\u{20}' && ch != '"' && ch != '\\')
        {
            if let Some(Token::String {
                role: Role::Value(path),
                text,
                escape: Escape::None,
                high: None,
            }) = &mut self.token
            {
                if let Some(Value::String(value)) = value_at_mut(&mut self.snapshot.value, path) {
                    text.push_str(delta);
                    value.push_str(delta);
                    self.source_bytes = size;
                    return true;
                }
            }
        }
        false
    }
    /// Consume a staged parser and return its updated state. On failure the
    /// staged parser is discarded; the caller's original clone is unaffected.
    /// This avoids a second full-tree clone in a larger host transaction.
    pub fn appended(mut self, delta: &str) -> Result<Self, Error> {
        let size = self
            .source_bytes
            .checked_add(delta.len())
            .ok_or(Error::SourceTooLarge)?;
        if size > self.limits.source_bytes {
            return Err(Error::SourceTooLarge);
        }
        if delta.is_empty() {
            return Ok(self);
        }
        for ch in delta.chars() {
            self.consume(ch)?;
        }
        self.source_bytes = size;
        self.publish_partial()?;
        Ok(self)
    }
    fn consume(&mut self, ch: char) -> Result<(), Error> {
        let mut reconsume = true;
        while reconsume {
            reconsume = false;
            if let Some(token) = self.token.take() {
                reconsume = self.consume_token(token, ch)?;
                continue;
            }
            match self.mode {
                Mode::Start => {
                    if whitespace(ch) {
                        continue;
                    }
                    if ch != '{' {
                        return Err(Error::InvalidSyntax);
                    }
                    self.mode = Mode::Parsing;
                    self.frames.push(Frame::Object {
                        path: vec![],
                        state: ObjectState::First,
                        key: None,
                    });
                }
                Mode::Complete => {
                    if !whitespace(ch) {
                        return Err(Error::InvalidSyntax);
                    }
                }
                Mode::Parsing => {
                    if whitespace(ch) {
                        continue;
                    }
                    let frame = self.frames.last().cloned().ok_or(Error::InvalidSyntax)?;
                    match frame {
                        Frame::Object { path, state, key } => match state {
                            ObjectState::First if ch == '}' => self.close()?,
                            ObjectState::First | ObjectState::Key if ch == '"' => {
                                self.set_object_state(ObjectState::Key)?;
                                self.token = Some(Token::String {
                                    role: Role::Key,
                                    text: String::new(),
                                    escape: Escape::None,
                                    high: None,
                                });
                            }
                            ObjectState::Colon if ch == ':' => {
                                self.set_object_state(ObjectState::Value)?
                            }
                            ObjectState::Value => {
                                let mut p = path;
                                p.push(key.ok_or(Error::InvalidSyntax)?);
                                self.start_value(ch, p)?;
                            }
                            ObjectState::After if ch == ',' => {
                                self.set_object_state(ObjectState::Key)?;
                            }
                            ObjectState::After if ch == '}' => self.close()?,
                            _ => return Err(Error::InvalidSyntax),
                        },
                        Frame::Array { path, state, index } => match state {
                            ArrayState::First if ch == ']' => self.close()?,
                            ArrayState::First | ArrayState::Value => {
                                self.set_array_state(ArrayState::Value)?;
                                let mut p = path;
                                p.push(index.to_string());
                                self.start_value(ch, p)?;
                            }
                            ArrayState::After if ch == ',' => {
                                if let Some(Frame::Array { state, index, .. }) =
                                    self.frames.last_mut()
                                {
                                    *state = ArrayState::Value;
                                    *index = index.checked_add(1).ok_or(Error::TooManyNodes)?;
                                }
                            }
                            ArrayState::After if ch == ']' => self.close()?,
                            _ => return Err(Error::InvalidSyntax),
                        },
                    }
                }
            }
        }
        Ok(())
    }
    fn set_object_state(&mut self, next: ObjectState) -> Result<(), Error> {
        if let Some(Frame::Object { state, .. }) = self.frames.last_mut() {
            *state = next;
            Ok(())
        } else {
            Err(Error::InvalidSyntax)
        }
    }
    fn set_array_state(&mut self, next: ArrayState) -> Result<(), Error> {
        if let Some(Frame::Array { state, .. }) = self.frames.last_mut() {
            *state = next;
            Ok(())
        } else {
            Err(Error::InvalidSyntax)
        }
    }
    fn start_value(&mut self, ch: char, path: Vec<String>) -> Result<(), Error> {
        self.nodes = self.nodes.checked_add(1).ok_or(Error::TooManyNodes)?;
        if self.nodes > self.limits.nodes {
            return Err(Error::TooManyNodes);
        }
        match ch {
            '{' | '[' => {
                if self.frames.len() >= self.limits.depth {
                    return Err(Error::TooDeep);
                }
                self.write(
                    &path,
                    if ch == '{' {
                        Value::Object(Map::new())
                    } else {
                        Value::Array(vec![])
                    },
                )?;
                self.frames.push(if ch == '{' {
                    Frame::Object {
                        path,
                        state: ObjectState::First,
                        key: None,
                    }
                } else {
                    Frame::Array {
                        path,
                        state: ArrayState::First,
                        index: 0,
                    }
                });
            }
            '"' => {
                self.write(&path, Value::String(String::new()))?;
                self.token = Some(Token::String {
                    role: Role::Value(path),
                    text: String::new(),
                    escape: Escape::None,
                    high: None,
                });
            }
            't' | 'f' | 'n' => {
                let (expected, value) = match ch {
                    't' => ("true", Value::Bool(true)),
                    'f' => ("false", Value::Bool(false)),
                    _ => ("null", Value::Null),
                };
                self.write(&path, value)?;
                self.token = Some(Token::Literal {
                    expected,
                    offset: 1,
                });
            }
            '-' | '0'..='9' => {
                let text = ch.to_string();
                if let Some(value) = number_value(&text)? {
                    self.write(&path, value)?;
                }
                let complete_len = if ch == '-' { 0 } else { 1 };
                self.token = Some(Token::Number {
                    path,
                    text,
                    complete_len,
                    phase: match ch {
                        '-' => NumberPhase::Minus,
                        '0' => NumberPhase::Zero,
                        _ => NumberPhase::Integer,
                    },
                });
            }
            _ => return Err(Error::InvalidSyntax),
        }
        Ok(())
    }
    fn consume_token(&mut self, token: Token, ch: char) -> Result<bool, Error> {
        match token {
            Token::String {
                role,
                mut text,
                mut escape,
                mut high,
            } => {
                match escape {
                    Escape::None => match ch {
                        '\\' => escape = Escape::Single,
                        '"' => {
                            if high.is_some() {
                                return Err(Error::UnsupportedUnicode);
                            }
                            match role {
                                Role::Key => {
                                    let parent =
                                        self.frames.last_mut().ok_or(Error::InvalidSyntax)?;
                                    if let Frame::Object { path, state, key } = parent {
                                        if text == "__proto__"
                                            || (text == "prototype"
                                                && path.last().map(String::as_str)
                                                    == Some("constructor"))
                                        {
                                            return Err(Error::UnsafeKey);
                                        }
                                        *key = Some(text);
                                        *state = ObjectState::Colon;
                                    } else {
                                        return Err(Error::InvalidSyntax);
                                    }
                                }
                                Role::Value(path) => {
                                    self.write(&path, Value::String(text))?;
                                    self.finish_value()?;
                                }
                            }
                            return Ok(false);
                        }
                        c if c < '\u{20}' => return Err(Error::InvalidSyntax),
                        c => {
                            if high.is_some() {
                                return Err(Error::UnsupportedUnicode);
                            }
                            text.push(c);
                        }
                    },
                    Escape::Single => {
                        if ch == 'u' {
                            escape = Escape::Unicode {
                                digits: 0,
                                value: 0,
                            };
                        } else {
                            if high.is_some() {
                                return Err(Error::UnsupportedUnicode);
                            }
                            text.push(match ch {
                                '"' => '"',
                                '\\' => '\\',
                                '/' => '/',
                                'b' => '\u{8}',
                                'f' => '\u{c}',
                                'n' => '\n',
                                'r' => '\r',
                                't' => '\t',
                                _ => return Err(Error::InvalidSyntax),
                            });
                            escape = Escape::None;
                        }
                    }
                    Escape::Unicode { digits, value } => {
                        let digit = ch.to_digit(16).ok_or(Error::InvalidSyntax)? as u16;
                        let value = value * 16 + digit;
                        if digits + 1 < 4 {
                            escape = Escape::Unicode {
                                digits: digits + 1,
                                value,
                            };
                        } else {
                            if (0xD800..=0xDBFF).contains(&value) {
                                if high.is_some() {
                                    return Err(Error::UnsupportedUnicode);
                                }
                                high = Some(value);
                            } else if (0xDC00..=0xDFFF).contains(&value) {
                                let h = high.take().ok_or(Error::UnsupportedUnicode)?;
                                let scalar = 0x10000
                                    + (((h as u32) - 0xD800) << 10)
                                    + ((value as u32) - 0xDC00);
                                text.push(char::from_u32(scalar).ok_or(Error::UnsupportedUnicode)?);
                            } else {
                                if high.is_some() {
                                    return Err(Error::UnsupportedUnicode);
                                }
                                text.push(
                                    char::from_u32(value as u32)
                                        .ok_or(Error::UnsupportedUnicode)?,
                                );
                            }
                            escape = Escape::None;
                        }
                    }
                }
                self.token = Some(Token::String {
                    role,
                    text,
                    escape,
                    high,
                });
            }
            Token::Number {
                path,
                mut text,
                mut complete_len,
                phase,
            } => {
                if matches!(ch, '0'..='9' | 'e' | 'E' | '+' | '-' | '.') {
                    let phase = phase.next(ch)?;
                    text.push(ch);
                    if phase.complete() {
                        complete_len = text.len();
                    }
                    self.token = Some(Token::Number {
                        path,
                        text,
                        complete_len,
                        phase,
                    });
                } else {
                    let value = number_value(&text)?.ok_or(Error::InvalidSyntax)?;
                    self.write(&path, value)?;
                    self.finish_value()?;
                    return Ok(true);
                }
            }
            Token::Literal { expected, offset } => {
                if expected.as_bytes().get(offset).copied() != Some(ch as u8) || !ch.is_ascii() {
                    return Err(Error::InvalidSyntax);
                }
                if offset + 1 == expected.len() {
                    self.finish_value()?;
                } else {
                    self.token = Some(Token::Literal {
                        expected,
                        offset: offset + 1,
                    });
                }
            }
        }
        Ok(false)
    }
    fn finish_value(&mut self) -> Result<(), Error> {
        match self.frames.last_mut() {
            Some(Frame::Object { state, .. }) if *state == ObjectState::Value => {
                *state = ObjectState::After;
                Ok(())
            }
            Some(Frame::Array { state, .. }) if *state == ArrayState::Value => {
                *state = ArrayState::After;
                Ok(())
            }
            _ => Err(Error::InvalidSyntax),
        }
    }
    fn close(&mut self) -> Result<(), Error> {
        self.frames.pop().ok_or(Error::InvalidSyntax)?;
        if self.frames.is_empty() {
            self.mode = Mode::Complete;
            Ok(())
        } else {
            self.finish_value()
        }
    }
    fn write(&mut self, path: &[String], value: Value) -> Result<(), Error> {
        let (last, ancestors) = path.split_last().ok_or(Error::InvalidSyntax)?;
        let mut target = &mut self.snapshot.value;
        for segment in ancestors {
            target = match target {
                Value::Object(object) => object.get_mut(segment),
                Value::Array(array) => segment.parse::<usize>().ok().and_then(|i| array.get_mut(i)),
                _ => None,
            }
            .ok_or(Error::InvalidSyntax)?;
        }
        match target {
            Value::Object(object) => {
                object.insert(last.clone(), value);
            }
            Value::Array(array) => {
                let index = last.parse::<usize>().map_err(|_| Error::InvalidSyntax)?;
                if index == array.len() {
                    array.push(value);
                } else if let Some(slot) = array.get_mut(index) {
                    *slot = value;
                } else {
                    return Err(Error::InvalidSyntax);
                }
            }
            _ => return Err(Error::InvalidSyntax),
        }
        Ok(())
    }
    fn publish_partial(&mut self) -> Result<(), Error> {
        let update = match &self.token {
            Some(Token::String {
                role: Role::Value(path),
                text,
                ..
            }) => Some((path.clone(), Value::String(text.clone()))),
            Some(Token::Number {
                path,
                text,
                complete_len,
                ..
            }) if *complete_len > 0 => {
                number_value(&text[..*complete_len])?.map(|v| (path.clone(), v))
            }
            _ => None,
        };
        if let Some((path, value)) = update {
            self.write(&path, value)?;
        }
        self.snapshot.state = if self.mode == Mode::Complete {
            State::Complete
        } else {
            State::Partial
        };
        self.snapshot.partial_path = if self.mode == Mode::Complete {
            vec![]
        } else {
            match &self.token {
                Some(Token::String {
                    role: Role::Value(path),
                    ..
                })
                | Some(Token::Number { path, .. }) => path.clone(),
                _ => self
                    .frames
                    .last()
                    .map(|frame| frame.path().clone())
                    .unwrap_or_default(),
            }
        };
        Ok(())
    }
}
fn value_at_mut<'a>(mut value: &'a mut Value, path: &[String]) -> Option<&'a mut Value> {
    for segment in path {
        value = match value {
            Value::Object(object) => object.get_mut(segment)?,
            Value::Array(array) => array.get_mut(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(value)
}
fn whitespace(ch: char) -> bool {
    matches!(ch, ' ' | '\n' | '\t' | '\r')
}

fn number_value(text: &str) -> Result<Option<Value>, Error> {
    if text == "-" || text.ends_with(['.', 'e', 'E', '+', '-']) {
        return Ok(None);
    }
    serde_json::from_str::<Value>(text)
        .map(Some)
        .map_err(|_| Error::UnsupportedNumber)
}
