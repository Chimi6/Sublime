//! Inline parsing and event rendering. Walks the block tree, parses each
//! block's text into inline nodes (delimiter stack for emphasis, bracket
//! stack for links), and emits the event stream.

use std::borrow::Cow;
use std::collections::HashMap;

use super::block::{Document, Kind, Node, Reference};
use super::scan::{
    FILTERED_TAGS, decode_entity, is_ascii_punctuation, is_unicode_punctuation,
    is_unicode_whitespace, normalize_label, scan_autolink, scan_inline_html, scan_link_destination,
    scan_link_label, scan_link_title, unescape_and_decode,
};
use super::{Alignment, CodeBlockKind, Event, Options, Tag, TagEnd};

/// Events for one top-level block (and everything inside it).
pub fn render_block<'a>(document: &Document, block: usize) -> Vec<Event<'a>> {
    if document.nodes[block].deleted {
        return Vec::new();
    }
    let mut renderer = Renderer {
        document,
        options: document.options,
        events: Vec::new(),
    };
    renderer.block(block);
    renderer.events
}

struct Renderer<'d, 'a> {
    document: &'d Document,
    options: Options,
    events: Vec<Event<'a>>,
}

fn owned<'a>(text: &str) -> Cow<'a, str> {
    Cow::Owned(text.to_string())
}

impl<'a> Renderer<'_, 'a> {
    fn node(&self, index: usize) -> &Node {
        &self.document.nodes[index]
    }

    fn block_children(&mut self, parent: usize) {
        let children = self.node(parent).children.clone();
        for child in children {
            if self.node(child).deleted {
                continue;
            }
            self.block(child);
        }
    }

    fn block(&mut self, index: usize) {
        let kind = self.node(index).kind.clone();
        match kind {
            Kind::Document => self.block_children(index),
            Kind::BlockQuote => {
                self.events.push(Event::Start(Tag::BlockQuote));
                self.block_children(index);
                self.events.push(Event::End(TagEnd::BlockQuote));
            }
            Kind::List(data) => {
                let start = if data.ordered { Some(data.start) } else { None };
                self.events.push(Event::Start(Tag::List {
                    start,
                    tight: data.tight,
                }));
                self.block_children(index);
                self.events.push(Event::End(TagEnd::List(data.ordered)));
            }
            Kind::Item(_) => {
                self.events.push(Event::Start(Tag::Item));
                self.item_children(index);
                self.events.push(Event::End(TagEnd::Item));
            }
            Kind::Paragraph => {
                self.events.push(Event::Start(Tag::Paragraph));
                let content = self.node(index).content.clone();
                self.inlines(&content);
                self.events.push(Event::End(TagEnd::Paragraph));
            }
            Kind::Heading { level, .. } => {
                self.events.push(Event::Start(Tag::Heading(level)));
                let content = self.node(index).content.clone();
                self.inlines(content.trim_matches([' ', '\t', '\n']));
                self.events.push(Event::End(TagEnd::Heading(level)));
            }
            Kind::ThematicBreak => self.events.push(Event::Rule),
            Kind::IndentedCode => {
                self.events
                    .push(Event::Start(Tag::CodeBlock(CodeBlockKind::Indented)));
                let content = self.node(index).content.clone();
                if !content.is_empty() {
                    self.events.push(Event::Text(owned(&content)));
                }
                self.events.push(Event::End(TagEnd::CodeBlock));
            }
            Kind::FencedCode { info, .. } => {
                self.events
                    .push(Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(owned(
                        &info,
                    )))));
                let content = self.node(index).content.clone();
                if !content.is_empty() {
                    self.events.push(Event::Text(owned(&content)));
                }
                self.events.push(Event::End(TagEnd::CodeBlock));
            }
            Kind::HtmlBlock { .. } => {
                self.events.push(Event::Start(Tag::HtmlBlock));
                let content = self.node(index).content.clone();
                let filtered = if self.options.tagfilter {
                    apply_tagfilter(&content)
                } else {
                    content
                };
                self.events.push(Event::Html(owned(&filtered)));
                self.events.push(Event::End(TagEnd::HtmlBlock));
            }
            Kind::Table { alignments, rows } => self.table(alignments, rows),
            Kind::FootnoteDefinition { label } => {
                self.events
                    .push(Event::Start(Tag::FootnoteDefinition(owned(&label))));
                self.block_children(index);
                self.events.push(Event::End(TagEnd::FootnoteDefinition));
            }
        }
    }

    /// A list item whose first paragraph starts with `[ ]` or `[x]` gets a
    /// task marker and the marker stripped from the paragraph.
    fn item_children(&mut self, item: usize) {
        let children = self.node(item).children.clone();
        let mut first_handled = false;
        for child in children {
            if self.node(child).deleted {
                continue;
            }
            let is_first = !first_handled;
            first_handled = true;
            let is_paragraph = matches!(self.node(child).kind, Kind::Paragraph);
            if is_first && is_paragraph && self.options.task_lists {
                let content = self.node(child).content.clone();
                if let Some((checked, rest)) = split_task_marker(&content) {
                    self.events.push(Event::Start(Tag::Paragraph));
                    self.events.push(Event::TaskListMarker(checked));
                    self.inlines(rest);
                    self.events.push(Event::End(TagEnd::Paragraph));
                    continue;
                }
            }
            self.block(child);
        }
    }

    fn table(&mut self, alignments: Vec<Alignment>, rows: Vec<Vec<String>>) {
        let column_count = alignments.len();
        self.events.push(Event::Start(Tag::Table(alignments)));
        let mut rows = rows.into_iter();
        if let Some(header) = rows.next() {
            self.events.push(Event::Start(Tag::TableHead));
            self.table_cells(&header, column_count);
            self.events.push(Event::End(TagEnd::TableHead));
        }
        for row in rows {
            self.events.push(Event::Start(Tag::TableRow));
            self.table_cells(&row, column_count);
            self.events.push(Event::End(TagEnd::TableRow));
        }
        self.events.push(Event::End(TagEnd::Table));
    }

    fn table_cells(&mut self, cells: &[String], column_count: usize) {
        for column in 0..column_count {
            self.events.push(Event::Start(Tag::TableCell));
            if let Some(cell) = cells.get(column) {
                self.inlines(cell);
            }
            self.events.push(Event::End(TagEnd::TableCell));
        }
    }

    fn inlines(&mut self, text: &str) {
        let trimmed = text.trim_end_matches(['\n', ' ', '\t']);
        let mut parser = InlineParser::new(trimmed, self.document, self.options);
        parser.parse();
        parser.emit(&mut self.events);
    }
}

