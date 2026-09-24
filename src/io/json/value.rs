//! A JSON document as a tree, for formats whose structure is JSON. Numbers
//! keep their text so callers parse them into exactly the type they need.

use std::io::Read;

use super::tokenizer::{JsonError, JsonTokenizer, Token};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonValue {
    Null,
    Bool(bool),
    /// The number as written.
    Number(String),
    String(String),
    Array(Vec<JsonValue>),
    /// Members in document order.
    Object(Vec<(String, JsonValue)>),
}

impl JsonValue {
    pub fn get(&self, key: &str) -> Option<&JsonValue> {
        match self {
            JsonValue::Object(members) => members
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            JsonValue::String(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[JsonValue]> {
        match self {
            JsonValue::Array(items) => Some(items),
            _ => None,
        }
    }
}

/// Parses one JSON document from `source`.
pub fn parse<R: Read>(source: R) -> Result<JsonValue, JsonError> {
    let mut tokens = JsonTokenizer::new(source);
    let first = tokens.next_token()?;
    let value = parse_value(&mut tokens, first)?;
    match tokens.next_token()? {
        Token::End => Ok(value),
        other => Err(JsonError::Unexpected {
            location: tokens.location(),
            message: format!("unexpected {} after the document", other.describe()),
        }),
    }
}

fn parse_value<R: Read>(
    tokens: &mut JsonTokenizer<R>,
    first: Token,
) -> Result<JsonValue, JsonError> {
    match first {
        Token::Null => Ok(JsonValue::Null),
        Token::True => Ok(JsonValue::Bool(true)),
        Token::False => Ok(JsonValue::Bool(false)),
        Token::Number => Ok(JsonValue::Number(tokens.text().to_string())),
        Token::String => Ok(JsonValue::String(tokens.text().to_string())),
        Token::BeginArray => {
            let mut items = Vec::new();
            let mut expect_value = true;
            loop {
                let token = tokens.next_token()?;
                match token {
                    Token::EndArray if !expect_value || items.is_empty() => {
                        return Ok(JsonValue::Array(items));
                    }
                    Token::Comma if !expect_value => expect_value = true,
                    other if expect_value => {
                        items.push(parse_value(tokens, other)?);
                        expect_value = false;
                    }
                    other => return Err(unexpected(tokens, other)),
                }
            }
        }
        Token::BeginObject => {
            let mut members = Vec::new();
            let mut expect_key = true;
            loop {
                let token = tokens.next_token()?;
                match token {
                    Token::EndObject if !expect_key || members.is_empty() => {
                        return Ok(JsonValue::Object(members));
                    }
                    Token::Comma if !expect_key => expect_key = true,
                    Token::String if expect_key => {
                        let key = tokens.text().to_string();
                        tokens.expect(Token::Colon)?;
                        let first = tokens.next_token()?;
                        let value = parse_value(tokens, first)?;
                        members.push((key, value));
                        expect_key = false;
                    }
                    other => return Err(unexpected(tokens, other)),
                }
            }
        }
        other => Err(unexpected(tokens, other)),
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

    #[test]
    fn parses_nested_documents() {
        let value = parse(&br#"{"a": [1, 2.5e3, "x", null, true], "b": {"c": -7}}"#[..]).unwrap();
        assert_eq!(value.get("a").unwrap().as_array().unwrap().len(), 5);
        assert_eq!(
            value.get("a").unwrap().as_array().unwrap()[1],
            JsonValue::Number("2.5e3".to_string())
        );
        assert_eq!(
            value.get("b").unwrap().get("c"),
            Some(&JsonValue::Number("-7".to_string()))
        );
    }

    #[test]
    fn rejects_malformed_documents() {
        assert!(parse(&b"[1,]"[..]).is_err());
        assert!(parse(&b"{\"a\" 1}"[..]).is_err());
        assert!(parse(&b"{} x"[..]).is_err());
    }
}
