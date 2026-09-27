//! PDF objects and the parser that reads them from bytes: numbers,
//! strings (literal and hex), names, arrays, dictionaries, references,
//! and a stream's dictionary with the byte range of its data.

use crate::io::pdf::PdfError;

/// A PDF object. A stream keeps its dictionary and where its raw data
/// lies in the bytes it was parsed from.
#[derive(Debug, Clone, PartialEq)]
pub enum Object {
    Null,
    Bool(bool),
    Integer(i64),
    Real(f64),
    String(Vec<u8>),
    Name(Vec<u8>),
    Array(Vec<Object>),
    Dictionary(Dictionary),
    Stream(Dictionary, std::ops::Range<usize>),
    Reference(u32, u16),
}

/// A dictionary: keys (names, without the slash) and values in order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Dictionary(pub Vec<(Vec<u8>, Object)>);

impl Dictionary {
    pub fn get(&self, key: &[u8]) -> Option<&Object> {
        self.0
            .iter()
            .find(|(name, _)| name.as_slice() == key)
            .map(|(_, value)| value)
    }
}

impl Object {
    pub fn as_integer(&self) -> Option<i64> {
        match self {
            Object::Integer(value) => Some(*value),
            Object::Real(value) => Some(*value as i64),
            _ => None,
        }
    }

