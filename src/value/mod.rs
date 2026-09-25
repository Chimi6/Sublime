//! The value hub: the tree the tree-shaped data formats (TOML, and later
//! YAML and XML) read into and write from, and the push-mode sink a reader
//! streams into when its format allows it. JSON reads into it through the
//! sink; a converter walks the tree to write.

use std::fmt;
use std::io;
use std::mem;

/// One value in the tree. Tables keep their members in document order.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Integer(i64),
    Float(f64),
    String(String),
    /// A date, time, or datetime as written (TOML's four forms).
    Datetime(String),
    Array(Vec<Value>),
    Table(Vec<(String, Value)>),
}

impl Value {
    pub fn is_table(&self) -> bool {
        matches!(self, Value::Table(_))
    }
}

/// A leaf as a reader hands it to a sink, borrowing the reader's text.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Scalar<'a> {
    Null,
    Bool(bool),
    Integer(i64),
    Float(f64),
    String(&'a str),
    Datetime(&'a str),
}

/// Where a reader pushes values. Every table member is a `key` followed by
/// one value; arrays are values between `begin_array` and `end_array`.
pub trait ValueSink {
    fn begin_table(&mut self) -> io::Result<()>;
    fn key(&mut self, key: &str) -> io::Result<()>;
    fn end_table(&mut self) -> io::Result<()>;
    fn begin_array(&mut self) -> io::Result<()>;
    fn end_array(&mut self) -> io::Result<()>;
    fn scalar(&mut self, scalar: Scalar<'_>) -> io::Result<()>;
}

/// Builds a `Value` tree from sink calls.
#[derive(Default)]
pub struct TreeBuilder {
    open: Vec<Frame>,
    pending_key: Option<String>,
    root: Option<Value>,
}

/// An open container and the key it will be placed under in its parent.
struct Frame {
    key: Option<String>,
    container: Container,
}

enum Container {
    Table(Vec<(String, Value)>),
    Array(Vec<Value>),
}

impl TreeBuilder {
    pub fn new() -> Self {
        TreeBuilder::default()
    }

    /// The finished tree, or `None` when nothing was pushed.
    pub fn finish(self) -> Option<Value> {
        self.root
    }

    fn place(&mut self, key: Option<String>, value: Value) {
        match self.open.last_mut().map(|frame| &mut frame.container) {
            Some(Container::Table(members)) => members.push((key.unwrap_or_default(), value)),
            Some(Container::Array(items)) => items.push(value),
            None => self.root = Some(value),
        }
    }

    fn open(&mut self, container: Container) {
        let key = self.pending_key.take();
        self.open.push(Frame { key, container });
    }

    fn close(&mut self) {
        let Some(frame) = self.open.pop() else {
            return;
        };
        let value = match frame.container {
            Container::Table(members) => Value::Table(members),
            Container::Array(items) => Value::Array(items),
        };
        self.place(frame.key, value);
    }
}

impl ValueSink for TreeBuilder {
    fn begin_table(&mut self) -> io::Result<()> {
        self.open(Container::Table(Vec::new()));
        Ok(())
    }

    fn key(&mut self, key: &str) -> io::Result<()> {
        self.pending_key = Some(key.to_string());
        Ok(())
    }

    fn end_table(&mut self) -> io::Result<()> {
        self.close();
        Ok(())
    }

    fn begin_array(&mut self) -> io::Result<()> {
        self.open(Container::Array(Vec::new()));
        Ok(())
    }

    fn end_array(&mut self) -> io::Result<()> {
        self.close();
        Ok(())
    }

    fn scalar(&mut self, scalar: Scalar<'_>) -> io::Result<()> {
        let value = match scalar {
            Scalar::Null => Value::Null,
            Scalar::Bool(flag) => Value::Bool(flag),
            Scalar::Integer(number) => Value::Integer(number),
            Scalar::Float(number) => Value::Float(number),
            Scalar::String(text) => Value::String(text.to_string()),
            Scalar::Datetime(text) => Value::Datetime(text.to_string()),
        };
        let key = self.pending_key.take();
        self.place(key, value);
        Ok(())
    }
}

/// Appends a finite float in the shortest form that reads back exactly:
/// exponent form past 1e21 or under 1e-6 (the JavaScript rule), otherwise
/// plain, always with a fraction or exponent so no format reads it as an
/// integer.
pub fn push_float(out: &mut String, number: f64) {
    use std::fmt::Write;
    let magnitude = number.abs();
    let use_exponent = magnitude != 0.0 && !(1e-6..1e21).contains(&magnitude);
    if use_exponent {
        let _ = write!(out, "{number:e}");
        return;
    }
    let start = out.len();
    let _ = write!(out, "{number}");
    let looks_integral = !out[start..].contains('.');
    if looks_integral {
        out.push_str(".0");
    }
}

/// Appends `text` as a double-quoted string with the C-style escapes TOML,
/// YAML, and JSON share (`\"`, `\\`, `\b`, `\t`, `\n`, `\f`, `\r`) and
/// `\uXXXX` for the other control characters.
pub fn push_double_quoted(out: &mut String, text: &str) {
    use std::fmt::Write;
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            control if (control as u32) < 0x20 || control == '\u{7f}' => {
                let _ = write!(out, "\\u{:04X}", control as u32);
            }
            other => out.push(other),
        }
    }
    out.push('"');
}

