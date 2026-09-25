//! YAML 1.2 reader, core schema. A recursive descent over the text with
//! indentation as the block structure: block sequences and mappings,
//! flow collections, the five scalar styles, anchors and aliases (copied
//! by node, the text shared), merge keys, tags (the core ones resolve, the
//! rest are noted and dropped), directives, and multi-document streams.
//! The stream is built straight into the value tree: aliases and merge
//! keys need the anchored subtrees in hand, so the reader does not
//! stream.
//!
//! Every node parser leaves the position at the end of its content on its
//! last line, never past the newline, so the caller decides whether the
//! next line is its own by looking ahead at the line's indentation.

use crate::converter::Location;
use crate::io::json::from_tree::compact_text;
use crate::value::{Data, MemberIndex, Span, Tree, push_float};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YamlError {
    pub location: Location,
    pub message: String,
}

/// The documents of a stream (nodes of `tree`) and what the reader could
/// not keep (tags it dropped, keys it wrote as text), one note per
/// distinct fact.
#[derive(Debug)]
pub struct Parsed {
    pub tree: Tree,
    pub documents: Vec<u32>,
    pub notes: Vec<String>,
}

/// Parses a YAML stream.
pub fn parse(text: &str) -> Result<Parsed, YamlError> {
    let mut parser = Parser::new(text);
    let documents = parser.stream()?;
    Ok(Parsed {
        tree: parser.tree,
        documents,
        notes: parser.notes,
    })
}

/// What a plain scalar's text resolves to under the core schema.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Plain {
    Null,
    Bool(bool),
    Integer(i64),
    Float(f64),
    Str,
}

/// No indentation: the parent of a top-level node.
const ROOT_INDENT: i64 = -1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum After {
    /// After `key:`; a sequence may sit at the key's own indentation.
    MapValue,
    /// After `- `; a nested collection may start on the same line.
    SeqEntry,
    /// After `---`.
    Document,
}

#[derive(Default)]
struct Properties {
    anchor: Option<String>,
    tag: Option<String>,
}

/// Where the next content line is, seen without moving.
struct Lookahead {
    indent: usize,
    content_pos: usize,
    /// Lines consumed to reach it (for the line counter).
    lines: u64,
    /// Whitespace-only lines among them (plain and quoted scalars fold
    /// them into newlines).
    empty_lines: usize,
    is_marker: bool,
    is_comment: bool,
}

