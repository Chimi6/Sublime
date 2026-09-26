//! Inline parsing and event rendering. Walks the block tree, parses each
//! block's text into inline nodes (delimiter stack for emphasis, bracket
//! stack for links), and emits the event stream.

use std::borrow::Cow;
use std::collections::HashMap;

use super::block::{Cell, Document, Kind, Line, Node, Reference, TableData, join_lines};
use crate::io::scan::find_any_of3;

use super::scan::{
    FILTERED_TAGS, decode_entity, is_ascii_punctuation, is_unicode_punctuation,
    is_unicode_whitespace, normalize_label, scan_autolink, scan_inline_html, scan_link_destination,
    scan_link_label, scan_link_title, unescape_and_decode_cow,
};
use super::{CodeBlockKind, Event, EventSink, Options, Tag, TagEnd};

/// Events for one top-level block (and everything inside it).
pub fn render_block<'a>(
    document: &Document,
    source: &'a str,
    block: usize,
    scratch: &mut InlineScratch,
    sink: &mut dyn EventSink<'a>,
) {
    if document.nodes[block].deleted {
        return;
    }
    let mut renderer = Renderer {
        document,
        source,
        options: document.options,
        scratch,
        sink,
    };
    renderer.block(block);
}

/// Arenas the inline parser reuses from block to block, so a document's
/// worth of paragraphs costs a handful of allocations instead of dozens
/// each.
#[derive(Default)]
pub struct InlineScratch {
    nodes: Vec<InlineNode>,
    delimiters: Vec<Delimiter>,
    brackets: Vec<Bracket>,
    /// Tree-walk stack for the post-parse passes.
    walk: Vec<usize>,
}

