//! TOML 1.0 reader. One forward pass over the text builds an arena of
//! tables, enforcing the specification's definition rules as tables are
//! created (a table defined twice, an inline table extended, a static
//! array appended to); the arena is then turned into a `Value` tree.
//!
//! A table's members are found by a linear scan while it is small and by
//! a lazily built index once it grows, so a document of many small tables
//! and a document of one huge table both stay linear.

use std::collections::HashMap;

use crate::converter::Location;
use crate::value::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TomlError {
    pub location: Location,
    pub message: String,
}

/// Parses one TOML document into a `Value::Table`.
pub fn parse(text: &str) -> Result<Value, TomlError> {
    let mut parser = Parser::new(text);
    parser.document()?;
    Ok(parser.finish())
}

/// Members beyond this count get a hash index.
const INDEX_THRESHOLD: usize = 16;

/// How a table came to exist; the rules for extending it depend on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// `[a.b]`: closed to another header, open to sub-tables.
    Header,
    /// Created on the way to a deeper header; a later header may claim it.
    Implicit,
    /// Created by dotted keys in a body; a header may not claim it.
    Dotted,
    /// `{ ... }`: closed to everything after its closing brace.
    Inline,
}

enum Slot {
    Value(Value),
    Table(u32),
    StaticArray(Vec<Slot>),
    TableArray(Vec<u32>),
}

struct Node {
    members: Vec<(String, Slot)>,
    index: Option<HashMap<String, usize>>,
    kind: Kind,
}

impl Node {
    fn new(kind: Kind) -> Node {
        Node {
            members: Vec::new(),
            index: None,
            kind,
        }
    }

    fn find(&self, key: &str) -> Option<usize> {
        match &self.index {
            Some(index) => index.get(key).copied(),
            None => self
                .members
                .iter()
                .position(|(existing, _)| existing == key),
        }
    }

    fn push(&mut self, key: String, slot: Slot) -> usize {
        let position = self.members.len();
        if let Some(index) = &mut self.index {
            index.insert(key.clone(), position);
        } else if position >= INDEX_THRESHOLD {
            let mut index: HashMap<String, usize> = HashMap::with_capacity(position * 2);
            for (existing_position, (existing, _)) in self.members.iter().enumerate() {
                index.insert(existing.clone(), existing_position);
            }
            index.insert(key.clone(), position);
            self.index = Some(index);
        }
        self.members.push((key, slot));
        position
    }
}

struct Parser<'a> {
    text: &'a str,
    bytes: &'a [u8],
    pos: usize,
    line: u64,
    line_start: usize,
    nodes: Vec<Node>,
    current: u32,
}