fn split_task_marker(content: &str) -> Option<(bool, &str)> {
    let bytes = content.as_bytes();
    if bytes.len() < 3 || bytes[0] != b'[' || bytes[2] != b']' {
        return None;
    }
    let checked = match bytes[1] {
        b' ' => false,
        b'x' | b'X' => true,
        _ => return None,
    };
    match bytes.get(3) {
        Some(b' ') | Some(b'\t') | Some(b'\n') => {}
        None => return Some((checked, "")),
        _ => return None,
    }
    let rest = content[3..].trim_start_matches([' ', '\t']);
    if rest.trim_matches([' ', '\t', '\n']).is_empty() {
        return None;
    }
    Some((checked, rest))
}

/// GFM tag filter: `<` becomes `&lt;` in front of disallowed tag names.
pub fn apply_tagfilter(html: &str) -> String {
    let bytes = html.as_bytes();
    let mut out = String::with_capacity(html.len());
    let mut index = 0usize;
    let mut segment_start = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'<' {
            let mut name_start = index + 1;
            if bytes.get(name_start) == Some(&b'/') {
                name_start += 1;
            }
            let mut name_end = name_start;
            while bytes
                .get(name_end)
                .is_some_and(|byte| byte.is_ascii_alphabetic())
            {
                name_end += 1;
            }
            let name = html[name_start..name_end].to_ascii_lowercase();
            let terminated = matches!(
                bytes.get(name_end),
                None | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'>') | Some(b'/')
            );
            if terminated && FILTERED_TAGS.contains(&name.as_str()) {
                out.push_str(&html[segment_start..index]);
                out.push_str("&lt;");
                segment_start = index + 1;
            }
        }
        index += 1;
    }
    out.push_str(&html[segment_start..]);
    out
}

// ----- inline tree -----

#[derive(Debug, Clone, PartialEq, Eq)]
enum InlineKind {
    Text(String),
    Code(String),
    Html(String),
    SoftBreak,
    HardBreak,
    Emphasis,
    Strong,
    Strikethrough,
    Link { destination: String, title: String },
    Image { destination: String, title: String },
    FootnoteReference(String),
}

#[derive(Debug)]
struct InlineNode {
    kind: InlineKind,
    parent: Option<usize>,
    first_child: Option<usize>,
    last_child: Option<usize>,
    prev: Option<usize>,
    next: Option<usize>,
    /// Text that came from a delimiter run and may still be wrapped.
    is_delimiter_run: bool,
}

#[derive(Debug, Clone)]
struct Delimiter {
    node: usize,
    ch: u8,
    count: usize,
    original_count: usize,
    can_open: bool,
    can_close: bool,
    removed: bool,
}

#[derive(Debug, Clone)]
struct Bracket {
    node: usize,
    is_image: bool,
    active: bool,
    /// Byte position just after the `[` in the source text.
    content_start: usize,
    /// Number of delimiters on the stack when this bracket was opened.
    delimiter_bottom: usize,
    /// Whether a bracket was matched (link created) after this one opened.
    bracket_after: bool,
}

struct InlineParser<'t, 'd> {
    text: &'t str,
    bytes: &'t [u8],
    position: usize,
    nodes: Vec<InlineNode>,
    root: usize,
    delimiters: Vec<Delimiter>,
    brackets: Vec<Bracket>,
    references: &'d HashMap<String, Reference>,
    footnotes: &'d HashMap<String, String>,
    options: Options,
}

const SPECIAL: [bool; 256] = {
    let mut table = [false; 256];
    table[b'\\' as usize] = true;
    table[b'`' as usize] = true;
    table[b'*' as usize] = true;
    table[b'_' as usize] = true;
    table[b'~' as usize] = true;
    table[b'[' as usize] = true;
    table[b']' as usize] = true;
    table[b'!' as usize] = true;
    table[b'<' as usize] = true;
    table[b'&' as usize] = true;
    table[b'\n' as usize] = true;
    table
};

