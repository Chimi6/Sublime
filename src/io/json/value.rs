//! JSON into the value hub: a token walk that pushes into a `ValueSink`,
//! and `parse`, which builds the whole tree. For converters that can stream
//! (JSON -> CSV), the tokenizer is used directly instead.

use std::io::Read;

use crate::io::json::tokenizer::{JsonError, JsonTokenizer, Token};
use crate::value::{Scalar, TreeBuilder, Value, ValueSink};

/// Parses one JSON document from `source` into a tree.
pub fn parse<R: Read>(source: R) -> Result<Value, JsonError> {
    let mut builder = TreeBuilder::new();
    parse_into(source, &mut builder)?;
    Ok(builder.finish().unwrap_or(Value::Null))
}

/// Pushes one JSON document from `source` into `sink`.
pub fn parse_into<R: Read>(source: R, sink: &mut dyn ValueSink) -> Result<(), JsonError> {
    let mut tokens = JsonTokenizer::new(source);
    let first = tokens.next_token()?;
    push_value(&mut tokens, first, sink)?;
    match tokens.next_token()? {
        Token::End => Ok(()),
        other => Err(JsonError::Unexpected {
            location: tokens.location(),
            message: format!("unexpected {} after the document", other.describe()),
        }),
    }
}

fn push_value<R: Read>(
    tokens: &mut JsonTokenizer<R>,
    first: Token,
    sink: &mut dyn ValueSink,
) -> Result<(), JsonError> {
    match first {
        Token::Null => sink.scalar(Scalar::Null)?,
        Token::True => sink.scalar(Scalar::Bool(true))?,
        Token::False => sink.scalar(Scalar::Bool(false))?,
        Token::Number => sink.scalar(number_scalar(tokens.text()))?,
        Token::String => sink.scalar(Scalar::String(tokens.text()))?,
        Token::BeginArray => push_array(tokens, sink)?,
        Token::BeginObject => push_object(tokens, sink)?,
        other => return Err(unexpected(tokens, other)),
    }
    Ok(())
}

fn push_array<R: Read>(
    tokens: &mut JsonTokenizer<R>,
    sink: &mut dyn ValueSink,
) -> Result<(), JsonError> {
    sink.begin_array()?;
    let mut expect_value = true;
    let mut is_empty = true;
    loop {
        let token = tokens.next_token()?;
        match token {
            Token::EndArray if !expect_value || is_empty => {
                sink.end_array()?;
                return Ok(());
            }
            Token::Comma if !expect_value => expect_value = true,
            other if expect_value => {
                push_value(tokens, other, sink)?;
                expect_value = false;
                is_empty = false;
            }
            other => return Err(unexpected(tokens, other)),
        }
    }
}

fn push_object<R: Read>(
    tokens: &mut JsonTokenizer<R>,
    sink: &mut dyn ValueSink,
) -> Result<(), JsonError> {
    sink.begin_table()?;
    let mut expect_key = true;
    let mut is_empty = true;
    loop {
        let token = tokens.next_token()?;
        match token {
            Token::EndObject if !expect_key || is_empty => {
                sink.end_table()?;
                return Ok(());
            }
            Token::Comma if !expect_key => expect_key = true,
            Token::String if expect_key => {
                sink.key(tokens.text())?;
                tokens.expect(Token::Colon)?;
                let first = tokens.next_token()?;
                push_value(tokens, first, sink)?;
                expect_key = false;
                is_empty = false;
            }
            other => return Err(unexpected(tokens, other)),
        }
    }
}

/// A JSON number as written becomes an integer when it has no fraction or
/// exponent and fits in 64 bits, otherwise a float.
fn number_scalar(text: &str) -> Scalar<'_> {
    let has_fraction_or_exponent = text.contains(['.', 'e', 'E']);
    if !has_fraction_or_exponent {
        if let Ok(integer) = text.parse::<i64>() {
            return Scalar::Integer(integer);
        }
    }
    match text.parse::<f64>() {
        Ok(float) => Scalar::Float(float),
        Err(_) => Scalar::String(text),
    }
}

fn unexpected<R: Read>(tokens: &JsonTokenizer<R>, token: Token) -> JsonError {
    JsonError::Unexpected {
        location: tokens.location(),
        message: format!("unexpected {}", token.describe()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_text(text: &str) -> Value {
        parse(text.as_bytes()).unwrap()
    }

    #[test]
    fn numbers_split_into_integers_and_floats() {
        let value = parse_text(r#"[1, -2, 1.5, 1e3, 99999999999999999999]"#);
        assert_eq!(
            value,
            Value::Array(vec![
                Value::Integer(1),
                Value::Integer(-2),
                Value::Float(1.5),
                Value::Float(1000.0),
                Value::Float(1e20),
            ])
        );
    }

    #[test]
    fn objects_keep_member_order() {
        let value = parse_text(r#"{"z": null, "a": {"b": true}}"#);
        assert_eq!(
            value,
            Value::Table(vec![
                ("z".to_string(), Value::Null),
                (
                    "a".to_string(),
                    Value::Table(vec![("b".to_string(), Value::Bool(true))])
                ),
            ])
        );
    }

    #[test]
    fn trailing_tokens_are_an_error() {
        assert!(parse("{} 1".as_bytes()).is_err());
        assert!(parse("[1,]".as_bytes()).is_err());
        assert!(parse(r#"{"a":1,}"#.as_bytes()).is_err());
    }
}