/// Rebuilds an event with every borrowed string copied, for text that was
/// joined from several source lines and cannot outlive the join.
pub(super) fn into_static(event: Event<'_>) -> Event<'static> {
    fn own(text: Cow<'_, str>) -> Cow<'static, str> {
        Cow::Owned(text.into_owned())
    }
    match event {
        Event::Start(tag) => Event::Start(match tag {
            Tag::Paragraph => Tag::Paragraph,
            Tag::Heading(level) => Tag::Heading(level),
            Tag::BlockQuote => Tag::BlockQuote,
            Tag::CodeBlock(CodeBlockKind::Indented) => Tag::CodeBlock(CodeBlockKind::Indented),
            Tag::CodeBlock(CodeBlockKind::Fenced(info)) => {
                Tag::CodeBlock(CodeBlockKind::Fenced(own(info)))
            }
            Tag::HtmlBlock => Tag::HtmlBlock,
            Tag::List { start, tight } => Tag::List { start, tight },
            Tag::Item => Tag::Item,
            Tag::FootnoteDefinition(label) => Tag::FootnoteDefinition(own(label)),
            Tag::Table(alignments) => Tag::Table(alignments),
            Tag::TableHead => Tag::TableHead,
            Tag::TableRow => Tag::TableRow,
            Tag::TableCell => Tag::TableCell,
            Tag::Emphasis => Tag::Emphasis,
            Tag::Strong => Tag::Strong,
            Tag::Strikethrough => Tag::Strikethrough,
            Tag::Link { destination, title } => Tag::Link {
                destination: own(destination),
                title: own(title),
            },
            Tag::Image { destination, title } => Tag::Image {
                destination: own(destination),
                title: own(title),
            },
        }),
        Event::End(end) => Event::End(end),
        Event::Text(text) => Event::Text(own(text)),
        Event::Code(text) => Event::Code(own(text)),
        Event::Html(text) => Event::Html(own(text)),
        Event::InlineHtml(text) => Event::InlineHtml(own(text)),
        Event::SoftBreak => Event::SoftBreak,
        Event::HardBreak => Event::HardBreak,
        Event::Rule => Event::Rule,
        Event::FootnoteReference(label) => Event::FootnoteReference(own(label)),
        Event::TaskListMarker(checked) => Event::TaskListMarker(checked),
    }
}

struct Renderer<'d, 'a> {
    document: &'d Document,
    source: &'a str,
    options: Options,
    scratch: &'d mut InlineScratch,
    sink: &'d mut dyn EventSink<'a>,
}

fn owned<'a>(text: &str) -> Cow<'a, str> {
    Cow::Owned(text.to_string())
}

impl<'a> Renderer<'_, 'a> {
    fn node(&self, index: usize) -> &Node {
        &self.document.nodes[index]
    }

    fn block_children(&mut self, parent: usize) {
        let document = self.document;
        for child in document.children(parent) {
            if document.nodes[child].deleted {
                continue;
            }
            self.block(child);
        }
    }

    fn block(&mut self, index: usize) {
        let document = self.document;
        match &document.nodes[index].kind {
            Kind::Document => self.block_children(index),
            Kind::BlockQuote => {
                self.sink.event(Event::Start(Tag::BlockQuote));
                self.block_children(index);
                self.sink.event(Event::End(TagEnd::BlockQuote));
            }
            Kind::List(data) => {
                let start = if data.ordered { Some(data.start) } else { None };
                let ordered = data.ordered;
                self.sink.event(Event::Start(Tag::List {
                    start,
                    tight: data.tight,
                }));
                self.block_children(index);
                self.sink.event(Event::End(TagEnd::List(ordered)));
            }
            Kind::Item(_) => {
                self.sink.event(Event::Start(Tag::Item));
                self.item_children(index);
                self.sink.event(Event::End(TagEnd::Item));
            }
            Kind::Paragraph => {
                self.sink.event(Event::Start(Tag::Paragraph));
                let content = join_lines(self.source, document.lines_of(index));
                self.inlines(content);
                self.sink.event(Event::End(TagEnd::Paragraph));
            }
            Kind::Heading { level, .. } => {
                let level = *level;
                self.sink.event(Event::Start(Tag::Heading(level)));
                let content = join_lines(self.source, document.lines_of(index));
                let trimmed = match content {
                    Cow::Borrowed(text) => Cow::Borrowed(text.trim_matches([' ', '\t', '\n'])),
                    Cow::Owned(text) => {
                        Cow::Owned(text.trim_matches([' ', '\t', '\n']).to_string())
                    }
                };
                self.inlines(trimmed);
                self.sink.event(Event::End(TagEnd::Heading(level)));
            }
            Kind::ThematicBreak => self.sink.event(Event::Rule),
            Kind::IndentedCode => {
                self.sink
                    .event(Event::Start(Tag::CodeBlock(CodeBlockKind::Indented)));
                self.code_lines(index);
                self.sink.event(Event::End(TagEnd::CodeBlock));
            }
            Kind::FencedCode { info, .. } => {
                self.sink
                    .event(Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(owned(
                        info,
                    )))));
                self.code_lines(index);
                self.sink.event(Event::End(TagEnd::CodeBlock));
            }
            Kind::HtmlBlock { .. } => {
                self.sink.event(Event::Start(Tag::HtmlBlock));
                for line in document.lines_of(index) {
                    let text = self.line_with_newline(line);
                    let html = if self.options.tagfilter {
                        match apply_tagfilter(&text) {
                            Cow::Owned(filtered) => Cow::Owned(filtered),
                            Cow::Borrowed(_) => text,
                        }
                    } else {
                        text
                    };
                    self.sink.event(Event::Html(html));
                }
                self.sink.event(Event::End(TagEnd::HtmlBlock));
            }
            Kind::Table(data) => self.table(data),
            Kind::FootnoteDefinition { label } => {
                self.sink
                    .event(Event::Start(Tag::FootnoteDefinition(owned(label))));
                self.block_children(index);
                self.sink.event(Event::End(TagEnd::FootnoteDefinition));
            }
        }
    }

    /// One `Text` event per line, borrowed from the source whenever the line
    /// is followed by a plain `\n` there.
    fn code_lines(&mut self, index: usize) {
        let document = self.document;
        for line in document.lines_of(index) {
            let text = self.line_with_newline(line);
            self.sink.event(Event::Text(text));
        }
    }

    fn line_with_newline(&self, line: &Line) -> Cow<'a, str> {
        let followed_by_newline = self.source.as_bytes().get(line.end) == Some(&b'\n');
        if line.leading_spaces == 0 && followed_by_newline {
            return Cow::Borrowed(&self.source[line.start..line.end + 1]);
        }
        let mut owned = line.text(self.source).into_owned();
        owned.push('\n');
        Cow::Owned(owned)
    }

    /// A list item whose first paragraph starts with `[ ]` or `[x]` gets a
    /// task marker and the marker stripped from the paragraph.
    fn item_children(&mut self, item: usize) {
        let document = self.document;
        let mut first_handled = false;
        for child in document.children(item) {
            if document.nodes[child].deleted {
                continue;
            }
            let is_first = !first_handled;
            first_handled = true;
            let is_paragraph = matches!(self.node(child).kind, Kind::Paragraph);
            if is_first && is_paragraph && self.options.task_lists {
                let content = join_lines(self.source, document.lines_of(child));
                let marker = match &content {
                    Cow::Borrowed(text) => split_task_marker(text)
                        .map(|(checked, rest)| (checked, Cow::Borrowed(rest))),
                    Cow::Owned(text) => split_task_marker(text)
                        .map(|(checked, rest)| (checked, Cow::Owned(rest.to_string()))),
                };
                if let Some((checked, rest)) = marker {
                    self.sink.event(Event::Start(Tag::Paragraph));
                    self.sink.event(Event::TaskListMarker(checked));
                    self.inlines(rest);
                    self.sink.event(Event::End(TagEnd::Paragraph));
                    continue;
                }
            }
            self.block(child);
        }
    }

    fn table(&mut self, data: &TableData) {
        let column_count = data.alignments.len();
        self.sink
            .event(Event::Start(Tag::Table(data.alignments.to_vec())));
        let mut rows = data.rows();
        if let Some(header) = rows.next() {
            self.sink.event(Event::Start(Tag::TableHead));
            self.table_cells(header, column_count);
            self.sink.event(Event::End(TagEnd::TableHead));
        }
        for row in rows {
            self.sink.event(Event::Start(Tag::TableRow));
            self.table_cells(row, column_count);
            self.sink.event(Event::End(TagEnd::TableRow));
        }
        self.sink.event(Event::End(TagEnd::Table));
    }

    fn table_cells(&mut self, cells: &[Cell], column_count: usize) {
        for column in 0..column_count {
            self.sink.event(Event::Start(Tag::TableCell));
            if let Some(cell) = cells.get(column) {
                let text = cell.text(self.source);
                self.inlines(text);
            }
            self.sink.event(Event::End(TagEnd::TableCell));
        }
    }

    /// Borrowed content yields events that borrow the source; joined
    /// content lives only for this call, so its events go out as transient.
    fn inlines(&mut self, content: Cow<'a, str>) {
        let sink = &mut *self.sink;
        match content {
            Cow::Borrowed(text) => {
                let trimmed = text.trim_end_matches(['\n', ' ', '\t']);
                if is_plain_inline(trimmed) {
                    // Nothing to parse: one text event, no node machinery.
                    if !trimmed.is_empty() {
                        sink.event(Event::Text(Cow::Borrowed(trimmed)));
                    }
                    return;
                }
                let mut parser =
                    InlineParser::new(trimmed, self.document, self.options, self.scratch);
                parser.parse();
                parser.emit(&mut |event| sink.event(event));
            }
            Cow::Owned(text) => {
                let trimmed = text.trim_end_matches(['\n', ' ', '\t']);
                let mut parser =
                    InlineParser::new(trimmed, self.document, self.options, self.scratch);
                parser.parse();
                parser.emit(&mut |event| sink.transient(event));
            }
        }
    }
}

