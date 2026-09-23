//! Streaming JSON tokenizer. Numbers stay as text; nothing is parsed into
//! floats. Strings are unescaped into a reusable scratch buffer.

use std::fmt;
use std::io::{self, Read};

use crate::converter::Location;

const BUFFER_SIZE: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    BeginObject,
    EndObject,
    BeginArray,
    EndArray,
    Colon,
    Comma,
    String,
    Number,
    True,
    False,
    Null,
    End,
}

impl Token {
    pub fn describe(&self) -> &'static str {
        match self {
            Token::BeginObject => "'{'",
            Token::EndObject => "'}'",
            Token::BeginArray => "'['",
            Token::EndArray => "']'",
            Token::Colon => "':'",
            Token::Comma => "','",
            Token::String => "a string",
            Token::Number => "a number",
            Token::True => "true",
            Token::False => "false",
            Token::Null => "null",
            Token::End => "end of input",
        }
    }
}

#[derive(Debug)]
pub enum JsonError {
    Io(io::Error),
    Unexpected { location: Location, message: String },
    InvalidUtf8 { location: Location },
}

impl From<io::Error> for JsonError {
    fn from(error: io::Error) -> Self {
        JsonError::Io(error)
    }
}

impl fmt::Display for JsonError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JsonError::Io(error) => write!(formatter, "{error}"),
            JsonError::Unexpected { location, message } => {
                write!(formatter, "{location}: {message}")
            }
            JsonError::InvalidUtf8 { location } => {
                write!(formatter, "{location}: invalid UTF-8 in string")
            }
        }
    }
}

impl std::error::Error for JsonError {}

pub struct JsonTokenizer<R: Read> {
    source: R,
    buffer: Vec<u8>,
    position: usize,
    filled: usize,
    reached_end: bool,
    line: u64,
    column: u64,
    scratch: String,
    bytes_consumed: u64,
}

impl<R: Read> JsonTokenizer<R> {
    pub fn new(source: R) -> Self {
        JsonTokenizer {
            source,
            buffer: vec![0; BUFFER_SIZE],
            position: 0,
            filled: 0,
            reached_end: false,
            line: 1,
            column: 0,
            scratch: String::new(),
            bytes_consumed: 0,
        }
    }

    /// Text of the last `String` (unescaped), `Number` (verbatim), or literal.
    pub fn text(&self) -> &str {
        &self.scratch
    }

    pub fn location(&self) -> Location {
        Location {
            line: self.line,
            column: self.column,
        }
    }

    pub fn bytes_consumed(&self) -> u64 {
        self.bytes_consumed
    }

    pub fn next_token(&mut self) -> Result<Token, JsonError> {
        let first = match self.skip_whitespace()? {
            Some(byte) => byte,
            None => return Ok(Token::End),
        };
        match first {
            b'{' => Ok(Token::BeginObject),
            b'}' => Ok(Token::EndObject),
            b'[' => Ok(Token::BeginArray),
            b']' => Ok(Token::EndArray),
            b':' => Ok(Token::Colon),
            b',' => Ok(Token::Comma),
            b'"' => {
                self.read_string()?;
                Ok(Token::String)
            }
            b'-' | b'0'..=b'9' => {
                self.read_number(first)?;
                Ok(Token::Number)
            }
            b't' => {
                self.read_literal(b"rue", "true")?;
                Ok(Token::True)
            }
            b'f' => {
                self.read_literal(b"alse", "false")?;
                Ok(Token::False)
            }
            b'n' => {
                self.read_literal(b"ull", "null")?;
                Ok(Token::Null)
            }
            other => {
                let message = format!("unexpected byte 0x{other:02x}");
                Err(self.unexpected(message))
            }
        }
    }

    pub fn expect(&mut self, expected: Token) -> Result<(), JsonError> {
        let actual = self.next_token()?;
        if actual == expected {
            return Ok(());
        }
        let message = format!(
            "expected {}, found {}",
            expected.describe(),
            actual.describe()
        );
        Err(self.unexpected(message))
    }

