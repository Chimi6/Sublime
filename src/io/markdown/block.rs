//! Block structure. One pass over the lines builds a tree of open blocks the
//! way the CommonMark specification's appendix describes: match the open
//! containers, try new block starts, then add the line's text to the tip.
//! Inline content stays as text here; `inline` parses it later.

use std::collections::HashMap;

use super::Alignment;
use crate::io::scan::{find_byte, find_line_ending};

use super::scan::{
    normalize_label, parse_reference_definition, scan_html_block_end, scan_html_block_start,
    unescape_and_decode_cow,
};

const TAB_STOP: usize = 4;
const CODE_INDENT: usize = 4;

/// Which extensions are enabled. Everything is on by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    pub tables: bool,
    pub strikethrough: bool,
    pub task_lists: bool,
    pub autolinks: bool,
    pub tagfilter: bool,
    pub footnotes: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            tables: true,
            strikethrough: true,
            task_lists: true,
            autolinks: true,
            tagfilter: true,
            footnotes: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListData {
    pub ordered: bool,
    pub start: u64,
    /// Bullet character, or the delimiter (`.` or `)`) for ordered lists.
    pub marker: u8,
    pub tight: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemData {
    pub marker_offset: u32,
    pub padding: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableData {
    pub alignments: Vec<Alignment>,
    /// Every row's cells in one vector, header row first, already split
    /// and trimmed; `row_ends[i]` is where row `i` stops.
    pub cells: Vec<Cell>,
    pub row_ends: Vec<u32>,
}

impl TableData {
    /// The rows, header first.
    pub fn rows(&self) -> impl Iterator<Item = &[Cell]> {
        let mut start = 0usize;
        self.row_ends.iter().map(move |end| {
            let row = &self.cells[start..*end as usize];
            start = *end as usize;
            row
        })
    }
}

/// A table cell: a trimmed range of the source, or an owned string when a
/// `\|` escape had to be resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cell {
    Range(usize, usize),
    Owned(String),
}

impl Cell {
    pub fn text<'a>(&self, source: &'a str) -> std::borrow::Cow<'a, str> {
        match self {
            Cell::Range(start, end) => std::borrow::Cow::Borrowed(&source[*start..*end]),
            Cell::Owned(text) => std::borrow::Cow::Owned(text.clone()),
        }
    }
}

/// One line of a leaf block's text: a byte range of the source plus spaces
/// synthesized from a partially consumed tab. Nothing is copied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Line {
    pub start: usize,
    pub end: usize,
    pub leading_spaces: u8,
}

impl Line {
    pub fn text<'a>(&self, source: &'a str) -> std::borrow::Cow<'a, str> {
        let text = &source[self.start..self.end];
        if self.leading_spaces == 0 {
            return std::borrow::Cow::Borrowed(text);
        }
        let mut owned = String::with_capacity(text.len() + usize::from(self.leading_spaces));
        for _ in 0..self.leading_spaces {
            owned.push(' ');
        }
        owned.push_str(text);
        std::borrow::Cow::Owned(owned)
    }

    pub fn len(&self) -> usize {
        self.end - self.start + usize::from(self.leading_spaces)
    }

    pub fn is_blank(&self, source: &str) -> bool {
        source[self.start..self.end]
            .bytes()
            .all(|byte| byte == b' ' || byte == b'\t')
    }
}

/// Joins lines with `\n`, borrowing whenever the lines are contiguous in
/// the source.
pub fn join_lines<'a>(source: &'a str, lines: &[Line]) -> std::borrow::Cow<'a, str> {
    let first = match lines.first() {
        Some(first) => first,
        None => return std::borrow::Cow::Borrowed(""),
    };
    // Lines that sit back to back in the source, separated by a single
    // `\n`, are already the joined text; that is every top-level paragraph.
    let mut contiguous = first.leading_spaces == 0;
    let mut previous_end = first.end;
    for line in &lines[1..] {
        let adjacent = line.leading_spaces == 0
            && line.start == previous_end + 1
            && source.as_bytes().get(previous_end) == Some(&b'\n');
        if !adjacent {
            contiguous = false;
            break;
        }
        previous_end = line.end;
    }
    if contiguous {
        return std::borrow::Cow::Borrowed(&source[first.start..previous_end]);
    }
    let total: usize = lines.iter().map(|line| line.len() + 1).sum();
    let mut joined = String::with_capacity(total);
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            joined.push('\n');
        }
        joined.push_str(&line.text(source));
    }
    std::borrow::Cow::Owned(joined)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Document,
    BlockQuote,
    List(ListData),
    Item(ItemData),
    Paragraph,
    Heading {
        level: u8,
        setext: bool,
    },
    ThematicBreak,
    IndentedCode,
    FencedCode {
        fence_char: u8,
        fence_length: u16,
        fence_offset: u8,
        closed: bool,
        info: Box<str>,
    },
    HtmlBlock {
        kind: u8,
    },
    Table(Box<TableData>),
    FootnoteDefinition {
        label: Box<str>,
    },
}

/// The parts of a node's kind the per-line algorithm needs, copied out so
/// no line ever clones a `Kind` (which can hold table rows).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    Document,
    BlockQuote,
    List,
    Item {
        marker_offset: usize,
        padding: usize,
    },
    Paragraph,
    Heading {
        setext: bool,
    },
    ThematicBreak,
    IndentedCode,
    FencedCode {
        fence_char: u8,
        fence_length: usize,
        fence_offset: usize,
    },
    HtmlBlock {
        kind: u8,
    },
    Table,
    FootnoteDefinition,
}

/// Index meaning "no node" in the sibling and parent links.
pub const NONE: u32 = u32::MAX;