impl<'t, 'd> InlineParser<'t, 'd> {
    fn new(text: &'t str, document: &'d Document, options: Options) -> Self {
        let root = InlineNode {
            kind: InlineKind::Emphasis,
            parent: None,
            first_child: None,
            last_child: None,
            prev: None,
            next: None,
            is_delimiter_run: false,
        };
        InlineParser {
            text,
            bytes: text.as_bytes(),
            position: 0,
            nodes: vec![root],
            root: 0,
            delimiters: Vec::new(),
            brackets: Vec::new(),
            references: &document.references,
            footnotes: &document.footnote_labels,
            options,
        }
    }

    // ----- tree operations -----

    fn append(&mut self, kind: InlineKind, is_delimiter_run: bool) -> usize {
        let node = InlineNode {
            kind,
            parent: Some(self.root),
            first_child: None,
            last_child: None,
            prev: self.nodes[self.root].last_child,
            next: None,
            is_delimiter_run,
        };
        self.nodes.push(node);
        let index = self.nodes.len() - 1;
        match self.nodes[self.root].last_child {
            Some(last) => self.nodes[last].next = Some(index),
            None => self.nodes[self.root].first_child = Some(index),
        }
        self.nodes[self.root].last_child = Some(index);
        index
    }

    fn append_text(&mut self, text: &str) {
        if let Some(last) = self.nodes[self.root].last_child {
            let mergeable = !self.nodes[last].is_delimiter_run;
            if let (true, InlineKind::Text(existing)) = (mergeable, &mut self.nodes[last].kind) {
                existing.push_str(text);
                return;
            }
        }
        self.append(InlineKind::Text(text.to_string()), false);
    }

    fn unlink(&mut self, node: usize) {
        let prev = self.nodes[node].prev;
        let next = self.nodes[node].next;
        let parent = self.nodes[node].parent;
        match prev {
            Some(prev) => self.nodes[prev].next = next,
            None => {
                if let Some(parent) = parent {
                    self.nodes[parent].first_child = next;
                }
            }
        }
        match next {
            Some(next) => self.nodes[next].prev = prev,
            None => {
                if let Some(parent) = parent {
                    self.nodes[parent].last_child = prev;
                }
            }
        }
        self.nodes[node].prev = None;
        self.nodes[node].next = None;
        self.nodes[node].parent = None;
    }

    /// Moves every sibling strictly between `after` and `before` (exclusive)
    /// into `container`, then inserts `container` right after `after`.
    fn wrap_between(&mut self, after: usize, before: Option<usize>, container: usize) {
        let mut cursor = self.nodes[after].next;
        while let Some(node) = cursor {
            if Some(node) == before {
                break;
            }
            let next = self.nodes[node].next;
            self.unlink(node);
            self.push_child(container, node);
            cursor = next;
        }
        self.insert_after(after, container);
    }

    /// Moves every sibling after `after` into `container`, then inserts
    /// `container` right after `after`.
    fn wrap_rest(&mut self, after: usize, container: usize) {
        self.wrap_between(after, None, container);
    }

    fn push_child(&mut self, parent: usize, child: usize) {
        self.nodes[child].parent = Some(parent);
        self.nodes[child].prev = self.nodes[parent].last_child;
        self.nodes[child].next = None;
        match self.nodes[parent].last_child {
            Some(last) => self.nodes[last].next = Some(child),
            None => self.nodes[parent].first_child = Some(child),
        }
        self.nodes[parent].last_child = Some(child);
    }

    fn insert_after(&mut self, after: usize, node: usize) {
        let parent = self.nodes[after].parent;
        let next = self.nodes[after].next;
        self.nodes[node].parent = parent;
        self.nodes[node].prev = Some(after);
        self.nodes[node].next = next;
        self.nodes[after].next = Some(node);
        match next {
            Some(next) => self.nodes[next].prev = Some(node),
            None => {
                if let Some(parent) = parent {
                    self.nodes[parent].last_child = Some(node);
                }
            }
        }
    }

    fn new_node(&mut self, kind: InlineKind) -> usize {
        self.nodes.push(InlineNode {
            kind,
            parent: None,
            first_child: None,
            last_child: None,
            prev: None,
            next: None,
            is_delimiter_run: false,
        });
        self.nodes.len() - 1
    }

    // ----- main loop -----

    fn parse(&mut self) {
        while self.position < self.bytes.len() {
            let byte = self.bytes[self.position];
            if !SPECIAL[byte as usize] {
                let start = self.position;
                let mut end = start + 1;
                while end < self.bytes.len() && !SPECIAL[self.bytes[end] as usize] {
                    end += 1;
                }
                self.append_text(&self.text[start..end]);
                self.position = end;
                continue;
            }
            match byte {
                b'\\' => self.backslash(),
                b'`' => self.code_span(),
                b'*' | b'_' | b'~' => self.delimiter_run(byte),
                b'[' => {
                    let node = self.append(InlineKind::Text("[".to_string()), true);
                    self.position += 1;
                    self.push_bracket(node, false);
                }
                b'!' => {
                    if self.bytes.get(self.position + 1) == Some(&b'[') {
                        let node = self.append(InlineKind::Text("![".to_string()), true);
                        self.position += 2;
                        self.push_bracket(node, true);
                    } else {
                        self.append_text("!");
                        self.position += 1;
                    }
                }
                b']' => self.close_bracket(),
                b'<' => self.angle(),
                b'&' => self.entity(),
                b'\n' => self.newline(),
                _ => {
                    self.append_text(&self.text[self.position..self.position + 1]);
                    self.position += 1;
                }
            }
        }
        self.process_emphasis(0);
        self.merge_text();
        if self.options.autolinks {
            self.autolink_literals(self.root);
        }
    }