    pub fn as_number(&self) -> Option<f64> {
        match self {
            Object::Integer(value) => Some(*value as f64),
            Object::Real(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_name(&self) -> Option<&[u8]> {
        match self {
            Object::Name(name) => Some(name),
            _ => None,
        }
    }

    pub fn as_dictionary(&self) -> Option<&Dictionary> {
        match self {
            Object::Dictionary(dictionary) | Object::Stream(dictionary, _) => Some(dictionary),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Object]> {
        match self {
            Object::Array(items) => Some(items),
            _ => None,
        }
    }
}

fn is_whitespace(byte: u8) -> bool {
    matches!(byte, 0 | 9 | 10 | 12 | 13 | 32)
}

fn is_delimiter(byte: u8) -> bool {
    matches!(
        byte,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

fn fail(message: &str) -> PdfError {
    PdfError(message.to_string())
}

/// Reads objects from `bytes` starting at a position.
pub struct Parser<'a> {
    pub bytes: &'a [u8],
    pub at: usize,
}

impl<'a> Parser<'a> {
    pub fn new(bytes: &'a [u8], at: usize) -> Parser<'a> {
        Parser { bytes, at }
    }

    /// Skips whitespace and comments.
    pub fn skip(&mut self) {
        while let Some(&byte) = self.bytes.get(self.at) {
            if is_whitespace(byte) {
                self.at += 1;
            } else if byte == b'%' {
                while let Some(&byte) = self.bytes.get(self.at) {
                    if byte == b'\n' || byte == b'\r' {
                        break;
                    }
                    self.at += 1;
                }
            } else {
                break;
            }
        }
    }

    /// A bare word (a keyword or a number), without advancing past it
    /// when `peek`.
    fn word(&mut self) -> &'a [u8] {
        let start = self.at;
        while let Some(&byte) = self.bytes.get(self.at) {
            if is_whitespace(byte) || is_delimiter(byte) {
                break;
            }
            self.at += 1;
        }
        &self.bytes[start..self.at]
    }

    /// Whether the next word is `keyword`; consumes it when it is.
    pub fn keyword(&mut self, keyword: &[u8]) -> bool {
        self.skip();
        let start = self.at;
        if self.word() == keyword {
            true
        } else {
            self.at = start;
            false
        }
    }

    /// The next object. References (`n g R`) are recognized; `stream`
    /// after a dictionary is handled by `indirect`.
    pub fn object(&mut self) -> Result<Object, PdfError> {
        self.object_at_depth(0)
    }

    fn object_at_depth(&mut self, depth: usize) -> Result<Object, PdfError> {
        if depth > 256 {
            return Err(fail("PDF objects nested too deeply"));
        }
        self.skip();
        let Some(&byte) = self.bytes.get(self.at) else {
            return Err(fail("PDF ends inside an object"));
        };
        match byte {
            b'/' => {
                self.at += 1;
                Ok(Object::Name(self.name()))
            }
            b'(' => {
                self.at += 1;
                self.literal_string().map(Object::String)
            }
            b'<' if self.bytes.get(self.at + 1) == Some(&b'<') => {
                self.at += 2;
                let mut entries = Vec::new();
                loop {
                    self.skip();
                    if self.bytes.get(self.at..self.at + 2) == Some(b">>") {
                        self.at += 2;
                        return Ok(Object::Dictionary(Dictionary(entries)));
                    }
                    if self.bytes.get(self.at) != Some(&b'/') {
                        return Err(fail("PDF dictionary key is not a name"));
                    }
                    self.at += 1;
                    let key = self.name();
                    let value = self.object_at_depth(depth + 1)?;
                    entries.push((key, value));
                }
            }
            b'<' => {
                self.at += 1;
                self.hex_string().map(Object::String)
            }
            b'[' => {
                self.at += 1;
                let mut items = Vec::new();
                loop {
                    self.skip();
                    match self.bytes.get(self.at) {
                        Some(b']') => {
                            self.at += 1;
                            return Ok(Object::Array(items));
                        }
                        None => return Err(fail("PDF ends inside an array")),
                        _ => items.push(self.object_at_depth(depth + 1)?),
                    }
                }
            }
            _ => self.number_or_keyword(),
        }
    }

    fn number_or_keyword(&mut self) -> Result<Object, PdfError> {
        let start = self.at;
        let word = self.word();
        if word.is_empty() {
            self.at += 1;
            return Err(fail("unexpected byte in PDF"));
        }
        match word {
            b"true" => return Ok(Object::Bool(true)),
            b"false" => return Ok(Object::Bool(false)),
            b"null" => return Ok(Object::Null),
            _ => {}
        }
        let text = std::str::from_utf8(word).map_err(|_| fail("PDF number is not text"))?;
        if let Ok(integer) = text.parse::<i64>() {
            // `n g R`: a reference, when two integers and an R follow.
            let after = self.at;
            self.skip();
            let generation_start = self.at;
            let generation = self.word();
            if let Ok(generation) = std::str::from_utf8(generation).unwrap_or("").parse::<u16>() {
                if generation_start != self.at && self.keyword(b"R") && integer >= 0 {
                    return Ok(Object::Reference(integer as u32, generation));
                }
            }
            self.at = after;
            return Ok(Object::Integer(integer));
        }
        match text.parse::<f64>() {
            Ok(real) => Ok(Object::Real(real)),
            Err(_) => {
                self.at = start + word.len();
                Err(fail("unexpected word in PDF"))
            }
        }
    }

    fn name(&mut self) -> Vec<u8> {
        let mut name = Vec::new();
        while let Some(&byte) = self.bytes.get(self.at) {
            if is_whitespace(byte) || is_delimiter(byte) {
                break;
            }
            self.at += 1;
            if byte == b'#' {
                let hex = self.bytes.get(self.at..self.at + 2).and_then(|pair| {
                    std::str::from_utf8(pair)
                        .ok()
                        .and_then(|text| u8::from_str_radix(text, 16).ok())
                });
                if let Some(value) = hex {
                    name.push(value);
                    self.at += 2;
                    continue;
                }
            }
            name.push(byte);
        }
        name
    }

    fn literal_string(&mut self) -> Result<Vec<u8>, PdfError> {
        let mut out = Vec::new();
        let mut depth = 1;
        loop {
            let Some(&byte) = self.bytes.get(self.at) else {
                return Err(fail("PDF ends inside a string"));
            };
            self.at += 1;
            match byte {
                b'(' => {
                    depth += 1;
                    out.push(byte);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(out);
                    }
                    out.push(byte);
                }
                b'\\' => {
                    let Some(&escaped) = self.bytes.get(self.at) else {
                        return Err(fail("PDF ends inside a string"));
                    };
                    self.at += 1;
                    match escaped {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'\r' => {
                            if self.bytes.get(self.at) == Some(&b'\n') {
                                self.at += 1;
                            }
                        }
                        b'\n' => {}
                        b'0'..=b'7' => {
                            let mut value = u32::from(escaped - b'0');
                            for _ in 0..2 {
                                match self.bytes.get(self.at) {
                                    Some(&digit @ b'0'..=b'7') => {
                                        value = value * 8 + u32::from(digit - b'0');
                                        self.at += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push(value as u8);
                        }
                        other => out.push(other),
                    }
                }
                _ => out.push(byte),
            }
        }
    }

    fn hex_string(&mut self) -> Result<Vec<u8>, PdfError> {
        let mut out = Vec::new();
        let mut high: Option<u8> = None;
        loop {
            let Some(&byte) = self.bytes.get(self.at) else {
                return Err(fail("PDF ends inside a hex string"));
            };
            self.at += 1;
            let digit = match byte {
                b'>' => break,
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                b'A'..=b'F' => byte - b'A' + 10,
                _ if is_whitespace(byte) => continue,
                _ => return Err(fail("bad hex string in PDF")),
            };
            match high.take() {
                None => high = Some(digit),
                Some(first) => out.push((first << 4) | digit),
            }
        }
        if let Some(first) = high {
            out.push(first << 4);
        }
        Ok(out)
    }

    /// An indirect object at the parser's position: `n g obj`, the
    /// object, and for a stream its data's range (the length given by
    /// `length`, which resolves a reference, or found by `endstream`).
    pub fn indirect(
        &mut self,
        length: &mut dyn FnMut(&Object) -> Option<usize>,
    ) -> Result<(u32, u16, Object), PdfError> {
        self.skip();
        let number = self.word();
        let number: u32 = std::str::from_utf8(number)
            .ok()
            .and_then(|text| text.parse().ok())
            .ok_or_else(|| fail("PDF object has no number"))?;
        self.skip();
        let generation: u16 = std::str::from_utf8(self.word())
            .ok()
            .and_then(|text| text.parse().ok())
            .ok_or_else(|| fail("PDF object has no generation"))?;
        if !self.keyword(b"obj") {
            return Err(fail("PDF object has no 'obj'"));
        }
        let object = self.object()?;
        let Object::Dictionary(dictionary) = object else {
            return Ok((number, generation, object));
        };
        if !self.keyword(b"stream") {
            return Ok((number, generation, Object::Dictionary(dictionary)));
        }
        // The data starts after the end of line that follows `stream`.
        match self.bytes.get(self.at) {
            Some(b'\r') if self.bytes.get(self.at + 1) == Some(&b'\n') => self.at += 2,
            Some(b'\r' | b'\n') => self.at += 1,
            _ => {}
        }
        let start = self.at;
        let declared = dictionary.get(b"Length").and_then(&mut *length);
        let end = match declared {
            Some(size) if self.fits(start, size) => start + size,
            _ => self.find_endstream(start)?,
        };
        self.at = end;
        Ok((number, generation, Object::Stream(dictionary, start..end)))
    }

    /// Whether `size` bytes from `start` end where `endstream` follows.
    fn fits(&self, start: usize, size: usize) -> bool {
        let Some(end) = start.checked_add(size) else {
            return false;
        };
        if end > self.bytes.len() {
            return false;
        }
        let mut probe = Parser::new(self.bytes, end);
        probe.keyword(b"endstream")
    }

    /// The end of a stream whose length is missing or wrong: the last
    /// end-of-line before `endstream`.
    fn find_endstream(&self, start: usize) -> Result<usize, PdfError> {
        let rest = &self.bytes[start..];
        let at = rest
            .windows(9)
            .position(|window| window == b"endstream")
            .ok_or_else(|| fail("PDF stream has no end"))?;
        let mut end = start + at;
        while end > start && matches!(self.bytes[end - 1], b'\n' | b'\r') {
            end -= 1;
        }
        Ok(end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Object {
        Parser::new(text.as_bytes(), 0).object().unwrap()
    }

    #[test]
    fn objects_parse() {
        assert_eq!(parse("42"), Object::Integer(42));
        assert_eq!(parse("-3.5"), Object::Real(-3.5));
        assert_eq!(parse(".5"), Object::Real(0.5));
        assert_eq!(parse("/Type"), Object::Name(b"Type".to_vec()));
        assert_eq!(parse("/A#20B"), Object::Name(b"A B".to_vec()));
        assert_eq!(
            parse("(a (b) \\\\ \\101 \\n)"),
            Object::String(b"a (b) \\ A \n".to_vec())
        );
        assert_eq!(parse("<48 65 6c6C 6>"), Object::String(b"Hell`".to_vec()));
        assert_eq!(parse("12 0 R"), Object::Reference(12, 0));
        assert_eq!(
            parse("[1 2 R 3 /N]"),
            Object::Array(vec![
                Object::Reference(1, 2),
                Object::Integer(3),
                Object::Name(b"N".to_vec())
            ])
        );
        let dictionary =
            parse("<< /Type /Page /Kids [4 0 R 5 0 R] /Count 2 % comment\n /Rect [0 0 1.5 2] >>");
        let dictionary = dictionary.as_dictionary().unwrap();
        assert_eq!(dictionary.get(b"Count"), Some(&Object::Integer(2)));
        assert_eq!(
            dictionary.get(b"Type").and_then(Object::as_name),
            Some(&b"Page"[..])
        );
    }

    #[test]
    fn a_stream_takes_its_length_or_finds_its_end() {
        let bytes = b"7 0 obj\n<< /Length 5 >>\nstream\r\nhello\nendstream\nendobj";
        let mut parser = Parser::new(bytes, 0);
        let (number, _, object) = parser
            .indirect(&mut |value| value.as_integer().map(|n| n as usize))
            .unwrap();
        assert_eq!(number, 7);
        let Object::Stream(_, range) = object else {
            panic!("not a stream")
        };
        assert_eq!(&bytes[range], b"hello");
        // A wrong length falls back to the end marker.
        let bytes = b"7 0 obj\n<< /Length 99 >>\nstream\nhello\nendstream\nendobj";
        let (_, _, object) = Parser::new(bytes, 0)
            .indirect(&mut |value| value.as_integer().map(|n| n as usize))
            .unwrap();
        let Object::Stream(_, range) = object else {
            panic!("not a stream")
        };
        assert_eq!(&bytes[range], b"hello");
    }
}