    /// Consumes the rest of the value that `first` started.
    pub fn skip_value(&mut self, first: Token) -> Result<(), JsonError> {
        let mut depth: u32 = match first {
            Token::BeginObject | Token::BeginArray => 1,
            Token::String | Token::Number | Token::True | Token::False | Token::Null => {
                return Ok(());
            }
            other => {
                let message = format!("expected a value, found {}", other.describe());
                return Err(self.unexpected(message));
            }
        };
        while depth > 0 {
            let token = self.next_token()?;
            match token {
                Token::BeginObject | Token::BeginArray => depth += 1,
                Token::EndObject | Token::EndArray => depth -= 1,
                Token::End => return Err(self.unexpected("unexpected end of input inside a value")),
                _ => {}
            }
        }
        Ok(())
    }

    fn unexpected(&self, message: impl Into<String>) -> JsonError {
        JsonError::Unexpected {
            location: self.location(),
            message: message.into(),
        }
    }

    fn skip_whitespace(&mut self) -> Result<Option<u8>, JsonError> {
        loop {
            let byte = match self.next_byte()? {
                Some(byte) => byte,
                None => return Ok(None),
            };
            let is_whitespace = matches!(byte, b' ' | b'\t' | b'\n' | b'\r');
            if !is_whitespace {
                return Ok(Some(byte));
            }
        }
    }

    fn read_string(&mut self) -> Result<(), JsonError> {
        let mut bytes = std::mem::take(&mut self.scratch).into_bytes();
        bytes.clear();
        loop {
            let byte = match self.next_byte()? {
                Some(byte) => byte,
                None => return Err(self.unexpected("unterminated string")),
            };
            match byte {
                b'"' => break,
                b'\\' => self.read_escape(&mut bytes)?,
                0x00..=0x1F => {
                    let message = format!("raw control character 0x{byte:02x} in string");
                    return Err(self.unexpected(message));
                }
                other => bytes.push(other),
            }
        }
        match String::from_utf8(bytes) {
            Ok(text) => {
                self.scratch = text;
                Ok(())
            }
            Err(_) => Err(JsonError::InvalidUtf8 {
                location: self.location(),
            }),
        }
    }

    fn read_escape(&mut self, bytes: &mut Vec<u8>) -> Result<(), JsonError> {
        let byte = match self.next_byte()? {
            Some(byte) => byte,
            None => return Err(self.unexpected("unterminated escape")),
        };
        match byte {
            b'"' => bytes.push(b'"'),
            b'\\' => bytes.push(b'\\'),
            b'/' => bytes.push(b'/'),
            b'b' => bytes.push(0x08),
            b'f' => bytes.push(0x0C),
            b'n' => bytes.push(b'\n'),
            b'r' => bytes.push(b'\r'),
            b't' => bytes.push(b'\t'),
            b'u' => {
                let code_point = self.read_unicode_escape()?;
                let mut encoded = [0u8; 4];
                let encoded_str = code_point.encode_utf8(&mut encoded);
                bytes.extend_from_slice(encoded_str.as_bytes());
            }
            other => {
                let message = format!("invalid escape '\\{}'", other as char);
                return Err(self.unexpected(message));
            }
        }
        Ok(())
    }

    fn read_unicode_escape(&mut self) -> Result<char, JsonError> {
        let first_unit = self.read_hex4()?;
        let is_high_surrogate = (0xD800..=0xDBFF).contains(&first_unit);
        let is_low_surrogate = (0xDC00..=0xDFFF).contains(&first_unit);
        if is_low_surrogate {
            return Err(self.unexpected("lone low surrogate in \\u escape"));
        }
        if !is_high_surrogate {
            return match char::from_u32(first_unit) {
                Some(character) => Ok(character),
                None => Err(self.unexpected("invalid code point in \\u escape")),
            };
        }
        let backslash = self.next_byte()?;
        let u_letter = self.next_byte()?;
        let has_second_escape = backslash == Some(b'\\') && u_letter == Some(b'u');
        if !has_second_escape {
            return Err(self.unexpected("high surrogate not followed by \\u low surrogate"));
        }
        let second_unit = self.read_hex4()?;
        let second_is_low = (0xDC00..=0xDFFF).contains(&second_unit);
        if !second_is_low {
            return Err(self.unexpected("high surrogate not followed by a low surrogate"));
        }
        let high_bits = (first_unit - 0xD800) << 10;
        let low_bits = second_unit - 0xDC00;
        let code_point = 0x10000 + high_bits + low_bits;
        match char::from_u32(code_point) {
            Some(character) => Ok(character),
            None => Err(self.unexpected("invalid surrogate pair")),
        }
    }