    fn backslash(&mut self) {
        let next = self.bytes.get(self.position + 1).copied();
        match next {
            Some(byte) if is_ascii_punctuation(byte) => {
                self.append_text(&self.text[self.position + 1..self.position + 2]);
                self.position += 2;
            }
            Some(b'\n') => {
                self.append(InlineKind::HardBreak, false);
                self.position += 2;
                self.skip_line_start_spaces();
            }
            _ => {
                self.append_text("\\");
                self.position += 1;
            }
        }
    }

    fn code_span(&mut self) {
        let start = self.position;
        let mut run = 0usize;
        while self.bytes.get(start + run) == Some(&b'`') {
            run += 1;
        }
        let mut index = start + run;
        while index < self.bytes.len() {
            if self.bytes[index] != b'`' {
                index += 1;
                continue;
            }
            let mut closing = 0usize;
            while self.bytes.get(index + closing) == Some(&b'`') {
                closing += 1;
            }
            if closing == run {
                let raw = &self.text[start + run..index];
                let content = normalize_code_span(raw);
                self.append(InlineKind::Code(content), false);
                self.position = index + closing;
                return;
            }
            index += closing;
        }
        self.append_text(&self.text[start..start + run]);
        self.position = start + run;
    }

    fn char_before(&self, position: usize) -> char {
        if position == 0 {
            return '\n';
        }
        self.text[..position].chars().next_back().unwrap_or('\n')
    }

    fn char_after(&self, position: usize) -> char {
        self.text[position..].chars().next().unwrap_or('\n')
    }

    fn delimiter_run(&mut self, ch: u8) {
        let start = self.position;
        let mut count = 0usize;
        while self.bytes.get(start + count) == Some(&ch) {
            count += 1;
        }
        let end = start + count;
        let before = self.char_before(start);
        let after = self.char_after(end);
        let before_whitespace = is_unicode_whitespace(before);
        let after_whitespace = is_unicode_whitespace(after);
        let before_punctuation = is_unicode_punctuation(before);
        let after_punctuation = is_unicode_punctuation(after);
        let left_flanking =
            !after_whitespace && (!after_punctuation || before_whitespace || before_punctuation);
        let right_flanking =
            !before_whitespace && (!before_punctuation || after_whitespace || after_punctuation);
        let (can_open, can_close) = match ch {
            b'_' => (
                left_flanking && (!right_flanking || before_punctuation),
                right_flanking && (!left_flanking || after_punctuation),
            ),
            b'~' => {
                if !self.options.strikethrough || count > 2 {
                    (false, false)
                } else {
                    (left_flanking, right_flanking)
                }
            }
            _ => (left_flanking, right_flanking),
        };
        let text = self.text[start..end].to_string();
        let is_run = can_open || can_close;
        let node = self.append(InlineKind::Text(text), is_run);
        self.position = end;
        if is_run {
            self.delimiters.push(Delimiter {
                node,
                ch,
                count,
                original_count: count,
                can_open,
                can_close,
                removed: false,
            });
        }
    }

    fn push_bracket(&mut self, node: usize, is_image: bool) {
        self.brackets.push(Bracket {
            node,
            is_image,
            active: true,
            content_start: self.position,
            delimiter_bottom: self.delimiters.len(),
            bracket_after: false,
        });
    }

    fn close_bracket(&mut self) {
        let close_position = self.position;
        self.position += 1;
        let opener = match self.brackets.pop() {
            Some(opener) => opener,
            None => {
                self.append_text("]");
                return;
            }
        };
        if !opener.active {
            self.append_text("]");
            return;
        }
        let raw_content = &self.text[opener.content_start..close_position];

        // Footnote reference: [^label] with a known definition. After an
        // image opener the `!` stays as text.
        if self.options.footnotes {
            if let Some(label) = raw_content.strip_prefix('^') {
                let valid = !label.is_empty()
                    && !label
                        .chars()
                        .any(|ch| ch.is_whitespace() || ch == '[' || ch == ']');
                if valid {
                    let key = normalize_label(label);
                    if let Some(original) = self.footnotes.get(&key) {
                        let node = self.new_node(InlineKind::FootnoteReference(original.clone()));
                        self.replace_bracket_content(&opener, node);
                        self.remove_delimiters_from(opener.delimiter_bottom);
                        return;
                    }
                }
            }
        }

        let mut link: Option<(String, String, usize)> = None;
        // Inline link.
        if self.bytes.get(self.position) == Some(&b'(') {
            if let Some((destination, title, consumed)) = self.scan_inline_link(self.position) {
                link = Some((destination, title, self.position + consumed));
            }
        }
        // Reference link.
        if link.is_none() {
            let rest = &self.bytes[self.position..];
            let mut label: Option<String> = None;
            let mut consumed = 0usize;
            if rest.starts_with(b"[]") {
                label = Some(raw_content.to_string());
                consumed = 2;
            } else if let Some(length) = scan_link_label(rest) {
                if length > 2 {
                    label =
                        Some(self.text[self.position + 1..self.position + length - 1].to_string());
                    consumed = length;
                }
            } else if !opener.bracket_after {
                label = Some(raw_content.to_string());
            }
            if let Some(label) = label {
                let key = normalize_label(&label);
                let valid_label = !key.is_empty()
                    && label.len() <= 999
                    && scan_link_label(format!("[{label}]").as_bytes()).is_some();
                if valid_label {
                    if let Some(reference) = self.references.get(&key) {
                        link = Some((
                            reference.destination.clone(),
                            reference.title.clone(),
                            self.position + consumed,
                        ));
                    }
                }
            }
        }

        let (destination, title, end) = match link {
            Some(link) => link,
            None => {
                self.append_text("]");
                return;
            }
        };
        let kind = if opener.is_image {
            InlineKind::Image { destination, title }
        } else {
            InlineKind::Link { destination, title }
        };
        let node = self.new_node(kind);
        self.wrap_rest(opener.node, node);
        self.process_emphasis(opener.delimiter_bottom);
        self.unlink(opener.node);
        self.remove_delimiters_from(opener.delimiter_bottom);
        if !opener.is_image {
            for bracket in self.brackets.iter_mut() {
                if !bracket.is_image {
                    bracket.active = false;
                }
            }
        }
        for bracket in self.brackets.iter_mut() {
            bracket.bracket_after = true;
        }
        self.position = end;
    }

