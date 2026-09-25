//! XML into the value hub under the mapping most tools share (xmltodict,
//! quick-xml's serde): an element is a table, its attributes are `@name`
//! members, its text is `#text` (or the element's whole value when it has
//! nothing else), repeated child elements become an array, and every
//! value is a string. Comments, processing instructions, the doctype, and
//! the order of text against children in mixed content are dropped and
//! noted once each. The tokenizer is strict: what a browser or `xmllint`
//! rejects, this rejects, with the line and column.

use crate::converter::Location;
use crate::value::{MemberIndex, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlError {
    pub location: Location,
    pub message: String,
}

/// The document as `{root: node}` and the notes on what was dropped.
#[derive(Debug)]
pub struct Parsed {
    pub document: Value,
    pub notes: Vec<String>,
}

pub fn parse(text: &str) -> Result<Parsed, XmlError> {
    let mut parser = Parser::new(text);
    let document = parser.document()?;
    Ok(Parsed {
        document,
        notes: parser.notes,
    })
}

struct Parser<'a> {
    text: &'a str,
    bytes: &'a [u8],
    pos: usize,
    line: u64,
    line_start: usize,
    notes: Vec<String>,
    path: String,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Parser<'a> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        Parser {
            text,
            bytes: text.as_bytes(),
            pos: 0,
            line: 1,
            line_start: 0,
            notes: Vec::new(),
            path: String::new(),
        }
    }

    fn error<T>(&self, message: &'static str) -> Result<T, XmlError> {
        Err(XmlError {
            location: Location {
                line: self.line,
                column: (self.pos - self.line_start) as u64 + 1,
            },
            message: message.to_string(),
        })
    }

    fn note(&mut self, note: String) {
        if !self.notes.contains(&note) {
            self.notes.push(note);
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn starts_with(&self, pattern: &[u8]) -> bool {
        self.bytes[self.pos..].starts_with(pattern)
    }

    /// Moves one byte, counting lines.
    fn bump(&mut self) {
        if self.peek() == Some(b'\n') {
            self.line += 1;
            self.line_start = self.pos + 1;
        }
        self.pos += 1;
    }

    fn skip_ws(&mut self) {
        while matches!(
            self.peek(),
            Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r')
        ) {
            self.bump();
        }
    }

    /// Moves past `terminator`, or fails with `message` at the end.
    fn skip_past(&mut self, terminator: &[u8], message: &'static str) -> Result<(), XmlError> {
        while self.pos < self.bytes.len() {
            if self.starts_with(terminator) {
                self.pos += terminator.len();
                return Ok(());
            }
            self.bump();
        }
        self.error(message)
    }

    // ---- document ----

    fn document(&mut self) -> Result<Value, XmlError> {
        self.skip_ws();
        if self.starts_with(b"<?xml") {
            self.skip_past(b"?>", "the XML declaration never closes")?;
        }
        self.misc()?;
        if self.starts_with(b"<!DOCTYPE") {
            self.doctype()?;
            self.note("the doctype is dropped".to_string());
            self.misc()?;
        }
        if self.peek() != Some(b'<') {
            return self.error("expected the root element");
        }
        let (name, node) = self.element()?;
        self.misc()?;
        if self.pos < self.bytes.len() {
            return self.error("content after the root element");
        }
        Ok(Value::Table(vec![(name, node)]))
    }

    /// Whitespace, comments, and processing instructions between things.
    fn misc(&mut self) -> Result<(), XmlError> {
        loop {
            self.skip_ws();
            if self.starts_with(b"<!--") {
                self.comment()?;
            } else if self.starts_with(b"<?") {
                self.instruction()?;
            } else {
                return Ok(());
            }
        }
    }

    fn comment(&mut self) -> Result<(), XmlError> {
        self.pos += 4;
        loop {
            if self.starts_with(b"-->") {
                self.pos += 3;
                self.note("comments are dropped".to_string());
                return Ok(());
            }
            if self.starts_with(b"--") {
                return self.error("'--' inside a comment");
            }
            if self.pos >= self.bytes.len() {
                return self.error("comment never closes");
            }
            self.bump();
        }
    }

    fn instruction(&mut self) -> Result<(), XmlError> {
        self.pos += 2;
        self.skip_past(b"?>", "processing instruction never closes")?;
        self.note("processing instructions are dropped".to_string());
        Ok(())
    }

    /// `<!DOCTYPE ...>` with an optional internal subset in brackets.
    fn doctype(&mut self) -> Result<(), XmlError> {
        self.pos += 9;
        let mut depth = 0;
        loop {
            match self.peek() {
                None => return self.error("doctype never closes"),
                Some(b'[') => depth += 1,
                Some(b']') => depth -= 1,
                Some(b'"') | Some(b'\'') => {
                    let quote = self.peek();
                    self.bump();
                    while self.peek() != quote {
                        if self.peek().is_none() {
                            return self.error("doctype never closes");
                        }
                        self.bump();
                    }
                }
                Some(b'>') if depth == 0 => {
                    self.pos += 1;
                    return Ok(());
                }
                _ => {}
            }
            self.bump();
        }
    }

    // ---- elements ----

    /// At `<`: the element and its name.
    fn element(&mut self) -> Result<(String, Value), XmlError> {
        self.pos += 1;
        let name = self.name()?;
        let path_length = self.path.len();
        if !self.path.is_empty() {
            self.path.push('.');
        }
        self.path.push_str(&name);
        let mut attributes: Vec<(String, Value)> = Vec::new();
        let mut self_closing = false;
        loop {
            let had_space = matches!(
                self.peek(),
                Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r')
            );
            self.skip_ws();
            match self.peek() {
                Some(b'/') => {
                    self.pos += 1;
                    if self.peek() != Some(b'>') {
                        return self.error("expected '>' after '/'");
                    }
                    self.pos += 1;
                    self_closing = true;
                    break;
                }
                Some(b'>') => {
                    self.pos += 1;
                    break;
                }
                None => return self.error("tag never closes"),
                _ => {
                    if !had_space {
                        return self.error("expected whitespace before the attribute");
                    }
                    let attribute = self.name()?;
                    self.skip_ws();
                    if self.peek() != Some(b'=') {
                        return self.error("expected '=' after the attribute name");
                    }
                    self.pos += 1;
                    self.skip_ws();
                    let value = self.attribute_value()?;
                    let key = format!("@{attribute}");
                    if attributes.iter().any(|(existing, _)| *existing == key) {
                        return self.error("attribute given twice");
                    }
                    attributes.push((key, Value::String(value)));
                }
            }
        }
        let mut node = Node {
            members: attributes,
            index: MemberIndex::default(),
            text: String::new(),
            pending: String::new(),
            has_children: false,
        };
        if !self_closing {
            self.content(&name, &mut node)?;
        }
        let value = self.finish_node(node);
        self.path.truncate(path_length);
        Ok((name, value))
    }

    fn content(&mut self, name: &str, node: &mut Node) -> Result<(), XmlError> {
        loop {
            match self.peek() {
                None => return self.error("element never closes"),
                Some(b'<') => {
                    if self.starts_with(b"</") {
                        self.pos += 2;
                        let end_name = self.name()?;
                        if end_name != name {
                            return self.error("end tag does not match the open element");
                        }
                        self.skip_ws();
                        if self.peek() != Some(b'>') {
                            return self.error("expected '>' to close the end tag");
                        }
                        self.pos += 1;
                        return Ok(());
                    }
                    if self.starts_with(b"<!--") {
                        self.comment()?;
                    } else if self.starts_with(b"<![CDATA[") {
                        self.pos += 9;
                        let start = self.pos;
                        loop {
                            if self.starts_with(b"]]>") {
                                break;
                            }
                            if self.pos >= self.bytes.len() {
                                return self.error("CDATA section never closes");
                            }
                            self.bump();
                        }
                        let section = &self.text[start..self.pos];
                        self.pos += 3;
                        node.push_text(section);
                    } else if self.starts_with(b"<?") {
                        self.instruction()?;
                    } else {
                        let (child_name, child) = self.element()?;
                        node.add_child(child_name, child);
                    }
                }
                Some(b'&') => {
                    let mut decoded = String::new();
                    self.reference(&mut decoded)?;
                    node.push_text(&decoded);
                }
                Some(_) => {
                    let start = self.pos;
                    while let Some(byte) = self.peek() {
                        if byte == b'<' || byte == b'&' {
                            break;
                        }
                        if byte == b']' && self.starts_with(b"]]>") {
                            return self.error("']]>' in text");
                        }
                        self.bump();
                    }
                    let run = &self.text[start..self.pos];
                    node.push_text(run);
                }
            }
        }
    }

    fn finish_node(&mut self, mut node: Node) -> Value {
        node.flush_pending();
        let text = node.text.as_str();
        let mixed = node.has_children && !text.is_empty();
        if mixed {
            let note = format!(
                "{}: mixed content, the order of text against elements is lost",
                self.path
            );
            self.note(note);
        }
        let mut members = node.members;
        if members.is_empty() {
            return if text.is_empty() {
                Value::Null
            } else {
                Value::String(normalize_newlines(text))
            };
        }
        if !text.is_empty() {
            members.push(("#text".to_string(), Value::String(normalize_newlines(text))));
        }
        Value::Table(members)
    }

    fn name(&mut self) -> Result<String, XmlError> {
        let start = self.pos;
        match self.peek() {
            Some(byte) if is_name_start(byte) => self.pos += 1,
            _ => return self.error("expected a name"),
        }
        while self.peek().is_some_and(is_name_byte) {
            self.pos += 1;
        }
        Ok(self.text[start..self.pos].to_string())
    }

    fn attribute_value(&mut self) -> Result<String, XmlError> {
        let quote = match self.peek() {
            Some(byte @ (b'"' | b'\'')) => byte,
            _ => return self.error("attribute value must be quoted"),
        };
        self.pos += 1;
        let mut out = String::new();
        loop {
            let start = self.pos;
            while let Some(byte) = self.peek() {
                if byte == quote || byte == b'&' || byte == b'<' {
                    break;
                }
                self.bump();
            }
            out.push_str(&self.text[start..self.pos]);
            match self.peek() {
                Some(byte) if byte == quote => {
                    self.pos += 1;
                    return Ok(normalize_newlines(&out));
                }
                Some(b'&') => self.reference(&mut out)?,
                Some(_) => return self.error("'<' in an attribute value"),
                None => return self.error("attribute value never closes"),
            }
        }
    }

    /// At `&`: a predefined or numeric character reference.
    fn reference(&mut self, out: &mut String) -> Result<(), XmlError> {
        self.pos += 1;
        let start = self.pos;
        while let Some(byte) = self.peek() {
            if byte == b';' {
                break;
            }
            if !(byte.is_ascii_alphanumeric() || byte == b'#') || self.pos - start > 10 {
                return self.error("malformed entity reference");
            }
            self.pos += 1;
        }
        if self.peek() != Some(b';') {
            return self.error("entity reference never closes");
        }
        let entity = &self.text[start..self.pos];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix('#')
                .and_then(|number| match number.strip_prefix('x') {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => number.parse::<u32>().ok(),
                })
                .and_then(char::from_u32),
        };
        match decoded {
            Some(character) => {
                out.push(character);
                self.pos += 1;
                Ok(())
            }
            None => self.error("undefined entity"),
        }
    }
}