const ROOT: u32 = 0;

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Parser<'a> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        Parser {
            text,
            bytes: text.as_bytes(),
            pos: 0,
            line: 1,
            line_start: 0,
            nodes: vec![Node::new(Kind::Header)],
            current: ROOT,
        }
    }

    fn finish(mut self) -> Value {
        let members = std::mem::take(&mut self.nodes[ROOT as usize].members);
        Value::Table(self.convert_members(members))
    }

    fn convert_members(&mut self, members: Vec<(String, Slot)>) -> Vec<(String, Value)> {
        let mut converted = Vec::with_capacity(members.len());
        for (key, slot) in members {
            converted.push((key, self.convert_slot(slot)));
        }
        converted
    }

    #[inline(never)]
    fn convert_slot(&mut self, slot: Slot) -> Value {
        match slot {
            Slot::Value(value) => value,
            Slot::Table(id) => self.convert_table(id),
            Slot::StaticArray(items) => {
                let mut values = Vec::with_capacity(items.len());
                for item in items {
                    values.push(self.convert_slot(item));
                }
                Value::Array(values)
            }
            Slot::TableArray(ids) => {
                let mut values = Vec::with_capacity(ids.len());
                for id in ids {
                    values.push(self.convert_table(id));
                }
                Value::Array(values)
            }
        }
    }

    fn convert_table(&mut self, id: u32) -> Value {
        let members = std::mem::take(&mut self.nodes[id as usize].members);
        Value::Table(self.convert_members(members))
    }

    // ---- errors and position ----

    fn error<T>(&self, message: impl Into<String>) -> Result<T, TomlError> {
        Err(self.error_at(self.pos, message))
    }

    fn error_at(&self, pos: usize, message: impl Into<String>) -> TomlError {
        let column = pos.saturating_sub(self.line_start) as u64 + 1;
        TomlError {
            location: Location {
                line: self.line,
                column,
            },
            message: message.into(),
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.pos + offset).copied()
    }

    fn starts_with(&self, pattern: &[u8]) -> bool {
        self.bytes[self.pos..].starts_with(pattern)
    }

    fn skip_ws(&mut self) {
        while let Some(byte) = self.peek() {
            if byte == b' ' || byte == b'\t' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    /// Consumes one newline (`\n` or `\r\n`).
    fn newline(&mut self) -> Result<(), TomlError> {
        match self.peek() {
            Some(b'\n') => {
                self.pos += 1;
            }
            Some(b'\r') if self.peek_at(1) == Some(b'\n') => {
                self.pos += 2;
            }
            _ => return self.error("expected a newline"),
        }
        self.line += 1;
        self.line_start = self.pos;
        Ok(())
    }

    fn at_newline(&self) -> bool {
        matches!(self.peek(), Some(b'\n') | Some(b'\r'))
    }

    fn skip_comment(&mut self) -> Result<(), TomlError> {
        if self.peek() != Some(b'#') {
            return Ok(());
        }
        while let Some(byte) = self.peek() {
            if byte == b'\n' || byte == b'\r' {
                break;
            }
            if is_control(byte) {
                return self.error("control character in a comment");
            }
            self.pos += 1;
        }
        Ok(())
    }

    /// After a statement: optional whitespace and comment, then a newline
    /// or the end of the document.
    fn end_of_line(&mut self) -> Result<(), TomlError> {
        self.skip_ws();
        self.skip_comment()?;
        if self.peek().is_none() {
            return Ok(());
        }
        if self.at_newline() {
            return self.newline();
        }
        self.error("expected a newline or a comment after the value")
    }

    /// Whitespace, newlines, and comments, in any mix (inside arrays).
    fn skip_blank(&mut self) -> Result<(), TomlError> {
        loop {
            self.skip_ws();
            self.skip_comment()?;
            if self.at_newline() {
                self.newline()?;
            } else {
                return Ok(());
            }
        }
    }

    // ---- document ----

    #[inline(never)]
    fn document(&mut self) -> Result<(), TomlError> {
        loop {
            self.skip_ws();
            match self.peek() {
                None => return Ok(()),
                Some(b'\n') | Some(b'\r') => self.newline()?,
                Some(b'#') => {
                    self.skip_comment()?;
                    if self.peek().is_some() {
                        self.newline()?;
                    }
                }
                Some(b'[') => self.header()?,
                Some(_) => {
                    let current = self.current;
                    self.key_value(current)?;
                    self.end_of_line()?;
                }
            }
        }
    }

    #[inline(never)]
    fn header(&mut self) -> Result<(), TomlError> {
        let start = self.pos;
        self.pos += 1;
        let is_array = self.peek() == Some(b'[');
        if is_array {
            self.pos += 1;
        }
        self.skip_ws();
        let path = self.key_path()?;
        self.skip_ws();
        let closing: &[u8] = if is_array { b"]]" } else { b"]" };
        if !self.starts_with(closing) {
            return self.error("expected the header to close");
        }
        self.pos += closing.len();
        let node = if is_array {
            self.open_table_array(&path, start)?
        } else {
            self.open_table(&path, start)?
        };
        self.current = node;
        self.end_of_line()
    }

    /// `[a.b.c]`: walks or creates the path from the root; the last
    /// segment must be new or implicit.
    #[inline(never)]
    fn open_table(&mut self, path: &[String], start: usize) -> Result<u32, TomlError> {
        let (last, parents) = split_last(path);
        let parent = self.walk_header_parents(parents, start)?;
        let existing = self.nodes[parent as usize].find(last);
        match existing {
            None => {
                let id = self.new_node(Kind::Header);
                self.nodes[parent as usize].push(last.clone(), Slot::Table(id));
                Ok(id)
            }
            Some(position) => match &self.nodes[parent as usize].members[position].1 {
                Slot::Table(id) => {
                    let id = *id;
                    if self.nodes[id as usize].kind == Kind::Implicit {
                        self.nodes[id as usize].kind = Kind::Header;
                        Ok(id)
                    } else {
                        Err(self.error_at(start, "table defined twice"))
                    }
                }
                Slot::TableArray(_) => {
                    Err(self.error_at(start, "already an array of tables, not a table"))
                }
                _ => Err(self.error_at(start, "already a value, not a table")),
            },
        }
    }

    /// `[[a.b]]`: appends a table to the array at the path, creating it.
    #[inline(never)]
    fn open_table_array(&mut self, path: &[String], start: usize) -> Result<u32, TomlError> {
        let (last, parents) = split_last(path);
        let parent = self.walk_header_parents(parents, start)?;
        let id = self.new_node(Kind::Header);
        let existing = self.nodes[parent as usize].find(last);
        match existing {
            None => {
                self.nodes[parent as usize].push(last.clone(), Slot::TableArray(vec![id]));
                Ok(id)
            }
            Some(position) => match &mut self.nodes[parent as usize].members[position].1 {
                Slot::TableArray(ids) => {
                    ids.push(id);
                    Ok(id)
                }
                Slot::StaticArray(_) => {
                    Err(self.error_at(start, "static array cannot be appended to"))
                }
                _ => Err(self.error_at(start, "already defined and not an array of tables")),
            },
        }
    }

    /// Every segment but the last of a header path: a table of any kind
    /// but inline, or the last table of an array of tables.
    #[inline(never)]
    fn walk_header_parents(&mut self, parents: &[String], start: usize) -> Result<u32, TomlError> {
        let mut node = ROOT;
        for segment in parents {
            let existing = self.nodes[node as usize].find(segment);
            node = match existing {
                None => {
                    let id = self.new_node(Kind::Implicit);
                    self.nodes[node as usize].push(segment.clone(), Slot::Table(id));
                    id
                }
                Some(position) => match &self.nodes[node as usize].members[position].1 {
                    Slot::Table(id) => {
                        let id = *id;
                        if self.nodes[id as usize].kind == Kind::Inline {
                            return Err(self.error_at(start, "inline table cannot be extended"));
                        }
                        id
                    }
                    Slot::TableArray(ids) => match ids.last() {
                        Some(last) => *last,
                        None => return Err(self.error_at(start, "empty array of tables")),
                    },
                    _ => {
                        return Err(self.error_at(start, "path crosses a value, not a table"));
                    }
                },
            };
        }
        Ok(node)
    }

    fn new_node(&mut self, kind: Kind) -> u32 {
        let id = self.nodes.len() as u32;
        self.nodes.push(Node::new(kind));
        id
    }

    // ---- key/value pairs ----

    /// `a.b.c = value` into `table`; dotted segments create or enter
    /// tables that dotted keys are allowed to touch.
    #[inline(never)]
    fn key_value(&mut self, table: u32) -> Result<(), TomlError> {
        let start = self.pos;
        let path = self.key_path()?;
        self.skip_ws();
        if self.peek() != Some(b'=') {
            return self.error("expected '=' after the key");
        }
        self.pos += 1;
        self.skip_ws();
        let slot = self.value()?;
        let (last, parents) = split_last(&path);
        let mut node = table;
        let inside_inline = self.nodes[table as usize].kind == Kind::Inline;
        for segment in parents {
            let existing = self.nodes[node as usize].find(segment);
            node = match existing {
                None => {
                    let kind = if inside_inline {
                        Kind::Inline
                    } else {
                        Kind::Dotted
                    };
                    let id = self.new_node(kind);
                    self.nodes[node as usize].push(segment.clone(), Slot::Table(id));
                    id
                }
                Some(position) => match &self.nodes[node as usize].members[position].1 {
                    Slot::Table(id) => {
                        let id = *id;
                        let kind = self.nodes[id as usize].kind;
                        let open_to_dotted = kind == Kind::Dotted
                            || kind == Kind::Implicit
                            || (inside_inline && kind == Kind::Inline);
                        if !open_to_dotted {
                            return Err(self.error_at(
                                start,
                                "dotted key cannot extend a table defined elsewhere",
                            ));
                        }
                        id
                    }
                    _ => {
                        return Err(self.error_at(start, "dotted key crosses a value, not a table"));
                    }
                },
            };
        }
        if self.nodes[node as usize].find(last).is_some() {
            return Err(self.error_at(start, "key defined twice"));
        }
        self.nodes[node as usize].push(last.clone(), slot);
        Ok(())
    }

    fn key_path(&mut self) -> Result<Vec<String>, TomlError> {
        let mut path = vec![self.simple_key()?];
        loop {
            self.skip_ws();
            if self.peek() != Some(b'.') {
                return Ok(path);
            }
            self.pos += 1;
            self.skip_ws();
            path.push(self.simple_key()?);
        }
    }

    fn simple_key(&mut self) -> Result<String, TomlError> {
        match self.peek() {
            Some(b'"') => {
                self.pos += 1;
                self.basic_string()
            }
            Some(b'\'') => {
                self.pos += 1;
                Ok(self.literal_string()?.to_string())
            }
            Some(byte) if is_bare_key_byte(byte) => {
                let start = self.pos;
                while self.peek().is_some_and(is_bare_key_byte) {
                    self.pos += 1;
                }
                Ok(self.text[start..self.pos].to_string())
            }
            _ => self.error("expected a key"),
        }
    }

    // ---- values ----

    fn value(&mut self) -> Result<Slot, TomlError> {
        match self.peek() {
            Some(b'"') => {
                if self.starts_with(b"\"\"\"") {
                    self.pos += 3;
                    Ok(Slot::Value(Value::String(self.multiline_basic_string()?)))
                } else {
                    self.pos += 1;
                    Ok(Slot::Value(Value::String(self.basic_string()?)))
                }
            }
            Some(b'\'') => {
                if self.starts_with(b"'''") {
                    self.pos += 3;
                    Ok(Slot::Value(Value::String(self.multiline_literal_string()?)))
                } else {
                    self.pos += 1;
                    Ok(Slot::Value(Value::String(
                        self.literal_string()?.to_string(),
                    )))
                }
            }
            Some(b'[') => self.array(),
            Some(b'{') => self.inline_table(),
            Some(b't') if self.starts_with(b"true") => {
                self.pos += 4;
                Ok(Slot::Value(Value::Bool(true)))
            }
            Some(b'f') if self.starts_with(b"false") => {
                self.pos += 5;
                Ok(Slot::Value(Value::Bool(false)))
            }
            Some(byte) if byte.is_ascii_digit() || matches!(byte, b'+' | b'-' | b'i' | b'n') => {
                Ok(Slot::Value(self.number_or_datetime()?))
            }
            Some(_) => self.error("expected a value"),
            None => self.error("expected a value, found the end of the document"),
        }
    }

    #[inline(never)]
    fn array(&mut self) -> Result<Slot, TomlError> {
        self.pos += 1;
        let mut items = Vec::new();
        loop {
            self.skip_blank()?;
            match self.peek() {
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Slot::StaticArray(items));
                }
                None => return self.error("array never closed"),
                _ => {}
            }
            items.push(self.value()?);
            self.skip_blank()?;
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Slot::StaticArray(items));
                }
                _ => return self.error("expected ',' or ']' in the array"),
            }
        }
    }

    #[inline(never)]
    fn inline_table(&mut self) -> Result<Slot, TomlError> {
        self.pos += 1;
        let id = self.new_node(Kind::Inline);
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Slot::Table(id));
        }
        loop {
            self.skip_ws();
            self.key_value(id)?;
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Slot::Table(id));
                }
                _ => return self.error("expected ',' or '}' in the inline table"),
            }
        }
    }

    // ---- strings ----

    /// After the opening quote. Escapes decoded, no newlines.
    fn basic_string(&mut self) -> Result<String, TomlError> {
        let mut out = String::new();
        loop {
            let run_start = self.pos;
            while let Some(byte) = self.peek() {
                if byte == b'"' || byte == b'\\' || is_control(byte) {
                    break;
                }
                self.pos += 1;
            }
            out.push_str(&self.text[run_start..self.pos]);
            match self.peek() {
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    self.escape(&mut out)?;
                }
                Some(_) => return self.error("control character in a string"),
                None => return self.error("string never closed"),
            }
        }
    }

    /// After the opening `"""`.
    #[inline(never)]
    fn multiline_basic_string(&mut self) -> Result<String, TomlError> {
        self.skip_first_newline()?;
        let mut out = String::new();
        loop {
            let run_start = self.pos;
            while let Some(byte) = self.peek() {
                if byte == b'\n' {
                    self.pos += 1;
                    self.line += 1;
                    self.line_start = self.pos;
                    continue;
                }
                if byte == b'"' || byte == b'\\' || byte == b'\r' || is_control(byte) {
                    break;
                }
                self.pos += 1;
            }
            out.push_str(&self.text[run_start..self.pos]);
            match self.peek() {
                Some(b'"') => {
                    let quotes = self.count_run(b'"');
                    if quotes < 3 {
                        self.pos += quotes;
                        out.push_str(&"\"".repeat(quotes));
                        continue;
                    }
                    if quotes > 5 {
                        return self.error("too many quotes at the end of the string");
                    }
                    self.pos += quotes;
                    out.push_str(&"\"".repeat(quotes - 3));
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    if self.is_line_ending_backslash() {
                        self.skip_ws();
                        if !self.at_newline() {
                            return self
                                .error("expected a newline after the line-ending backslash");
                        }
                        loop {
                            self.skip_ws();
                            if self.at_newline() {
                                self.newline()?;
                            } else {
                                break;
                            }
                        }
                    } else {
                        self.escape(&mut out)?;
                    }
                }
                Some(b'\r') => {
                    if self.peek_at(1) == Some(b'\n') {
                        out.push_str("\r\n");
                        self.pos += 2;
                        self.line += 1;
                        self.line_start = self.pos;
                    } else {
                        return self.error("bare carriage return in a string");
                    }
                }
                Some(_) => return self.error("control character in a string"),
                None => return self.error("string never closed"),
            }
        }
    }

    fn is_line_ending_backslash(&self) -> bool {
        let mut offset = 0;
        while let Some(byte) = self.peek_at(offset) {
            match byte {
                b' ' | b'\t' => offset += 1,
                b'\n' | b'\r' => return true,
                _ => return false,
            }
        }
        false
    }

    /// After the opening `'`. Verbatim, no newlines.
    fn literal_string(&mut self) -> Result<&'a str, TomlError> {
        let start = self.pos;
        while let Some(byte) = self.peek() {
            if byte == b'\'' {
                let text = &self.text[start..self.pos];
                self.pos += 1;
                return Ok(text);
            }
            if is_control(byte) {
                return self.error("control character in a literal string");
            }
            self.pos += 1;
        }
        self.error("literal string never closed")
    }

    /// After the opening `'''`.
    #[inline(never)]
    fn multiline_literal_string(&mut self) -> Result<String, TomlError> {
        self.skip_first_newline()?;
        let start = self.pos;
        loop {
            match self.peek() {
                Some(b'\'') => {
                    let quotes = self.count_run(b'\'');
                    if quotes < 3 {
                        self.pos += quotes;
                        continue;
                    }
                    if quotes > 5 {
                        return self.error("too many quotes at the end of the string");
                    }
                    let text = &self.text[start..self.pos + quotes - 3];
                    self.pos += quotes;
                    return Ok(text.to_string());
                }
                Some(b'\n') => {
                    self.pos += 1;
                    self.line += 1;
                    self.line_start = self.pos;
                }
                Some(b'\r') => {
                    if self.peek_at(1) != Some(b'\n') {
                        return self.error("bare carriage return in a string");
                    }
                    self.pos += 2;
                    self.line += 1;
                    self.line_start = self.pos;
                }
                Some(byte) if is_control(byte) => {
                    return self.error("control character in a literal string");
                }
                Some(_) => self.pos += 1,
                None => return self.error("literal string never closed"),
            }
        }
    }

    fn skip_first_newline(&mut self) -> Result<(), TomlError> {
        if self.at_newline() {
            self.newline()?;
        }
        Ok(())
    }

    fn count_run(&self, byte: u8) -> usize {
        let mut count = 0;
        while self.peek_at(count) == Some(byte) {
            count += 1;
        }
        count
    }

    /// After the backslash.
    #[inline(never)]
    fn escape(&mut self, out: &mut String) -> Result<(), TomlError> {
        let escape_start = self.pos - 1;
        let Some(byte) = self.peek() else {
            return self.error("string never closed");
        };
        self.pos += 1;
        let decoded = match byte {
            b'b' => '\u{8}',
            b't' => '\t',
            b'n' => '\n',
            b'f' => '\u{c}',
            b'r' => '\r',
            b'"' => '"',
            b'\\' => '\\',
            b'u' => self.unicode_escape(4, escape_start)?,
            b'U' => self.unicode_escape(8, escape_start)?,
            _ => return Err(self.error_at(escape_start, "invalid escape sequence")),
        };
        out.push(decoded);
        Ok(())
    }

    #[inline(never)]
    fn unicode_escape(&mut self, digits: usize, escape_start: usize) -> Result<char, TomlError> {
        let end = self.pos + digits;
        let Some(hex) = self.text.get(self.pos..end) else {
            return Err(self.error_at(escape_start, "unicode escape cut short"));
        };
        let code = u32::from_str_radix(hex, 16)
            .map_err(|_| self.error_at(escape_start, "unicode escape is not hexadecimal"))?;
        self.pos = end;
        char::from_u32(code)
            .ok_or_else(|| self.error_at(escape_start, "unicode escape is not a scalar value"))
    }

    // ---- numbers and datetimes ----

    #[inline(never)]
    fn number_or_datetime(&mut self) -> Result<Value, TomlError> {
        let start = self.pos;
        while self.peek().is_some_and(is_token_byte) {
            self.pos += 1;
        }
        let mut token = &self.text[start..self.pos];
        // `1979-05-27 07:32:00`: a space may separate the date and time.
        let is_date_then_time = looks_like_date(token.as_bytes())
            && self.peek() == Some(b' ')
            && self.peek_at(1).is_some_and(|byte| byte.is_ascii_digit());
        if is_date_then_time {
            self.pos += 1;
            while self.peek().is_some_and(is_token_byte) {
                self.pos += 1;
            }
            token = &self.text[start..self.pos];
        }
        if token.is_empty() {
            return self.error("expected a value");
        }
        let bytes = token.as_bytes();
        if looks_like_date(bytes) || looks_like_time(bytes) {
            return match datetime_is_valid(bytes) {
                true => Ok(Value::Datetime(token.to_string())),
                false => Err(self.error_at(start, "invalid date or time")),
            };
        }
        parse_number(token).map_err(|message| self.error_at(start, message))
    }
}