/// Text the inline parser would hand back as one text event: no inline
/// syntax bytes, and nothing that could start an autolink literal (an
/// email's `@`, a scheme's `:`, or `www`). Table cells of plain data
/// take this path; prose rarely does, and need not.
fn is_plain_inline(text: &str) -> bool {
    text.bytes()
        .all(|byte| !SPECIAL[byte as usize] && !matches!(byte, b'@' | b':' | b'w' | b'W'))
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
pub fn apply_tagfilter(html: &str) -> Cow<'_, str> {
    let bytes = html.as_bytes();
    let mut out = String::new();
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
                if out.is_empty() {
                    out.reserve(html.len() + 8);
                }
                out.push_str(&html[segment_start..index]);
                out.push_str("&lt;");
                segment_start = index + 1;
            }
        }
        index += 1;
    }
    if segment_start == 0 {
        return Cow::Borrowed(html);
    }
    out.push_str(&html[segment_start..]);
    Cow::Owned(out)
}

// ----- inline tree -----

/// Text of an inline node: a range of the block's text, or an owned string
/// when escapes or entities changed it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Span {
    Range(usize, usize),
    Owned(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum InlineKind {
    Text(Span),
    Code(Span),
    Html(Span),
    SoftBreak,
    HardBreak,
    Emphasis,
    Strong,
    Strikethrough,
    Link(Box<LinkData>),
    Image(Box<LinkData>),
    FootnoteReference(String),
}

/// Destination and title of a link or image, boxed to keep nodes small.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LinkData {
    destination: Span,
    title: Span,
}

/// Index meaning "no node" in the inline tree links.
const NO_NODE: u32 = u32::MAX;

fn link_get(link: u32) -> Option<usize> {
    if link == NO_NODE {
        None
    } else {
        Some(link as usize)
    }
}

fn link_from(index: Option<usize>) -> u32 {
    match index {
        Some(index) => index as u32,
        None => NO_NODE,
    }
}

#[derive(Debug)]
struct InlineNode {
    kind: InlineKind,
    parent: u32,
    first_child: u32,
    last_child: u32,
    prev: u32,
    next: u32,
    /// Text that came from a delimiter run and may still be wrapped.
    is_delimiter_run: bool,
}

impl InlineNode {
    /// An unlinked node; callers fill the slot in place after pushing it.
    const TEMPLATE: InlineNode = InlineNode {
        kind: InlineKind::SoftBreak,
        parent: NO_NODE,
        first_child: NO_NODE,
        last_child: NO_NODE,
        prev: NO_NODE,
        next: NO_NODE,
        is_delimiter_run: false,
    };
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
    nodes: &'d mut Vec<InlineNode>,
    root: usize,
    delimiters: &'d mut Vec<Delimiter>,
    brackets: &'d mut Vec<Bracket>,
    walk: &'d mut Vec<usize>,
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
    fn new(
        text: &'t str,
        document: &'d Document,
        options: Options,
        scratch: &'d mut InlineScratch,
    ) -> Self {
        scratch.nodes.clear();
        scratch.delimiters.clear();
        scratch.brackets.clear();
        scratch.walk.clear();
        scratch.nodes.push(InlineNode::TEMPLATE);
        InlineParser {
            text,
            bytes: text.as_bytes(),
            position: 0,
            nodes: &mut scratch.nodes,
            root: 0,
            delimiters: &mut scratch.delimiters,
            brackets: &mut scratch.brackets,
            walk: &mut scratch.walk,
            references: &document.references,
            footnotes: &document.footnote_labels,
            options,
        }
    }