#[derive(Debug)]
pub struct Node {
    pub kind: Kind,
    pub parent: u32,
    pub first_child: u32,
    pub last_child: u32,
    pub next_sibling: u32,
    /// This block's text lines: a range of `Document::lines`.
    pub lines_start: u32,
    pub lines_len: u32,
    pub start_line: u32,
    pub open: bool,
    pub last_line_blank: bool,
    /// Set when a paragraph turned out to hold only reference definitions.
    pub deleted: bool,
}

impl Node {
    /// An open, childless node; `add_child` fills in the rest in place.
    const TEMPLATE: Node = Node {
        kind: Kind::Document,
        parent: NONE,
        first_child: NONE,
        last_child: NONE,
        next_sibling: NONE,
        lines_start: 0,
        lines_len: 0,
        start_line: 0,
        open: true,
        last_line_blank: false,
        deleted: false,
    };
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub destination: String,
    pub title: String,
}

pub struct Document {
    pub nodes: Vec<Node>,
    /// Every leaf block's lines, contiguous per block. One arena instead of
    /// a vector per node.
    pub lines: Vec<Line>,
    pub references: HashMap<String, Reference>,
    pub footnote_labels: HashMap<String, String>,
    pub options: Options,
}

impl Document {
    pub fn lines_of(&self, node: usize) -> &[Line] {
        let start = self.nodes[node].lines_start as usize;
        let end = start + self.nodes[node].lines_len as usize;
        &self.lines[start..end]
    }

    /// Children of `node` in order.
    pub fn children(&self, node: usize) -> Children<'_> {
        Children {
            document: self,
            next: self.nodes[node].first_child,
        }
    }
}

pub struct Children<'d> {
    document: &'d Document,
    next: u32,
}

impl Iterator for Children<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        if self.next == NONE {
            return None;
        }
        let current = self.next as usize;
        self.next = self.document.nodes[current].next_sibling;
        Some(current)
    }
}

/// Feeds the document to the block parser one line at a time. Lines end
/// at `\n`, `\r\n`, or `\r`; a final unterminated line counts, and a
/// terminator at the very end does not add an empty line.
pub fn parse(text: &str, options: Options) -> Document {
    let bytes = text.as_bytes();
    let estimated_lines = bytes.len() / 40 + 1;
    let mut parser = BlockParser::new(text, options, estimated_lines);
    let mut line_number = 0usize;
    let mut start = 0usize;
    while start < bytes.len() {
        let (end, next) = match find_line_ending(&bytes[start..]) {
            Some(relative) => {
                let end = start + relative;
                let mut next = end + 1;
                if bytes[end] == b'\r' && next < bytes.len() && bytes[next] == b'\n' {
                    next += 1;
                }
                (end, next)
            }
            None => (bytes.len(), bytes.len()),
        };
        line_number += 1;
        parser.process_line(start, &text[start..end], line_number);
        start = next;
    }
    parser.finish()
}

struct BlockParser<'s> {
    source: &'s str,
    options: Options,
    nodes: Vec<Node>,
    lines: Vec<Line>,
    /// The deepest open block.
    tip: usize,
    references: HashMap<String, Reference>,
    footnote_labels: HashMap<String, String>,
    // Per-line scanning state.
    offset: usize,
    column: usize,
    partially_consumed_tab: bool,
    first_nonspace: usize,
    first_nonspace_column: usize,
    indent: usize,
    blank: bool,
    line_number: usize,
    try_open_table_result: usize,
}

impl<'s> BlockParser<'s> {
    fn new(source: &'s str, options: Options, line_count: usize) -> Self {
        let root = Node {
            kind: Kind::Document,
            parent: NONE,
            first_child: NONE,
            last_child: NONE,
            next_sibling: NONE,
            lines_start: 0,
            lines_len: 0,
            start_line: 1,
            open: true,
            last_line_blank: false,
            deleted: false,
        };
        let mut nodes = Vec::with_capacity(line_count / 2 + 8);
        nodes.push(root);
        BlockParser {
            source,
            options,
            nodes,
            lines: Vec::with_capacity(line_count),
            tip: 0,
            references: HashMap::new(),
            footnote_labels: HashMap::new(),
            offset: 0,
            column: 0,
            partially_consumed_tab: false,
            first_nonspace: 0,
            first_nonspace_column: 0,
            indent: 0,
            blank: false,
            line_number: 0,
            try_open_table_result: 0,
        }
    }

    fn finish(mut self) -> Document {
        while let Some(parent) = self.finalize(self.tip) {
            self.tip = parent;
        }
        Document {
            nodes: self.nodes,
            lines: self.lines,
            references: self.references,
            footnote_labels: self.footnote_labels,
            options: self.options,
        }
    }

    // ----- line scanning helpers -----

    fn peek(&self, line: &[u8], at: usize) -> u8 {
        line.get(at).copied().unwrap_or(b'\n')
    }

    fn advance_offset(&mut self, line: &[u8], mut count: usize, columns: bool) {
        while count > 0 && self.offset < line.len() {
            let byte = line[self.offset];
            if byte == b'\t' {
                let chars_to_tab = TAB_STOP - (self.column % TAB_STOP);
                if columns {
                    self.partially_consumed_tab = chars_to_tab > count;
                    let chars_to_advance = chars_to_tab.min(count);
                    self.column += chars_to_advance;
                    if !self.partially_consumed_tab {
                        self.offset += 1;
                    }
                    count -= chars_to_advance;
                } else {
                    self.partially_consumed_tab = false;
                    self.column += chars_to_tab;
                    self.offset += 1;
                    count -= 1;
                }
            } else {
                self.partially_consumed_tab = false;
                self.offset += 1;
                self.column += 1;
                count -= 1;
            }
        }
    }