    /// Replaces the opener node and everything after it with `node`. An
    /// image opener keeps its `!` as text.
    fn replace_bracket_content(&mut self, opener: &Bracket, node: usize) {
        let mut cursor = self.nodes[opener.node].next;
        while let Some(current) = cursor {
            cursor = self.nodes[current].next;
            self.unlink(current);
        }
        self.insert_after(opener.node, node);
        if opener.is_image {
            self.nodes[opener.node].kind = InlineKind::Text("!".to_string());
            self.nodes[opener.node].is_delimiter_run = false;
        } else {
            self.unlink(opener.node);
        }
    }

    fn remove_delimiters_from(&mut self, bottom: usize) {
        for delimiter in self.delimiters.iter_mut().skip(bottom) {
            delimiter.removed = true;
        }
    }

    /// `(` destination title `)` at `at`. Returns decoded destination and
    /// title plus the bytes consumed including both parentheses.
    fn scan_inline_link(&self, at: usize) -> Option<(String, String, usize)> {
        let bytes = self.bytes;
        let mut index = at + 1;
        index = skip_whitespace(bytes, index);
        if bytes.get(index) == Some(&b')') {
            return Some((String::new(), String::new(), index + 1 - at));
        }
        let (dest_start, dest_end, dest_consumed) = scan_link_destination(&bytes[index..])?;
        let destination = unescape_and_decode(&self.text[index + dest_start..index + dest_end]);
        index += dest_consumed;
        let after_destination = index;
        index = skip_whitespace(bytes, index);
        let mut title = String::new();
        if index > after_destination || dest_consumed == 0 {
            if let Some((title_start, title_end, title_consumed)) = scan_link_title(&bytes[index..])
            {
                title = unescape_and_decode(&self.text[index + title_start..index + title_end]);
                index += title_consumed;
                index = skip_whitespace(bytes, index);
            }
        }
        if bytes.get(index) != Some(&b')') {
            return None;
        }
        Some((destination, title, index + 1 - at))
    }

    fn angle(&mut self) {
        let rest = &self.bytes[self.position..];
        if let Some((length, is_email)) = scan_autolink(rest) {
            let inner = &self.text[self.position + 1..self.position + length - 1];
            let destination = if is_email {
                format!("mailto:{inner}")
            } else {
                inner.to_string()
            };
            let link = self.append(
                InlineKind::Link {
                    destination,
                    title: String::new(),
                },
                false,
            );
            let text = self.new_node(InlineKind::Text(inner.to_string()));
            self.push_child(link, text);
            self.position += length;
            return;
        }
        if let Some(length) = scan_inline_html(rest) {
            let html = self.text[self.position..self.position + length].to_string();
            let filtered = if self.options.tagfilter {
                apply_tagfilter(&html)
            } else {
                html
            };
            self.append(InlineKind::Html(filtered), false);
            self.position += length;
            return;
        }
        self.append_text("<");
        self.position += 1;
    }

    fn entity(&mut self) {
        match decode_entity(&self.bytes[self.position..]) {
            Some((replacement, consumed)) => {
                self.append_text(&replacement);
                self.position += consumed;
            }
            None => {
                self.append_text("&");
                self.position += 1;
            }
        }
    }

    fn newline(&mut self) {
        self.position += 1;
        let mut hard = false;
        if let Some(last) = self.nodes[self.root].last_child {
            if let InlineKind::Text(text) = &mut self.nodes[last].kind {
                let trimmed_length = text.trim_end_matches(' ').len();
                let trailing_spaces = text.len() - trimmed_length;
                hard = trailing_spaces >= 2;
                text.truncate(trimmed_length);
                if text.is_empty() && !self.nodes[last].is_delimiter_run {
                    self.unlink(last);
                }
            }
        }
        let kind = if hard {
            InlineKind::HardBreak
        } else {
            InlineKind::SoftBreak
        };
        self.append(kind, false);
        self.skip_line_start_spaces();
    }

    fn skip_line_start_spaces(&mut self) {
        while matches!(self.bytes.get(self.position), Some(b' ') | Some(b'\t')) {
            self.position += 1;
        }
    }

    // ----- emphasis -----

