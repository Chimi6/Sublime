//! Compact JSON writer. Tracks nesting so callers never write commas.

use std::io::{self, Write};

pub struct JsonWriter<W: Write> {
    sink: W,
    needs_comma: Vec<bool>,
}

impl<W: Write> JsonWriter<W> {
    pub fn new(sink: W) -> Self {
        JsonWriter {
            sink,
            needs_comma: Vec::new(),
        }
    }

    pub fn begin_array(&mut self) -> io::Result<()> {
        self.before_value()?;
        self.sink.write_all(b"[")?;
        self.needs_comma.push(false);
        Ok(())
    }

    pub fn end_array(&mut self) -> io::Result<()> {
        self.needs_comma.pop();
        self.sink.write_all(b"]")
    }

    pub fn begin_object(&mut self) -> io::Result<()> {
        self.before_value()?;
        self.sink.write_all(b"{")?;
        self.needs_comma.push(false);
        Ok(())
    }

    pub fn end_object(&mut self) -> io::Result<()> {
        self.needs_comma.pop();
        self.sink.write_all(b"}")
    }

    pub fn key(&mut self, key: &str) -> io::Result<()> {
        self.before_value()?;
        self.write_escaped(key)?;
        self.sink.write_all(b":")?;
        if let Some(flag) = self.needs_comma.last_mut() {
            *flag = false;
        }
        Ok(())
    }

    pub fn string(&mut self, value: &str) -> io::Result<()> {
        self.before_value()?;
        self.write_escaped(value)
    }

    /// Writes `text` as is. For numbers and literals.
    pub fn raw(&mut self, text: &str) -> io::Result<()> {
        self.before_value()?;
        self.sink.write_all(text.as_bytes())
    }

    pub fn null(&mut self) -> io::Result<()> {
        self.raw("null")
    }

    pub fn flush(&mut self) -> io::Result<()> {
        self.sink.flush()
    }

    pub fn into_inner(self) -> W {
        self.sink
    }

    fn before_value(&mut self) -> io::Result<()> {
        let flag = match self.needs_comma.last_mut() {
            Some(flag) => flag,
            None => return Ok(()),
        };
        if *flag {
            self.sink.write_all(b",")?;
        }
        *flag = true;
        Ok(())
    }

    fn write_escaped(&mut self, value: &str) -> io::Result<()> {
        self.sink.write_all(b"\"")?;
        let bytes = value.as_bytes();
        let mut segment_start = 0usize;
        for (index, byte) in bytes.iter().enumerate() {
            let needs_escape = *byte == b'"' || *byte == b'\\' || *byte < 0x20;
            if !needs_escape {
                continue;
            }
            self.sink.write_all(&bytes[segment_start..index])?;
            self.write_escape_sequence(*byte)?;
            segment_start = index + 1;
        }
        self.sink.write_all(&bytes[segment_start..])?;
        self.sink.write_all(b"\"")
    }

    fn write_escape_sequence(&mut self, byte: u8) -> io::Result<()> {
        match byte {
            b'"' => self.sink.write_all(b"\\\""),
            b'\\' => self.sink.write_all(b"\\\\"),
            b'\n' => self.sink.write_all(b"\\n"),
            b'\r' => self.sink.write_all(b"\\r"),
            b'\t' => self.sink.write_all(b"\\t"),
            0x08 => self.sink.write_all(b"\\b"),
            0x0C => self.sink.write_all(b"\\f"),
            other => write!(self.sink, "\\u{other:04x}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finish(writer: JsonWriter<Vec<u8>>) -> String {
        let bytes = writer.into_inner();
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
    fn top_level_scalar_has_no_comma() {
        let mut writer = JsonWriter::new(Vec::new());
        writer.raw("42").unwrap();
        assert_eq!(finish(writer), "42");
    }
}