    fn find_first_nonspace(&mut self, line: &[u8]) {
        let mut chars_to_tab = TAB_STOP - (self.column % TAB_STOP);
        self.first_nonspace = self.offset;
        self.first_nonspace_column = self.column;
        loop {
            match line.get(self.first_nonspace) {
                Some(b' ') => {
                    self.first_nonspace += 1;
                    self.first_nonspace_column += 1;
                    chars_to_tab -= 1;
                    if chars_to_tab == 0 {
                        chars_to_tab = TAB_STOP;
                    }
                }
                Some(b'\t') => {
                    self.first_nonspace += 1;
                    self.first_nonspace_column += chars_to_tab;
                    chars_to_tab = TAB_STOP;
                }
                _ => break,
            }
        }
        self.indent = self.first_nonspace_column - self.column;
        self.blank = self.first_nonspace >= line.len();
    }

    // ----- tree helpers -----

    fn kind(&self, node: usize) -> &Kind {
        &self.nodes[node].kind
    }

    fn shape(&self, node: usize) -> Shape {
        match &self.nodes[node].kind {
            Kind::Document => Shape::Document,
            Kind::BlockQuote => Shape::BlockQuote,
            Kind::List(_) => Shape::List,
            Kind::Item(data) => Shape::Item {
                marker_offset: data.marker_offset as usize,
                padding: data.padding as usize,
            },
            Kind::Paragraph => Shape::Paragraph,
            Kind::Heading { setext, .. } => Shape::Heading { setext: *setext },
            Kind::ThematicBreak => Shape::ThematicBreak,
            Kind::IndentedCode => Shape::IndentedCode,
            Kind::FencedCode {
                fence_char,
                fence_length,
                fence_offset,
                ..
            } => Shape::FencedCode {
                fence_char: *fence_char,
                fence_length: *fence_length as usize,
                fence_offset: *fence_offset as usize,
            },
            Kind::HtmlBlock { kind } => Shape::HtmlBlock { kind: *kind },
            Kind::Table(_) => Shape::Table,
            Kind::FootnoteDefinition { .. } => Shape::FootnoteDefinition,
        }
    }

    fn last_open_child(&self, node: usize) -> Option<usize> {
        let last = self.nodes[node].last_child;
        if last == NONE {
            return None;
        }
        let last = last as usize;
        if self.nodes[last].open {
            Some(last)
        } else {
            None
        }
    }

    fn can_contain(&self, parent: usize, child: &Kind) -> bool {
        match self.kind(parent) {
            Kind::Document | Kind::BlockQuote | Kind::Item(_) | Kind::FootnoteDefinition { .. } => {
                !matches!(child, Kind::Item(_))
            }
            Kind::List(_) => matches!(child, Kind::Item(_)),
            _ => false,
        }
    }

    fn accepts_lines(&self, node: usize) -> bool {
        matches!(
            self.kind(node),
            Kind::Paragraph
                | Kind::Heading { .. }
                | Kind::IndentedCode
                | Kind::FencedCode { .. }
                | Kind::HtmlBlock { .. }
                | Kind::Table(_)
        )
    }

    fn add_child(&mut self, mut parent: usize, kind: Kind) -> usize {
        while !self.can_contain(parent, &kind) {
            parent = match self.finalize(parent) {
                Some(grandparent) => grandparent,
                None => break,
            };
        }
        // Push a constant template and fill the slot in place: building the
        // node on the stack and copying it stalls on store forwarding.
        let index = self.nodes.len();
        self.nodes.push(Node::TEMPLATE);
        let node = &mut self.nodes[index];
        // The template's kind owns nothing, so skipping its drop glue is safe.
        std::mem::forget(std::mem::replace(&mut node.kind, kind));
        node.parent = parent as u32;
        node.start_line = self.line_number as u32;
        let last = self.nodes[parent].last_child;
        if last == NONE {
            self.nodes[parent].first_child = index as u32;
        } else {
            self.nodes[last as usize].next_sibling = index as u32;
        }
        self.nodes[parent].last_child = index as u32;
        index
    }

    /// Appends a line to `node`, which must be the block currently
    /// receiving lines, so its lines stay contiguous in the arena.
    fn push_line(&mut self, node: usize, line: Line) {
        if self.nodes[node].lines_len == 0 {
            self.nodes[node].lines_start = self.lines.len() as u32;
        }
        self.lines.push(line);
        self.nodes[node].lines_len += 1;
    }

    fn node_lines(&self, node: usize) -> &[Line] {
        let start = self.nodes[node].lines_start as usize;
        let end = start + self.nodes[node].lines_len as usize;
        &self.lines[start..end]
    }

    fn add_line(&mut self, node: usize, line_start: usize, line: &[u8]) {
        let mut start = line_start + self.offset;
        let mut leading_spaces = 0u8;
        if self.partially_consumed_tab {
            start += 1;
            leading_spaces = (TAB_STOP - (self.column % TAB_STOP)) as u8;
        }
        let end = line_start + line.len();
        let start = start.min(end);
        self.push_line(
            node,
            Line {
                start,
                end,
                leading_spaces,
            },
        );
    }