    // ----- tree operations -----

    fn append(&mut self, kind: InlineKind, is_delimiter_run: bool) -> usize {
        // Push a constant template and fill the slot in place: building the
        // node on the stack and copying it stalls on store forwarding.
        let index = self.nodes.len();
        let prev = self.nodes[self.root].last_child;
        self.nodes.push(InlineNode::TEMPLATE);
        let node = &mut self.nodes[index];
        // The template's kind owns nothing, so skipping its drop glue is safe.
        std::mem::forget(std::mem::replace(&mut node.kind, kind));
        node.parent = self.root as u32;
        node.prev = prev;
        node.is_delimiter_run = is_delimiter_run;
        match link_get(self.nodes[self.root].last_child) {
            Some(last) => self.nodes[last].next = index as u32,
            None => self.nodes[self.root].first_child = index as u32,
        }
        self.nodes[self.root].last_child = index as u32;
        index
    }

    fn span_text<'s>(&'s self, span: &'s Span) -> &'s str {
        match span {
            Span::Range(start, end) => &self.text[*start..*end],
            Span::Owned(text) => text,
        }
    }

    /// Appends text that is a slice of the block's text, extending the
    /// previous range when it is adjacent.
    fn append_range(&mut self, start: usize, end: usize) {
        if let Some(last) = link_get(self.nodes[self.root].last_child) {
            let mergeable = !self.nodes[last].is_delimiter_run;
            if let (true, InlineKind::Text(Span::Range(_, last_end))) =
                (mergeable, &mut self.nodes[last].kind)
            {
                if *last_end == start {
                    *last_end = end;
                    return;
                }
            }
        }
        self.append(InlineKind::Text(Span::Range(start, end)), false);
    }

    /// Appends text that is not a slice of the block's text.
    fn append_text(&mut self, text: &str) {
        if let Some(last) = link_get(self.nodes[self.root].last_child) {
            let mergeable = !self.nodes[last].is_delimiter_run;
            if mergeable {
                if let InlineKind::Text(span) = &self.nodes[last].kind {
                    let mut merged = self.span_text(span).to_string();
                    merged.push_str(text);
                    self.nodes[last].kind = InlineKind::Text(Span::Owned(merged));
                    return;
                }
            }
        }
        self.append(InlineKind::Text(Span::Owned(text.to_string())), false);
    }

    fn unlink(&mut self, node: usize) {
        let prev = link_get(self.nodes[node].prev);
        let next = link_get(self.nodes[node].next);
        let parent = link_get(self.nodes[node].parent);
        match prev {
            Some(prev) => self.nodes[prev].next = link_from(next),
            None => {
                if let Some(parent) = parent {
                    self.nodes[parent].first_child = link_from(next);
                }
            }
        }
        match next {
            Some(next) => self.nodes[next].prev = link_from(prev),
            None => {
                if let Some(parent) = parent {
                    self.nodes[parent].last_child = link_from(prev);
                }
            }
        }
        self.nodes[node].prev = NO_NODE;
        self.nodes[node].next = NO_NODE;
        self.nodes[node].parent = NO_NODE;
    }