    fn read_hex4(&mut self) -> Result<u32, JsonError> {
        let mut value: u32 = 0;
        for _ in 0..4 {
            let byte = match self.next_byte()? {
                Some(byte) => byte,
                None => return Err(self.unexpected("unterminated \\u escape")),
            };
            let digit = match (byte as char).to_digit(16) {
                Some(digit) => digit,
                None => return Err(self.unexpected("non-hex digit in \\u escape")),
            };
            value = (value << 4) | digit;
        }
        Ok(value)
    }

    fn read_number(&mut self, first: u8) -> Result<(), JsonError> {
        self.scratch.clear();
        self.scratch.push(first as char);
        while let Some(byte) = self.peek_byte()? {
            let is_number_byte = matches!(byte, b'0'..=b'9' | b'+' | b'-' | b'.' | b'e' | b'E');
            if !is_number_byte {
                break;
            }
            self.next_byte()?;
            self.scratch.push(byte as char);
        }
        if !is_valid_number(&self.scratch) {
            let message = format!("invalid number '{}'", self.scratch);
            return Err(self.unexpected(message));
        }
        Ok(())
    }

    fn read_literal(&mut self, rest: &[u8], text: &str) -> Result<(), JsonError> {
        for expected in rest {
            let actual = self.next_byte()?;
            if actual != Some(*expected) {
                let message = format!("invalid literal, expected '{text}'");
                return Err(self.unexpected(message));
            }
        }
        self.scratch.clear();
        self.scratch.push_str(text);
        Ok(())
    }

    fn fill(&mut self) -> io::Result<bool> {
        if self.reached_end {
            return Ok(false);
        }
        let read_count = self.source.read(&mut self.buffer)?;
        if read_count == 0 {
            self.reached_end = true;
            return Ok(false);
        }
        self.position = 0;
        self.filled = read_count;
        Ok(true)
    }

    fn next_byte(&mut self) -> io::Result<Option<u8>> {
        if self.position == self.filled {
            let has_more = self.fill()?;
            if !has_more {
                return Ok(None);
            }
        }
        let byte = self.buffer[self.position];
        self.position += 1;
        self.bytes_consumed += 1;
        if byte == b'\n' {
            self.line += 1;
            self.column = 0;
        } else {
            self.column += 1;
        }
        Ok(Some(byte))
    }

    fn peek_byte(&mut self) -> io::Result<Option<u8>> {
        if self.position == self.filled {
            let has_more = self.fill()?;
            if !has_more {
                return Ok(None);
            }
        }
        Ok(Some(self.buffer[self.position]))
    }
}

/// Checks `text` against the JSON number grammar.
fn is_valid_number(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut index = 0usize;
    if bytes.get(index) == Some(&b'-') {
        index += 1;
    }
    match bytes.get(index) {
        Some(b'0') => index += 1,
        Some(b'1'..=b'9') => index += skip_digits(bytes, index),
        _ => return false,
    }
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        let fraction_digits = skip_digits(bytes, index);
        if fraction_digits == 0 {
            return false;
        }
        index += fraction_digits;
    }
    let has_exponent = matches!(bytes.get(index), Some(b'e') | Some(b'E'));
    if has_exponent {
        index += 1;
        let has_sign = matches!(bytes.get(index), Some(b'+') | Some(b'-'));
        if has_sign {
            index += 1;
        }
        let exponent_digits = skip_digits(bytes, index);
        if exponent_digits == 0 {
            return false;
        }
        index += exponent_digits;
    }
    index == bytes.len()
}