    /// Closes `node`, post-processing its content. Returns its parent.
    fn finalize(&mut self, node: usize) -> Option<usize> {
        let parent = match self.nodes[node].parent {
            NONE => None,
            parent => Some(parent as usize),
        };
        if !self.nodes[node].open {
            return parent;
        }
        self.nodes[node].open = false;
        match &self.nodes[node].kind {
            Kind::Paragraph => {
                let has_content = self.resolve_reference_definitions(node);
                if !has_content {
                    self.nodes[node].deleted = true;
                }
            }
            Kind::IndentedCode => {
                while let Some(last) = self.node_lines(node).last() {
                    if !last.is_blank(self.source) {
                        break;
                    }
                    self.nodes[node].lines_len -= 1;
                }
            }
            Kind::FencedCode { .. } => {
                let info: Box<str> = match self.node_lines(node).first() {
                    Some(first) => {
                        Box::from(&*unescape_and_decode_cow(first.text(self.source).trim()))
                    }
                    None => Box::from(""),
                };
                if self.nodes[node].lines_len > 0 {
                    self.nodes[node].lines_start += 1;
                    self.nodes[node].lines_len -= 1;
                }
                if let Kind::FencedCode { info: slot, .. } = &mut self.nodes[node].kind {
                    *slot = info;
                }
            }
            Kind::List(_) => self.finalize_list(node),
            _ => {}
        }
        parent
    }

    fn finalize_list(&mut self, list: usize) {
        let mut tight = true;
        let mut item = self.nodes[list].first_child;
        while item != NONE {
            let item_index = item as usize;
            let is_last_item = self.nodes[item_index].next_sibling == NONE;
            if self.nodes[item_index].last_line_blank && !is_last_item {
                tight = false;
                break;
            }
            let mut child = self.nodes[item_index].first_child;
            while child != NONE {
                let child_index = child as usize;
                let is_last_child = self.nodes[child_index].next_sibling == NONE;
                if self.ends_with_blank_line(child_index) && (!is_last_item || !is_last_child) {
                    tight = false;
                    break;
                }
                child = self.nodes[child_index].next_sibling;
            }
            if !tight {
                break;
            }
            item = self.nodes[item_index].next_sibling;
        }
        if let Kind::List(data) = &mut self.nodes[list].kind {
            data.tight = tight;
        }
    }

    fn ends_with_blank_line(&self, mut node: usize) -> bool {
        loop {
            if self.nodes[node].last_line_blank {
                return true;
            }
            match self.kind(node) {
                Kind::List(_) | Kind::Item(_) => {
                    let last = self.nodes[node].last_child;
                    if last == NONE {
                        return false;
                    }
                    node = last as usize;
                }
                _ => return false,
            }
        }
    }

    /// Consumes leading link reference definitions from a paragraph.
    /// Returns whether any content remains.
    fn resolve_reference_definitions(&mut self, node: usize) -> bool {
        loop {
            let lines = self.node_lines(node);
            let first = match lines.first() {
                Some(first) => *first,
                None => break,
            };
            if first.leading_spaces > 0 || !self.source[first.start..first.end].starts_with('[') {
                break;
            }
            if !self.has_label_close(node) {
                break;
            }
            let mut joined = join_lines(self.source, lines).into_owned();
            joined.push('\n');
            let parsed = match parse_reference_definition(&joined) {
                Some(parsed) => parsed,
                None => break,
            };
            let (label, destination, title, consumed) = parsed;
            let key = normalize_label(&label);
            self.references
                .entry(key)
                .or_insert(Reference { destination, title });
            // The definition always ends at a line end, so drop whole lines.
            let mut covered = 0usize;
            let mut removed = 0u32;
            for line in self.node_lines(node) {
                if covered >= consumed {
                    break;
                }
                covered += line.len() + 1;
                removed += 1;
            }
            self.nodes[node].lines_start += removed;
            self.nodes[node].lines_len -= removed;
        }
        let source = self.source;
        self.node_lines(node)
            .iter()
            .any(|line| !line.is_blank(source))
    }

    /// A definition's label always ends in `]:` within one line; a
    /// paragraph without that pair cannot hold one, so it is not joined.
    fn has_label_close(&self, node: usize) -> bool {
        let source = self.source.as_bytes();
        self.node_lines(node).iter().any(|line| {
            let bytes = &source[line.start..line.end];
            let mut from = 0usize;
            while let Some(relative) = find_byte(&bytes[from..], b']') {
                let index = from + relative;
                if bytes.get(index + 1) == Some(&b':') {
                    return true;
                }
                from = index + 1;
            }
            false
        })
    }

    // ----- the per-line algorithm -----

