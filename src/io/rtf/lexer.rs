//! RTF tokens: group braces, control words with their numeric parameter,
//! control symbols, `\'hh` bytes, `\binN` data, and runs of plain text.
//! The lexer knows nothing of what a word means; `reader` does.

/// One token of an RTF stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token<'a> {
    /// `{`
    Open,
    /// `}`
    Close,
    /// `\name` or `\nameN`.
    Word {
        name: &'a str,
        parameter: Option<i32>,
    },
    /// A control symbol: `\\`, `\{`, `\}`, `\~`, `\-`, `\_`, `\*`, `\:`,
    /// `\|`, and `\` before a line break (`b'\n'`, a paragraph end).
    Symbol(u8),
    /// `\'hh`: a byte in the current code page.
    Byte(u8),
    /// Text between control sequences: bytes in the current code page,
    /// with line breaks (which RTF ignores) already removed.
    Text(&'a [u8]),
    /// `\binN` and the `N` raw bytes after it.
    Binary(&'a [u8]),
}

pub struct Lexer<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(bytes: &'a [u8]) -> Lexer<'a> {
        Lexer { bytes, at: 0 }
    }

    /// How far into the input the lexer is, in bytes.
    pub fn position(&self) -> usize {
        self.at
    }

    /// The next token, or `None` at the end.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<Token<'a>> {
        loop {
            let byte = *self.bytes.get(self.at)?;
            match byte {
                b'{' => {
                    self.at += 1;
                    return Some(Token::Open);
                }
                b'}' => {
                    self.at += 1;
                    return Some(Token::Close);
                }
                b'\\' => return Some(self.control()),
                // Line breaks in the source mean nothing.
                b'\r' | b'\n' => self.at += 1,
                _ => return Some(self.text()),
            }
        }
    }

    fn text(&mut self) -> Token<'a> {
        let start = self.at;
        while let Some(&byte) = self.bytes.get(self.at) {
            if matches!(byte, b'{' | b'}' | b'\\' | b'\r' | b'\n') {
                break;
            }
            self.at += 1;
        }
        Token::Text(&self.bytes[start..self.at])
    }

    fn control(&mut self) -> Token<'a> {
        // Past the backslash.
        self.at += 1;
        let Some(&first) = self.bytes.get(self.at) else {
            return Token::Symbol(b'\\');
        };
        if !first.is_ascii_alphabetic() {
            self.at += 1;
            return match first {
                b'\'' => self.hex_byte(),
                b'\r' | b'\n' => Token::Symbol(b'\n'),
                other => Token::Symbol(other),
            };
        }
        let name_start = self.at;
        while self
            .bytes
            .get(self.at)
            .is_some_and(|byte| byte.is_ascii_alphabetic())
        {
            self.at += 1;
        }
        let name = std::str::from_utf8(&self.bytes[name_start..self.at]).unwrap_or("");
        let parameter = self.parameter();
        // One space ends a control word and is part of it.
        if self.bytes.get(self.at) == Some(&b' ') {
            self.at += 1;
        }
        if name == "bin" {
            let length = parameter.unwrap_or(0).max(0) as usize;
            let start = self.at.min(self.bytes.len());
            let end = start.saturating_add(length).min(self.bytes.len());
            self.at = end;
            return Token::Binary(&self.bytes[start..end]);
        }
        Token::Word { name, parameter }
    }

    fn parameter(&mut self) -> Option<i32> {
        let start = self.at;
        let negative = self.bytes.get(self.at) == Some(&b'-');
        if negative {
            self.at += 1;
        }
        let digits_start = self.at;
        let mut value: i64 = 0;
        while let Some(&byte) = self.bytes.get(self.at) {
            if !byte.is_ascii_digit() {
                break;
            }
            value = (value * 10 + i64::from(byte - b'0')).min(i64::from(i32::MAX) + 1);
            self.at += 1;
        }
        if self.at == digits_start {
            // A lone '-' is not a parameter.
            self.at = start;
            return None;
        }
        let value = if negative { -value } else { value };
        Some(value.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32)
    }

    fn hex_byte(&mut self) -> Token<'a> {
        let digit = |byte: u8| (byte as char).to_digit(16);
        let high = self.bytes.get(self.at).copied().and_then(digit);
        let low = self.bytes.get(self.at + 1).copied().and_then(digit);
        match (high, low) {
            (Some(high), Some(low)) => {
                self.at += 2;
                Token::Byte((high * 16 + low) as u8)
            }
            // A malformed escape stands for nothing.
            _ => Token::Text(&[]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(source: &str) -> Vec<Token<'_>> {
        let mut lexer = Lexer::new(source.as_bytes());
        let mut out = Vec::new();
        while let Some(token) = lexer.next() {
            out.push(token);
        }
        out
    }

    #[test]
    fn words_take_a_parameter_and_one_space() {
        assert_eq!(
            tokens("{\\b0 bold\\fs-24x}"),
            vec![
                Token::Open,
                Token::Word {
                    name: "b",
                    parameter: Some(0)
                },
                Token::Text(b"bold"),
                Token::Word {
                    name: "fs",
                    parameter: Some(-24)
                },
                Token::Text(b"x"),
                Token::Close,
            ]
        );
    }

    #[test]
    fn symbols_bytes_and_line_breaks() {
        assert_eq!(
            tokens("a\\'e9\r\nb\\\\\\{\\\n"),
            vec![
                Token::Text(b"a"),
                Token::Byte(0xE9),
                Token::Text(b"b"),
                Token::Symbol(b'\\'),
                Token::Symbol(b'{'),
                Token::Symbol(b'\n'),
            ]
        );
    }

    #[test]
    fn binary_data_is_taken_raw() {
        assert_eq!(
            tokens("\\bin3 {}\\x"),
            vec![Token::Binary(b"{}\\"), Token::Text(b"x"),]
        );
    }
}
