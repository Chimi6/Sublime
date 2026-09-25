//! The value hub: the tree the tree-shaped data formats (TOML, YAML, XML,
//! JSON) read into and write from, as an arena (`tree`), the push-mode
//! sink readers stream into, and the small text helpers the writers share.

pub mod tree;

use std::fmt;
use std::io;

pub use tree::{Children, Data, MemberIndex, NONE, Node, Span, Tree, TreeSink};

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

#[cfg(test)]
mod tests {
    use super::*;

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
