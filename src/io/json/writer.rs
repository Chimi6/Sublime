//! Compact JSON writer. Tracks nesting so callers never write commas.

use std::io::{self, Write};

use crate::io::scan::find_json_escape;

/// Bytes accumulated before handing a chunk to the sink. Keeps every push an
/// inlined `Vec` append instead of a call through `dyn Write`.
const FLUSH_THRESHOLD: usize = 64 * 1024;

/// A key already escaped and suffixed with `:`. See `JsonWriter::prepare_key`.
pub struct PreparedKey {
    bytes: Vec<u8>,
}

pub struct JsonWriter<W: Write> {
    sink: W,
    buffer: Vec<u8>,
    needs_comma: Vec<bool>,
}

impl<W: Write> JsonWriter<W> {
    pub fn new(sink: W) -> Self {
        JsonWriter {
            sink,
            buffer: Vec::with_capacity(FLUSH_THRESHOLD),
            needs_comma: Vec::new(),
        }
    }

    pub fn begin_array(&mut self) -> io::Result<()> {
        self.before_value();
        self.buffer.push(b'[');
        self.needs_comma.push(false);
        self.flush_if_full()
    }

    pub fn end_array(&mut self) -> io::Result<()> {
        self.needs_comma.pop();
        self.buffer.push(b']');
        self.flush_if_full()
    }

    pub fn begin_object(&mut self) -> io::Result<()> {
        self.before_value();
        self.buffer.push(b'{');
        self.needs_comma.push(false);
        self.flush_if_full()
    }

    pub fn end_object(&mut self) -> io::Result<()> {
        self.needs_comma.pop();
        self.buffer.push(b'}');
        self.flush_if_full()
    }

    pub fn key(&mut self, key: &str) -> io::Result<()> {
        self.before_value();
        self.push_escaped(key);
        self.buffer.push(b':');
        if let Some(flag) = self.needs_comma.last_mut() {
            *flag = false;
        }
        self.flush_if_full()
    }

    /// Escapes a key once so it can be written many times without rescanning.
    pub fn prepare_key(key: &str) -> PreparedKey {
        let mut scratch = JsonWriter::new(Vec::new());
        scratch.push_escaped(key);
        scratch.buffer.push(b':');
        PreparedKey {
            bytes: scratch.buffer,
        }
    }

    /// Writes a key prepared by `prepare_key`.
    pub fn prepared_key(&mut self, key: &PreparedKey) -> io::Result<()> {
        self.before_value();
        self.buffer.extend_from_slice(&key.bytes);
        if let Some(flag) = self.needs_comma.last_mut() {
            *flag = false;
        }
        self.flush_if_full()
    }

    pub fn string(&mut self, value: &str) -> io::Result<()> {
        self.before_value();
        self.push_escaped(value);
        self.flush_if_full()
    }

    /// Writes `bytes` as a base64 string, straight into the buffer.
    pub fn base64_string(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.before_value();
        self.buffer.push(b'"');
        crate::io::base64::encode_into(bytes, &mut self.buffer);
        self.buffer.push(b'"');
        self.flush_if_full()
    }

    /// Writes `text` as is. For numbers and literals.
    pub fn raw(&mut self, text: &str) -> io::Result<()> {
        self.before_value();
        self.buffer.extend_from_slice(text.as_bytes());
        self.flush_if_full()
    }

    pub fn null(&mut self) -> io::Result<()> {
        self.raw("null")
    }

    /// Hands every buffered byte to the sink and flushes the sink. Call this
    /// before reading the sink or dropping the writer.
    pub fn flush(&mut self) -> io::Result<()> {
        self.flush_buffer()?;
        self.sink.flush()
    }

    /// Flushes and returns the sink.
    pub fn into_inner(mut self) -> io::Result<W> {
        self.flush()?;
        Ok(self.sink)
    }

    fn flush_if_full(&mut self) -> io::Result<()> {
        if self.buffer.len() < FLUSH_THRESHOLD {
            return Ok(());
        }
        self.flush_buffer()
    }