struct Parser<'a> {
    text: &'a str,
    bytes: &'a [u8],
    pos: usize,
    line: u64,
    line_start: usize,
    tree: Tree,
    index: MemberIndex,
    anchors: Vec<(String, u32)>,
    notes: Vec<String>,
    flow_depth: u32,
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
            tree: Tree::with_capacity(text.len()),
            index: MemberIndex::default(),
            anchors: Vec::new(),
            notes: Vec::new(),
            flow_depth: 0,
        }
    }

    // ---- nodes ----

    fn null(&mut self) -> u32 {
        self.tree.push(Span::default(), Data::Null)
    }

    fn leaf(&mut self, data: Data) -> u32 {
        self.tree.push(Span::default(), data)
    }

    fn text_leaf(&mut self, text: &str) -> u32 {
        let span = self.tree.intern(text);
        self.tree.push(Span::default(), Data::Text(span))
    }

    /// Keys the unlinked `node` and appends it to `table`.
    fn add_member(&mut self, table: u32, key: &str, node: u32) {
        self.tree.nodes[node as usize].key = self.tree.intern(key);
        self.tree.append(table, node);
        self.index.record(&self.tree, table, node);
    }

    // ---- errors and position ----

    fn error<T>(&self, message: impl Into<String>) -> Result<T, YamlError> {
        Err(self.error_at(self.pos, message))
    }

    /// An error at an earlier position: its line is found by counting the
    /// line breaks back from the current position.
    fn error_at(&self, pos: usize, message: impl Into<String>) -> YamlError {
        let pos = pos.min(self.bytes.len());
        let breaks_since = self.bytes[pos..self.pos.max(pos)]
            .iter()
            .enumerate()
            .filter(|(index, byte)| {
                **byte == b'\n'
                    || (**byte == b'\r' && self.bytes.get(pos + index + 1) != Some(&b'\n'))
            })
            .count() as u64;
        let mut line_start = pos;
        while line_start > 0 && !matches!(self.bytes[line_start - 1], b'\n' | b'\r') {
            line_start -= 1;
        }
        YamlError {
            location: Location {
                line: self.line.saturating_sub(breaks_since),
                column: (pos - line_start) as u64 + 1,
            },
            message: message.into(),
        }
    }

    fn note(&mut self, note: String) {
        if !self.notes.contains(&note) {
            self.notes.push(note);
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.pos + offset).copied()
    }

    fn column(&self) -> usize {
        self.pos - self.line_start
    }

    fn skip_spaces(&mut self) {
        while matches!(self.peek(), Some(b' ') | Some(b'\t')) {
            self.pos += 1;
        }
    }

    fn at_eol(&self) -> bool {
        matches!(self.peek(), None | Some(b'\n') | Some(b'\r'))
    }

    /// True when only whitespace or a comment remains on the line.
    fn at_line_end(&self) -> bool {
        let mut offset = 0;
        while let Some(byte) = self.peek_at(offset) {
            match byte {
                b' ' | b'\t' => offset += 1,
                b'\n' | b'\r' => return true,
                b'#' => return offset > 0 || self.pos == self.line_start,
                _ => return false,
            }
        }
        true
    }

    /// Whitespace and a comment, then the line must end.
    fn end_of_inline(&mut self) -> Result<(), YamlError> {
        self.skip_spaces();
        if self.peek() == Some(b'#') {
            while !self.at_eol() {
                self.pos += 1;
            }
        }
        if self.at_eol() {
            Ok(())
        } else {
            self.error("unexpected content after the value")
        }
    }

    /// Scans forward from the current line's end to the next line that
    /// holds content, without moving. `None` at the end of the stream.
    fn scan_next_content(&self) -> Option<Lookahead> {
        self.scan_next_content_from(self.pos)
    }

    fn scan_next_content_from(&self, from: usize) -> Option<Lookahead> {
        let mut at = from;
        let mut lines = 0;
        let mut empty_lines = 0;
        // Finish the current line.
        loop {
            match self.bytes.get(at) {
                None => return None,
                Some(b'\n') => {
                    at += 1;
                    break;
                }
                Some(b'\r') => {
                    at += 1;
                    if self.bytes.get(at) == Some(&b'\n') {
                        at += 1;
                    }
                    break;
                }
                Some(_) => at += 1,
            }
        }
        lines += 1;
        loop {
            let line_start = at;
            let mut has_tab = false;
            while let Some(byte) = self.bytes.get(at) {
                if *byte == b' ' {
                    at += 1;
                } else if *byte == b'\t' {
                    has_tab = true;
                    at += 1;
                } else {
                    break;
                }
            }
            match self.bytes.get(at) {
                None => return None,
                Some(b'\n') | Some(b'\r') => {
                    empty_lines += 1;
                    lines += 1;
                    at += 1;
                    if self.bytes.get(at - 1) == Some(&b'\r') && self.bytes.get(at) == Some(&b'\n')
                    {
                        at += 1;
                    }
                }
                Some(b'#') => {
                    return Some(Lookahead {
                        indent: at - line_start,
                        content_pos: at,
                        lines,
                        empty_lines,
                        is_marker: false,
                        is_comment: true,
                    });
                }
                Some(_) => {
                    let indent = at - line_start;
                    let is_marker = indent == 0 && self.marker_at(at);
                    return Some(Lookahead {
                        indent: if has_tab { usize::MAX } else { indent },
                        content_pos: at,
                        lines,
                        empty_lines,
                        is_marker,
                        is_comment: false,
                    });
                }
            }
        }
    }

    /// The next content line that is not a comment; comment lines are
    /// skipped the way blank lines are.
    fn next_content(&self) -> Option<Lookahead> {
        let mut probe = self.pos;
        let mut total_lines = 0;
        let mut total_empty = 0;
        loop {
            let look = self.scan_next_content_from(probe)?;
            total_lines += look.lines;
            total_empty += look.empty_lines;
            if look.is_comment {
                probe = look.content_pos;
                continue;
            }
            return Some(Lookahead {
                indent: look.indent,
                content_pos: look.content_pos,
                lines: total_lines,
                empty_lines: total_empty,
                is_marker: look.is_marker,
                is_comment: false,
            });
        }
    }

    fn jump(&mut self, look: &Lookahead) -> Result<(), YamlError> {
        self.line += look.lines;
        self.pos = look.content_pos;
        if look.indent == usize::MAX {
            let mut start = look.content_pos;
            while start > 0 && !matches!(self.bytes[start - 1], b'\n' | b'\r') {
                start -= 1;
            }
            self.line_start = start;
            return self.error("tabs are not allowed for indentation");
        }
        self.line_start = look.content_pos - look.indent;
        Ok(())
    }

    fn marker_at(&self, at: usize) -> bool {
        let rest = &self.bytes[at..];
        let is_word = rest.starts_with(b"---") || rest.starts_with(b"...");
        is_word
            && matches!(
                rest.get(3),
                None | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r')
            )
    }

    fn at_document_marker(&self) -> bool {
        self.column() == 0 && self.marker_at(self.pos)
    }

    // ---- stream and documents ----

    fn stream(&mut self) -> Result<Vec<u32>, YamlError> {
        let mut documents = Vec::new();
        let mut saw_directive = false;
        if !self.at_first_content()? {
            return Ok(documents);
        }
        loop {
            if self.column() == 0 && self.peek() == Some(b'%') {
                while !self.at_eol() {
                    self.pos += 1;
                }
                saw_directive = true;
                if !self.advance_to_content()? {
                    return self.error("directives need a '---' after them");
                }
                continue;
            }
            if self.at_document_marker() && self.bytes[self.pos..].starts_with(b"...") {
                self.pos += 3;
                self.end_of_inline()?;
                if !self.advance_to_content()? {
                    break;
                }
                continue;
            }
            let document = if self.at_document_marker() {
                self.pos += 3;
                saw_directive = false;
                self.node_after_indicator(ROOT_INDENT, After::Document)?
            } else {
                if saw_directive {
                    return self.error("directives need a '---' after them");
                }
                let indent = self.column() as i64;
                self.block_node(indent, ROOT_INDENT)?
            };
            self.end_of_inline()?;
            documents.push(document);
            self.anchors.clear();
            if !self.advance_to_content()? {
                break;
            }
            let at_marker =
                self.at_document_marker() || (self.column() == 0 && self.peek() == Some(b'%'));
            if !at_marker {
                return self.error("expected a document marker or the end of the stream");
            }
        }
        Ok(documents)
    }

    /// Moves to the first content line of the stream. False when empty.
    fn at_first_content(&mut self) -> Result<bool, YamlError> {
        let mut at = 0;
        let mut has_tab = false;
        loop {
            let line_start = at;
            while let Some(byte) = self.bytes.get(at) {
                if *byte == b' ' {
                    at += 1;
                } else if *byte == b'\t' {
                    has_tab = true;
                    at += 1;
                } else {
                    break;
                }
            }
            match self.bytes.get(at) {
                None => return Ok(false),
                Some(b'\n') | Some(b'\r') => {
                    at += 1;
                    if self.bytes.get(at - 1) == Some(&b'\r') && self.bytes.get(at) == Some(&b'\n')
                    {
                        at += 1;
                    }
                    self.line += 1;
                    has_tab = false;
                }
                Some(b'#') => {
                    while let Some(byte) = self.bytes.get(at) {
                        if *byte == b'\n' || *byte == b'\r' {
                            break;
                        }
                        at += 1;
                    }
                }
                Some(_) => {
                    self.pos = at;
                    self.line_start = line_start;
                    if has_tab {
                        return self.error("tabs are not allowed for indentation");
                    }
                    return Ok(true);
                }
            }
        }
    }

    /// Moves to the next content line after the current one. False at the end.
    fn advance_to_content(&mut self) -> Result<bool, YamlError> {
        match self.next_content() {
            Some(look) => {
                self.jump(&look)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    // ---- block nodes ----

    /// The node after `key:`, `- `, or `---`: inline on the same line, or
    /// on the following lines, or empty. Returned unlinked and unkeyed.
    fn node_after_indicator(&mut self, parent_indent: i64, after: After) -> Result<u32, YamlError> {
        self.skip_spaces();
        let props = self.properties()?;
        self.skip_spaces();
        if self.at_line_end() {
            let node = match self.next_content() {
                Some(look) if !look.is_marker && look.indent != usize::MAX => {
                    let indent = look.indent as i64;
                    let dash_may_share =
                        after == After::MapValue && self.dash_entry_at(look.content_pos);
                    if indent > parent_indent || (indent == parent_indent && dash_may_share) {
                        self.jump(&look)?;
                        self.block_node(indent, parent_indent)?
                    } else {
                        self.null()
                    }
                }
                Some(look) if look.indent == usize::MAX => {
                    self.jump(&look)?;
                    self.null()
                }
                _ => self.null(),
            };
            return self.finish_properties(props, node);
        }
        let column = self.column() as i64;
        let compact_sequence = after != After::MapValue && self.dash_entry_at(self.pos);
        if compact_sequence {
            let node = self.block_sequence(column)?;
            return self.finish_properties(props, node);
        }
        let compact_mapping = after != After::MapValue && self.block_mapping_starts_here();
        if compact_mapping {
            let node = self.block_mapping(column)?;
            return self.finish_properties(props, node);
        }
        self.inline_node(parent_indent, props)
    }

    /// A node whose first content byte is at the current position, at
    /// column `indent`.
    fn block_node(&mut self, indent: i64, parent_indent: i64) -> Result<u32, YamlError> {
        if self.dash_entry_at(self.pos) {
            return self.block_sequence(indent);
        }
        if self.block_mapping_starts_here() {
            return self.block_mapping(indent);
        }
        let props = self.properties()?;
        self.skip_spaces();
        if self.at_line_end() && (props.anchor.is_some() || props.tag.is_some()) {
            let node = match self.next_content() {
                Some(look)
                    if !look.is_marker
                        && look.indent != usize::MAX
                        && look.indent as i64 > parent_indent =>
                {
                    self.jump(&look)?;
                    let inner = look.indent as i64;
                    self.block_node(inner, parent_indent)?
                }
                _ => self.null(),
            };
            return self.finish_properties(props, node);
        }
        self.inline_node(parent_indent, props)
    }

    fn dash_entry_at(&self, at: usize) -> bool {
        self.bytes.get(at) == Some(&b'-')
            && matches!(
                self.bytes.get(at + 1),
                None | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r')
            )
    }

    /// `? ` or an implicit key followed by `:`.
    fn block_mapping_starts_here(&self) -> bool {
        if self.peek() == Some(b'?')
            && matches!(
                self.peek_at(1),
                None | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r')
            )
        {
            return true;
        }
        self.implicit_key_end().is_some()
    }

    /// Where the `:` of an implicit key on this line is, if the line
    /// starts with one. Properties before the key are looked past.
    fn implicit_key_end(&self) -> Option<usize> {
        let mut at = self.pos;
        while matches!(self.bytes.get(at), Some(b'&') | Some(b'!')) {
            while let Some(byte) = self.bytes.get(at) {
                if matches!(byte, b' ' | b'\t' | b'\n' | b'\r') {
                    break;
                }
                at += 1;
            }
            while matches!(self.bytes.get(at), Some(b' ') | Some(b'\t')) {
                at += 1;
            }
        }
        match self.bytes.get(at)? {
            b'"' | b'\'' => {
                let quote = self.bytes[at];
                at += 1;
                loop {
                    match self.bytes.get(at)? {
                        b'\n' | b'\r' => return None,
                        b'\\' if quote == b'"' => at += 2,
                        byte if *byte == quote => {
                            if quote == b'\'' && self.bytes.get(at + 1) == Some(&b'\'') {
                                at += 2;
                                continue;
                            }
                            at += 1;
                            break;
                        }
                        _ => at += 1,
                    }
                }
                while matches!(self.bytes.get(at), Some(b' ') | Some(b'\t')) {
                    at += 1;
                }
                if self.bytes.get(at) == Some(&b':') && self.value_indicator_at(at) {
                    Some(at)
                } else {
                    None
                }
            }
            b'*' => {
                while let Some(byte) = self.bytes.get(at) {
                    if matches!(byte, b' ' | b'\t' | b'\n' | b'\r') {
                        break;
                    }
                    at += 1;
                }
                while matches!(self.bytes.get(at), Some(b' ') | Some(b'\t')) {
                    at += 1;
                }
                if self.bytes.get(at) == Some(&b':') && self.value_indicator_at(at) {
                    Some(at)
                } else {
                    None
                }
            }
            b'[' | b'{' | b'#' | b'|' | b'>' | b'%' | b'@' | b'`' => None,
            b'-' | b'?' | b':'
                if matches!(
                    self.bytes.get(at + 1),
                    None | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r')
                ) =>
            {
                None
            }
            b',' | b']' | b'}' => None,
            _ => {
                let mut previous = b' ';
                while let Some(byte) = self.bytes.get(at) {
                    match byte {
                        b'\n' | b'\r' => return None,
                        b'#' if previous == b' ' || previous == b'\t' => return None,
                        b':' if self.value_indicator_at(at) => return Some(at),
                        b',' | b']' | b'}' | b'[' | b'{' if self.flow_depth > 0 => return None,
                        _ => {}
                    }
                    previous = *byte;
                    at += 1;
                }
                None
            }
        }
    }

    /// A `:` that ends a key: followed by whitespace, the line end, or in
    /// flow context a flow indicator.
    fn value_indicator_at(&self, at: usize) -> bool {
        match self.bytes.get(at + 1) {
            None | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r') => true,
            Some(b',') | Some(b']') | Some(b'}') | Some(b'[') | Some(b'{') => self.flow_depth > 0,
            _ => false,
        }
    }

    fn block_sequence(&mut self, indent: i64) -> Result<u32, YamlError> {
        let array = self.tree.push_array(Span::default());
        loop {
            self.pos += 1;
            let item = self.node_after_indicator(indent, After::SeqEntry)?;
            self.tree.append(array, item);
            self.end_of_inline()?;
            match self.next_content() {
                Some(look) if look.indent == usize::MAX => {
                    self.jump(&look)?;
                }
                Some(look)
                    if !look.is_marker
                        && look.indent as i64 == indent
                        && self.dash_entry_at(look.content_pos) =>
                {
                    self.jump(&look)?;
                }
                Some(look) if !look.is_marker && look.indent as i64 > indent => {
                    self.jump(&look)?;
                    return self.error("bad indentation: this line belongs to no node");
                }
                _ => return Ok(array),
            }
        }
    }

    fn block_mapping(&mut self, indent: i64) -> Result<u32, YamlError> {
        let table = self.tree.push_table(Span::default());
        let mut merges: Vec<(u32, u32)> = Vec::new();
        loop {
            let entry_start = self.pos;
            let explicit = self.peek() == Some(b'?')
                && matches!(
                    self.peek_at(1),
                    None | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r')
                );
            let (key, value) = if explicit {
                self.pos += 1;
                let key_node = self.node_after_indicator(indent, After::SeqEntry)?;
                let key = self.key_text(key_node);
                self.end_of_inline()?;
                let value = match self.next_content() {
                    Some(look)
                        if !look.is_marker
                            && look.indent as i64 == indent
                            && self.bytes.get(look.content_pos) == Some(&b':')
                            && matches!(
                                self.bytes.get(look.content_pos + 1),
                                None | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r')
                            ) =>
                    {
                        self.jump(&look)?;
                        self.pos += 1;
                        self.node_after_indicator(indent, After::MapValue)?
                    }
                    _ => self.null(),
                };
                (key, value)
            } else {
                let props = self.properties()?;
                self.skip_spaces();
                let key = self.implicit_key()?;
                if let Some(anchor) = props.anchor {
                    let node = self.text_leaf(&key);
                    self.anchors.push((anchor, node));
                }
                self.skip_spaces();
                if self.peek() != Some(b':') {
                    return self.error("expected ':' after the key");
                }
                self.pos += 1;
                let value = self.node_after_indicator(indent, After::MapValue)?;
                (key, value)
            };
            self.add_entry(table, &key, value, &mut merges, entry_start)?;
            self.end_of_inline()?;
            match self.next_content() {
                Some(look) if look.indent == usize::MAX => {
                    self.jump(&look)?;
                }
                Some(look) if !look.is_marker && look.indent as i64 == indent => {
                    if self.dash_entry_at(look.content_pos) {
                        self.jump(&look)?;
                        return self.error("a sequence entry where a mapping key was expected");
                    }
                    self.jump(&look)?;
                }
                Some(look) if !look.is_marker && look.indent as i64 > indent => {
                    self.jump(&look)?;
                    return self.error("bad indentation: this line belongs to no node");
                }
                _ => break,
            }
        }
        self.resolve_merges(table, merges);
        Ok(table)
    }

    /// Appends a mapping entry, or a placeholder for a `<<` merge; a real
    /// key given twice is an error.
    fn add_entry(
        &mut self,
        table: u32,
        key: &str,
        value: u32,
        merges: &mut Vec<(u32, u32)>,
        entry_start: usize,
    ) -> Result<(), YamlError> {
        if key == "<<" {
            let placeholder = self.null();
            self.tree.nodes[placeholder as usize].key = self.tree.intern("<<");
            self.tree.append(table, placeholder);
            merges.push((placeholder, value));
            return Ok(());
        }
        if self.index.find(&self.tree, table, key).is_some() {
            return Err(self.error_at(entry_start, "duplicate key"));
        }
        self.add_member(table, key, value);
        Ok(())
    }

    /// Applies `<<` merges: a mapping's own keys win wherever they sit;
    /// merged keys take the merge's position, earlier sources first.
    fn resolve_merges(&mut self, table: u32, merges: Vec<(u32, u32)>) {
        for (placeholder, source) in merges {
            let sources: Vec<u32> = match self.tree.data(source) {
                Data::Array(_) => self.tree.children(source).collect(),
                _ => vec![source],
            };
            let mut tail = placeholder;
            for source in sources {
                if !self.tree.data(source).is_table() {
                    continue;
                }
                let members: Vec<u32> = self.tree.children(source).collect();
                for member in members {
                    let key = self.tree.key(member);
                    if self.tree.find_member(table, key).is_some() {
                        continue;
                    }
                    let copy = self.tree.copy_subtree(member);
                    self.tree.insert_after(table, tail, copy);
                    tail = copy;
                }
            }
            self.tree.unlink(table, placeholder);
        }
    }

    /// A single-line key: plain (as written), quoted, or an alias.
    fn implicit_key(&mut self) -> Result<String, YamlError> {
        match self.peek() {
            Some(b'"') => {
                self.pos += 1;
                self.double_quoted()
            }
            Some(b'\'') => {
                self.pos += 1;
                self.single_quoted()
            }
            Some(b'*') => {
                let node = self.alias()?;
                Ok(self.key_text(node))
            }
            Some(b'[') | Some(b'{') => self.error("flow collections as keys are not supported"),
            Some(_) => {
                let Some(end) = self.implicit_key_end() else {
                    return self.error("expected a key");
                };
                let mut text_end = end;
                while text_end > self.pos && matches!(self.bytes[text_end - 1], b' ' | b'\t') {
                    text_end -= 1;
                }
                let key = self.text[self.pos..text_end].to_string();
                self.pos = end;
                Ok(key)
            }
            None => self.error("expected a key"),
        }
    }

    /// A key node written as text: scalars by their text, collections as
    /// compact JSON (noted as a loss).
    fn key_text(&mut self, key: u32) -> String {
        match self.tree.data(key) {
            Data::Text(span) | Data::Datetime(span) => self.tree.str(span).to_string(),
            Data::Null => "null".to_string(),
            Data::Bool(flag) => flag.to_string(),
            Data::Integer(number) => number.to_string(),
            Data::Float(number) => {
                let mut out = String::new();
                push_float(&mut out, number);
                out
            }
            Data::Array(_) | Data::Table(_) => {
                self.note("a collection used as a key is written as its JSON text".to_string());
                compact_text(&self.tree, key)
            }
        }
    }

    // ---- inline nodes ----

    fn inline_node(&mut self, parent_indent: i64, props: Properties) -> Result<u32, YamlError> {
        let node = match self.peek() {
            Some(b'*') => {
                let node = self.alias()?;
                return self.finish_properties(props, node);
            }
            Some(b'[') => self.flow_sequence()?,
            Some(b'{') => self.flow_mapping()?,
            Some(b'"') => {
                self.pos += 1;
                let text = self.double_quoted()?;
                return self.tagged_scalar(props, text, false);
            }
            Some(b'\'') => {
                self.pos += 1;
                let text = self.single_quoted()?;
                return self.tagged_scalar(props, text, false);
            }
            Some(b'|') | Some(b'>') if self.flow_depth == 0 => {
                let literal = self.peek() == Some(b'|');
                self.pos += 1;
                let text = self.block_scalar(parent_indent, literal)?;
                return self.tagged_scalar(props, text, false);
            }
            Some(b'#') => {
                let node = self.null();
                return self.finish_properties(props, node);
            }
            Some(_) => {
                let text = self.plain_scalar(parent_indent)?;
                return self.tagged_scalar(props, text, true);
            }
            None => self.null(),
        };
        self.finish_properties(props, node)
    }

    fn properties(&mut self) -> Result<Properties, YamlError> {
        let mut props = Properties::default();
        loop {
            match self.peek() {
                Some(b'&') => {
                    self.pos += 1;
                    let start = self.pos;
                    while let Some(byte) = self.peek() {
                        if matches!(
                            byte,
                            b' ' | b'\t' | b'\n' | b'\r' | b',' | b']' | b'}' | b'[' | b'{'
                        ) {
                            break;
                        }
                        self.pos += 1;
                    }
                    if start == self.pos {
                        return self.error("anchor without a name");
                    }
                    props.anchor = Some(self.text[start..self.pos].to_string());
                }
                Some(b'!') => {
                    let start = self.pos;
                    if self.peek_at(1) == Some(b'<') {
                        while let Some(byte) = self.peek() {
                            self.pos += 1;
                            if byte == b'>' {
                                break;
                            }
                        }
                    } else {
                        while let Some(byte) = self.peek() {
                            if matches!(
                                byte,
                                b' ' | b'\t' | b'\n' | b'\r' | b',' | b']' | b'}' | b'[' | b'{'
                            ) {
                                break;
                            }
                            self.pos += 1;
                        }
                    }
                    props.tag = Some(self.text[start..self.pos].to_string());
                }
                _ => return Ok(props),
            }
            self.skip_spaces();
        }
    }

    /// Registers the anchor and applies a collection tag.
    fn finish_properties(&mut self, props: Properties, node: u32) -> Result<u32, YamlError> {
        let mut node = node;
        if let Some(tag) = props.tag {
            match tag.as_str() {
                "!!map" | "!!seq" | "!!set" | "!!omap" | "!!pairs" | "!" => {}
                "!!str" | "!!int" | "!!float" | "!!bool" | "!!null" => {
                    if self.tree.data(node).is_container() {
                        return self.error("a scalar tag on a collection");
                    }
                    node = self.tagged_node(&tag, node)?;
                }
                other => self.note(format!("tag {other} dropped")),
            }
        }
        if let Some(anchor) = props.anchor {
            self.anchors.push((anchor, node));
        }
        Ok(node)
    }

    /// A scalar's text with its tag applied: plain text resolves by the
    /// core schema unless a tag says otherwise; quoted text is a string.
    fn tagged_scalar(
        &mut self,
        props: Properties,
        text: String,
        plain: bool,
    ) -> Result<u32, YamlError> {
        let node = match &props.tag {
            None => {
                if plain {
                    self.resolved_leaf(&text)
                } else {
                    self.text_leaf(&text)
                }
            }
            Some(tag) => {
                let raw = self.text_leaf(&text);
                self.tagged_node(tag, raw)?
            }
        };
        if let Some(anchor) = props.anchor {
            self.anchors.push((anchor, node));
        }
        Ok(node)
    }

    /// A plain scalar's text as the node the core schema makes of it.
    fn resolved_leaf(&mut self, text: &str) -> u32 {
        match resolve_plain(text) {
            Plain::Null => self.null(),
            Plain::Bool(flag) => self.leaf(Data::Bool(flag)),
            Plain::Integer(number) => self.leaf(Data::Integer(number)),
            Plain::Float(number) => self.leaf(Data::Float(number)),
            Plain::Str => self.text_leaf(text),
        }
    }

    /// The scalar `node` re-read under `tag`.
    fn tagged_node(&mut self, tag: &str, node: u32) -> Result<u32, YamlError> {
        let text = match self.tree.data(node) {
            Data::Text(span) | Data::Datetime(span) => self.tree.str(span).to_string(),
            _ => compact_text(&self.tree, node),
        };
        match tag {
            "!!str" | "!" => Ok(self.text_leaf(&text)),
            "!!null" => Ok(self.null()),
            "!!int" => match resolve_plain(&text) {
                Plain::Integer(number) => Ok(self.leaf(Data::Integer(number))),
                _ => self.error("tagged !!int is not an integer"),
            },
            "!!float" => match resolve_plain(&text) {
                Plain::Integer(number) => Ok(self.leaf(Data::Float(number as f64))),
                Plain::Float(number) => Ok(self.leaf(Data::Float(number))),
                _ => self.error("tagged !!float is not a float"),
            },
            "!!bool" => match resolve_plain(&text) {
                Plain::Bool(flag) => Ok(self.leaf(Data::Bool(flag))),
                _ => self.error("tagged !!bool is not a boolean"),
            },
            other => {
                self.note(format!("tag {other} dropped"));
                Ok(self.text_leaf(&text))
            }
        }
    }

    fn alias(&mut self) -> Result<u32, YamlError> {
        let start = self.pos;
        self.pos += 1;
        let name_start = self.pos;
        while let Some(byte) = self.peek() {
            if matches!(
                byte,
                b' ' | b'\t' | b'\n' | b'\r' | b',' | b']' | b'}' | b'[' | b'{'
            ) {
                break;
            }
            self.pos += 1;
        }
        let name = &self.text[name_start..self.pos];
        let anchored = self
            .anchors
            .iter()
            .rev()
            .find(|(anchor, _)| anchor == name)
            .map(|(_, node)| *node);
        match anchored {
            Some(node) => Ok(self.tree.copy_subtree(node)),
            None => Err(self.error_at(start, "unknown anchor")),
        }
    }

    // ---- scalars ----

    /// A plain scalar, possibly over several lines; returns its text.
    fn plain_scalar(&mut self, parent_indent: i64) -> Result<String, YamlError> {
        let mut out = String::new();
        if !self.plain_line(&mut out)? {
            return Ok(out);
        }
        loop {
            let Some(look) = self.scan_next_content() else {
                return Ok(out);
            };
            if look.is_comment || look.is_marker || look.indent == usize::MAX {
                return Ok(out);
            }
            if look.indent as i64 <= parent_indent {
                return Ok(out);
            }
            if self.flow_depth > 0 && matches!(self.bytes[look.content_pos], b',' | b']' | b'}') {
                return Ok(out);
            }
            self.jump(&look)?;
            if self.flow_depth == 0 && self.implicit_key_end().is_some() {
                return self.error("a mapping key inside a plain scalar (the value above needs quotes or more indentation)");
            }
            if look.empty_lines == 0 {
                out.push(' ');
            } else {
                for _ in 0..look.empty_lines {
                    out.push('\n');
                }
            }
            if !self.plain_line(&mut out)? {
                return Ok(out);
            }
        }
    }

    /// One line of a plain scalar, trailing whitespace trimmed. True when
    /// the line ran to its end, so the scalar may continue on the next.
    fn plain_line(&mut self, out: &mut String) -> Result<bool, YamlError> {
        let start = self.pos;
        let mut end = start;
        let mut previous = b' ';
        while let Some(byte) = self.peek() {
            match byte {
                b'\n' | b'\r' => break,
                b'#' if previous == b' ' || previous == b'\t' => break,
                b':' if self.value_indicator_at(self.pos) => break,
                b',' | b']' | b'}' | b'[' | b'{' if self.flow_depth > 0 => break,
                _ => {}
            }
            previous = byte;
            self.pos += 1;
            if byte != b' ' && byte != b'\t' {
                end = self.pos;
            }
        }
        out.push_str(&self.text[start..end]);
        let ran_to_end = self.at_eol();
        self.pos = end.max(start);
        Ok(ran_to_end)
    }

    /// After the opening `"`.
    fn double_quoted(&mut self) -> Result<String, YamlError> {
        let mut out = String::new();
        loop {
            let run_start = self.pos;
            while let Some(byte) = self.peek() {
                if matches!(byte, b'"' | b'\\' | b'\n' | b'\r') {
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
                    if self.at_eol() {
                        self.fold_quoted(&mut out, false)?;
                    } else {
                        self.escape(&mut out)?;
                    }
                }
                Some(_) => self.fold_quoted(&mut out, true)?,
                None => return self.error("string never closed"),
            }
        }
    }

    /// After the opening `'`.
    fn single_quoted(&mut self) -> Result<String, YamlError> {
        let mut out = String::new();
        loop {
            let run_start = self.pos;
            while let Some(byte) = self.peek() {
                if matches!(byte, b'\'' | b'\n' | b'\r') {
                    break;
                }
                self.pos += 1;
            }
            out.push_str(&self.text[run_start..self.pos]);
            match self.peek() {
                Some(b'\'') => {
                    self.pos += 1;
                    if self.peek() == Some(b'\'') {
                        self.pos += 1;
                        out.push('\'');
                        continue;
                    }
                    return Ok(out);
                }
                Some(_) => self.fold_quoted(&mut out, true)?,
                None => return self.error("string never closed"),
            }
        }
    }

    /// A line break inside a quoted scalar: trailing whitespace trimmed,
    /// empty lines become newlines, otherwise a space (or nothing after an
    /// escaped break); leading whitespace of the next line is skipped.
    fn fold_quoted(&mut self, out: &mut String, fold_to_space: bool) -> Result<(), YamlError> {
        while out.ends_with(' ') || out.ends_with('\t') {
            out.pop();
        }
        let mut empty_lines = 0;
        loop {
            match self.peek() {
                Some(b'\n') => self.pos += 1,
                Some(b'\r') => {
                    self.pos += 1;
                    if self.peek() == Some(b'\n') {
                        self.pos += 1;
                    }
                }
                _ => return self.error("string never closed"),
            }
            self.line += 1;
            self.line_start = self.pos;
            self.skip_spaces();
            match self.peek() {
                Some(b'\n') | Some(b'\r') => empty_lines += 1,
                None => return self.error("string never closed"),
                _ => break,
            }
        }
        if empty_lines > 0 {
            for _ in 0..empty_lines {
                out.push('\n');
            }
        } else if fold_to_space {
            out.push(' ');
        }
        Ok(())
    }

    /// After the backslash of a double-quoted escape.
    fn escape(&mut self, out: &mut String) -> Result<(), YamlError> {
        let escape_start = self.pos - 1;
        let Some(byte) = self.peek() else {
            return self.error("string never closed");
        };
        self.pos += 1;
        let decoded = match byte {
            b'0' => '\0',
            b'a' => '\u{7}',
            b'b' => '\u{8}',
            b't' | b'\t' => '\t',
            b'n' => '\n',
            b'v' => '\u{b}',
            b'f' => '\u{c}',
            b'r' => '\r',
            b'e' => '\u{1b}',
            b' ' => ' ',
            b'"' => '"',
            b'/' => '/',
            b'\\' => '\\',
            b'N' => '\u{85}',
            b'_' => '\u{a0}',
            b'L' => '\u{2028}',
            b'P' => '\u{2029}',
            b'x' => self.unicode_escape(2, escape_start)?,
            b'u' => self.unicode_escape(4, escape_start)?,
            b'U' => self.unicode_escape(8, escape_start)?,
            _ => return Err(self.error_at(escape_start, "invalid escape sequence")),
        };
        out.push(decoded);
        Ok(())
    }

    fn unicode_escape(&mut self, digits: usize, escape_start: usize) -> Result<char, YamlError> {
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

    /// After `|` or `>`: the header, then the indented lines.
    fn block_scalar(&mut self, parent_indent: i64, literal: bool) -> Result<String, YamlError> {
        let mut chomp = Chomp::Clip;
        let mut explicit_indent: Option<usize> = None;
        for _ in 0..2 {
            match self.peek() {
                Some(b'-') => {
                    chomp = Chomp::Strip;
                    self.pos += 1;
                }
                Some(b'+') => {
                    chomp = Chomp::Keep;
                    self.pos += 1;
                }
                Some(digit @ b'1'..=b'9') => {
                    explicit_indent = Some((digit - b'0') as usize);
                    self.pos += 1;
                }
                _ => {}
            }
        }
        self.end_of_inline()?;
        let base = parent_indent.max(0) as usize;
        let mut content_indent = explicit_indent.map(|m| base + m);
        let mut lines: Vec<&'a str> = Vec::new();
        let mut trailing_empty = 0usize;
        loop {
            let mut at = self.pos;
            match self.bytes.get(at) {
                None => break,
                Some(b'\n') => at += 1,
                Some(b'\r') => {
                    at += 1;
                    if self.bytes.get(at) == Some(&b'\n') {
                        at += 1;
                    }
                }
                Some(_) => return self.error("unexpected content after the block scalar header"),
            }
            let line_start = at;
            let mut spaces = 0;
            while self.bytes.get(at) == Some(&b' ') {
                at += 1;
                spaces += 1;
            }
            let mut line_end = at;
            while let Some(byte) = self.bytes.get(line_end) {
                if *byte == b'\n' || *byte == b'\r' {
                    break;
                }
                line_end += 1;
            }
            let rest = &self.text[at..line_end];
            let is_empty = rest.trim_matches(['\t', ' ']).is_empty() && !rest.contains('\t');
            let indent = match content_indent {
                Some(indent) => indent,
                None => {
                    if is_empty {
                        lines.push("");
                        trailing_empty += 1;
                        self.consume_line(line_end, line_start);
                        continue;
                    }
                    if spaces as i64 <= parent_indent {
                        break;
                    }
                    content_indent = Some(spaces);
                    spaces
                }
            };
            if is_empty {
                let extra = if spaces > indent {
                    &self.text[line_start + indent..line_end]
                } else {
                    ""
                };
                lines.push(extra);
                trailing_empty += 1;
                self.consume_line(line_end, line_start);
                continue;
            }
            if spaces < indent {
                break;
            }
            if spaces == 0 && self.marker_at(line_start) {
                break;
            }
            lines.push(&self.text[line_start + indent..line_end]);
            trailing_empty = 0;
            self.consume_line(line_end, line_start);
        }
        let content_lines = lines.len() - trailing_empty;
        let mut out = String::new();
        if literal {
            for (index, line) in lines[..content_lines].iter().enumerate() {
                if index > 0 {
                    out.push('\n');
                }
                out.push_str(line);
            }
        } else {
            let mut previous_normal = false;
            let mut pending_empty = 0;
            let mut first = true;
            for line in &lines[..content_lines] {
                if line.is_empty() {
                    pending_empty += 1;
                    continue;
                }
                let normal = !line.starts_with(' ') && !line.starts_with('\t');
                if !first {
                    if previous_normal && normal {
                        if pending_empty == 0 {
                            out.push(' ');
                        } else {
                            for _ in 0..pending_empty {
                                out.push('\n');
                            }
                        }
                    } else {
                        for _ in 0..=pending_empty {
                            out.push('\n');
                        }
                    }
                }
                out.push_str(line);
                previous_normal = normal;
                pending_empty = 0;
                first = false;
            }
        }
        match chomp {
            Chomp::Strip => {}
            Chomp::Clip => {
                if content_lines > 0 {
                    out.push('\n');
                }
            }
            Chomp::Keep => {
                if content_lines > 0 {
                    out.push('\n');
                }
                for _ in 0..trailing_empty {
                    out.push('\n');
                }
            }
        }
        Ok(out)
    }

    /// Moves past a block scalar line: position at its end, counters updated.
    fn consume_line(&mut self, line_end: usize, line_start: usize) {
        self.line += 1;
        self.line_start = line_start;
        self.pos = line_end;
    }

    // ---- flow collections ----

    fn skip_flow_space(&mut self) -> Result<(), YamlError> {
        loop {
            self.skip_spaces();
            match self.peek() {
                Some(b'#') => {
                    while !self.at_eol() {
                        self.pos += 1;
                    }
                }
                Some(b'\n') => {
                    self.pos += 1;
                    self.line += 1;
                    self.line_start = self.pos;
                }
                Some(b'\r') => {
                    self.pos += 1;
                    if self.peek() == Some(b'\n') {
                        self.pos += 1;
                    }
                    self.line += 1;
                    self.line_start = self.pos;
                }
                _ => return Ok(()),
            }
        }
    }

    fn flow_sequence(&mut self) -> Result<u32, YamlError> {
        let start = self.pos;
        self.pos += 1;
        self.flow_depth += 1;
        let array = self.tree.push_array(Span::default());
        loop {
            self.skip_flow_space()?;
            match self.peek() {
                Some(b']') => break,
                None => return Err(self.error_at(start, "flow sequence never closed")),
                _ => {}
            }
            let explicit = self.peek() == Some(b'?')
                && matches!(
                    self.peek_at(1),
                    Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r')
                );
            if explicit {
                self.pos += 1;
                self.skip_flow_space()?;
            }
            let item = self.flow_node()?;
            self.skip_flow_space()?;
            let item = if self.peek() == Some(b':') && self.value_indicator_at(self.pos) {
                self.pos += 1;
                self.skip_flow_space()?;
                let value = self.flow_node()?;
                let key = self.key_text(item);
                let pair = self.tree.push_table(Span::default());
                self.add_member(pair, &key, value);
                pair
            } else {
                item
            };
            self.tree.append(array, item);
            self.skip_flow_space()?;
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => break,
                _ => return self.error("expected ',' or ']' in the flow sequence"),
            }
        }
        self.pos += 1;
        self.flow_depth -= 1;
        Ok(array)
    }

    fn flow_mapping(&mut self) -> Result<u32, YamlError> {
        let start = self.pos;
        self.pos += 1;
        self.flow_depth += 1;
        let table = self.tree.push_table(Span::default());
        let mut merges: Vec<(u32, u32)> = Vec::new();
        loop {
            self.skip_flow_space()?;
            match self.peek() {
                Some(b'}') => break,
                None => return Err(self.error_at(start, "flow mapping never closed")),
                _ => {}
            }
            let entry_start = self.pos;
            let explicit = self.peek() == Some(b'?')
                && matches!(
                    self.peek_at(1),
                    Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r')
                );
            if explicit {
                self.pos += 1;
                self.skip_flow_space()?;
            }
            let key_node = self.flow_node()?;
            let key = self.key_text(key_node);
            self.skip_flow_space()?;
            let value = if self.peek() == Some(b':') {
                self.pos += 1;
                self.skip_flow_space()?;
                self.flow_node()?
            } else {
                self.null()
            };
            self.add_entry(table, &key, value, &mut merges, entry_start)?;
            self.skip_flow_space()?;
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => break,
                _ => return self.error("expected ',' or '}' in the flow mapping"),
            }
        }
        self.pos += 1;
        self.flow_depth -= 1;
        self.resolve_merges(table, merges);
        Ok(table)
    }

    /// A node inside a flow collection; empty before `,`, `]`, `}`, or `:`.
    fn flow_node(&mut self) -> Result<u32, YamlError> {
        let props = self.properties()?;
        self.skip_flow_space()?;
        match self.peek() {
            Some(b',') | Some(b']') | Some(b'}') | None => {
                let node = self.null();
                self.finish_properties(props, node)
            }
            Some(b':') if self.value_indicator_at(self.pos) => {
                let node = self.null();
                self.finish_properties(props, node)
            }
            _ => self.inline_node(ROOT_INDENT, props),
        }
    }
}

#[derive(Clone, Copy)]
enum Chomp {
    Strip,
    Clip,
    Keep,
}

/// Core schema resolution of a plain scalar.
pub fn resolve_plain(text: &str) -> Plain {
    match text {
        "" | "~" | "null" | "Null" | "NULL" => return Plain::Null,
        "true" | "True" | "TRUE" => return Plain::Bool(true),
        "false" | "False" | "FALSE" => return Plain::Bool(false),
        ".inf" | ".Inf" | ".INF" | "+.inf" | "+.Inf" | "+.INF" => {
            return Plain::Float(f64::INFINITY);
        }
        "-.inf" | "-.Inf" | "-.INF" => return Plain::Float(f64::NEG_INFINITY),
        ".nan" | ".NaN" | ".NAN" => return Plain::Float(f64::NAN),
        _ => {}
    }
    let bytes = text.as_bytes();
    if let Some(rest) = text.strip_prefix("0x") {
        if !rest.is_empty() && rest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            if let Ok(number) = i64::from_str_radix(rest, 16) {
                return Plain::Integer(number);
            }
        }
        return Plain::Str;
    }
    if let Some(rest) = text.strip_prefix("0o") {
        if !rest.is_empty() && rest.bytes().all(|byte| (b'0'..=b'7').contains(&byte)) {
            if let Ok(number) = i64::from_str_radix(rest, 8) {
                return Plain::Integer(number);
            }
        }
        return Plain::Str;
    }
    let unsigned = match bytes.first() {
        Some(b'+') | Some(b'-') => &text[1..],
        _ => text,
    };
    if !unsigned.is_empty() && unsigned.bytes().all(|byte| byte.is_ascii_digit()) {
        let digits = text.strip_prefix('+').unwrap_or(text);
        if let Ok(number) = digits.parse::<i64>() {
            return Plain::Integer(number);
        }
        if let Ok(number) = digits.parse::<f64>() {
            return Plain::Float(number);
        }
        return Plain::Str;
    }
    if looks_like_float(unsigned) {
        let digits = text.strip_prefix('+').unwrap_or(text);
        if let Ok(number) = digits.parse::<f64>() {
            return Plain::Float(number);
        }
    }
    Plain::Str
}

