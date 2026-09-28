//! Tokens of a content stream or a CMap: operands (any PDF object but a
//! reference) and the bare words between them, which are operators in a
//! content stream and keywords in a CMap.

use crate::io::pdf::object::{Object, Parser};

pub enum Token<'a> {
    Operand(Object),
    Operator(&'a [u8]),
}

pub struct Lexer<'a> {
    parser: Parser<'a>,
}

impl<'a> Lexer<'a> {
    pub fn new(bytes: &'a [u8]) -> Lexer<'a> {
        Lexer {
            parser: Parser::new(bytes, 0),
        }
    }

    pub fn position(&self) -> usize {
        self.parser.at
    }

    pub fn set_position(&mut self, at: usize) {
        self.parser.at = at;
    }

    pub fn bytes(&self) -> &'a [u8] {
        self.parser.bytes
    }

    /// The next token, or `None` at the end. A malformed operand is
    /// skipped a byte at a time, as viewers do.
    pub fn next_token(&mut self) -> Option<Token<'a>> {
        loop {
            self.parser.skip();
            let &byte = self.parser.bytes.get(self.parser.at)?;
            let starts_operand = byte.is_ascii_digit()
                || matches!(byte, b'+' | b'-' | b'.' | b'/' | b'(' | b'<' | b'[');
            if starts_operand {
                let start = self.parser.at;
                match self.parser.object() {
                    // Content streams have no references: `1 0 R` there
                    // is never meant, so a stray one reads as numbers.
                    Ok(Object::Reference(number, _)) => {
                        self.parser.at = start;
                        self.word();
                        return Some(Token::Operand(Object::Integer(i64::from(number))));
                    }
                    Ok(object) => return Some(Token::Operand(object)),
                    Err(_) => {
                        self.parser.at = start + 1;
                        continue;
                    }
                }
            }
            if matches!(byte, b']' | b')' | b'>' | b'{' | b'}') {
                self.parser.at += 1;
                continue;
            }
            let word = self.word();
            return Some(match word {
                b"true" => Token::Operand(Object::Bool(true)),
                b"false" => Token::Operand(Object::Bool(false)),
                b"null" => Token::Operand(Object::Null),
                _ => Token::Operator(word),
            });
        }
    }

    /// A run of regular characters.
    fn word(&mut self) -> &'a [u8] {
        let bytes = self.parser.bytes;
        let start = self.parser.at;
        while let Some(&byte) = bytes.get(self.parser.at) {
            if byte.is_ascii_whitespace()
                || byte == 0
                || matches!(
                    byte,
                    b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
                )
            {
                break;
            }
            self.parser.at += 1;
        }
        if self.parser.at == start {
            self.parser.at += 1;
        }
        &bytes[start..self.parser.at]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operands_and_operators_alternate() {
        let mut lexer = Lexer::new(b"BT /F1 12 Tf [(a)-250(b)] TJ 0 Tw (x) ' ET");
        let mut seen = Vec::new();
        while let Some(token) = lexer.next_token() {
            seen.push(match token {
                Token::Operand(Object::Name(name)) => {
                    format!("/{}", String::from_utf8_lossy(&name))
                }
                Token::Operand(Object::Integer(value)) => value.to_string(),
                Token::Operand(Object::Array(items)) => format!("[{}]", items.len()),
                Token::Operand(Object::String(bytes)) => {
                    format!("({})", String::from_utf8_lossy(&bytes))
                }
                Token::Operand(_) => "?".to_string(),
                Token::Operator(word) => String::from_utf8_lossy(word).into_owned(),
            });
        }
        assert_eq!(
            seen,
            [
                "BT", "/F1", "12", "Tf", "[3]", "TJ", "0", "Tw", "(x)", "'", "ET"
            ]
        );
    }
}