fn split_last(path: &[String]) -> (&String, &[String]) {
    match path.split_last() {
        Some((last, parents)) => (last, parents),
        None => unreachable_empty_path(path),
    }
}

/// `key_path` never returns an empty path; this keeps the split total.
fn unreachable_empty_path(path: &[String]) -> (&String, &[String]) {
    static EMPTY: String = String::new();
    (&EMPTY, path)
}

fn is_bare_key_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'_' | b'.' | b':')
}

fn is_control(byte: u8) -> bool {
    (byte < 0x20 && byte != b'\t') || byte == 0x7f
}

fn looks_like_date(bytes: &[u8]) -> bool {
    bytes.len() >= 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[..4].iter().all(u8::is_ascii_digit)
}

fn looks_like_time(bytes: &[u8]) -> bool {
    bytes.len() >= 8 && bytes[2] == b':' && bytes[..2].iter().all(u8::is_ascii_digit)
}

fn two_digits(bytes: &[u8], at: usize) -> Option<u32> {
    let tens = *bytes.get(at)?;
    let ones = *bytes.get(at + 1)?;
    if tens.is_ascii_digit() && ones.is_ascii_digit() {
        Some(u32::from(tens - b'0') * 10 + u32::from(ones - b'0'))
    } else {
        None
    }
}