    /// Moves every sibling strictly between `after` and `before` (exclusive)
    /// into `container`, then inserts `container` right after `after`.
    fn wrap_between(&mut self, after: usize, before: Option<usize>, container: usize) {
        let mut cursor = link_get(self.nodes[after].next);
        while let Some(node) = cursor {
            if Some(node) == before {
                break;
            }
            let next = link_get(self.nodes[node].next);
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
        self.nodes[child].parent = parent as u32;
        self.nodes[child].prev = self.nodes[parent].last_child;
        self.nodes[child].next = NO_NODE;
        match link_get(self.nodes[parent].last_child) {
            Some(last) => self.nodes[last].next = child as u32,
            None => self.nodes[parent].first_child = child as u32,
        }
        self.nodes[parent].last_child = child as u32;
    }

    fn insert_after(&mut self, after: usize, node: usize) {
        let parent = link_get(self.nodes[after].parent);
        let next = link_get(self.nodes[after].next);
        self.nodes[node].parent = link_from(parent);
        self.nodes[node].prev = after as u32;
        self.nodes[node].next = link_from(next);
        self.nodes[after].next = node as u32;
        match next {
            Some(next) => self.nodes[next].prev = node as u32,
            None => {
                if let Some(parent) = parent {
                    self.nodes[parent].last_child = node as u32;
                }
            }
        }
    }

    fn new_node(&mut self, kind: InlineKind) -> usize {
        let index = self.nodes.len();
        self.nodes.push(InlineNode::TEMPLATE);
        std::mem::forget(std::mem::replace(&mut self.nodes[index].kind, kind));
        index
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
                self.append_range(start, end);
                self.position = end;
                continue;
            }
            match byte {
                b'\\' => self.backslash(),
                b'`' => self.code_span(),
                b'*' | b'_' | b'~' => self.delimiter_run(byte),
                b'[' => {
                    let node = self.append(
                        InlineKind::Text(Span::Range(self.position, self.position + 1)),
                        true,
                    );
                    self.position += 1;
                    self.push_bracket(node, false);
                }
                b'!' => {
                    if self.bytes.get(self.position + 1) == Some(&b'[') {
                        let node = self.append(
                            InlineKind::Text(Span::Range(self.position, self.position + 2)),
                            true,
                        );
                        self.position += 2;
                        self.push_bracket(node, true);
                    } else {
                        self.append_range(self.position, self.position + 1);
                        self.position += 1;
                    }
                }
                b']' => self.close_bracket(),
                b'<' => self.angle(),
                b'&' => self.entity(),
                b'\n' => self.newline(),
                _ => {
                    self.append_range(self.position, self.position + 1);
                    self.position += 1;
                }
            }
        }
        self.process_emphasis(0);
        // Adjacent text nodes only need joining for the autolink pass, and
        // that pass only matters when the text could hold an autolink.
        if self.options.autolinks && autolink_candidate(self.bytes) {
            self.merge_text();
            self.autolink_literals(self.root);
        }
    }

    fn backslash(&mut self) {
        let next = self.bytes.get(self.position + 1).copied();
        match next {
            Some(byte) if is_ascii_punctuation(byte) => {
                self.append_range(self.position + 1, self.position + 2);
                self.position += 2;
            }
            Some(b'\n') => {
                self.append(InlineKind::HardBreak, false);
                self.position += 2;
                self.skip_line_start_spaces();
            }
            _ => {
                self.append_range(self.position, self.position + 1);
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
                let span = match normalize_code_span(raw) {
                    Some(normalized) => Span::Owned(normalized),
                    None => Span::Range(start + run, index),
                };
                self.append(InlineKind::Code(span), false);
                self.position = index + closing;
                return;
            }
            index += closing;
        }
        self.append_range(start, start + run);
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
        let is_run = can_open || can_close;
        let node = self.append(InlineKind::Text(Span::Range(start, end)), is_run);
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
                self.append_range(close_position, close_position + 1);
                return;
            }
        };
        if !opener.active {
            self.append_range(close_position, close_position + 1);
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

        let mut link: Option<(Span, Span, usize)> = None;
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
                            Span::Owned(reference.destination.clone()),
                            Span::Owned(reference.title.clone()),
                            self.position + consumed,
                        ));
                    }
                }
            }
        }

        let (destination, title, end) = match link {
            Some(link) => link,
            None => {
                self.append_range(close_position, close_position + 1);
                return;
            }
        };
        let data = Box::new(LinkData { destination, title });
        let kind = if opener.is_image {
            InlineKind::Image(data)
        } else {
            InlineKind::Link(data)
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
        let mut cursor = link_get(self.nodes[opener.node].next);
        while let Some(current) = cursor {
            cursor = link_get(self.nodes[current].next);
            self.unlink(current);
        }
        self.insert_after(opener.node, node);
        if opener.is_image {
            let bang = match &self.nodes[opener.node].kind {
                InlineKind::Text(Span::Range(start, _)) => Span::Range(*start, *start + 1),
                _ => Span::Owned("!".to_string()),
            };
            self.nodes[opener.node].kind = InlineKind::Text(bang);
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
    fn scan_inline_link(&self, at: usize) -> Option<(Span, Span, usize)> {
        let bytes = self.bytes;
        let mut index = at + 1;
        index = skip_whitespace(bytes, index);
        if bytes.get(index) == Some(&b')') {
            return Some((
                Span::Range(index, index),
                Span::Range(index, index),
                index + 1 - at,
            ));
        }
        let (dest_start, dest_end, dest_consumed) = scan_link_destination(&bytes[index..])?;
        let destination = self.decoded_span(index + dest_start, index + dest_end);
        index += dest_consumed;
        let after_destination = index;
        index = skip_whitespace(bytes, index);
        let mut title = Span::Range(index, index);
        if index > after_destination || dest_consumed == 0 {
            if let Some((title_start, title_end, title_consumed)) = scan_link_title(&bytes[index..])
            {
                title = self.decoded_span(index + title_start, index + title_end);
                index += title_consumed;
                index = skip_whitespace(bytes, index);
            }
        }
        if bytes.get(index) != Some(&b')') {
            return None;
        }
        Some((destination, title, index + 1 - at))
    }

    /// A range of the block text with escapes and entities resolved:
    /// borrowed when there are none.
    fn decoded_span(&self, start: usize, end: usize) -> Span {
        match unescape_and_decode_cow(&self.text[start..end]) {
            Cow::Borrowed(_) => Span::Range(start, end),
            Cow::Owned(decoded) => Span::Owned(decoded),
        }
    }

    fn angle(&mut self) {
        let rest = &self.bytes[self.position..];
        if let Some((length, is_email)) = scan_autolink(rest) {
            let inner_start = self.position + 1;
            let inner_end = self.position + length - 1;
            let destination = if is_email {
                Span::Owned(format!("mailto:{}", &self.text[inner_start..inner_end]))
            } else {
                Span::Range(inner_start, inner_end)
            };
            let link = self.append(
                InlineKind::Link(Box::new(LinkData {
                    destination,
                    title: Span::Range(inner_end, inner_end),
                })),
                false,
            );
            let text = self.new_node(InlineKind::Text(Span::Range(inner_start, inner_end)));
            self.push_child(link, text);
            self.position += length;
            return;
        }
        if let Some(length) = scan_inline_html(rest) {
            let html = &self.text[self.position..self.position + length];
            let span = if self.options.tagfilter {
                match apply_tagfilter(html) {
                    Cow::Owned(filtered) => Span::Owned(filtered),
                    Cow::Borrowed(_) => Span::Range(self.position, self.position + length),
                }
            } else {
                Span::Range(self.position, self.position + length)
            };
            self.append(InlineKind::Html(span), false);
            self.position += length;
            return;
        }
        self.append_range(self.position, self.position + 1);
        self.position += 1;
    }

    fn entity(&mut self) {
        match decode_entity(&self.bytes[self.position..]) {
            Some((replacement, consumed)) => {
                self.append_text(&replacement);
                self.position += consumed;
            }
            None => {
                self.append_range(self.position, self.position + 1);
                self.position += 1;
            }
        }
    }

    fn newline(&mut self) {
        self.position += 1;
        let mut hard = false;
        if let Some(last) = link_get(self.nodes[self.root].last_child) {
            let is_delimiter_run = self.nodes[last].is_delimiter_run;
            let mut became_empty = false;
            match &mut self.nodes[last].kind {
                InlineKind::Text(Span::Range(start, end)) => {
                    let text = &self.text[*start..*end];
                    let trimmed_length = text.trim_end_matches(' ').len();
                    hard = text.len() - trimmed_length >= 2;
                    *end = *start + trimmed_length;
                    became_empty = *end == *start;
                }
                InlineKind::Text(Span::Owned(text)) => {
                    let trimmed_length = text.trim_end_matches(' ').len();
                    hard = text.len() - trimmed_length >= 2;
                    text.truncate(trimmed_length);
                    became_empty = text.is_empty();
                }
                _ => {}
            }
            if became_empty && !is_delimiter_run {
                self.unlink(last);
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
        match &mut self.nodes[node].kind {
            InlineKind::Text(Span::Range(start, end)) => {
                *end = (*end).saturating_sub(count).max(*start);
            }
            InlineKind::Text(Span::Owned(text)) => {
                let keep = text.len().saturating_sub(count);
                text.truncate(keep);
            }
            _ => {}
        }
    }

    fn truncate_text_start(&mut self, node: usize, count: usize) {
        match &mut self.nodes[node].kind {
            InlineKind::Text(Span::Range(start, end)) => {
                *start = (*start + count).min(*end);
            }
            InlineKind::Text(Span::Owned(text)) => {
                let drained: String = text.chars().skip(count).collect();
                *text = drained;
            }
            _ => {}
        }
    }

    /// Joins adjacent text nodes everywhere in the tree.
    fn merge_text(&mut self) {
        self.walk.push(self.root);
        while let Some(parent) = self.walk.pop() {
            let mut cursor = link_get(self.nodes[parent].first_child);
            while let Some(node) = cursor {
                let next = link_get(self.nodes[node].next);
                self.nodes[node].is_delimiter_run = false;
                let both_text = matches!(self.nodes[node].kind, InlineKind::Text(_))
                    && next
                        .is_some_and(|next| matches!(self.nodes[next].kind, InlineKind::Text(_)));
                if both_text {
                    let next_index = next.unwrap_or(node);
                    let merged = match (&self.nodes[node].kind, &self.nodes[next_index].kind) {
                        (
                            InlineKind::Text(Span::Range(a, b)),
                            InlineKind::Text(Span::Range(c, d)),
                        ) if b == c => Span::Range(*a, *d),
                        (InlineKind::Text(left), InlineKind::Text(right)) => {
                            let mut joined = self.span_text(left).to_string();
                            joined.push_str(self.span_text(right));
                            Span::Owned(joined)
                        }
                        _ => continue,
                    };
                    self.nodes[node].kind = InlineKind::Text(merged);
                    self.unlink(next_index);
                    continue;
                }
                if link_get(self.nodes[node].first_child).is_some() {
                    self.walk.push(node);
                }
                cursor = next;
            }
        }
    }

    // ----- GFM autolink literals -----

    fn autolink_literals(&mut self, parent: usize) {
        let mut cursor = link_get(self.nodes[parent].first_child);
        while let Some(node) = cursor {
            cursor = link_get(self.nodes[node].next);
            match &self.nodes[node].kind {
                InlineKind::Link(_) => continue,
                InlineKind::Text(Span::Range(start, end)) => {
                    let (start, end) = (*start, *end);
                    let text: &'t str = self.text;
                    let slice = &text[start..end];
                    if !autolink_candidate(slice.as_bytes()) {
                        continue;
                    }
                    self.split_autolinks(node, slice, Some(start));
                }
                InlineKind::Text(Span::Owned(owned)) => {
                    if !autolink_candidate(owned.as_bytes()) {
                        continue;
                    }
                    let copy = owned.clone();
                    self.split_autolinks(node, &copy, None);
                }
                _ => {
                    if link_get(self.nodes[node].first_child).is_some() {
                        self.autolink_literals(node);
                    }
                }
            }
        }
    }

    /// Splits a text node around autolink literals. With `base`, `text` is
    /// the block text from that offset and every piece stays a range of it;
    /// without it the pieces are copied.
    fn split_autolinks(&mut self, node: usize, text: &str, base: Option<usize>) {
        let mut plain_start = 0usize;
        let mut index = 0usize;
        let mut after = node;
        let mut found = false;
        while index < text.len() {
            let (start, end, prefix) = match find_autolink_literal(text, index) {
                Some(literal) => literal,
                None => break,
            };
            found = true;
            if start > plain_start {
                let plain = self.new_node(InlineKind::Text(piece(text, plain_start, start, base)));
                self.insert_after(after, plain);
                after = plain;
            }
            let destination = if prefix.is_empty() {
                piece(text, start, end, base)
            } else {
                Span::Owned(format!("{prefix}{}", &text[start..end]))
            };
            let link = self.new_node(InlineKind::Link(Box::new(LinkData {
                destination,
                title: Span::Owned(String::new()),
            })));
            let inner = self.new_node(InlineKind::Text(piece(text, start, end, base)));
            self.push_child(link, inner);
            self.insert_after(after, link);
            after = link;
            plain_start = end;
            index = end;
        }
        if !found {
            return;
        }
        if plain_start < text.len() {
            let plain = self.new_node(InlineKind::Text(piece(text, plain_start, text.len(), base)));
            self.insert_after(after, plain);
        }
        self.unlink(node);
    }

    // ----- emit -----

    /// Moves the tree out as events. Text that is a range of the block's
    /// text is borrowed; owned strings are moved, not copied. The arena is
    /// cleared by the next block anyway.
    /// Walks the finished tree, handing each event to `events`.
    fn emit<F: FnMut(Event<'t>)>(&mut self, events: &mut F) {
        self.emit_children(self.root, events);
    }

    fn emit_children<F: FnMut(Event<'t>)>(&mut self, parent: usize, events: &mut F) {
        let mut cursor = link_get(self.nodes[parent].first_child);
        while let Some(node) = cursor {
            self.emit_node(node, events);
            cursor = link_get(self.nodes[node].next);
        }
    }

    fn emit_node<F: FnMut(Event<'t>)>(&mut self, node: usize, events: &mut F) {
        // Kinds that own nothing are read in place; the rest are moved out.
        let text = self.text;
        match &self.nodes[node].kind {
            InlineKind::Text(Span::Range(start, end)) => {
                if end > start {
                    events(Event::Text(Cow::Borrowed(&text[*start..*end])));
                }
                return;
            }
            InlineKind::Code(Span::Range(start, end)) => {
                events(Event::Code(Cow::Borrowed(&text[*start..*end])));
                return;
            }
            InlineKind::Html(Span::Range(start, end)) => {
                events(Event::InlineHtml(Cow::Borrowed(&text[*start..*end])));
                return;
            }
            InlineKind::SoftBreak => {
                events(Event::SoftBreak);
                return;
            }
            InlineKind::HardBreak => {
                events(Event::HardBreak);
                return;
            }
            InlineKind::Emphasis => {
                self.emit_container(node, Tag::Emphasis, events);
                return;
            }
            InlineKind::Strong => {
                self.emit_container(node, Tag::Strong, events);
                return;
            }
            InlineKind::Strikethrough => {
                self.emit_container(node, Tag::Strikethrough, events);
                return;
            }
            _ => {}
        }
        let kind = std::mem::replace(&mut self.nodes[node].kind, InlineKind::SoftBreak);
        match kind {
            InlineKind::Text(Span::Owned(owned)) => {
                if !owned.is_empty() {
                    events(Event::Text(Cow::Owned(owned)));
                }
            }
            InlineKind::Code(code) => events(Event::Code(self.take_span(code))),
            InlineKind::Html(html) => events(Event::InlineHtml(self.take_span(html))),
            InlineKind::FootnoteReference(label) => {
                events(Event::FootnoteReference(Cow::Owned(label)));
            }
            InlineKind::Link(data) => {
                let data = *data;
                let tag = Tag::Link {
                    destination: self.take_span(data.destination),
                    title: self.take_span(data.title),
                };
                self.emit_container(node, tag, events);
            }
            InlineKind::Image(data) => {
                let data = *data;
                let tag = Tag::Image {
                    destination: self.take_span(data.destination),
                    title: self.take_span(data.title),
                };
                self.emit_container(node, tag, events);
            }
            _ => {}
        }
    }

    fn take_span(&self, span: Span) -> Cow<'t, str> {
        match span {
            Span::Range(start, end) => Cow::Borrowed(&self.text[start..end]),
            Span::Owned(text) => Cow::Owned(text),
        }
    }

    fn emit_container<F: FnMut(Event<'t>)>(&mut self, node: usize, tag: Tag<'t>, events: &mut F) {
        let end = tag.end();
        events(Event::Start(tag));
        self.emit_children(node, events);
        events(Event::End(end));
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
/// Line endings become spaces; one leading and trailing space are stripped
/// when both exist and the content is not all spaces. Returns `None` when
/// the content is already in normal form.
fn normalize_code_span(raw: &str) -> Option<String> {
    let has_newline = raw.contains('\n');
    let all_spaces = raw.bytes().all(|byte| byte == b' ' || byte == b'\n');
    let strippable =
        !all_spaces && raw.len() >= 2 && raw.starts_with([' ', '\n']) && raw.ends_with([' ', '\n']);
    if !has_newline && !strippable {
        return None;
    }
    let replaced: String = raw
        .chars()
        .map(|ch| if ch == '\n' { ' ' } else { ch })
        .collect();
    if strippable {
        return Some(replaced[1..replaced.len() - 1].to_string());
    }
    Some(replaced)
}

// ----- autolink literal scanning -----

/// Cheap single pass: could this text hold an autolink literal at all?
fn autolink_candidate(bytes: &[u8]) -> bool {
    let mut from = 0usize;
    while let Some(relative) = find_any_of3(&bytes[from..], b'@', b':', b'.') {
        let index = from + relative;
        match bytes[index] {
            b'@' => return true,
            b':' => {
                // Every scheme we recognize ends right before this colon.
                let head = &bytes[..index];
                let schemes: [&[u8]; 5] = [b"http", b"https", b"ftp", b"mailto", b"xmpp"];
                for scheme in schemes {
                    if head.len() >= scheme.len()
                        && head[head.len() - scheme.len()..].eq_ignore_ascii_case(scheme)
                    {
                        return true;
                    }
                }
            }
            _ => {
                if index >= 3 && bytes[index - 3..index].eq_ignore_ascii_case(b"www") {
                    return true;
                }
            }
        }
        from = index + 1;
    }
    false
}

/// Finds the next `www.`, `http://`, `https://`, `mailto:`, `xmpp:`, or
/// email autolink in `text` at or after `from`. Returns `(start, end, url)`.
/// A piece of a split text node: a range of the block text when the node
/// was one, otherwise a copy.
fn piece(text: &str, start: usize, end: usize, base: Option<usize>) -> Span {
    match base {
        Some(base) => Span::Range(base + start, base + end),
        None => Span::Owned(text[start..end].to_string()),
    }
}

/// Returns `(start, end, prefix)`: the literal's bounds and the scheme the
/// destination needs in front of it (empty when the text already has one).
fn find_autolink_literal(text: &str, from: usize) -> Option<(usize, usize, &'static str)> {
    let bytes = text.as_bytes();
    let mut index = from;
    while index < bytes.len() {
        let byte = bytes[index];
        let could_start_scheme = matches!(byte, b'w' | b'W' | b'h' | b'H' | b'f' | b'F');
        if !could_start_scheme && byte != b'@' {
            index += 1;
            continue;
        }
        // `www.` needs whitespace or one of `*_~(` before it; a scheme only
        // needs a non-alphanumeric character (quotes and brackets count).
        let strict_boundary = index == 0
            || matches!(
                bytes[index - 1],
                b' ' | b'\t' | b'\n' | b'*' | b'_' | b'~' | b'('
            );
        let loose_boundary = index == 0 || !bytes[index - 1].is_ascii_alphanumeric();
        if loose_boundary {
            if let Some((end, prefix, is_www)) = scan_www_or_scheme(text, index) {
                if !is_www || strict_boundary {
                    return Some((index, end, prefix));
                }
            }
        }
        if bytes[index] == b'@' {
            if let Some((start, end, prefix)) = scan_email_literal(text, index) {
                return Some((start, end, prefix));
            }
        }
        index += 1;
    }
    None
}

/// Returns `(end, prefix, is_www)`; `www.` literals need `http://` added.
fn scan_www_or_scheme(text: &str, start: usize) -> Option<(usize, &'static str, bool)> {
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
    let prefix = if add_scheme { "http://" } else { "" };
    Some((end, prefix, add_scheme))
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

/// Returns `(start, end, prefix)`; a bare address needs `mailto:` added.
fn scan_email_literal(text: &str, at_position: usize) -> Option<(usize, usize, &'static str)> {
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
    let (link_start, prefix) = if mailto {
        (start - 7, "")
    } else if xmpp {
        (start - 5, "")
    } else {
        (start, "mailto:")
    };
    Some((link_start, end, prefix))
}