/// An element being read: attributes first, then children grouped by
/// name. Text accumulates raw in `pending` until a child element or the
/// end tag; each stretch is then trimmed and joined to the rest with one
/// space, so entity references never split a stretch.
struct Node {
    members: Vec<(String, Value)>,
    index: MemberIndex,
    text: String,
    pending: String,
    has_children: bool,
}

impl Node {
    fn push_text(&mut self, run: &str) {
        self.pending.push_str(run);
    }

    fn flush_pending(&mut self) {
        let trimmed = self.pending.trim();
        if !trimmed.is_empty() {
            if !self.text.is_empty() {
                self.text.push(' ');
            }
            self.text.push_str(trimmed);
        }
        self.pending.clear();
    }

    fn add_child(&mut self, name: String, child: Value) {
        self.flush_pending();
        self.has_children = true;
        match self.index.find(&self.members, &name) {
            Some(position) => {
                let slot = &mut self.members[position].1;
                match slot {
                    Value::Array(items) => items.push(child),
                    _ => {
                        let first = std::mem::replace(slot, Value::Null);
                        *slot = Value::Array(vec![first, child]);
                    }
                }
            }
            None => {
                let position = self.members.len();
                self.members.push((name, child));
                let key = &self.members[position].0;
                self.index.record(&self.members, key, position);
            }
        }
    }
}