    fn process_line(&mut self, line_start: usize, raw_line: &str, line_number: usize) {
        self.line_number = line_number;
        let line = raw_line.as_bytes();
        self.offset = 0;
        self.column = 0;
        self.partially_consumed_tab = false;

        // 1. Match open containers.
        let mut container = 0usize;
        let mut line_consumed = false;
        while let Some(child) = self.last_open_child(container) {
            container = child;
            self.find_first_nonspace(line);
            let matched = match self.shape(container) {
                Shape::BlockQuote => {
                    if !self.indented() && self.peek(line, self.first_nonspace) == b'>' {
                        let to_marker = self.first_nonspace + 1 - self.offset;
                        self.advance_offset(line, to_marker, false);
                        if is_space_or_tab(self.peek(line, self.offset)) {
                            self.advance_offset(line, 1, true);
                        }
                        true
                    } else {
                        false
                    }
                }
                Shape::Item {
                    marker_offset,
                    padding,
                } => {
                    if self.blank {
                        let has_children = self.nodes[container].first_child != NONE;
                        if has_children {
                            let to_end = self.first_nonspace - self.offset;
                            self.advance_offset(line, to_end, false);
                            true
                        } else {
                            false
                        }
                    } else if self.indent >= marker_offset + padding {
                        self.advance_offset(line, marker_offset + padding, true);
                        true
                    } else {
                        false
                    }
                }
                Shape::FootnoteDefinition => {
                    if self.blank {
                        let to_end = self.first_nonspace - self.offset;
                        self.advance_offset(line, to_end, false);
                        true
                    } else if self.indent >= CODE_INDENT {
                        self.advance_offset(line, CODE_INDENT, true);
                        true
                    } else {
                        false
                    }
                }
                Shape::IndentedCode => {
                    if self.indent >= CODE_INDENT {
                        self.advance_offset(line, CODE_INDENT, true);
                        true
                    } else if self.blank {
                        let to_end = self.first_nonspace - self.offset;
                        self.advance_offset(line, to_end, false);
                        true
                    } else {
                        false
                    }
                }
                Shape::FencedCode {
                    fence_char,
                    fence_length,
                    fence_offset,
                } => {
                    let closes = !self.indented()
                        && self.peek(line, self.first_nonspace) == fence_char
                        && scan_closing_fence(line, self.first_nonspace, fence_char)
                            >= fence_length;
                    if closes {
                        let to_end = line.len().saturating_sub(self.offset);
                        self.advance_offset(line, to_end, false);
                        if let Kind::FencedCode { closed, .. } = &mut self.nodes[container].kind {
                            *closed = true;
                        }
                        let parent = self.finalize(container);
                        self.tip = parent.unwrap_or(0);
                        line_consumed = true;
                        true
                    } else {
                        let mut remaining = fence_offset;
                        while remaining > 0 && is_space_or_tab(self.peek(line, self.offset)) {
                            self.advance_offset(line, 1, true);
                            remaining -= 1;
                        }
                        true
                    }
                }
                Shape::HtmlBlock { kind } => !(self.blank && (kind == 6 || kind == 7)),
                Shape::Paragraph => !self.blank,
                Shape::Table => !self.blank,
                Shape::Heading { .. } | Shape::ThematicBreak => false,
                Shape::Document | Shape::List => true,
            };
            if line_consumed {
                return;
            }
            if !matched {
                container = match self.nodes[container].parent {
                    NONE => 0,
                    parent => parent as usize,
                };
                break;
            }
        }
        let last_matched_container = container;

        // 2. Try new block starts.
        let maybe_lazy = matches!(self.shape(self.tip), Shape::Paragraph);
        loop {
            let is_code_or_html = matches!(
                self.shape(container),
                Shape::IndentedCode | Shape::FencedCode { .. } | Shape::HtmlBlock { .. }
            );
            if is_code_or_html {
                break;
            }
            self.find_first_nonspace(line);
            let indented = self.indented();
            let first = self.peek(line, self.first_nonspace);
            let in_paragraph = matches!(self.shape(container), Shape::Paragraph);

            if !indented && first == b'>' {
                let to_marker = self.first_nonspace + 1 - self.offset;
                self.advance_offset(line, to_marker, false);
                if is_space_or_tab(self.peek(line, self.offset)) {
                    self.advance_offset(line, 1, true);
                }
                container = self.add_child(container, Kind::BlockQuote);
            } else if !indented
                && first == b'#'
                && scan_atx_heading(line, self.first_nonspace).is_some()
            {
                let level = scan_atx_heading(line, self.first_nonspace).unwrap_or(1);
                let to_content = self.first_nonspace + level as usize - self.offset;
                self.advance_offset(line, to_content, false);
                container = self.add_child(
                    container,
                    Kind::Heading {
                        level,
                        setext: false,
                    },
                );
            } else if !indented
                && (first == b'`' || first == b'~')
                && scan_opening_fence(line, self.first_nonspace).is_some()
            {
                let (fence_char, fence_length) =
                    scan_opening_fence(line, self.first_nonspace).unwrap_or((b'`', 3));
                let fence_offset = self.first_nonspace - self.offset;
                let kind = Kind::FencedCode {
                    fence_char,
                    fence_length: fence_length.min(u16::MAX as usize) as u16,
                    fence_offset: fence_offset as u8,
                    closed: false,
                    info: Box::from(""),
                };
                container = self.add_child(container, kind);
                self.advance_offset(
                    line,
                    self.first_nonspace + fence_length - self.offset,
                    false,
                );
            } else if !indented
                && first == b'<'
                && scan_html_block_start(&line[self.first_nonspace..], in_paragraph).is_some()
            {
                let kind =
                    scan_html_block_start(&line[self.first_nonspace..], in_paragraph).unwrap_or(7);
                container = self.add_child(container, Kind::HtmlBlock { kind });
            } else if !indented
                && in_paragraph
                && (first == b'=' || first == b'-')
                && scan_setext_underline(line, self.first_nonspace).is_some()
                && self.resolve_reference_definitions(container)
            {
                let level = scan_setext_underline(line, self.first_nonspace).unwrap_or(1);
                self.nodes[container].kind = Kind::Heading {
                    level,
                    setext: true,
                };
                let to_end = line.len().saturating_sub(self.offset);
                self.advance_offset(line, to_end, false);
            } else if !indented
                && in_paragraph
                && self.options.tables
                && self.try_open_table(container, line).is_some()
            {
                container = self.try_open_table_result;
                let to_end = line.len().saturating_sub(self.offset);
                self.advance_offset(line, to_end, false);
            } else if !indented
                && (first == b'*' || first == b'-' || first == b'_')
                && scan_thematic_break(line, self.first_nonspace)
            {
                container = self.add_child(container, Kind::ThematicBreak);
                let to_end = line.len().saturating_sub(self.offset);
                self.advance_offset(line, to_end, false);
            } else if !indented
                && self.options.footnotes
                && first == b'['
                && scan_footnote_definition(line, self.first_nonspace).is_some()
            {
                let (label, consumed) =
                    scan_footnote_definition(line, self.first_nonspace).unwrap_or_default();
                let to_content = self.first_nonspace + consumed - self.offset;
                self.advance_offset(line, to_content, false);
                while is_space_or_tab(self.peek(line, self.offset)) {
                    self.advance_offset(line, 1, true);
                }
                let key = normalize_label(&label);
                self.footnote_labels.entry(key).or_insert(label.clone());
                container = self.add_child(
                    container,
                    Kind::FootnoteDefinition {
                        label: label.into_boxed_str(),
                    },
                );
            } else if self.indent < CODE_INDENT
                && parse_list_marker(line, self.first_nonspace, in_paragraph).is_some()
            {
                let (ordered, start, marker, marker_width) =
                    parse_list_marker(line, self.first_nonspace, in_paragraph)
                        .unwrap_or((false, 0, b'-', 1));
                let to_marker = self.first_nonspace - self.offset;
                self.advance_offset(line, to_marker, false);
                self.advance_offset(line, marker_width, false);
                let saved_partial = self.partially_consumed_tab;
                let saved_offset = self.offset;
                let saved_column = self.column;
                while self.column - saved_column <= 5
                    && is_space_or_tab(self.peek(line, self.offset))
                {
                    self.advance_offset(line, 1, true);
                }
                let spaces = self.column - saved_column;
                let padding = if !(1..5).contains(&spaces) || self.peek(line, self.offset) == b'\n'
                {
                    self.offset = saved_offset;
                    self.column = saved_column;
                    self.partially_consumed_tab = saved_partial;
                    if spaces > 0 {
                        self.advance_offset(line, 1, true);
                    }
                    marker_width + 1
                } else {
                    marker_width + spaces
                };
                let marker_offset = self.indent;
                let same_list = match self.kind(container) {
                    Kind::List(data) => data.ordered == ordered && data.marker == marker,
                    _ => false,
                };
                if !same_list {
                    let data = ListData {
                        ordered,
                        start,
                        marker,
                        tight: true,
                    };
                    container = self.add_child(container, Kind::List(data));
                }
                container = self.add_child(
                    container,
                    Kind::Item(ItemData {
                        marker_offset: marker_offset as u32,
                        padding: padding as u32,
                    }),
                );
            } else if indented && !maybe_lazy && !self.blank {
                self.advance_offset(line, CODE_INDENT, true);
                container = self.add_child(container, Kind::IndentedCode);
            } else {
                break;
            }
            if self.accepts_lines(container) {
                break;
            }
        }

        // 3. Add the line to the right block.
        self.find_first_nonspace(line);
        if self.blank {
            let last = self.nodes[container].last_child;
            if last != NONE {
                self.nodes[last as usize].last_line_blank = true;
            }
        }
        let shape = self.shape(container);
        let never_blank = matches!(
            shape,
            Shape::BlockQuote
                | Shape::Heading { .. }
                | Shape::ThematicBreak
                | Shape::FencedCode { .. }
        );
        let is_empty_new_item = matches!(shape, Shape::Item { .. })
            && self.nodes[container].first_child == NONE
            && self.nodes[container].start_line as usize == self.line_number;
        let last_line_blank = self.blank && !never_blank && !is_empty_new_item;
        self.nodes[container].last_line_blank = last_line_blank;
        let mut ancestor = self.nodes[container].parent;
        while ancestor != NONE {
            let node = ancestor as usize;
            self.nodes[node].last_line_blank = false;
            ancestor = self.nodes[node].parent;
        }

        let tip_is_paragraph = matches!(self.shape(self.tip), Shape::Paragraph);
        let is_lazy = self.tip != last_matched_container
            && container == last_matched_container
            && !self.blank
            && tip_is_paragraph;
        if is_lazy {
            let tip = self.tip;
            self.add_line(tip, line_start, line);
            return;
        }

        while self.tip != last_matched_container {
            match self.finalize(self.tip) {
                Some(parent) => self.tip = parent,
                None => break,
            }
        }

        match self.shape(container) {
            Shape::IndentedCode | Shape::FencedCode { .. } => {
                self.add_line(container, line_start, line)
            }
            Shape::HtmlBlock { kind } => {
                self.add_line(container, line_start, line);
                if scan_html_block_end(kind, &line[self.first_nonspace.min(line.len())..]) {
                    let parent = self.finalize(container);
                    self.tip = parent.unwrap_or(0);
                    return;
                }
            }
            Shape::Table => {
                if !self.blank {
                    let cell_start = self.first_nonspace.min(line.len());
                    let rest = &raw_line[cell_start..];
                    if let Kind::Table(data) = &mut self.nodes[container].kind {
                        split_table_row_into(rest, Some(line_start + cell_start), &mut data.cells);
                        data.row_ends.push(data.cells.len() as u32);
                    }
                }
            }
            _ if self.blank => {}
            Shape::Heading { setext: false } => {
                let to_content = self.first_nonspace - self.offset;
                self.advance_offset(line, to_content, false);
                let rest = &raw_line[self.offset.min(line.len())..];
                let content = strip_atx_closing(rest);
                if content.is_empty() {
                    self.tip = container;
                    return;
                }
                // `content` is a subslice of `rest`; recover its offsets.
                let skipped = content.as_ptr() as usize - rest.as_ptr() as usize;
                let start = line_start + self.offset + skipped;
                let end = start + content.len();
                self.push_line(
                    container,
                    Line {
                        start,
                        end,
                        leading_spaces: 0,
                    },
                );
            }
            Shape::Paragraph | Shape::Heading { .. } => {
                let to_content = self.first_nonspace - self.offset;
                self.advance_offset(line, to_content, false);
                self.add_line(container, line_start, line);
            }
            _ => {
                let paragraph = self.add_child(container, Kind::Paragraph);
                let to_content = self.first_nonspace - self.offset;
                self.advance_offset(line, to_content, false);
                self.add_line(paragraph, line_start, line);
                container = paragraph;
            }
        }
        self.tip = container;
    }

