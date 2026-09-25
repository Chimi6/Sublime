//! Copies one JSON value from a tokenizer to a writer, token by token, so
//! JSON Lines and JSON arrays convert into each other without a tree.

use std::io::{Read, Write};

use crate::io::json::tokenizer::{JsonError, JsonTokenizer, Token};
use crate::io::json::writer::JsonWriter;

/// Copies the value that `first` opens. Strings are re-escaped, numbers
/// and literals pass through as written.
pub fn copy_value<R: Read, W: Write>(
    tokens: &mut JsonTokenizer<R>,
    first: Token,
    writer: &mut JsonWriter<W>,
) -> Result<(), JsonError> {
    match first {
        Token::Null => writer.null()?,
        Token::True => writer.raw("true")?,
        Token::False => writer.raw("false")?,
        Token::Number => writer.raw(tokens.text())?,
        Token::String => writer.string(tokens.text())?,
        Token::BeginArray => {
            writer.begin_array()?;
            let mut expect_value = true;
            let mut is_empty = true;
            loop {
                let token = tokens.next_token()?;
                match token {
                    Token::EndArray if !expect_value || is_empty => break,
                    Token::Comma if !expect_value => expect_value = true,
                    other if expect_value => {
                        copy_value(tokens, other, writer)?;
                        expect_value = false;
                        is_empty = false;
                    }
                    other => return Err(unexpected(tokens, other)),
                }
            }
            writer.end_array()?;
        }
        Token::BeginObject => {
            writer.begin_object()?;
            let mut expect_key = true;
            let mut is_empty = true;
            loop {
                let token = tokens.next_token()?;
                match token {
                    Token::EndObject if !expect_key || is_empty => break,
                    Token::Comma if !expect_key => expect_key = true,
                    Token::String if expect_key => {
                        writer.key(tokens.text())?;
                        tokens.expect(Token::Colon)?;
                        let first = tokens.next_token()?;
                        copy_value(tokens, first, writer)?;
                        expect_key = false;
                        is_empty = false;
                    }
                    other => return Err(unexpected(tokens, other)),
                }
            }
            writer.end_object()?;
        }
        other => return Err(unexpected(tokens, other)),
    }
    Ok(())
}

fn unexpected<R: Read>(tokens: &JsonTokenizer<R>, token: Token) -> JsonError {
    JsonError::Unexpected {
        location: tokens.location(),
        message: format!("unexpected {}", token.describe()),
    }
}