fn normalize_newlines(text: &str) -> String {
    if !text.contains('\r') {
        return text.to_string();
    }
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// ASCII letters, `_`, `:`, and every non-ASCII byte start a name.
pub fn is_name_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte == b':' || byte >= 0x80
}

pub fn is_name_byte(byte: u8) -> bool {
    is_name_start(byte) || byte.is_ascii_digit() || byte == b'-' || byte == b'.'
}

/// True when `name` is an XML element or attribute name.
pub fn is_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    match bytes.next() {
        Some(first) if is_name_start(first) => bytes.all(is_name_byte),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_children_become_an_array_at_the_first_position() {
        let parsed = parse("<r><a>1</a><b>2</b><a>3</a></r>").unwrap();
        assert_eq!(
            parsed.document,
            Value::Table(vec![(
                "r".to_string(),
                Value::Table(vec![
                    (
                        "a".to_string(),
                        Value::Array(vec![
                            Value::String("1".to_string()),
                            Value::String("3".to_string())
                        ])
                    ),
                    ("b".to_string(), Value::String("2".to_string())),
                ])
            )])
        );
    }

    #[test]
    fn errors_carry_the_line() {
        let error = parse("<r>\n  <a>\n</r>").unwrap_err();
        assert_eq!(error.location.line, 3);
    }

    #[test]
    fn a_big_element_uses_the_index_without_changing_results() {
        let mut text = String::from("<r>");
        for index in 0..40 {
            text.push_str(&format!("<k{index}>{index}</k{index}>"));
        }
        text.push_str("<k5>again</k5></r>");
        let parsed = parse(&text).unwrap();
        let Value::Table(root) = parsed.document else {
            panic!()
        };
        let Value::Table(members) = &root[0].1 else {
            panic!()
        };
        assert_eq!(members.len(), 40);
        assert!(matches!(&members[5].1, Value::Array(items) if items.len() == 2));
    }
}