    fn indented(&self) -> bool {
        self.indent >= CODE_INDENT
    }

    /// GFM table: the current line is a delimiter row and the paragraph's
    /// last line has the same number of cells.
    fn try_open_table(&mut self, paragraph: usize, line: &[u8]) -> Option<usize> {
        let rest = std::str::from_utf8(&line[self.first_nonspace.min(line.len())..]).ok()?;
        let alignments = parse_delimiter_row(rest)?;
        let header_line = *self.node_lines(paragraph).last()?;
        let header_base = if header_line.leading_spaces == 0 {
            Some(header_line.start)
        } else {
            None
        };
        let header = split_table_row(&header_line.text(self.source), header_base);
        if header.len() != alignments.len() {
            return None;
        }
        let parent = match self.nodes[paragraph].parent {
            NONE => 0,
            parent => parent as usize,
        };
        if self.nodes[paragraph].lines_len > 1 {
            self.nodes[paragraph].lines_len -= 1;
            self.finalize(paragraph);
        } else {
            self.nodes[paragraph].open = false;
            self.nodes[paragraph].deleted = true;
        }
        let header_end = header.len() as u32;
        let table = Kind::Table(Box::new(TableData {
            alignments,
            cells: header,
            row_ends: vec![header_end],
        }));
        let index = self.add_child(parent, table);
        self.try_open_table_result = index;
        Some(index)
    }
}