/// `[0-9]+(\.[0-9]*)?([eE][-+]?[0-9]+)?` or `\.[0-9]+([eE][-+]?[0-9]+)?`.
fn looks_like_float(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut at = 0;
    let integer_digits = bytes
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    at += integer_digits;
    let mut fraction_digits = 0;
    if bytes.get(at) == Some(&b'.') {
        at += 1;
        fraction_digits = bytes[at..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        at += fraction_digits;
    }
    if integer_digits == 0 && fraction_digits == 0 {
        return false;
    }
    let has_dot = integer_digits + fraction_digits < at;
    if matches!(bytes.get(at), Some(b'e') | Some(b'E')) {
        at += 1;
        if matches!(bytes.get(at), Some(b'+') | Some(b'-')) {
            at += 1;
        }
        let exponent_digits = bytes[at..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        if exponent_digits == 0 {
            return false;
        }
        at += exponent_digits;
        return at == bytes.len();
    }
    has_dot && at == bytes.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(text: &str) -> String {
        let parsed = parse(text).unwrap();
        assert_eq!(parsed.documents.len(), 1, "{text}");
        compact_text(&parsed.tree, parsed.documents[0])
    }

    #[test]
    fn core_schema_resolves_plain_scalars() {
        assert_eq!(resolve_plain("12"), Plain::Integer(12));
        assert_eq!(resolve_plain("-0x1f"), Plain::Str);
        assert_eq!(resolve_plain("1e3"), Plain::Float(1000.0));
        assert_eq!(resolve_plain("1."), Plain::Float(1.0));
        assert_eq!(resolve_plain(".5e1"), Plain::Float(5.0));
        assert_eq!(resolve_plain("e5"), Plain::Str);
        assert_eq!(resolve_plain("1.2.3"), Plain::Str);
        assert_eq!(resolve_plain("NULL"), Plain::Null);
        assert_eq!(resolve_plain("yes"), Plain::Str);
    }

    #[test]
    fn sequence_under_key_may_share_its_indent() {
        assert_eq!(one("a:\n- 1\n- 2\nb: 3\n"), r#"{"a":[1,2],"b":3}"#);
    }

    #[test]
    fn compact_mapping_in_sequence_entry() {
        assert_eq!(one("- a: 1\n  b: 2\n- c\n"), r#"[{"a":1,"b":2},"c"]"#);
    }

    #[test]
    fn merges_take_their_position_and_own_keys_win() {
        assert_eq!(
            one("b: &b {x: 1, y: 2}\nm:\n  <<: *b\n  y: 3\n  z: 4\n"),
            r#"{"b":{"x":1,"y":2},"m":{"x":1,"y":3,"z":4}}"#
        );
    }

    #[test]
    fn errors_carry_the_line() {
        let error = parse("a: 1\nb: [\n").unwrap_err();
        assert_eq!(error.location.line, 2);
    }

    #[test]
    fn a_big_mapping_still_catches_duplicates() {
        let mut text = String::new();
        for index in 0..40 {
            text.push_str(&format!("k{index}: {index}\n"));
        }
        assert!(parse(&text).is_ok());
        text.push_str("k7: 0\n");
        assert!(parse(&text).is_err());
    }
}
