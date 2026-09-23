//! Block structure. One pass over the lines builds a tree of open blocks the
//! way the CommonMark specification's appendix describes: match the open
//! containers, try new block starts, then add the line's text to the tip.
//! Inline content stays as text here; `inline` parses it later.

use std::collections::HashMap;

use super::Alignment;
use super::scan::{
    normalize_label, parse_reference_definition, scan_html_block_end, scan_html_block_start,
    unescape_and_decode,
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
    pub marker_offset: usize,
    pub padding: usize,
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
        fence_length: usize,
        fence_offset: usize,
        info: String,
        closed: bool,
    },
    HtmlBlock {
        kind: u8,
    },
    Table {
        alignments: Vec<Alignment>,
        rows: Vec<Vec<String>>,
    },
    FootnoteDefinition {
        label: String,
    },
}

#[derive(Debug)]
pub struct Node {
    pub kind: Kind,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    /// Text lines for leaf blocks, each ending in `\n`.
    pub content: String,
    pub open: bool,
    pub last_line_blank: bool,
    pub start_line: usize,
    /// Set when a paragraph turned out to hold only reference definitions.
    pub deleted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub destination: String,
    pub title: String,
}

pub struct Document {
    pub nodes: Vec<Node>,
    pub references: HashMap<String, Reference>,
    pub footnote_labels: HashMap<String, String>,
    pub options: Options,
}

pub fn parse(text: &str, options: Options) -> Document {
    let mut parser = BlockParser::new(options);
    let mut line_number = 0usize;
    for raw_line in split_lines(text) {
        line_number += 1;
        parser.process_line(raw_line, line_number);
    }
    parser.finish()
}

/// Splits on `\n`, `\r\n`, or `\r`, dropping the terminators. A final
/// unterminated line counts; a terminator at the very end does not add an
/// empty line.
fn split_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let bytes = text.as_bytes();
    let mut start = 0usize;
    let mut index = 0usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'\n' {
            lines.push(&text[start..index]);
            index += 1;
            start = index;
        } else if byte == b'\r' {
            lines.push(&text[start..index]);
            index += 1;
            if index < bytes.len() && bytes[index] == b'\n' {
                index += 1;
            }
            start = index;
        } else {
            index += 1;
        }
    }
    if start < bytes.len() {
        lines.push(&text[start..]);
    }
    lines
}

struct BlockParser {
    options: Options,
    nodes: Vec<Node>,
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

impl BlockParser {
    fn new(options: Options) -> Self {
        let root = Node {
            kind: Kind::Document,
            parent: None,
            children: Vec::new(),
            content: String::new(),
            open: true,
            last_line_blank: false,
            start_line: 1,
            deleted: false,
        };
        BlockParser {
            options,
            nodes: vec![root],
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

    fn last_open_child(&self, node: usize) -> Option<usize> {
        let last = *self.nodes[node].children.last()?;
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
                | Kind::Table { .. }
        )
    }

    fn add_child(&mut self, mut parent: usize, kind: Kind) -> usize {
        while !self.can_contain(parent, &kind) {
            parent = match self.finalize(parent) {
                Some(grandparent) => grandparent,
                None => break,
            };
        }
        let node = Node {
            kind,
            parent: Some(parent),
            children: Vec::new(),
            content: String::new(),
            open: true,
            last_line_blank: false,
            start_line: self.line_number,
            deleted: false,
        };
        self.nodes.push(node);
        let index = self.nodes.len() - 1;
        self.nodes[parent].children.push(index);
        index
    }

    fn add_line(&mut self, node: usize, line: &[u8]) {
        let mut offset = self.offset;
        if self.partially_consumed_tab {
            offset += 1;
            let spaces = TAB_STOP - (self.column % TAB_STOP);
            for _ in 0..spaces {
                self.nodes[node].content.push(' ');
            }
        }
        let rest = std::str::from_utf8(&line[offset.min(line.len())..]).unwrap_or("");
        self.nodes[node].content.push_str(rest);
        self.nodes[node].content.push('\n');
    }

    /// Closes `node`, post-processing its content. Returns its parent.
    fn finalize(&mut self, node: usize) -> Option<usize> {
        let parent = self.nodes[node].parent;
        if !self.nodes[node].open {
            return parent;
        }
        self.nodes[node].open = false;
        match self.nodes[node].kind.clone() {
            Kind::Paragraph => {
                let has_content = self.resolve_reference_definitions(node);
                if !has_content {
                    self.nodes[node].deleted = true;
                }
            }
            Kind::IndentedCode => {
                let content = &mut self.nodes[node].content;
                // Remove trailing blank lines.
                while content.ends_with("\n\n") || content.ends_with("\n \n") {
                    let trimmed_length = content.trim_end_matches([' ', '\t', '\n']).len();
                    let keep = content[..trimmed_length].len();
                    content.truncate(keep);
                    content.push('\n');
                    if !content.ends_with("\n\n") {
                        break;
                    }
                }
                if content == "\n" {
                    content.clear();
                }
            }
            Kind::FencedCode {
                fence_char,
                fence_length,
                fence_offset,
                closed,
                ..
            } => {
                let content = std::mem::take(&mut self.nodes[node].content);
                let (first_line, rest) = match content.find('\n') {
                    Some(index) => (&content[..index], &content[index + 1..]),
                    None => (content.as_str(), ""),
                };
                let info = unescape_and_decode(first_line.trim());
                self.nodes[node].content = rest.to_string();
                self.nodes[node].kind = Kind::FencedCode {
                    fence_char,
                    fence_length,
                    fence_offset,
                    info,
                    closed,
                };
            }
            Kind::List(_) => self.finalize_list(node),
            _ => {}
        }
        parent
    }

    fn finalize_list(&mut self, list: usize) {
        let items = self.nodes[list].children.clone();
        let mut tight = true;
        for (position, item) in items.iter().enumerate() {
            let is_last_item = position + 1 == items.len();
            if self.nodes[*item].last_line_blank && !is_last_item {
                tight = false;
                break;
            }
            let children = self.nodes[*item].children.clone();
            for (child_position, child) in children.iter().enumerate() {
                let is_last_child = child_position + 1 == children.len();
                if self.ends_with_blank_line(*child) && (!is_last_item || !is_last_child) {
                    tight = false;
                    break;
                }
            }
            if !tight {
                break;
            }
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
                Kind::List(_) | Kind::Item(_) => match self.nodes[node].children.last() {
                    Some(last) => node = *last,
                    None => return false,
                },
                _ => return false,
            }
        }
    }

