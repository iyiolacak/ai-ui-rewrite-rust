//! Bounded, transactional application of assistant-ui state operations.
//!
//! This crate owns no transport, thread, clock or session. The host binds each
//! instance to the originating run, retains any raw frames it needs, and clears
//! it on scope teardown. A successful batch changes the state atomically; a
//! rejected batch leaves the previous state intact.
#![forbid(unsafe_code)]

use serde_json::{Map, Value};
use std::fmt;

/// JSON values are cloned and updated recursively; keep this independent of
/// host configuration so untrusted inputs cannot exhaust the call stack.
pub const MAX_DEPTH: usize = 128;

/// Limits cover retained JSON data and operation shape, not allocator overhead,
/// caller-owned inputs or transient copies. Counts include JSON containers.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Most operations in one update.
    pub operations_per_batch: usize,
    /// Most segments in an operation's path.
    pub path_segments: usize,
    /// Most bytes in an operation's path.
    pub path_bytes: usize,
    /// Deepest nesting accepted.
    pub depth: usize,
    /// Most values (including containers) accepted.
    pub nodes: usize,
    /// Most bytes in one value.
    pub value_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            operations_per_batch: 256,
            path_segments: 128,
            path_bytes: 16 * 1024,
            depth: 128,
            nodes: 100_000,
            value_bytes: 4 * 1024 * 1024,
        }
    }
}

/// One update to the message's state.
#[derive(Clone, PartialEq)]
#[non_exhaustive]
pub enum Operation {
    /// Set the value at `path`, creating containers as needed.
    /// Set the value at `path`, creating containers as needed.
    Set {
        /// Property names and array indices from the root; empty is the root.
        path: Vec<String>,
        /// The value to store.
        value: Value,
    },
    /// Append text to the string at `path`.
    /// Append text to the string at `path`.
    AppendText {
        /// Property names and array indices from the root.
        path: Vec<String>,
        /// The text to append.
        value: String,
    },
}

// Both paths and values are supplied by the stream and can contain private data.
impl fmt::Debug for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let variant = match self {
            Self::Set { .. } => "Set",
            Self::AppendText { .. } => "AppendText",
        };
        f.debug_struct(variant).finish_non_exhaustive()
    }
}

/// Why a state update was rejected; the state is left unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// A limit was zero or beyond the hard bound.
    InvalidLimits,
    /// An update has more operations than allowed.
    TooManyOperations,
    /// A path has more segments or bytes than allowed.
    PathTooLong,
    /// An object key such as `__proto__` is refused.
    UnsafeKey,
    /// A path does not lead anywhere an operation can act.
    InvalidPath,
    /// A path segment is not a valid array index.
    InvalidArrayIndex,
    /// An array index is past the end of the array.
    ArrayIndexOutOfBounds,
    /// Appending text needs a string at the path.
    ExpectedString,
    /// The value nests deeper than allowed.
    TooDeep,
    /// The value has more nodes than allowed.
    TooManyNodes,
    /// A value exceeds the byte limit.
    ValueTooLarge,
    /// The operation is not one of the supported kinds.
    InvalidOperation,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}

/// Decode the `operations` array from a validated `update-state` frame.
/// String paths are preserved. Integral numeric path segments within the JS
/// safe-integer range are normalized to JavaScript's decimal property names;
/// other numeric segments are rejected rather than silently renamed.
pub fn decode_operations(value: &Value, limits: Limits) -> Result<Vec<Operation>, Error> {
    validate_limits(limits)?;
    // Bound the whole decoded operation batch before allocating owned copies.
    validate_value(value, limits)?;
    let items = value.as_array().ok_or(Error::InvalidOperation)?;
    if items.len() > limits.operations_per_batch {
        return Err(Error::TooManyOperations);
    }
    let mut result = Vec::with_capacity(items.len());
    for item in items {
        let object = item.as_object().ok_or(Error::InvalidOperation)?;
        let segments = object
            .get("path")
            .and_then(Value::as_array)
            .ok_or(Error::InvalidOperation)?;
        if segments.len() > limits.path_segments || segments.len() > limits.depth {
            return Err(Error::PathTooLong);
        }
        let mut path = Vec::with_capacity(segments.len());
        let mut path_bytes = 0usize;
        for segment in segments {
            let segment = match segment {
                Value::String(value) => value.clone(),
                Value::Number(value) => {
                    let value = value.as_f64().ok_or(Error::InvalidOperation)?;
                    if !value.is_finite()
                        || value.fract() != 0.0
                        || value.abs() > 9_007_199_254_740_991.0
                    {
                        return Err(Error::InvalidOperation);
                    }
                    if value == 0.0 {
                        "0".to_owned()
                    } else {
                        format!("{value:.0}")
                    }
                }
                _ => return Err(Error::InvalidOperation),
            };
            if is_unsafe(&segment) {
                return Err(Error::UnsafeKey);
            }
            path_bytes = path_bytes
                .checked_add(segment.len())
                .ok_or(Error::PathTooLong)?;
            if path_bytes > limits.path_bytes {
                return Err(Error::PathTooLong);
            }
            path.push(segment);
        }
        let operation = match object.get("type").and_then(Value::as_str) {
            Some("set") => {
                let value = object.get("value").ok_or(Error::InvalidOperation)?;
                validate_value(value, limits)?;
                Operation::Set {
                    path,
                    value: value.clone(),
                }
            }
            Some("append-text") => {
                let value = object
                    .get("value")
                    .and_then(Value::as_str)
                    .ok_or(Error::InvalidOperation)?;
                if value.len() > limits.value_bytes {
                    return Err(Error::ValueTooLarge);
                }
                Operation::AppendText {
                    path,
                    value: value.to_owned(),
                }
            }
            _ => return Err(Error::InvalidOperation),
        };
        result.push(operation);
    }
    Ok(result)
}

