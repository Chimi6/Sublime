//! XML into the value hub under the mapping most tools share (xmltodict,
//! quick-xml's serde): an element is a table, its attributes are `@name`
//! members, its text is `#text` (or the element's whole value when it has
//! nothing else), repeated child elements become an array, and every
//! value is a string. Comments, processing instructions, the doctype, and
//! the order of text against children in mixed content are dropped and
//! noted once each. The tokenizer is strict: what a browser or `xmllint`
//! rejects, this rejects, with the line and column.
//!
//! The input is pulled through a sliding window (`CHUNK` bytes at a
//! time), never held whole: XML data files are large, and the tree is
//! the only thing worth the memory. Positions are absolute offsets into
//! the stream; the window keeps bytes from `mark` on, and every scan
//! that will slice its bytes sets `mark` first.

use std::io::Read;

use crate::io::scan::{find_any_of3, find_byte};

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

/// Parses a document from text (tests and small callers).
pub fn parse(text: &str) -> Result<Parsed, XmlError> {
    let mut bytes = text.as_bytes();
    parse_reader(&mut bytes)
}

/// Parses a document pulled from `source`.
pub fn parse_reader(source: &mut dyn Read) -> Result<Parsed, XmlError> {
    let mut parser = Parser::new(source);
    let document = parser.document();
    if let Some(failure) = parser.failure.take() {
        return Err(failure);
    }
    Ok(Parsed {
        document: document?,
        notes: parser.notes,
    })
}

const CHUNK: usize = 256 * 1024;

struct Parser<'a> {
    source: &'a mut dyn Read,
    /// The window: `buffer[0]` is stream offset `base`.
    buffer: Vec<u8>,
    base: usize,
    /// Absolute offset of the next byte.
    pos: usize,
    /// Bytes from here on stay in the window across refills.
    mark: usize,
    eof: bool,
    failure: Option<XmlError>,
    line: u64,
    line_start: usize,
    notes: Vec<String>,
    path: String,
}