    fn process_emphasis(&mut self, bottom: usize) {
        // openers_bottom[ch index][can_open as usize][count % 3]
        let mut openers_bottom = [[[bottom; 3]; 2]; 3];
        let mut closer_index = bottom;
        while closer_index < self.delimiters.len() {
            let closer = self.delimiters[closer_index].clone();
            if closer.removed || !closer.can_close {
                closer_index += 1;
                continue;
            }
            let ch_index = delimiter_char_index(closer.ch);
            let bottom_key = [
                ch_index,
                closer.can_open as usize,
                closer.original_count % 3,
            ];
            let mut opener_index = closer_index;
            let mut found: Option<usize> = None;
            while opener_index > openers_bottom[bottom_key[0]][bottom_key[1]][bottom_key[2]] {
                opener_index -= 1;
                let opener = &self.delimiters[opener_index];
                if opener.removed || !opener.can_open || opener.ch != closer.ch {
                    continue;
                }
                let odd_match = (closer.can_open || opener.can_close)
                    && closer.original_count % 3 != 0
                    && (opener.original_count + closer.original_count) % 3 == 0;
                if closer.ch != b'~' && odd_match {
                    continue;
                }
                if closer.ch == b'~' && opener.count != closer.count {
                    continue;
                }
                found = Some(opener_index);
                break;
            }
            let opener_index = match found {
                Some(index) => index,
                None => {
                    openers_bottom[bottom_key[0]][bottom_key[1]][bottom_key[2]] = closer_index;
                    if !closer.can_open {
                        self.delimiters[closer_index].removed = true;
                    }
                    closer_index += 1;
                    continue;
                }
            };
            let use_count = if closer.ch == b'~' {
                closer.count
            } else if self.delimiters[opener_index].count >= 2 && closer.count >= 2 {
                2
            } else {
                1
            };
            let kind = match (closer.ch, use_count) {
                (b'~', _) => InlineKind::Strikethrough,
                (_, 2) => InlineKind::Strong,
                _ => InlineKind::Emphasis,
            };
            self.delimiters[opener_index].count -= use_count;
            self.delimiters[closer_index].count -= use_count;
            let opener_node = self.delimiters[opener_index].node;
            let closer_node = self.delimiters[closer_index].node;
            self.truncate_text_end(opener_node, use_count);
            self.truncate_text_start(closer_node, use_count);
            let container = self.new_node(kind);
            self.wrap_between(opener_node, Some(closer_node), container);
            for delimiter in self
                .delimiters
                .iter_mut()
                .take(closer_index)
                .skip(opener_index + 1)
            {
                delimiter.removed = true;
            }
            if self.delimiters[opener_index].count == 0 {
                self.delimiters[opener_index].removed = true;
                self.unlink(opener_node);
            }
            if self.delimiters[closer_index].count == 0 {
                self.delimiters[closer_index].removed = true;
                self.unlink(closer_node);
                closer_index += 1;
            }
        }
        self.remove_delimiters_from(bottom);
    }

    fn truncate_text_end(&mut self, node: usize, count: usize) {
        if let InlineKind::Text(text) = &mut self.nodes[node].kind {
            let keep = text.len().saturating_sub(count);
            text.truncate(keep);
        }
    }

    fn truncate_text_start(&mut self, node: usize, count: usize) {
        if let InlineKind::Text(text) = &mut self.nodes[node].kind {
            let drained: String = text.chars().skip(count).collect();
            *text = drained;
        }
    }

    /// Joins adjacent text nodes everywhere in the tree.
    fn merge_text(&mut self) {
        let mut stack = vec![self.root];
        while let Some(parent) = stack.pop() {
            let mut cursor = self.nodes[parent].first_child;
            while let Some(node) = cursor {
                let next = self.nodes[node].next;
                self.nodes[node].is_delimiter_run = false;
                let both_text = matches!(self.nodes[node].kind, InlineKind::Text(_))
                    && next
                        .is_some_and(|next| matches!(self.nodes[next].kind, InlineKind::Text(_)));
                if both_text {
                    let next_index = next.unwrap_or(node);
                    let appended = match &self.nodes[next_index].kind {
                        InlineKind::Text(text) => text.clone(),
                        _ => String::new(),
                    };
                    if let InlineKind::Text(text) = &mut self.nodes[node].kind {
                        text.push_str(&appended);
                    }
                    self.unlink(next_index);
                    continue;
                }
                if self.nodes[node].first_child.is_some() {
                    stack.push(node);
                }
                cursor = next;
            }
        }
    }

    // ----- GFM autolink literals -----

    fn autolink_literals(&mut self, parent: usize) {
        let mut cursor = self.nodes[parent].first_child;
        while let Some(node) = cursor {
            cursor = self.nodes[node].next;
            match &self.nodes[node].kind {
                InlineKind::Link { .. } => continue,
                InlineKind::Text(text) => {
                    let text = text.clone();
                    self.split_autolinks(node, &text);
                }
                _ => {
                    if self.nodes[node].first_child.is_some() {
                        self.autolink_literals(node);
                    }
                }
            }
        }
    }

    fn split_autolinks(&mut self, node: usize, text: &str) {
        let mut pieces: Vec<(String, Option<String>)> = Vec::new();
        let mut plain_start = 0usize;
        let mut index = 0usize;
        let bytes = text.as_bytes();
        while index < bytes.len() {
            let candidate = find_autolink_literal(text, index);
            match candidate {
                Some((start, end, url)) => {
                    if start > plain_start {
                        pieces.push((text[plain_start..start].to_string(), None));
                    }
                    pieces.push((text[start..end].to_string(), Some(url)));
                    plain_start = end;
                    index = end;
                }
                None => break,
            }
        }
        if pieces.is_empty() {
            return;
        }
        if plain_start < bytes.len() {
            pieces.push((text[plain_start..].to_string(), None));
        }
        let mut after = node;
        for (piece, url) in pieces {
            let new_node = match url {
                Some(url) => {
                    let link = self.new_node(InlineKind::Link {
                        destination: url,
                        title: String::new(),
                    });
                    let inner = self.new_node(InlineKind::Text(piece));
                    self.push_child(link, inner);
                    link
                }
                None => self.new_node(InlineKind::Text(piece)),
            };
            self.insert_after(after, new_node);
            after = new_node;
        }
        self.unlink(node);
    }