/// Mutable state owned by one host scope. `value` is always a completed batch.
pub struct State {
    value: Value,
    limits: Limits,
}

impl State {
    /// A state tree starting from `value`.
    pub fn new(value: Value, limits: Limits) -> Result<Self, Error> {
        if let Err(error) = validate_limits(limits).and_then(|()| validate_value(&value, limits)) {
            drop_iteratively(value);
            return Err(error);
        }
        Ok(Self { value, limits })
    }

    /// The current state.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Apply operations in order. The entire batch is rejected on any error.
    /// Cloning is bounded by `Limits`, but is not an incremental-performance claim.
    pub fn apply(&mut self, operations: &[Operation]) -> Result<(), Error> {
        if operations.is_empty() {
            return Ok(());
        }
        self.value = self.prepare(operations)?.value;
        Ok(())
    }

    /// Build the completed next state without changing the current state.
    /// The caller can publish the returned state only after another operation
    /// succeeds. This stages a bounded clone, not a durable transaction.
    pub fn prepare(&self, operations: &[Operation]) -> Result<Self, Error> {
        if operations.len() > self.limits.operations_per_batch {
            return Err(Error::TooManyOperations);
        }
        if operations.is_empty() {
            return Ok(Self {
                value: self.value.clone(),
                limits: self.limits,
            });
        }
        // Validate input before cloning any caller-supplied values.
        // Bound the whole batch so intermediate candidates remain bounded even
        // when an early operation is subsequently replaced in the same batch.
        let mut input_bytes = 0usize;
        let mut input_nodes = 0usize;
        for operation in operations {
            let path = match operation {
                Operation::Set { path, value } => {
                    let (nodes, bytes) = measure_value(value, self.limits)?;
                    input_nodes = input_nodes.checked_add(nodes).ok_or(Error::TooManyNodes)?;
                    input_bytes = input_bytes.checked_add(bytes).ok_or(Error::ValueTooLarge)?;
                    path
                }
                Operation::AppendText { path, value } => {
                    input_bytes = input_bytes
                        .checked_add(value.len())
                        .ok_or(Error::ValueTooLarge)?;
                    path
                }
            };
            validate_path(path, self.limits)?;
            input_nodes = input_nodes
                .checked_add(path.len())
                .ok_or(Error::TooManyNodes)?;
            for segment in path {
                input_bytes = input_bytes
                    .checked_add(segment.len())
                    .ok_or(Error::ValueTooLarge)?;
            }
            if input_nodes > self.limits.nodes {
                return Err(Error::TooManyNodes);
            }
            if input_bytes > self.limits.value_bytes {
                return Err(Error::ValueTooLarge);
            }
        }
        let mut candidate = self.value.clone();
        for operation in operations {
            match operation {
                Operation::Set { path, value } => update(&mut candidate, path, &mut |slot| {
                    *slot = value.clone();
                    Ok(())
                })?,
                Operation::AppendText { path, value } => {
                    update(&mut candidate, path, &mut |slot| {
                        let text = slot.as_str().ok_or(Error::ExpectedString)?;
                        if text.len().saturating_add(value.len()) > self.limits.value_bytes {
                            return Err(Error::ValueTooLarge);
                        }
                        if let Value::String(text) = slot {
                            text.push_str(value);
                        }
                        Ok(())
                    })?
                }
            }
        }
        validate_value(&candidate, self.limits)?;
        Ok(Self {
            value: candidate,
            limits: self.limits,
        })
    }
}