impl<'a> Parser<'a> {
    fn new(source: &'a mut dyn Read) -> Parser<'a> {
        Parser {
            source,
            buffer: Vec::with_capacity(CHUNK * 2),
            base: 0,
            pos: 0,
            mark: 0,
            eof: false,
            failure: None,
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

    // ---- the window ----

    /// Reads another chunk, first dropping what lies before `mark`. A
    /// read failure is kept and the stream looks ended.
    fn fill(&mut self) {
        if self.eof {
            return;
        }
        let drop = self.mark.saturating_sub(self.base).min(self.buffer.len());
        if drop > 0 {
            self.buffer.drain(..drop);
            self.base += drop;
        }
        let old_len = self.buffer.len();
        self.buffer.resize(old_len + CHUNK, 0);
        loop {
            match self.source.read(&mut self.buffer[old_len..]) {
                Ok(0) => {
                    self.eof = true;
                    break;
                }
                Ok(count) => {
                    self.buffer.truncate(old_len + count);
                    return;
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    self.eof = true;
                    self.failure = Some(XmlError {
                        location: Location {
                            line: self.line,
                            column: (self.pos - self.line_start) as u64 + 1,
                        },
                        message: format!("read failed: {error}"),
                    });
                    break;
                }
            }
        }
        self.buffer.truncate(old_len);
    }

    /// Makes at least `count` bytes from `pos` available, or as many as
    /// the stream has; returns how many are.
    fn available(&mut self, count: usize) -> usize {
        loop {
            let have = (self.base + self.buffer.len()).saturating_sub(self.pos);
            if have >= count || self.eof {
                return have;
            }
            self.fill();
        }
    }

    /// Moves `pos` to the next `<`, `&`, or `]` (or the end), scanning the
    /// window a word at a time and counting the newlines passed in bulk.
    fn scan_to_text_stop(&mut self) {
        loop {
            let offset = self.pos - self.base;
            let window = &self.buffer[offset..];
            let found = find_any_of3(window, b'<', b'&', b']');
            let run = found.unwrap_or(window.len());
            self.count_newlines(offset, run);
            self.pos += run;
            if found.is_some() {
                return;
            }
            let before = self.buffer.len() + self.base;
            self.fill();
            if self.buffer.len() + self.base == before {
                return;
            }
        }
    }

    /// Like `scan_to_text_stop` for an attribute value: stops at the
    /// quote, `&`, or `<`.
    fn scan_to_attribute_stop(&mut self, quote: u8) {
        loop {
            let offset = self.pos - self.base;
            let window = &self.buffer[offset..];
            let found = find_any_of3(window, quote, b'&', b'<');
            let run = found.unwrap_or(window.len());
            self.count_newlines(offset, run);
            self.pos += run;
            if found.is_some() {
                return;
            }
            let before = self.buffer.len() + self.base;
            self.fill();
            if self.buffer.len() + self.base == before {
                return;
            }
        }
    }

    /// Updates the line counters for the run `buffer[offset..offset + run]`
    /// that `pos` is about to move past.
    fn count_newlines(&mut self, offset: usize, run: usize) {
        let mut at = offset;
        let end = offset + run;
        while let Some(index) = find_byte(&self.buffer[at..end], b'\n') {
            self.line += 1;
            at += index + 1;
            self.line_start = self.base + at;
        }
    }

    /// The bytes from `start` to `pos` as text.
    fn slice(&self, start: usize, end: usize) -> Result<&str, XmlError> {
        let bytes = &self.buffer[start - self.base..end - self.base];
        match std::str::from_utf8(bytes) {
            Ok(text) => Ok(text),
            Err(_) => self.error("invalid UTF-8"),
        }
    }

    fn note(&mut self, note: String) {
        if !self.notes.contains(&note) {
            self.notes.push(note);
        }
    }

    fn peek(&mut self) -> Option<u8> {
        if self.pos >= self.base + self.buffer.len() {
            self.fill();
            if self.pos >= self.base + self.buffer.len() {
                return None;
            }
        }
        Some(self.buffer[self.pos - self.base])
    }

    fn at_end(&mut self) -> bool {
        self.peek().is_none()
    }

    fn starts_with(&mut self, pattern: &[u8]) -> bool {
        if self.available(pattern.len()) < pattern.len() {
            return false;
        }
        self.buffer[self.pos - self.base..].starts_with(pattern)
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
        loop {
            let offset = self.pos - self.base;
            let window_len = self.buffer.len() - offset;
            let run = self.buffer[offset..]
                .iter()
                .take_while(|byte| matches!(**byte, b' ' | b'\t' | b'\n' | b'\r'))
                .count();
            self.count_newlines(offset, run);
            self.pos += run;
            if run < window_len {
                return;
            }
            let before = self.buffer.len() + self.base;
            self.fill();
            if self.buffer.len() + self.base == before {
                return;
            }
        }
    }

    /// Moves past `terminator`, or fails with `message` at the end.
    fn skip_past(&mut self, terminator: &[u8], message: &'static str) -> Result<(), XmlError> {
        while !self.at_end() {
            self.mark = self.pos;
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
        if self.starts_with(b"\xef\xbb\xbf") {
            self.pos += 3;
        }
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
        if !self.at_end() {
            return self.error("content after the root element");
        }
        Ok(Value::Table(vec![(name, node)]))
    }

    /// Whitespace, comments, and processing instructions between things.
    fn misc(&mut self) -> Result<(), XmlError> {
        loop {
            self.skip_ws();
            self.mark = self.pos;
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
            self.mark = self.pos;
            if self.starts_with(b"-->") {
                self.pos += 3;
                self.note("comments are dropped".to_string());
                return Ok(());
            }
            if self.starts_with(b"--") {
                return self.error("'--' inside a comment");
            }
            if self.at_end() {
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
            self.mark = self.pos;
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
        self.mark = self.pos;
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
                    let mut key = String::with_capacity(attribute.len() + 1);
                    key.push('@');
                    key.push_str(&attribute);
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
            self.mark = self.pos;
            match self.peek() {
                None => return self.error("element never closes"),
                Some(b'<') => {
                    if self.starts_with(b"</") {
                        self.pos += 2;
                        if !self.end_name_matches(name) {
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
                        self.mark = start;
                        loop {
                            if self.starts_with(b"]]>") {
                                break;
                            }
                            if self.at_end() {
                                return self.error("CDATA section never closes");
                            }
                            self.bump();
                        }
                        let section = self.slice(start, self.pos)?;
                        node.push_text(section);
                        self.pos += 3;
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
                    self.mark = start;
                    loop {
                        self.scan_to_text_stop();
                        match self.peek() {
                            Some(b']') => {
                                if self.starts_with(b"]]>") {
                                    return self.error("']]>' in text");
                                }
                                self.pos += 1;
                            }
                            _ => break,
                        }
                    }
                    let run = self.slice(start, self.pos)?;
                    node.push_text(run);
                }
            }
        }
    }

    fn finish_node(&mut self, mut node: Node) -> Value {
        node.flush_pending();
        let mixed = node.has_children && !node.text.is_empty();
        if mixed {
            let note = format!(
                "{}: mixed content, the order of text against elements is lost",
                self.path
            );
            self.note(note);
        }
        let text = normalize_newlines(node.text);
        let mut members = node.members;
        if members.is_empty() {
            return if text.is_empty() {
                Value::Null
            } else {
                Value::String(text)
            };
        }
        if !text.is_empty() {
            members.push(("#text".to_string(), Value::String(text)));
        }
        Value::Table(members)
    }

    fn name(&mut self) -> Result<String, XmlError> {
        let start = self.pos;
        self.mark = start;
        match self.peek() {
            Some(byte) if is_name_start(byte) => self.pos += 1,
            _ => return self.error("expected a name"),
        }
        self.scan_name_bytes();
        Ok(self.slice(start, self.pos)?.to_string())
    }

    /// Reads the end tag's name and says whether it is `name`, without
    /// allocating.
    fn end_name_matches(&mut self, name: &str) -> bool {
        let start = self.pos;
        self.mark = start;
        match self.peek() {
            Some(byte) if is_name_start(byte) => self.pos += 1,
            _ => return false,
        }
        self.scan_name_bytes();
        &self.buffer[start - self.base..self.pos - self.base] == name.as_bytes()
    }

    /// Moves `pos` past the name bytes, a window slice at a time.
    fn scan_name_bytes(&mut self) {
        loop {
            let offset = self.pos - self.base;
            let window = &self.buffer[offset..];
            let run = window
                .iter()
                .take_while(|byte| is_name_byte(**byte))
                .count();
            self.pos += run;
            if run < window.len() {
                return;
            }
            let before = self.buffer.len() + self.base;
            self.fill();
            if self.buffer.len() + self.base == before {
                return;
            }
        }
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
            self.mark = start;
            self.scan_to_attribute_stop(quote);
            out.push_str(self.slice(start, self.pos)?);
            match self.peek() {
                Some(byte) if byte == quote => {
                    self.pos += 1;
                    return Ok(normalize_newlines(out));
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
        self.mark = start;
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
        let entity = self.slice(start, self.pos)?;
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

    /// Trims the pending stretch in place and moves it into the text
    /// (the first stretch is taken over, not copied).
    fn flush_pending(&mut self) {
        let end = self.pending.trim_end().len();
        self.pending.truncate(end);
        let leading = self.pending.len() - self.pending.trim_start().len();
        if leading > 0 {
            self.pending.drain(..leading);
        }
        if self.pending.is_empty() {
            return;
        }
        if self.text.is_empty() {
            std::mem::swap(&mut self.text, &mut self.pending);
        } else {
            self.text.push(' ');
            self.text.push_str(&self.pending);
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

fn normalize_newlines(text: String) -> String {
    if !text.contains('\r') {
        return text;
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

    /// Hands out one byte per read, so every window boundary is crossed.
    struct OneByte<'a>(&'a [u8]);

    impl Read for OneByte<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if self.0.is_empty() || buffer.is_empty() {
                return Ok(0);
            }
            buffer[0] = self.0[0];
            self.0 = &self.0[1..];
            Ok(1)
        }
    }

    #[test]
    fn a_stream_read_one_byte_at_a_time_parses_the_same() {
        let text = "<?xml version=\"1.0\"?><!-- c --><r a=\"x &amp; y\"><![CDATA[<z>]]><k>1</k><k>2</k>t &#169;</r>";
        let mut whole = OneByte(text.as_bytes());
        let streamed = parse_reader(&mut whole).unwrap();
        let direct = parse(text).unwrap();
        assert_eq!(streamed.document, direct.document);
        assert_eq!(streamed.notes, direct.notes);
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