    // ----- emit -----

    fn emit<'e>(&self, events: &mut Vec<Event<'e>>) {
        self.emit_children(self.root, events);
    }

    fn emit_children<'e>(&self, parent: usize, events: &mut Vec<Event<'e>>) {
        let mut cursor = self.nodes[parent].first_child;
        while let Some(node) = cursor {
            self.emit_node(node, events);
            cursor = self.nodes[node].next;
        }
    }

    fn emit_node<'e>(&self, node: usize, events: &mut Vec<Event<'e>>) {
        match &self.nodes[node].kind {
            InlineKind::Text(text) => {
                if !text.is_empty() {
                    events.push(Event::Text(owned(text)));
                }
            }
            InlineKind::Code(code) => events.push(Event::Code(owned(code))),
            InlineKind::Html(html) => events.push(Event::InlineHtml(owned(html))),
            InlineKind::SoftBreak => events.push(Event::SoftBreak),
            InlineKind::HardBreak => events.push(Event::HardBreak),
            InlineKind::FootnoteReference(label) => {
                events.push(Event::FootnoteReference(owned(label)))
            }
            InlineKind::Emphasis => self.emit_container(node, Tag::Emphasis, events),
            InlineKind::Strong => self.emit_container(node, Tag::Strong, events),
            InlineKind::Strikethrough => self.emit_container(node, Tag::Strikethrough, events),
            InlineKind::Link { destination, title } => {
                let tag = Tag::Link {
                    destination: owned(destination),
                    title: owned(title),
                };
                self.emit_container(node, tag, events);
            }
            InlineKind::Image { destination, title } => {
                let tag = Tag::Image {
                    destination: owned(destination),
                    title: owned(title),
                };
                self.emit_container(node, tag, events);
            }
        }
    }

    fn emit_container<'e>(&self, node: usize, tag: Tag<'e>, events: &mut Vec<Event<'e>>) {
        let end = tag.end();
        events.push(Event::Start(tag));
        self.emit_children(node, events);
        events.push(Event::End(end));
    }
}

fn delimiter_char_index(ch: u8) -> usize {
    match ch {
        b'*' => 0,
        b'_' => 1,
        _ => 2,
    }
}

fn skip_whitespace(bytes: &[u8], mut index: usize) -> usize {
    while matches!(bytes.get(index), Some(b' ') | Some(b'\t') | Some(b'\n')) {
        index += 1;
    }
    index
}

/// Line endings become spaces; one leading and trailing space are stripped
/// when both exist and the content is not all spaces.
fn normalize_code_span(raw: &str) -> String {
    let replaced: String = raw
        .chars()
        .map(|ch| if ch == '\n' { ' ' } else { ch })
        .collect();
    let all_spaces = replaced.chars().all(|ch| ch == ' ');
    if !all_spaces && replaced.starts_with(' ') && replaced.ends_with(' ') && replaced.len() >= 2 {
        return replaced[1..replaced.len() - 1].to_string();
    }
    replaced
}

// ----- autolink literal scanning -----

/// Finds the next `www.`, `http://`, `https://`, `mailto:`, `xmpp:`, or
/// email autolink in `text` at or after `from`. Returns `(start, end, url)`.
fn find_autolink_literal(text: &str, from: usize) -> Option<(usize, usize, String)> {
    let bytes = text.as_bytes();
    let mut index = from;
    while index < bytes.len() {
        // `www.` needs whitespace or one of `*_~(` before it; a scheme only
        // needs a non-alphanumeric character (quotes and brackets count).
        let strict_boundary = index == 0
            || matches!(
                bytes[index - 1],
                b' ' | b'\t' | b'\n' | b'*' | b'_' | b'~' | b'('
            );
        let loose_boundary = index == 0 || !bytes[index - 1].is_ascii_alphanumeric();
        if loose_boundary {
            if let Some((end, url, is_www)) = scan_www_or_scheme(text, index) {
                if !is_www || strict_boundary {
                    return Some((index, end, url));
                }
            }
        }
        if bytes[index] == b'@' {
            if let Some((start, end, url)) = scan_email_literal(text, index) {
                return Some((start, end, url));
            }
        }
        index += 1;
    }
    None
}