fn is_space_or_tab(byte: u8) -> bool {
    byte == b' ' || byte == b'\t'
}

/// `#{1,6}` followed by a space, tab, or end of line. Returns the level.
fn scan_atx_heading(line: &[u8], at: usize) -> Option<u8> {
    let mut count = 0usize;
    while line.get(at + count) == Some(&b'#') {
        count += 1;
    }
    if count == 0 || count > 6 {
        return None;
    }
    match line.get(at + count) {
        None | Some(b' ') | Some(b'\t') => Some(count as u8),
        _ => None,
    }
}

/// Removes an optional closing sequence of `#`s and surrounding whitespace.
fn strip_atx_closing(content: &str) -> &str {
    let trimmed = content.trim_end_matches([' ', '\t']);
    let without_hashes = trimmed.trim_end_matches('#');
    if without_hashes.len() == trimmed.len() {
        return trimmed.trim_start_matches([' ', '\t']);
    }
    if without_hashes.is_empty() {
        return "";
    }
    if without_hashes.ends_with([' ', '\t']) {
        return without_hashes.trim_matches([' ', '\t']);
    }
    trimmed.trim_start_matches([' ', '\t'])
}

/// Three or more backticks or tildes; a backtick fence's info string may
/// not contain backticks. Returns `(fence char, length)`.
fn scan_opening_fence(line: &[u8], at: usize) -> Option<(u8, usize)> {
    let fence_char = *line.get(at)?;
    let mut length = 0usize;
    while line.get(at + length) == Some(&fence_char) {
        length += 1;
    }
    if length < 3 {
        return None;
    }
    if fence_char == b'`' && line[at + length..].contains(&b'`') {
        return None;
    }
    Some((fence_char, length))
}

/// Length of a closing fence run followed only by spaces or tabs, else 0.
fn scan_closing_fence(line: &[u8], at: usize, fence_char: u8) -> usize {
    let mut length = 0usize;
    while line.get(at + length) == Some(&fence_char) {
        length += 1;
    }
    let rest_is_blank = line[at + length..]
        .iter()
        .all(|byte| is_space_or_tab(*byte));
    if rest_is_blank { length } else { 0 }
}

/// A run of `=` (level 1) or `-` (level 2) followed only by whitespace.
fn scan_setext_underline(line: &[u8], at: usize) -> Option<u8> {
    let marker = *line.get(at)?;
    let level = match marker {
        b'=' => 1,
        b'-' => 2,
        _ => return None,
    };
    let mut index = at;
    while line.get(index) == Some(&marker) {
        index += 1;
    }
    let rest_is_blank = line[index..].iter().all(|byte| is_space_or_tab(*byte));
    if rest_is_blank { Some(level) } else { None }
}

/// Three or more `*`, `-`, or `_` with optional spaces or tabs between.
fn scan_thematic_break(line: &[u8], at: usize) -> bool {
    let marker = match line.get(at) {
        Some(marker) => *marker,
        None => return false,
    };
    let mut count = 0usize;
    for byte in &line[at..] {
        if *byte == marker {
            count += 1;
        } else if !is_space_or_tab(*byte) {
            return false;
        }
    }
    count >= 3
}

/// `[^label]:` at `at`. Returns the label and the bytes consumed.
fn scan_footnote_definition(line: &[u8], at: usize) -> Option<(String, usize)> {
    if line.get(at) != Some(&b'[') || line.get(at + 1) != Some(&b'^') {
        return None;
    }
    let mut index = at + 2;
    while let Some(byte) = line.get(index) {
        if *byte == b']' {
            break;
        }
        if *byte == b'[' || is_space_or_tab(*byte) {
            return None;
        }
        index += 1;
    }
    if line.get(index) != Some(&b']') || line.get(index + 1) != Some(&b':') {
        return None;
    }
    if index == at + 2 {
        return None;
    }
    let label = std::str::from_utf8(&line[at + 2..index]).ok()?.to_string();
    Some((label, index + 2 - at))
}