/// `YYYY-MM-DD`, consuming 10 bytes.
fn date_is_valid(bytes: &[u8]) -> bool {
    if bytes.len() < 10 || !bytes[..4].iter().all(u8::is_ascii_digit) {
        return false;
    }
    let (Some(month), Some(day)) = (two_digits(bytes, 5), two_digits(bytes, 8)) else {
        return false;
    };
    bytes[4] == b'-' && bytes[7] == b'-' && (1..=12).contains(&month) && (1..=31).contains(&day)
}

/// `HH:MM:SS` with an optional fraction; returns the bytes consumed.
fn time_length(bytes: &[u8]) -> Option<usize> {
    let hour = two_digits(bytes, 0)?;
    let minute = two_digits(bytes, 3)?;
    let second = two_digits(bytes, 6)?;
    let separators_ok = bytes.get(2) == Some(&b':') && bytes.get(5) == Some(&b':');
    if !separators_ok || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let mut length = 8;
    if bytes.get(8) == Some(&b'.') {
        let digits = bytes[9..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        if digits == 0 {
            return None;
        }
        length = 9 + digits;
    }
    Some(length)
}

/// `Z`, `z`, or `+HH:MM`; returns the bytes consumed.
fn offset_length(bytes: &[u8]) -> Option<usize> {
    match bytes.first()? {
        b'Z' | b'z' => Some(1),
        b'+' | b'-' => {
            let hour = two_digits(bytes, 1)?;
            let minute = two_digits(bytes, 4)?;
            if bytes.get(3) == Some(&b':') && hour <= 23 && minute <= 59 {
                Some(6)
            } else {
                None
            }
        }
        _ => None,
    }
}

#[inline(never)]
fn datetime_is_valid(bytes: &[u8]) -> bool {
    if looks_like_time(bytes) {
        return time_length(bytes) == Some(bytes.len());
    }
    if !date_is_valid(bytes) {
        return false;
    }
    if bytes.len() == 10 {
        return true;
    }
    if !matches!(bytes[10], b'T' | b't' | b' ') {
        return false;
    }
    let rest = &bytes[11..];
    let Some(time) = time_length(rest) else {
        return false;
    };
    let after_time = &rest[time..];
    if after_time.is_empty() {
        return true;
    }
    offset_length(after_time) == Some(after_time.len())
}

#[inline(never)]
fn parse_number(token: &str) -> Result<Value, &'static str> {
    let (negative, unsigned) = match token.as_bytes().first() {
        Some(b'+') => (false, &token[1..]),
        Some(b'-') => (true, &token[1..]),
        _ => (false, token),
    };
    match unsigned {
        "inf" => {
            return Ok(Value::Float(if negative {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            }));
        }
        "nan" => return Ok(Value::Float(if negative { -f64::NAN } else { f64::NAN })),
        _ => {}
    }
    if unsigned.is_empty() {
        return Err("expected digits after the sign");
    }
    let radix = match unsigned.as_bytes() {
        [b'0', b'x', ..] => 16,
        [b'0', b'o', ..] => 8,
        [b'0', b'b', ..] => 2,
        _ => 10,
    };
    if radix != 10 {
        if unsigned.len() != token.len() {
            return Err("prefixed integers cannot carry a sign");
        }
        let digits = strip_underscores(&unsigned[2..], radix)?;
        let magnitude =
            i64::from_str_radix(&digits, radix).map_err(|_| "integer out of the 64-bit range")?;
        return Ok(Value::Integer(magnitude));
    }
    let is_float = unsigned.contains(['.', 'e', 'E']);
    if is_float {
        return parse_float(token, unsigned);
    }
    let digits = strip_underscores(unsigned, 10)?;
    if digits.len() > 1 && digits.starts_with('0') {
        return Err("leading zeros are not allowed");
    }
    let signed = if negative {
        format!("-{digits}")
    } else {
        digits
    };
    signed
        .parse::<i64>()
        .map(Value::Integer)
        .map_err(|_| "integer out of the 64-bit range")
}