fn validate_limits(limits: Limits) -> Result<(), Error> {
    if limits.operations_per_batch == 0
        || limits.path_segments == 0
        || limits.path_bytes == 0
        || limits.depth == 0
        || limits.nodes == 0
        || limits.value_bytes == 0
        || limits.depth > MAX_DEPTH
        || limits.path_segments > MAX_DEPTH
    {
        return Err(Error::InvalidLimits);
    }
    Ok(())
}

// A caller can construct a JSON tree deeper than serde_json's decoder accepts.
// Dropping rejected owned input recursively could itself overflow the stack.
fn drop_iteratively(value: Value) {
    let mut stack = vec![value];
    while let Some(value) = stack.pop() {
        match value {
            Value::Array(items) => stack.extend(items),
            Value::Object(items) => stack.extend(items.into_values()),
            _ => {}
        }
    }
}

fn validate_path(path: &[String], limits: Limits) -> Result<(), Error> {
    if path.len() > limits.path_segments || path.len() > limits.depth {
        return Err(Error::PathTooLong);
    }
    let mut bytes = 0usize;
    for segment in path {
        if is_unsafe(segment) {
            return Err(Error::UnsafeKey);
        }
        bytes = bytes.checked_add(segment.len()).ok_or(Error::PathTooLong)?;
        if bytes > limits.path_bytes {
            return Err(Error::PathTooLong);
        }
    }
    Ok(())
}

fn is_unsafe(key: &str) -> bool {
    matches!(key, "__proto__" | "constructor" | "prototype")
}

fn validate_value(value: &Value, limits: Limits) -> Result<(), Error> {
    measure_value(value, limits).map(|_| ())
}

fn measure_value(value: &Value, limits: Limits) -> Result<(usize, usize), Error> {
    let mut stack = vec![(value, 0usize)];
    let mut nodes = 0usize;
    let mut bytes = 0usize;
    while let Some((value, depth)) = stack.pop() {
        if depth > limits.depth {
            return Err(Error::TooDeep);
        }
        nodes = nodes.checked_add(1).ok_or(Error::TooManyNodes)?;
        if nodes > limits.nodes {
            return Err(Error::TooManyNodes);
        }
        match value {
            Value::Array(items) => {
                if items.len()
                    > limits
                        .nodes
                        .saturating_sub(nodes.saturating_add(stack.len()))
                {
                    return Err(Error::TooManyNodes);
                }
                stack.extend(items.iter().map(|child| (child, depth + 1)));
            }
            Value::Object(items) => {
                if items.len()
                    > limits
                        .nodes
                        .saturating_sub(nodes.saturating_add(stack.len()))
                {
                    return Err(Error::TooManyNodes);
                }
                for (key, child) in items {
                    // Match secure-json-parse's unsafe-object patterns.
                    if key == "__proto__"
                        || (key == "constructor"
                            && child
                                .as_object()
                                .is_some_and(|o| o.contains_key("prototype")))
                    {
                        return Err(Error::UnsafeKey);
                    }
                    bytes = bytes.checked_add(key.len()).ok_or(Error::ValueTooLarge)?;
                    stack.push((child, depth + 1));
                }
            }
            Value::String(s) => bytes = bytes.checked_add(s.len()).ok_or(Error::ValueTooLarge)?,
            Value::Number(n) => {
                bytes = bytes
                    .checked_add(n.to_string().len())
                    .ok_or(Error::ValueTooLarge)?
            }
            Value::Bool(_) => bytes = bytes.checked_add(5).ok_or(Error::ValueTooLarge)?,
            Value::Null => bytes = bytes.checked_add(4).ok_or(Error::ValueTooLarge)?,
        }
        if bytes > limits.value_bytes {
            return Err(Error::ValueTooLarge);
        }
    }
    Ok((nodes, bytes))
}

fn update(
    slot: &mut Value,
    path: &[String],
    change: &mut impl FnMut(&mut Value) -> Result<(), Error>,
) -> Result<(), Error> {
    let Some((key, rest)) = path.split_first() else {
        return change(slot);
    };
    if slot.is_null() {
        *slot = Value::Object(Map::new());
    }
    match slot {
        Value::Object(object) => update(
            object.entry(key.clone()).or_insert(Value::Null),
            rest,
            change,
        ),
        Value::Array(array) => {
            if key.is_empty()
                || (key.len() > 1 && key.starts_with('0'))
                || !key.bytes().all(|b| b.is_ascii_digit())
            {
                return Err(Error::InvalidArrayIndex);
            }
            let index = key.parse::<usize>().map_err(|_| Error::InvalidArrayIndex)?;
            if index > array.len() {
                return Err(Error::ArrayIndexOutOfBounds);
            }
            if index == array.len() {
                array.push(Value::Null);
            }
            update(&mut array[index], rest, change)
        }
        _ => Err(Error::InvalidPath),
    }
}