/// Returns `(ordered, start, marker, marker width)` for a list marker at
/// `at`. When interrupting a paragraph, only `1.`/`1)` ordered markers and
/// non-empty items qualify.
fn parse_list_marker(
    line: &[u8],
    at: usize,
    interrupts_paragraph: bool,
) -> Option<(bool, u64, u8, usize)> {
    let first = *line.get(at)?;
    let (ordered, start, marker, width) = if first == b'*' || first == b'+' || first == b'-' {
        (false, 0u64, first, 1usize)
    } else if first.is_ascii_digit() {
        let mut digits = 0usize;
        let mut value: u64 = 0;
        while let Some(byte) = line.get(at + digits) {
            if !byte.is_ascii_digit() {
                break;
            }
            value = value * 10 + u64::from(byte - b'0');
            digits += 1;
            if digits > 9 {
                return None;
            }
        }
        let delimiter = *line.get(at + digits)?;
        if delimiter != b'.' && delimiter != b')' {
            return None;
        }
        if interrupts_paragraph && value != 1 {
            return None;
        }
        (true, value, delimiter, digits + 1)
    } else {
        return None;
    };
    let after = line.get(at + width).copied();
    let followed_ok = match after {
        None => true,
        Some(byte) => is_space_or_tab(byte),
    };
    if !followed_ok {
        return None;
    }
    if interrupts_paragraph {
        let rest_blank = line[at + width..].iter().all(|byte| is_space_or_tab(*byte));
        if rest_blank {
            return None;
        }
    }
    Some((ordered, start, marker, width))
}

/// GFM delimiter row: cells of `-`+ with optional leading/trailing `:`.
fn parse_delimiter_row(line: &str) -> Option<Vec<Alignment>> {
    // Cheap rejection before splitting: only pipes, dashes, colons, and
    // whitespace may appear, and at least one dash must.
    let only_delimiter_bytes = line
        .bytes()
        .all(|byte| matches!(byte, b'|' | b'-' | b':' | b' ' | b'\t'));
    if !only_delimiter_bytes || !line.contains('-') {
        return None;
    }
    let cells = split_table_row(line, Some(0));
    if cells.is_empty() {
        return None;
    }
    let trimmed = line.trim();
    if !trimmed.contains('-') {
        return None;
    }
    let mut alignments = Vec::with_capacity(cells.len());
    for cell in &cells {
        let cell_text = cell.text(line);
        let cell = cell_text.trim();
        let left = cell.starts_with(':');
        let right = cell.ends_with(':') && cell.len() > 1;
        let dashes = cell.trim_start_matches(':').trim_end_matches(':');
        if dashes.is_empty() || !dashes.bytes().all(|byte| byte == b'-') {
            return None;
        }
        let alignment = match (left, right) {
            (true, true) => Alignment::Center,
            (true, false) => Alignment::Left,
            (false, true) => Alignment::Right,
            (false, false) => Alignment::None,
        };
        alignments.push(alignment);
    }
    // A single-cell row must be delimited by pipes to count as a table.
    if cells.len() == 1 && !trimmed.starts_with('|') && !trimmed.ends_with('|') {
        return None;
    }
    Some(alignments)
}

/// Splits a table row into trimmed cells. Leading and trailing pipes are
/// optional; `\|` yields a literal pipe. Pipes inside code spans still split
/// (GFM requires escaping them). With `base`, plain cells are ranges of the
/// source starting there; without it, every cell is owned.
pub fn split_table_row(line: &str, base: Option<usize>) -> Vec<Cell> {
    let mut cells: Vec<Cell> = Vec::new();
    split_table_row_into(line, base, &mut cells);
    cells
}

/// `split_table_row` appending to `cells`, so a table's rows share one
/// vector instead of one allocation each.
pub fn split_table_row_into(line: &str, base: Option<usize>, cells: &mut Vec<Cell>) {
    let bytes = line.as_bytes();
    let mut index = 0usize;
    // Skip leading whitespace and one leading pipe.
    while index < bytes.len() && (bytes[index] == b' ' || bytes[index] == b'\t') {
        index += 1;
    }
    if index < bytes.len() && bytes[index] == b'|' {
        index += 1;
    }
    let mut end = bytes.len();
    while end > index && (bytes[end - 1] == b' ' || bytes[end - 1] == b'\t') {
        end -= 1;
    }
    let mut cell_start = index;
    let mut escaped = false;
    let mut needs_unescape = false;
    let mut ended_with_pipe = false;
    let mut cursor = index;
    let push_cell = |cells: &mut Vec<Cell>, start: usize, stop: usize, needs_unescape: bool| {
        let mut trimmed_start = start;
        let mut trimmed_end = stop;
        while trimmed_start < trimmed_end
            && (bytes[trimmed_start] == b' ' || bytes[trimmed_start] == b'\t')
        {
            trimmed_start += 1;
        }
        while trimmed_end > trimmed_start
            && (bytes[trimmed_end - 1] == b' ' || bytes[trimmed_end - 1] == b'\t')
        {
            trimmed_end -= 1;
        }
        let cell = match (needs_unescape, base) {
            (false, Some(base)) => Cell::Range(base + trimmed_start, base + trimmed_end),
            (false, None) => Cell::Owned(line[trimmed_start..trimmed_end].to_string()),
            (true, _) => Cell::Owned(line[trimmed_start..trimmed_end].replace("\\|", "|")),
        };
        cells.push(cell);
    };
    while cursor < end {
        let byte = bytes[cursor];
        ended_with_pipe = false;
        if escaped {
            if byte == b'|' {
                needs_unescape = true;
            }
            escaped = false;
            cursor += 1;
            continue;
        }
        if byte == b'\\' {
            escaped = true;
            cursor += 1;
            continue;
        }
        if byte == b'|' {
            push_cell(cells, cell_start, cursor, needs_unescape);
            needs_unescape = false;
            cursor += 1;
            cell_start = cursor;
            ended_with_pipe = true;
            continue;
        }
        cursor += 1;
    }
    let trailing_is_blank = line[cell_start..end].trim_matches([' ', '\t']).is_empty();
    if !(ended_with_pipe && trailing_is_blank) {
        push_cell(cells, cell_start, end, needs_unescape);
    }
}