#[inline(never)]
fn parse_float(token: &str, unsigned: &str) -> Result<Value, &'static str> {
    let (mantissa, exponent) = match unsigned.find(['e', 'E']) {
        Some(at) => (&unsigned[..at], Some(&unsigned[at + 1..])),
        None => (unsigned, None),
    };
    let (integer_part, fraction) = match mantissa.find('.') {
        Some(at) => (&mantissa[..at], Some(&mantissa[at + 1..])),
        None => (mantissa, None),
    };
    let integer_digits = strip_underscores(integer_part, 10)?;
    if integer_digits.len() > 1 && integer_digits.starts_with('0') {
        return Err("leading zeros are not allowed");
    }
    let mut normalized = String::with_capacity(token.len());
    if token.starts_with('-') {
        normalized.push('-');
    }
    normalized.push_str(&integer_digits);
    if let Some(fraction) = fraction {
        normalized.push('.');
        normalized.push_str(&strip_underscores(fraction, 10)?);
    }
    if let Some(exponent) = exponent {
        normalized.push('e');
        let (sign, digits) = match exponent.as_bytes().first() {
            Some(b'+') => ("", &exponent[1..]),
            Some(b'-') => ("-", &exponent[1..]),
            _ => ("", exponent),
        };
        normalized.push_str(sign);
        normalized.push_str(&strip_underscores(digits, 10)?);
    }
    normalized
        .parse::<f64>()
        .map(Value::Float)
        .map_err(|_| "invalid float")
}