/// Returns `(end, url, is_www)`.
fn scan_www_or_scheme(text: &str, start: usize) -> Option<(usize, String, bool)> {
    let bytes = text.as_bytes();
    let rest = &bytes[start..];
    let (prefix_length, add_scheme) = if rest.len() >= 4 && rest[..4].eq_ignore_ascii_case(b"www.")
    {
        (0usize, true)
    } else if rest.len() >= 7 && rest[..7].eq_ignore_ascii_case(b"http://") {
        (7usize, false)
    } else if rest.len() >= 8 && rest[..8].eq_ignore_ascii_case(b"https://") {
        (8usize, false)
    } else if rest.len() >= 6 && rest[..6].eq_ignore_ascii_case(b"ftp://") {
        (6usize, false)
    } else {
        return None;
    };
    let domain_start = start + prefix_length;
    let requires_dot = add_scheme;
    let domain_end = scan_domain(bytes, domain_start, requires_dot)?;
    let mut end = domain_end;
    while end < bytes.len() && !matches!(bytes[end], b' ' | b'\t' | b'\n' | b'<') {
        end += 1;
    }
    end = trim_autolink_end(bytes, start, end);
    if end <= domain_start {
        return None;
    }
    let matched = &text[start..end];
    let url = if add_scheme {
        format!("http://{matched}")
    } else {
        matched.to_string()
    };
    Some((end, url, add_scheme))
}

/// Segments of alphanumerics, `_`, `-` separated by `.`, at least one `.`,
/// no `_` in the last two segments. Returns the end of the domain.
fn scan_domain(bytes: &[u8], start: usize, requires_dot: bool) -> Option<usize> {
    let mut index = start;
    let mut segments: Vec<(usize, usize)> = Vec::new();
    loop {
        let segment_start = index;
        while index < bytes.len() && is_domain_byte(bytes[index]) {
            index += 1;
        }
        if index == segment_start {
            break;
        }
        segments.push((segment_start, index));
        if bytes.get(index) == Some(&b'.')
            && bytes
                .get(index + 1)
                .is_some_and(|byte| is_domain_byte(*byte))
        {
            index += 1;
            continue;
        }
        break;
    }
    if segments.is_empty() || (requires_dot && segments.len() < 2) {
        return None;
    }
    let last_two_start = segments.len().saturating_sub(2);
    let last_two = &segments[last_two_start..];
    for (segment_start, segment_end) in last_two {
        if bytes[*segment_start..*segment_end].contains(&b'_') {
            return None;
        }
    }
    Some(index)
}

/// Alphanumerics, `_`, `-`, and any non-ASCII byte.
fn is_domain_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-' || byte >= 0x80
}

/// Trailing punctuation, unbalanced `)`, and a trailing entity are not part
/// of the link.
fn trim_autolink_end(bytes: &[u8], start: usize, mut end: usize) -> usize {
    loop {
        if end <= start {
            return end;
        }
        let last = bytes[end - 1];
        if matches!(
            last,
            b'?' | b'!' | b'.' | b',' | b':' | b'*' | b'_' | b'~' | b'\'' | b'"'
        ) {
            end -= 1;
            continue;
        }
        if last == b')' {
            let opens = bytes[start..end]
                .iter()
                .filter(|byte| **byte == b'(')
                .count();
            let closes = bytes[start..end]
                .iter()
                .filter(|byte| **byte == b')')
                .count();
            if closes > opens {
                end -= 1;
                continue;
            }
        }
        if last == b';' {
            let mut index = end - 1;
            while index > start && bytes[index - 1].is_ascii_alphanumeric() {
                index -= 1;
            }
            if index > start && bytes[index - 1] == b'&' && index < end - 1 {
                end = index - 1;
                continue;
            }
        }
        return end;
    }
}

fn scan_email_literal(text: &str, at_position: usize) -> Option<(usize, usize, String)> {
    let bytes = text.as_bytes();
    let mut start = at_position;
    while start > 0
        && (bytes[start - 1].is_ascii_alphanumeric()
            || matches!(bytes[start - 1], b'.' | b'-' | b'_' | b'+'))
    {
        start -= 1;
    }
    if start == at_position {
        return None;
    }
    let mut index = at_position + 1;
    let domain_start = index;
    let mut saw_dot = false;
    while index < bytes.len()
        && (bytes[index].is_ascii_alphanumeric() || matches!(bytes[index], b'-' | b'_' | b'.'))
    {
        if bytes[index] == b'.' {
            saw_dot = true;
        }
        index += 1;
    }
    let mut end = index;
    while end > domain_start && bytes[end - 1] == b'.' {
        end -= 1;
    }
    if end <= domain_start || !saw_dot {
        return None;
    }
    if matches!(bytes[end - 1], b'-' | b'_') {
        return None;
    }
    let prefix_at_boundary =
        |prefix_start: usize| prefix_start == 0 || !bytes[prefix_start - 1].is_ascii_alphanumeric();
    let mailto = start >= 7
        && text[start - 7..start].eq_ignore_ascii_case("mailto:")
        && prefix_at_boundary(start - 7);
    let xmpp = start >= 5
        && text[start - 5..start].eq_ignore_ascii_case("xmpp:")
        && prefix_at_boundary(start - 5);
    if xmpp && bytes.get(end) == Some(&b'/') {
        // An XMPP address may carry a resource after a slash.
        let mut resource_end = end + 1;
        while resource_end < bytes.len()
            && (bytes[resource_end].is_ascii_alphanumeric()
                || matches!(bytes[resource_end], b'.' | b'-' | b'_'))
        {
            resource_end += 1;
        }
        while resource_end > end + 1 && bytes[resource_end - 1] == b'.' {
            resource_end -= 1;
        }
        if resource_end > end + 1 {
            end = resource_end;
        }
    }
    let (link_start, url) = if mailto {
        (start - 7, text[start - 7..end].to_string())
    } else if xmpp {
        (start - 5, text[start - 5..end].to_string())
    } else {
        (start, format!("mailto:{}", &text[start..end]))
    };
    Some((link_start, end, url))
}