    /// Consumes leading link reference definitions from a paragraph.
    /// Returns whether any content remains.
    fn resolve_reference_definitions(&mut self, node: usize) -> bool {
        loop {
            let content = &self.nodes[node].content;
            if !content.starts_with('[') {
                break;
            }
            let parsed = match parse_reference_definition(content) {
                Some(parsed) => parsed,
                None => break,
            };
            let (label, destination, title, consumed) = parsed;
            let key = normalize_label(&label);
            self.references
                .entry(key)
                .or_insert(Reference { destination, title });
            let remaining = self.nodes[node].content[consumed..].to_string();
            self.nodes[node].content = remaining;
        }
        let content = &self.nodes[node].content;
        !content.trim_matches([' ', '\t', '\n']).is_empty()
    }

    // ----- the per-line algorithm -----

    fn process_line(&mut self, raw_line: &str, line_number: usize) {
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
            let matched = match self.kind(container).clone() {
                Kind::BlockQuote => {
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
                Kind::Item(data) => {
                    if self.blank {
                        let has_children = !self.nodes[container].children.is_empty();
                        if has_children {
                            let to_end = self.first_nonspace - self.offset;
                            self.advance_offset(line, to_end, false);
                            true
                        } else {
                            false
                        }
                    } else if self.indent >= data.marker_offset + data.padding {
                        self.advance_offset(line, data.marker_offset + data.padding, true);
                        true
                    } else {
                        false
                    }
                }
                Kind::FootnoteDefinition { .. } => {
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
                Kind::IndentedCode => {
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
                Kind::FencedCode {
                    fence_char,
                    fence_length,
                    fence_offset,
                    ..
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
                Kind::HtmlBlock { kind } => !(self.blank && (kind == 6 || kind == 7)),
                Kind::Paragraph => !self.blank,
                Kind::Table { .. } => !self.blank,
                Kind::Heading { .. } | Kind::ThematicBreak => false,
                Kind::Document | Kind::List(_) => true,
            };
            if line_consumed {
                return;
            }
            if !matched {
                container = self.nodes[container].parent.unwrap_or(0);
                break;
            }
        }
        let last_matched_container = container;

        // 2. Try new block starts.
        let maybe_lazy = matches!(self.kind(self.tip), Kind::Paragraph);
        loop {
            let is_code_or_html = matches!(
                self.kind(container),
                Kind::IndentedCode | Kind::FencedCode { .. } | Kind::HtmlBlock { .. }
            );
            if is_code_or_html {
                break;
            }
            self.find_first_nonspace(line);
            let indented = self.indented();
            let first = self.peek(line, self.first_nonspace);
            let in_paragraph = matches!(self.kind(container), Kind::Paragraph);

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
                    fence_length,
                    fence_offset,
                    info: String::new(),
                    closed: false,
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
                container = self.add_child(container, Kind::FootnoteDefinition { label });
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
                        marker_offset,
                        padding,
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
            if let Some(last) = self.nodes[container].children.last().copied() {
                self.nodes[last].last_line_blank = true;
            }
        }
        let kind = self.kind(container).clone();
        let never_blank = matches!(
            kind,
            Kind::BlockQuote | Kind::Heading { .. } | Kind::ThematicBreak | Kind::FencedCode { .. }
        );
        let is_empty_new_item = matches!(kind, Kind::Item(_))
            && self.nodes[container].children.is_empty()
            && self.nodes[container].start_line == self.line_number;
        let last_line_blank = self.blank && !never_blank && !is_empty_new_item;
        self.nodes[container].last_line_blank = last_line_blank;
        let mut ancestor = self.nodes[container].parent;
        while let Some(node) = ancestor {
            self.nodes[node].last_line_blank = false;
            ancestor = self.nodes[node].parent;
        }

        let tip_is_paragraph = matches!(self.kind(self.tip), Kind::Paragraph);
        let is_lazy = self.tip != last_matched_container
            && container == last_matched_container
            && !self.blank
            && tip_is_paragraph;
        if is_lazy {
            let tip = self.tip;
            self.add_line(tip, line);
            return;
        }

        while self.tip != last_matched_container {
            match self.finalize(self.tip) {
                Some(parent) => self.tip = parent,
                None => break,
            }
        }

        match self.kind(container).clone() {
            Kind::IndentedCode | Kind::FencedCode { .. } => self.add_line(container, line),
            Kind::HtmlBlock { kind } => {
                self.add_line(container, line);
                if scan_html_block_end(kind, &line[self.first_nonspace.min(line.len())..]) {
                    let parent = self.finalize(container);
                    self.tip = parent.unwrap_or(0);
                    return;
                }
            }
            Kind::Table { .. } => {
                if !self.blank {
                    let rest = std::str::from_utf8(&line[self.first_nonspace.min(line.len())..])
                        .unwrap_or("");
                    let cells = split_table_row(rest);
                    if let Kind::Table { rows, .. } = &mut self.nodes[container].kind {
                        rows.push(cells);
                    }
                }
            }
            _ if self.blank => {}
            Kind::Heading { setext: false, .. } => {
                let to_content = self.first_nonspace - self.offset;
                self.advance_offset(line, to_content, false);
                let rest = std::str::from_utf8(&line[self.offset.min(line.len())..]).unwrap_or("");
                let content = strip_atx_closing(rest);
                self.nodes[container].content.push_str(content);
                self.nodes[container].content.push('\n');
            }
            Kind::Paragraph | Kind::Heading { .. } => {
                let to_content = self.first_nonspace - self.offset;
                self.advance_offset(line, to_content, false);
                self.add_line(container, line);
            }
            _ => {
                let paragraph = self.add_child(container, Kind::Paragraph);
                let to_content = self.first_nonspace - self.offset;
                self.advance_offset(line, to_content, false);
                self.add_line(paragraph, line);
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
        let content = self.nodes[paragraph]
            .content
            .trim_end_matches('\n')
            .to_string();
        let (before, header_line) = match content.rfind('\n') {
            Some(index) => (Some(content[..index].to_string()), &content[index + 1..]),
            None => (None, content.as_str()),
        };
        let header = split_table_row(header_line);
        if header.len() != alignments.len() {
            return None;
        }
        let parent = self.nodes[paragraph].parent.unwrap_or(0);
        match before {
            Some(remaining) => {
                self.nodes[paragraph].content = remaining;
                self.nodes[paragraph].content.push('\n');
                self.finalize(paragraph);
            }
            None => {
                self.nodes[paragraph].open = false;
                self.nodes[paragraph].deleted = true;
            }
        }
        let table = Kind::Table {
            alignments,
            rows: vec![header],
        };
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
    let cells = split_table_row(line);
    if cells.is_empty() {
        return None;
    }
    let trimmed = line.trim();
    if !trimmed.contains('-') {
        return None;
    }
    let mut alignments = Vec::with_capacity(cells.len());
    for cell in &cells {
        let cell = cell.trim();
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
/// (GFM requires escaping them).
pub fn split_table_row(line: &str) -> Vec<String> {
    let trimmed = line.trim_matches([' ', '\t']);
    let mut cells: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut chars = trimmed.chars().peekable();
    let mut started_with_pipe = false;
    if chars.peek() == Some(&'|') {
        chars.next();
        started_with_pipe = true;
    }
    let mut escaped = false;
    let mut ended_with_pipe = false;
    for ch in chars {
        ended_with_pipe = false;
        if escaped {
            if ch != '|' {
                current.push('\\');
            }
            current.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '|' {
            cells.push(current.trim().to_string());
            current.clear();
            ended_with_pipe = true;
            continue;
        }
        current.push(ch);
    }
    if escaped {
        current.push('\\');
    }
    let trailing = current.trim();
    if !(ended_with_pipe && trailing.is_empty()) {
        cells.push(trailing.to_string());
    }
    let _ = started_with_pipe;
    cells
}