/// Text output handed to a sink in 64 KiB chunks, so a writer never
/// holds a whole document. Write failures are kept and returned by
/// `finish`, so the writers stay plain string-building code.
pub struct ChunkedText<'a> {
    buffer: String,
    sink: &'a mut dyn io::Write,
    failure: Option<io::Error>,
    total: usize,
}

const CHUNK: usize = 64 * 1024;

impl<'a> ChunkedText<'a> {
    pub fn new(sink: &'a mut dyn io::Write) -> ChunkedText<'a> {
        ChunkedText {
            buffer: String::with_capacity(CHUNK + 1024),
            sink,
            failure: None,
            total: 0,
        }
    }

    pub fn push(&mut self, character: char) {
        self.buffer.push(character);
        self.flush_if_full();
    }

    pub fn push_str(&mut self, text: &str) {
        self.buffer.push_str(text);
        self.flush_if_full();
    }

    /// True until anything has been pushed.
    pub fn is_empty(&self) -> bool {
        self.total == 0 && self.buffer.is_empty()
    }

    fn flush_if_full(&mut self) {
        if self.buffer.len() >= CHUNK {
            self.flush_buffer();
        }
    }

    fn flush_buffer(&mut self) {
        if self.failure.is_none() {
            if let Err(error) = self.sink.write_all(self.buffer.as_bytes()) {
                self.failure = Some(error);
            }
        }
        self.total += self.buffer.len();
        self.buffer.clear();
    }

    /// Writes what is left and reports the first failure.
    pub fn finish(mut self) -> io::Result<()> {
        self.flush_buffer();
        match self.failure.take() {
            Some(error) => Err(error),
            None => self.sink.flush(),
        }
    }
}

impl fmt::Write for ChunkedText<'_> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.push_str(text);
        Ok(())
    }
}

/// Takes the members out of a table value, leaving it empty.
pub fn take_members(value: &mut Value) -> Vec<(String, Value)> {
    match value {
        Value::Table(members) => mem::take(members),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_nests_tables_and_arrays_in_order() {
        let mut builder = TreeBuilder::new();
        builder.begin_table().unwrap();
        builder.key("a").unwrap();
        builder.begin_array().unwrap();
        builder.scalar(Scalar::Integer(1)).unwrap();
        builder.scalar(Scalar::String("x")).unwrap();
        builder.end_array().unwrap();
        builder.key("b").unwrap();
        builder.begin_table().unwrap();
        builder.key("c").unwrap();
        builder.scalar(Scalar::Null).unwrap();
        builder.end_table().unwrap();
        builder.end_table().unwrap();
        let expected = Value::Table(vec![
            (
                "a".to_string(),
                Value::Array(vec![Value::Integer(1), Value::String("x".to_string())]),
            ),
            (
                "b".to_string(),
                Value::Table(vec![("c".to_string(), Value::Null)]),
            ),
        ]);
        assert_eq!(builder.finish(), Some(expected));
    }

    #[test]
    fn floats_always_carry_a_fraction_or_exponent() {
        let mut out = String::new();
        push_float(&mut out, 72.0);
        out.push(' ');
        push_float(&mut out, 1e21);
        out.push(' ');
        push_float(&mut out, 6.626e-34);
        out.push(' ');
        push_float(&mut out, -0.01);
        out.push(' ');
        push_float(&mut out, 5e20);
        assert_eq!(out, "72.0 1e21 6.626e-34 -0.01 500000000000000000000.0");
    }
}