fn skip_digits(bytes: &[u8], start: usize) -> usize {
    let mut count = 0usize;
    while let Some(byte) = bytes.get(start + count) {
        if !byte.is_ascii_digit() {
            break;
        }
        count += 1;
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens_of(input: &[u8]) -> Result<Vec<(Token, String)>, JsonError> {
        let mut tokenizer = JsonTokenizer::new(input);
        let mut collected = Vec::new();
        loop {
            let token = tokenizer.next_token()?;
            let text = tokenizer.text().to_string();
            collected.push((token, text));
            if token == Token::End {
                break;
            }
        }
        Ok(collected)
    }

    fn kinds(input: &[u8]) -> Vec<Token> {
        tokens_of(input)
            .unwrap()
            .into_iter()
            .map(|(token, _)| token)
            .collect()
    }

    #[test]
    fn tokenizes_structure() {
        let expected = vec![
            Token::BeginArray,
            Token::BeginObject,
            Token::String,
            Token::Colon,
            Token::Number,
            Token::Comma,
            Token::String,
            Token::Colon,
            Token::True,
            Token::EndObject,
            Token::Comma,
            Token::Null,
            Token::EndArray,
            Token::End,
        ];
        assert_eq!(kinds(b" [ {\"a\" : 1 , \"b\":true} , null ] "), expected);
    }

    #[test]
    fn string_text_is_unescaped() {
        let tokens = tokens_of(b"\"a\\\"b\\\\c\\/d\\n\\t\\u0041\"").unwrap();
        assert_eq!(tokens[0].0, Token::String);
        assert_eq!(tokens[0].1, "a\"b\\c/d\n\tA");
    }

    #[test]
    fn surrogate_pairs_decode_to_one_char() {
        let tokens = tokens_of(b"\"\\uD83D\\uDE00\"").unwrap();
        assert_eq!(tokens[0].1, "\u{1F600}");
    }

    #[test]
    fn lone_surrogate_is_an_error() {
        let error = tokens_of(b"\"\\uD83D\"").unwrap_err();
        let is_unexpected = matches!(error, JsonError::Unexpected { .. });
        assert!(is_unexpected);
    }

    #[test]
    fn raw_control_character_in_string_is_an_error() {
        let error = tokens_of(b"\"a\x01b\"").unwrap_err();
        let is_unexpected = matches!(error, JsonError::Unexpected { .. });
        assert!(is_unexpected);
    }

    #[test]
    fn number_text_is_verbatim() {
        let tokens = tokens_of(b"-12.50e+3").unwrap();
        assert_eq!(tokens[0].0, Token::Number);
        assert_eq!(tokens[0].1, "-12.50e+3");
    }

    #[test]
    fn invalid_numbers_are_errors() {
        for bad in [&b"01"[..], b"1.", b"-", b".5", b"1e", b"+1"] {
            let result = tokens_of(bad);
            assert!(
                result.is_err(),
                "{:?} should be rejected",
                std::str::from_utf8(bad)
            );
        }
    }

    #[test]
    fn literals_set_text() {
        let tokens = tokens_of(b"[true,false,null]").unwrap();
        assert_eq!(tokens[1].1, "true");
        assert_eq!(tokens[3].1, "false");
        assert_eq!(tokens[5].1, "null");
    }

    #[test]
    fn bad_literal_is_an_error() {
        assert!(tokens_of(b"tru").is_err());
        assert!(tokens_of(b"nul").is_err());
    }

    #[test]
    fn unterminated_string_is_an_error() {
        assert!(tokens_of(b"\"abc").is_err());
    }

    #[test]
    fn unexpected_byte_reports_location() {
        let error = tokens_of(b"[\n  @").unwrap_err();
        match error {
            JsonError::Unexpected { location, .. } => {
                assert_eq!(location.line, 2);
                assert_eq!(location.column, 3);
            }
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[test]
    fn expect_checks_the_next_token() {
        let mut tokenizer = JsonTokenizer::new(&b"[]"[..]);
        tokenizer.expect(Token::BeginArray).unwrap();
        assert!(tokenizer.expect(Token::BeginObject).is_err());
    }

    #[test]
    fn skip_value_consumes_nested_values() {
        let mut tokenizer = JsonTokenizer::new(&b"[{\"a\":[1,{\"b\":2}]},7]"[..]);
        tokenizer.expect(Token::BeginArray).unwrap();
        let first = tokenizer.next_token().unwrap();
        tokenizer.skip_value(first).unwrap();
        assert_eq!(tokenizer.next_token().unwrap(), Token::Comma);
        assert_eq!(tokenizer.next_token().unwrap(), Token::Number);
        assert_eq!(tokenizer.text(), "7");
    }

    #[test]
    fn invalid_utf8_in_string_is_an_error() {
        let error = tokens_of(b"\"\xFF\"").unwrap_err();
        let is_invalid = matches!(error, JsonError::InvalidUtf8 { .. });
        assert!(is_invalid);
    }

    #[test]
    fn non_ascii_passes_through() {
        let tokens = tokens_of("\"héllo ✓\"".as_bytes()).unwrap();
        assert_eq!(tokens[0].1, "héllo ✓");
    }
}