/// Digits with underscores only between digits, or an error.
#[inline(never)]
fn strip_underscores(text: &str, radix: u32) -> Result<String, &'static str> {
    if text.is_empty() {
        return Err("expected digits");
    }
    let mut out = String::with_capacity(text.len());
    let mut previous_was_digit = false;
    let bytes = text.as_bytes();
    for (index, &byte) in bytes.iter().enumerate() {
        if byte == b'_' {
            let next_is_digit = bytes
                .get(index + 1)
                .is_some_and(|next| (*next as char).is_digit(radix));
            if !previous_was_digit || !next_is_digit {
                return Err("underscores must sit between digits");
            }
            previous_was_digit = false;
            continue;
        }
        if !(byte as char).is_digit(radix) {
            return Err("invalid character in a number");
        }
        out.push(byte as char);
        previous_was_digit = true;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(text: &str) -> Value {
        parse(text).unwrap()
    }

    fn member<'v>(value: &'v Value, key: &str) -> &'v Value {
        match value {
            Value::Table(members) => &members.iter().find(|(k, _)| k == key).unwrap().1,
            _ => panic!("not a table"),
        }
    }

    #[test]
    fn numbers_take_every_spelling() {
        let value = parse_ok("a = 0xff\nb = 1_000\nc = -2.5e3\nd = +0\ne = 0o17\nf = 0b101\n");
        assert_eq!(member(&value, "a"), &Value::Integer(255));
        assert_eq!(member(&value, "b"), &Value::Integer(1000));
        assert_eq!(member(&value, "c"), &Value::Float(-2500.0));
        assert_eq!(member(&value, "d"), &Value::Integer(0));
        assert_eq!(member(&value, "e"), &Value::Integer(15));
        assert_eq!(member(&value, "f"), &Value::Integer(5));
    }

    #[test]
    fn datetimes_are_validated_and_kept_as_written() {
        let value = parse_ok("a = 1979-05-27T07:32:00Z\nb = 07:32:00.5\nc = 1979-05-27 07:32:00\n");
        assert_eq!(
            member(&value, "a"),
            &Value::Datetime("1979-05-27T07:32:00Z".to_string())
        );
        assert_eq!(
            member(&value, "b"),
            &Value::Datetime("07:32:00.5".to_string())
        );
        assert_eq!(
            member(&value, "c"),
            &Value::Datetime("1979-05-27 07:32:00".to_string())
        );
        assert!(parse("a = 1979-13-01\n").is_err());
        assert!(parse("a = 25:00:00\n").is_err());
        assert!(parse("a = 1979-05-27T07:32\n").is_err());
    }

    #[test]
    fn errors_carry_the_line() {
        let error = parse("a = 1\nb = \n").unwrap_err();
        assert_eq!(error.location.line, 2);
    }

    #[test]
    fn a_big_table_uses_the_index_without_changing_results() {
        let mut text = String::new();
        for index in 0..40 {
            text.push_str(&format!("k{index} = {index}\n"));
        }
        let value = parse_ok(&text);
        assert_eq!(member(&value, "k39"), &Value::Integer(39));
        text.push_str("k5 = 0\n");
        assert!(parse(&text).is_err());
    }
}
