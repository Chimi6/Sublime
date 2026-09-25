//! TOML 1.0 reader. One forward pass over the text builds the value tree
//! directly, enforcing the specification's definition rules as tables are
//! created (a table defined twice, an inline table extended, a static
//! array appended to). Each node carries a kind beside the tree so those
//! rules are one match at the point of use; members are found through
//! the hub's lazy index, so a document of many small tables and a
//! document of one huge table both stay linear.

use crate::converter::Location;
use crate::value::{Data, MemberIndex, NONE, Span, Tree};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TomlError {
    pub location: Location,
    pub message: String,
}

/// Parses one TOML document; the tree's root is its table.
pub fn parse(text: &str) -> Result<Tree, TomlError> {
    let mut parser = Parser::new(text);
    parser.document()?;
    Ok(parser.tree)
}

/// How a node came to exist; the rules for extending it depend on it.
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
    /// `[[a]]`: open to more `[[a]]`.
    TableArray,
    /// `[ ... ]`: closed.
    StaticArray,
    Leaf,
}

struct Parser<'a> {
    text: &'a str,
    bytes: &'a [u8],
    pos: usize,
    line: u64,
    line_start: usize,
    tree: Tree,
    /// One kind per tree node.
    kinds: Vec<Kind>,
    index: MemberIndex,
    current: u32,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Parser<'a> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let mut tree = Tree::with_capacity(text.len());
        let root = tree.push_table(Span::default());
        tree.root = root;
        Parser {
            text,
            bytes: text.as_bytes(),
            pos: 0,
            line: 1,
            line_start: 0,
            tree,
            kinds: vec![Kind::Header],
            index: MemberIndex::default(),
            current: root,
        }
    }

    // ---- nodes ----

    fn new_table(&mut self, kind: Kind, key: &str) -> u32 {
        let key = self.tree.intern(key);
        let id = self.tree.push_table(key);
        self.kinds.push(kind);
        id
    }

    fn new_array(&mut self, kind: Kind, key: &str) -> u32 {
        let key = self.tree.intern(key);
        let id = self.tree.push_array(key);
        self.kinds.push(kind);
        id
    }

    fn leaf(&mut self, data: Data) -> u32 {
        let id = self.tree.push(Span::default(), data);
        self.kinds.push(Kind::Leaf);
        id
    }

    fn text_leaf(&mut self, text: &str) -> u32 {
        let span = self.tree.intern(text);
        self.leaf(Data::Text(span))
    }

    fn kind(&self, id: u32) -> Kind {
        self.kinds[id as usize]
    }

    /// Appends `child` under `parent` with `key`, keeping the index current.
    fn add_member(&mut self, parent: u32, key: &str, child: u32) {
        self.tree.nodes[child as usize].key = self.tree.intern(key);
        self.tree.append(parent, child);
        self.index.record(&self.tree, parent, child);
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
        match self.index.find(&self.tree, parent, last) {
            None => {
                let id = self.new_table(Kind::Header, "");
                self.add_member(parent, last, id);
                Ok(id)
            }
            Some(id) => match self.kind(id) {
                Kind::Implicit => {
                    self.kinds[id as usize] = Kind::Header;
                    Ok(id)
                }
                Kind::Header | Kind::Dotted | Kind::Inline => {
                    Err(self.error_at(start, "table defined twice"))
                }
                Kind::TableArray => {
                    Err(self.error_at(start, "already an array of tables, not a table"))
                }
                Kind::StaticArray | Kind::Leaf => {
                    Err(self.error_at(start, "already a value, not a table"))
                }
            },
        }
    }

    /// `[[a.b]]`: appends a table to the array at the path, creating it.
    #[inline(never)]
    fn open_table_array(&mut self, path: &[String], start: usize) -> Result<u32, TomlError> {
        let (last, parents) = split_last(path);
        let parent = self.walk_header_parents(parents, start)?;
        let table = self.new_table(Kind::Header, "");
        match self.index.find(&self.tree, parent, last) {
            None => {
                let array = self.new_array(Kind::TableArray, "");
                self.add_member(parent, last, array);
                self.tree.append(array, table);
                Ok(table)
            }
            Some(id) => match self.kind(id) {
                Kind::TableArray => {
                    self.tree.append(id, table);
                    Ok(table)
                }
                Kind::StaticArray => {
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
        let mut node = self.tree.root;
        for segment in parents {
            node = match self.index.find(&self.tree, node, segment) {
                None => {
                    let id = self.new_table(Kind::Implicit, "");
                    self.add_member(node, segment, id);
                    id
                }
                Some(id) => match self.kind(id) {
                    Kind::Inline => {
                        return Err(self.error_at(start, "inline table cannot be extended"));
                    }
                    Kind::Header | Kind::Implicit | Kind::Dotted => id,
                    Kind::TableArray => match self.tree.data(id) {
                        Data::Array(children) if children.last != NONE => children.last,
                        _ => return Err(self.error_at(start, "empty array of tables")),
                    },
                    Kind::StaticArray | Kind::Leaf => {
                        return Err(self.error_at(start, "path crosses a value, not a table"));
                    }
                },
            };
        }
        Ok(node)
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
        let value = self.value()?;
        let (last, parents) = split_last(&path);
        let mut node = table;
        let inside_inline = self.kind(table) == Kind::Inline;
        for segment in parents {
            node = match self.index.find(&self.tree, node, segment) {
                None => {
                    let kind = if inside_inline {
                        Kind::Inline
                    } else {
                        Kind::Dotted
                    };
                    let id = self.new_table(kind, "");
                    self.add_member(node, segment, id);
                    id
                }
                Some(id) => {
                    let kind = self.kind(id);
                    let open_to_dotted = kind == Kind::Dotted
                        || kind == Kind::Implicit
                        || (inside_inline && kind == Kind::Inline);
                    if !open_to_dotted {
                        let message = if self.tree.data(id).is_table() {
                            "dotted key cannot extend a table defined elsewhere"
                        } else {
                            "dotted key crosses a value, not a table"
                        };
                        return Err(self.error_at(start, message));
                    }
                    id
                }
            };
        }
        if self.index.find(&self.tree, node, last).is_some() {
            return Err(self.error_at(start, "key defined twice"));
        }
        self.add_member(node, last, value);
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

    /// A value as an unlinked node with no key yet.
    fn value(&mut self) -> Result<u32, TomlError> {
        match self.peek() {
            Some(b'"') => {
                if self.starts_with(b"\"\"\"") {
                    self.pos += 3;
                    let text = self.multiline_basic_string()?;
                    Ok(self.text_leaf(&text))
                } else {
                    self.pos += 1;
                    let text = self.basic_string()?;
                    Ok(self.text_leaf(&text))
                }
            }
            Some(b'\'') => {
                if self.starts_with(b"'''") {
                    self.pos += 3;
                    let text = self.multiline_literal_string()?;
                    Ok(self.text_leaf(&text))
                } else {
                    self.pos += 1;
                    let text = self.literal_string()?;
                    Ok(self.text_leaf(text))
                }
            }
            Some(b'[') => self.array(),
            Some(b'{') => self.inline_table(),
            Some(b't') if self.starts_with(b"true") => {
                self.pos += 4;
                Ok(self.leaf(Data::Bool(true)))
            }
            Some(b'f') if self.starts_with(b"false") => {
                self.pos += 5;
                Ok(self.leaf(Data::Bool(false)))
            }
            Some(byte) if byte.is_ascii_digit() || matches!(byte, b'+' | b'-' | b'i' | b'n') => {
                self.number_or_datetime()
            }
            Some(_) => self.error("expected a value"),
            None => self.error("expected a value, found the end of the document"),
        }
    }

    #[inline(never)]
    fn array(&mut self) -> Result<u32, TomlError> {
        self.pos += 1;
        let array = self.new_array(Kind::StaticArray, "");
        loop {
            self.skip_blank()?;
            match self.peek() {
                Some(b']') => {
                    self.pos += 1;
                    return Ok(array);
                }
                None => return self.error("array never closed"),
                _ => {}
            }
            let item = self.value()?;
            self.tree.append(array, item);
            self.skip_blank()?;
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(array);
                }
                _ => return self.error("expected ',' or ']' in the array"),
            }
        }
    }

    #[inline(never)]
    fn inline_table(&mut self) -> Result<u32, TomlError> {
        self.pos += 1;
        let table = self.new_table(Kind::Inline, "");
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(table);
        }
        loop {
            self.skip_ws();
            self.key_value(table)?;
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(table);
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
    fn number_or_datetime(&mut self) -> Result<u32, TomlError> {
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
            if !datetime_is_valid(bytes) {
                return Err(self.error_at(start, "invalid date or time"));
            }
            let span = self.tree.intern(token);
            return Ok(self.leaf(Data::Datetime(span)));
        }
        let data = parse_number(token).map_err(|message| self.error_at(start, message))?;
        Ok(self.leaf(data))
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
fn parse_number(token: &str) -> Result<Data, &'static str> {
    let (negative, unsigned) = match token.as_bytes().first() {
        Some(b'+') => (false, &token[1..]),
        Some(b'-') => (true, &token[1..]),
        _ => (false, token),
    };
    match unsigned {
        "inf" => {
            return Ok(Data::Float(if negative {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            }));
        }
        "nan" => return Ok(Data::Float(if negative { -f64::NAN } else { f64::NAN })),
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
        return Ok(Data::Integer(magnitude));
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
        .map(Data::Integer)
        .map_err(|_| "integer out of the 64-bit range")
}

#[inline(never)]
fn parse_float(token: &str, unsigned: &str) -> Result<Data, &'static str> {
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
        .map(Data::Float)
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
    use crate::io::json::from_tree::compact_text;

    fn json(text: &str) -> String {
        let tree = parse(text).unwrap();
        compact_text(&tree, tree.root)
    }

    #[test]
    fn numbers_take_every_spelling() {
        assert_eq!(
            json("a = 0xff\nb = 1_000\nc = -2.5e3\nd = +0\ne = 0o17\nf = 0b101\n"),
            r#"{"a":255,"b":1000,"c":-2500.0,"d":0,"e":15,"f":5}"#
        );
    }

    #[test]
    fn datetimes_are_validated_and_kept_as_written() {
        assert_eq!(
            json("a = 1979-05-27T07:32:00Z\nb = 07:32:00.5\nc = 1979-05-27 07:32:00\n"),
            r#"{"a":"1979-05-27T07:32:00Z","b":"07:32:00.5","c":"1979-05-27 07:32:00"}"#
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
        let tree = parse(&text).unwrap();
        let last = tree.find_member(tree.root, "k39").unwrap();
        assert_eq!(tree.data(last), Data::Integer(39));
        text.push_str("k5 = 0\n");
        assert!(parse(&text).is_err());
    }
}