    fn flush_buffer(&mut self) -> io::Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        self.sink.write_all(&self.buffer)?;
        self.buffer.clear();
        Ok(())
    }

    fn before_value(&mut self) {
        let flag = match self.needs_comma.last_mut() {
            Some(flag) => flag,
            None => return,
        };
        if *flag {
            self.buffer.push(b',');
        }
        *flag = true;
    }

    fn push_escaped(&mut self, value: &str) {
        self.buffer.push(b'"');
        let mut remaining = value.as_bytes();
        while let Some(index) = find_json_escape(remaining) {
            self.buffer.extend_from_slice(&remaining[..index]);
            self.push_escape_sequence(remaining[index]);
            remaining = &remaining[index + 1..];
        }
        self.buffer.extend_from_slice(remaining);
        self.buffer.push(b'"');
    }

    fn push_escape_sequence(&mut self, byte: u8) {
        match byte {
            b'"' => self.buffer.extend_from_slice(b"\\\""),
            b'\\' => self.buffer.extend_from_slice(b"\\\\"),
            b'\n' => self.buffer.extend_from_slice(b"\\n"),
            b'\r' => self.buffer.extend_from_slice(b"\\r"),
            b'\t' => self.buffer.extend_from_slice(b"\\t"),
            0x08 => self.buffer.extend_from_slice(b"\\b"),
            0x0C => self.buffer.extend_from_slice(b"\\f"),
            other => {
                let text = format!("\\u{other:04x}");
                self.buffer.extend_from_slice(text.as_bytes());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finish(writer: JsonWriter<Vec<u8>>) -> String {
        let bytes = writer.into_inner().unwrap();
        String::from_utf8(bytes).unwrap()
    }

    #[test]
    fn writes_nested_structure_with_commas() {
        let mut writer = JsonWriter::new(Vec::new());
        writer.begin_array().unwrap();
        writer.begin_object().unwrap();
        writer.key("a").unwrap();
        writer.string("x").unwrap();
        writer.key("b").unwrap();
        writer.raw("1").unwrap();
        writer.key("c").unwrap();
        writer.null().unwrap();
        writer.end_object().unwrap();
        writer.begin_array().unwrap();
        writer.end_array().unwrap();
        writer.raw("true").unwrap();
        writer.end_array().unwrap();
        assert_eq!(finish(writer), "[{\"a\":\"x\",\"b\":1,\"c\":null},[],true]");
    }

    #[test]
    fn escapes_strings() {
        let mut writer = JsonWriter::new(Vec::new());
        writer.string("q\"b\\n\nr\rt\tc\x01 é").unwrap();
        assert_eq!(finish(writer), "\"q\\\"b\\\\n\\nr\\rt\\tc\\u0001 é\"");
    }

    #[test]
    fn prepared_key_writes_the_same_bytes_as_key() {
        let prepared = JsonWriter::<Vec<u8>>::prepare_key("a\"b");
        let mut with_prepared = JsonWriter::new(Vec::new());
        with_prepared.begin_object().unwrap();
        with_prepared.prepared_key(&prepared).unwrap();
        with_prepared.raw("1").unwrap();
        with_prepared.prepared_key(&prepared).unwrap();
        with_prepared.raw("2").unwrap();
        with_prepared.end_object().unwrap();
        let mut with_key = JsonWriter::new(Vec::new());
        with_key.begin_object().unwrap();
        with_key.key("a\"b").unwrap();
        with_key.raw("1").unwrap();
        with_key.key("a\"b").unwrap();
        with_key.raw("2").unwrap();
        with_key.end_object().unwrap();
        assert_eq!(finish(with_prepared), finish(with_key));
        assert_eq!(finish(JsonWriter::new(Vec::new())), "");
    }

    #[test]
    fn top_level_scalar_has_no_comma() {
        let mut writer = JsonWriter::new(Vec::new());
        writer.raw("42").unwrap();
        assert_eq!(finish(writer), "42");
    }
}
