//! Reads HTML into the Markdown event stream: the path from web pages
//! and exported documents into Markdown, text, and (through the events
//! bridge) Word.
//!
//! This is not a browser's parser. It tokenizes tags, attributes, text,
//! comments, and entities, and builds the event stream straight from the
//! tokens with an element stack, the way a reader skims a page: block
//! elements open and close blocks, phrasing elements open and close
//! formatting, text between them collapses its whitespace, and tag soup
//! (unclosed `p` and `li`, stray end tags, uppercase names, unquoted
//! attributes) is taken in stride. Elements with no Markdown form are
//! transparent (their text flows through); scripts, styles, and the head
//! are skipped.
//!
//! The footnote and task-list markup of cmark-gfm (what `HtmlWriter`
//! produces) is read back as footnotes and task markers.

use std::borrow::Cow;

use crate::io::markdown::entities;
use crate::io::markdown::{Alignment, CodeBlockKind, Event, EventSink, Tag, TagEnd};
use crate::io::scan::find_byte;

/// Parses `html` and pushes every event into `sink`.
pub fn parse_into<'a>(html: &'a str, sink: &mut dyn EventSink<'a>) {
    let mut reader = Reader {
        html,
        position: 0,
        sink,
        stack: Vec::new(),
        pending_space: false,
        at_start: true,
        table: None,
        scratch: String::new(),
    };
    reader.run();
}

/// An open element and what closing it emits.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Node {
    /// A paragraph opened by `<p>` or, when `implicit`, by text that
    /// arrived outside any phrasing container.
    Paragraph {
        implicit: bool,
    },
    Heading(u8),
    BlockQuote,
    /// `<pre>`: text is verbatim; `ended_with_newline` decides the final
    /// newline every code block carries.
    Pre {
        ended_with_newline: bool,
    },
    List,
    Item,
    Table,
    TableHead,
    TableRow,
    TableCell,
    Emphasis,
    Strong,
    Strike,
    Link,
    /// A `<code>` span; its inner tags are ignored.
    Code,
    FootnoteDefinition,
    /// A block container with no Markdown form (`div`, `section`): its
    /// blocks flow through, and closing it closes them.
    Block,
    /// An inline element with no Markdown form (`span`, `font`): its text
    /// flows through, and closing it closes nothing.
    Transparent,
}

struct Open<'a> {
    name: &'a str,
    node: Node,
}

/// The events of a table, held until its first row tells the columns and
/// their alignments.
struct TableBuffer<'a> {
    events: Vec<Event<'a>>,
    alignments: Vec<Alignment>,
    /// Cells of the head row seen so far.
    head_cells: usize,
    in_head: bool,
    head_done: bool,
}

struct Reader<'a, 's> {
    html: &'a str,
    position: usize,
    sink: &'s mut dyn EventSink<'a>,
    stack: Vec<Open<'a>>,
    /// Collapsed whitespace waiting for the next text.
    pending_space: bool,
    /// At the start of a phrasing container: leading whitespace is dropped.
    at_start: bool,
    table: Option<TableBuffer<'a>>,
    scratch: String,
}

/// A parsed start tag's attributes: (name, raw value).
type Attributes<'a> = Vec<(&'a str, &'a str)>;

impl<'a> Reader<'a, '_> {
    fn run(&mut self) {
        while self.position < self.html.len() {
            let rest = &self.html[self.position..];
            if let Some(tag_start) = find_byte(rest.as_bytes(), b'<') {
                if tag_start > 0 {
                    self.text(&rest[..tag_start]);
                }
                self.position += tag_start;
                self.tag();
            } else {
                self.text(rest);
                self.position = self.html.len();
            }
        }
        self.close_all();
    }

    // ----- the tokenizer -----

    /// Reads the tag at `position` (which is `<`) and acts on it.
    fn tag(&mut self) {
        let html = self.html;
        let rest = &html[self.position..];
        let after = &rest[1..];
        if let Some(comment) = after.strip_prefix("!--") {
            let end = comment.find("-->").map_or(after.len(), |index| index + 3);
            self.position += 1 + end;
            return;
        }
        if after.starts_with('!') || after.starts_with('?') {
            let end = after.find('>').map_or(after.len(), |index| index + 1);
            self.position += 1 + end;
            return;
        }
        if let Some(end_tag) = after.strip_prefix('/') {
            let end = end_tag.find('>').unwrap_or(end_tag.len());
            let name = end_tag[..end].trim();
            self.position += 2 + end + 1;
            self.position = self.position.min(self.html.len());
            self.end_tag(name);
            return;
        }
        let name_end = after
            .find(|ch: char| ch.is_ascii_whitespace() || ch == '>' || ch == '/')
            .unwrap_or(after.len());
        let name = &after[..name_end];
        if name.is_empty() {
            // A bare `<` is text.
            self.text("<");
            self.position += 1;
            return;
        }
        let (attributes, self_closing, consumed) = parse_attributes(&after[name_end..]);
        self.position += 1 + name_end + consumed;
        self.start_tag(name, &attributes, self_closing);
    }

    /// Skips to the end tag of a raw-text element such as `script`.
    fn skip_raw_text(&mut self, name: &str) {
        let rest = &self.html[self.position..];
        let bytes = rest.as_bytes();
        let mut index = 0usize;
        while let Some(found) = find_byte(&bytes[index..], b'<') {
            let at = index + found;
            let candidate = &rest[at + 1..];
            if let Some(after_slash) = candidate.strip_prefix('/')
                && after_slash.len() >= name.len()
                && after_slash.is_char_boundary(name.len())
                && after_slash[..name.len()].eq_ignore_ascii_case(name)
            {
                let close = after_slash.find('>').map_or(after_slash.len(), |i| i + 1);
                self.position += at + 2 + close;
                return;
            }
            index = at + 1;
        }
        self.position = self.html.len();
    }

    // ----- elements -----

    fn start_tag(&mut self, name: &'a str, attributes: &[(&'a str, &'a str)], self_closing: bool) {
        let lower = LowerName::of(name);
        // Inside a code span, inline tags are ignored; a block tag ends it.
        if self.is_in(Node::Code) {
            if is_block_name(lower.as_str()) {
                self.close_code();
            } else {
                return;
            }
        }
        if self.in_pre() {
            if lower.is("br") {
                self.pre_text("\n");
            }
            return;
        }
        match lower.as_str() {
            "script" | "style" | "head" | "template" | "svg" | "iframe" | "noscript" | "select"
            | "textarea" | "canvas" | "video" | "audio" | "object" | "title" | "math" => {
                if !self_closing {
                    self.skip_raw_text(name);
                }
            }
            "p" => {
                self.close_blocks_for_block();
                self.push(name, Node::Paragraph { implicit: false }, Tag::Paragraph);
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.close_blocks_for_block();
                let level = lower.as_str().as_bytes()[1] - b'0';
                self.push(name, Node::Heading(level), Tag::Heading(level));
            }
            "blockquote" => {
                self.close_blocks_for_block();
                self.push(name, Node::BlockQuote, Tag::BlockQuote);
            }
            "pre" => {
                self.close_blocks_for_block();
                let language = self.peek_code_language();
                let kind = CodeBlockKind::Fenced(Cow::Borrowed(language));
                self.push(
                    name,
                    Node::Pre {
                        ended_with_newline: true,
                    },
                    Tag::CodeBlock(kind),
                );
                // One newline right after <pre> is part of the markup.
                if self.html[self.position..].starts_with('\n') {
                    self.position += 1;
                }
            }
            "ul" | "ol" if self.in_footnotes() => self.push_block_container(name),
            "ul" | "ol" => {
                self.close_blocks_for_block();
                let start = if lower.is("ol") {
                    Some(
                        attribute(attributes, "start")
                            .and_then(|value| value.trim().parse::<u64>().ok())
                            .unwrap_or(1),
                    )
                } else {
                    None
                };
                let tight = !self.peek_loose_list();
                self.push(name, Node::List, Tag::List { start, tight });
            }
            "li" => {
                if self.in_footnotes() {
                    self.close_to_footnotes_list();
                    let label = attribute(attributes, "id")
                        .and_then(|id| id.strip_prefix("fn-"))
                        .unwrap_or("");
                    self.push(
                        name,
                        Node::FootnoteDefinition,
                        Tag::FootnoteDefinition(Cow::Borrowed(label)),
                    );
                    return;
                }
                self.close_to_list_for_item();
                self.push(name, Node::Item, Tag::Item);
            }
            "hr" => {
                self.close_blocks_for_block();
                self.emit(Event::Rule);
            }
            "table" => {
                self.close_blocks_for_block();
                if self.table.is_some() {
                    // A nested table has no Markdown form; its text flows.
                    self.push_transparent(name);
                    return;
                }
                self.table = Some(TableBuffer {
                    events: Vec::new(),
                    alignments: Vec::new(),
                    head_cells: 0,
                    in_head: false,
                    head_done: false,
                });
                self.stack.push(Open {
                    name,
                    node: Node::Table,
                });
            }
            "thead" | "tbody" | "tfoot" => {
                if self.table.is_some() {
                    self.close_to(Node::Table);
                }
                self.push_block_container(name);
            }
            "tr" => {
                if self.table.is_none() {
                    self.push_transparent(name);
                    return;
                }
                self.close_row();
                let head_done = self.table.as_ref().is_some_and(|table| table.head_done);
                if head_done {
                    self.push(name, Node::TableRow, Tag::TableRow);
                } else {
                    if let Some(table) = self.table.as_mut() {
                        table.in_head = true;
                    }
                    self.push(name, Node::TableHead, Tag::TableHead);
                }
            }
            "td" | "th" => {
                if self.table.is_none() {
                    self.push_transparent(name);
                    return;
                }
                self.close_cell();
                if !self.is_in(Node::TableHead) && !self.is_in(Node::TableRow) {
                    // A cell with no row: open one.
                    let head_done = self.table.as_ref().is_some_and(|table| table.head_done);
                    if head_done {
                        self.push("tr", Node::TableRow, Tag::TableRow);
                    } else {
                        if let Some(table) = self.table.as_mut() {
                            table.in_head = true;
                        }
                        self.push("tr", Node::TableHead, Tag::TableHead);
                    }
                }
                if let Some(table) = self.table.as_mut()
                    && table.in_head
                {
                    let alignment = cell_alignment(attributes);
                    table.alignments.push(alignment);
                    table.head_cells += 1;
                }
                self.push(name, Node::TableCell, Tag::TableCell);
            }
            "br" => {
                self.ensure_phrasing();
                self.emit(Event::HardBreak);
                self.pending_space = false;
                self.at_start = true;
            }
            "img" => {
                self.ensure_phrasing();
                let destination = attribute(attributes, "src").unwrap_or("");
                let title = attribute(attributes, "title").unwrap_or("");
                let alt = attribute(attributes, "alt").unwrap_or("");
                self.flush_space();
                self.emit(Event::Start(Tag::Image {
                    destination: decode_attribute(destination),
                    title: decode_attribute(title),
                }));
                if !alt.is_empty() {
                    self.emit_text(alt);
                }
                self.emit(Event::End(TagEnd::Image));
            }
            "a" => {
                if attribute(attributes, "class")
                    .is_some_and(|class| class.contains("footnote-backref"))
                {
                    // The way back from a footnote is not content.
                    self.skip_raw_text(name);
                    return;
                }
                let Some(href) = attribute(attributes, "href") else {
                    self.push_transparent(name);
                    return;
                };
                self.ensure_phrasing();
                self.flush_space();
                let title = attribute(attributes, "title").unwrap_or("");
                self.push(
                    name,
                    Node::Link,
                    Tag::Link {
                        destination: decode_attribute(href),
                        title: decode_attribute(title),
                    },
                );
            }
            "em" | "i" | "cite" | "var" | "dfn" => {
                self.ensure_phrasing();
                self.flush_space();
                self.push(name, Node::Emphasis, Tag::Emphasis);
            }
            "strong" | "b" => {
                self.ensure_phrasing();
                self.flush_space();
                self.push(name, Node::Strong, Tag::Strong);
            }
            "s" | "del" | "strike" => {
                self.ensure_phrasing();
                self.flush_space();
                self.push(name, Node::Strike, Tag::Strikethrough);
            }
            "code" | "kbd" | "samp" | "tt" => {
                self.ensure_phrasing();
                self.flush_space();
                self.scratch.clear();
                self.stack.push(Open {
                    name,
                    node: Node::Code,
                });
            }
            "sup"
                if attribute(attributes, "class")
                    .is_some_and(|class| class.contains("footnote-ref")) =>
            {
                self.ensure_phrasing();
                self.flush_space();
                let label = self.peek_footnote_label();
                self.emit(Event::FootnoteReference(Cow::Borrowed(label)));
                self.skip_raw_text(name);
            }
            "input" => {
                if attribute(attributes, "type")
                    .is_some_and(|kind| kind.eq_ignore_ascii_case("checkbox"))
                {
                    self.ensure_phrasing();
                    let checked = attribute(attributes, "checked").is_some();
                    self.emit(Event::TaskListMarker(checked));
                    self.pending_space = false;
                    self.at_start = true;
                }
            }
            "section"
                if attribute(attributes, "class")
                    .is_some_and(|class| class.contains("footnotes")) =>
            {
                self.close_blocks_for_block();
                // Named for the list and items inside to find.
                self.stack.push(Open {
                    name: "section.footnotes",
                    node: Node::Block,
                });
            }
            "div" | "section" | "article" | "main" | "header" | "footer" | "nav" | "aside"
            | "figure" | "figcaption" | "details" | "summary" | "dl" | "dt" | "dd" | "address"
            | "center" | "form" | "fieldset" | "body" | "html" => {
                self.close_blocks_for_block();
                self.push_block_container(name);
            }
            _ => {
                if !self_closing {
                    self.push_transparent(name);
                }
            }
        }
    }

    fn end_tag(&mut self, name: &str) {
        let lower = LowerName::of(name);
        if self.is_in(Node::Code) {
            if lower.is("code") || lower.is("kbd") || lower.is("samp") || lower.is("tt") {
                self.close_code();
            }
            return;
        }
        if self.in_pre() {
            if lower.is("pre") {
                self.close_to_and_pop(Node::Pre {
                    ended_with_newline: false,
                });
            }
            return;
        }
        // The nearest open element with this name; nothing above it survives.
        let Some(index) = self.stack.iter().rposition(|open| {
            open.name.eq_ignore_ascii_case(name)
                || (open.name == "section.footnotes" && lower.is("section"))
        }) else {
            return;
        };
        if self.stack[index].node == Node::Transparent {
            self.stack.remove(index);
            return;
        }
        while self.stack.len() > index {
            self.pop();
        }
    }

    // ----- the stack -----

    fn push(&mut self, name: &'a str, node: Node, tag: Tag<'a>) {
        self.emit(Event::Start(tag));
        self.stack.push(Open { name, node });
        if matches!(
            node,
            Node::Paragraph { .. }
                | Node::Heading(_)
                | Node::TableCell
                | Node::Item
                | Node::BlockQuote
                | Node::Pre { .. }
                | Node::FootnoteDefinition
        ) {
            self.pending_space = false;
            self.at_start = true;
        }
    }

    fn push_transparent(&mut self, name: &'a str) {
        self.stack.push(Open {
            name,
            node: Node::Transparent,
        });
    }

    fn push_block_container(&mut self, name: &'a str) {
        self.stack.push(Open {
            name,
            node: Node::Block,
        });
    }

    fn pop(&mut self) {
        let Some(open) = self.stack.pop() else {
            return;
        };
        self.pending_space = false;
        let end = match open.node {
            Node::Paragraph { .. } => Some(TagEnd::Paragraph),
            Node::Heading(level) => Some(TagEnd::Heading(level)),
            Node::BlockQuote => Some(TagEnd::BlockQuote),
            Node::Pre { ended_with_newline } => {
                if !ended_with_newline {
                    self.emit(Event::Text(Cow::Borrowed("\n")));
                }
                Some(TagEnd::CodeBlock)
            }
            Node::List => Some(TagEnd::List(open.name.eq_ignore_ascii_case("ol"))),
            Node::Item => Some(TagEnd::Item),
            Node::Table => {
                self.finish_table();
                None
            }
            Node::TableHead => {
                if let Some(table) = self.table.as_mut() {
                    table.in_head = false;
                    table.head_done = true;
                }
                Some(TagEnd::TableHead)
            }
            Node::TableRow => Some(TagEnd::TableRow),
            Node::TableCell => Some(TagEnd::TableCell),
            Node::Emphasis => Some(TagEnd::Emphasis),
            Node::Strong => Some(TagEnd::Strong),
            Node::Strike => Some(TagEnd::Strikethrough),
            Node::Link => Some(TagEnd::Link),
            Node::Code => {
                self.close_code_events();
                None
            }
            Node::FootnoteDefinition => Some(TagEnd::FootnoteDefinition),
            Node::Block | Node::Transparent => None,
        };
        if let Some(end) = end {
            self.emit(Event::End(end));
        }
    }

    fn close_all(&mut self) {
        while !self.stack.is_empty() {
            self.pop();
        }
        self.finish_table();
    }

    fn is_in(&self, node: Node) -> bool {
        self.stack
            .last()
            .is_some_and(|open| std::mem::discriminant(&open.node) == std::mem::discriminant(&node))
    }

    fn in_pre(&self) -> bool {
        self.stack
            .iter()
            .any(|open| matches!(open.node, Node::Pre { .. }))
    }

    fn in_footnotes(&self) -> bool {
        self.stack
            .iter()
            .any(|open| open.name == "section.footnotes")
    }

    /// A block is starting: any implicit paragraph and the inline
    /// formatting above the nearest block container close.
    fn close_blocks_for_block(&mut self) {
        while let Some(open) = self.stack.last() {
            match open.node {
                Node::Paragraph { .. }
                | Node::Heading(_)
                | Node::Emphasis
                | Node::Strong
                | Node::Strike
                | Node::Link
                | Node::Code => self.pop(),
                _ => break,
            }
        }
    }

    /// An item is starting: the previous item of the nearest list closes.
    fn close_to_list_for_item(&mut self) {
        let Some(list_index) = self.stack.iter().rposition(|open| open.node == Node::List) else {
            // An item with no list: give it one.
            self.close_blocks_for_block();
            self.push(
                "ul",
                Node::List,
                Tag::List {
                    start: None,
                    tight: true,
                },
            );
            return;
        };
        while self.stack.len() > list_index + 1 {
            self.pop();
        }
    }

    /// A footnote is starting: whatever the previous one left open closes,
    /// down to the section's list.
    fn close_to_footnotes_list(&mut self) {
        while let Some(open) = self.stack.last() {
            let is_list = open.node == Node::Block
                && (open.name.eq_ignore_ascii_case("ol") || open.name == "section.footnotes");
            if is_list {
                break;
            }
            self.pop();
        }
    }

    fn close_to(&mut self, node: Node) {
        while let Some(open) = self.stack.last() {
            if std::mem::discriminant(&open.node) == std::mem::discriminant(&node) {
                break;
            }
            self.pop();
        }
    }

    fn close_to_and_pop(&mut self, node: Node) {
        self.close_to(node);
        if self
            .stack
            .last()
            .is_some_and(|open| std::mem::discriminant(&open.node) == std::mem::discriminant(&node))
        {
            self.pop();
        }
    }

    fn close_row(&mut self) {
        while let Some(open) = self.stack.last() {
            match open.node {
                Node::Table | Node::Block => break,
                _ => self.pop(),
            }
        }
    }

    fn close_cell(&mut self) {
        while let Some(open) = self.stack.last() {
            match open.node {
                Node::TableHead | Node::TableRow | Node::Table | Node::Block => break,
                _ => self.pop(),
            }
        }
    }

    /// Text needs a paragraph unless a heading, cell, or paragraph is
    /// already open. Inline elements always open one before themselves,
    /// so none is ever open without a container beneath it.
    fn ensure_phrasing(&mut self) {
        let has_container = self.stack.iter().rev().any(|open| {
            matches!(
                open.node,
                Node::Paragraph { .. } | Node::Heading(_) | Node::TableCell | Node::Pre { .. }
            )
        });
        if has_container {
            return;
        }
        self.emit(Event::Start(Tag::Paragraph));
        self.stack.push(Open {
            name: "",
            node: Node::Paragraph { implicit: true },
        });
        self.at_start = true;
        self.pending_space = false;
    }

    // ----- text -----

    fn text(&mut self, text: &'a str) {
        if self.is_in(Node::Code) {
            decode_into(text, &mut self.scratch);
            return;
        }
        if self.in_pre() {
            self.pre_text(text);
            return;
        }
        // Between a table's cells, and in a footnote section outside a
        // definition, text is markup.
        let in_table_gap = self.table.is_some()
            && !self
                .stack
                .iter()
                .rev()
                .any(|open| open.node == Node::TableCell);
        if in_table_gap && text.trim().is_empty() {
            return;
        }
        if self.in_footnotes()
            && !self
                .stack
                .iter()
                .any(|open| open.node == Node::FootnoteDefinition)
        {
            return;
        }
        let mut rest = text;
        loop {
            let trimmed = rest.trim_start_matches(is_html_space);
            if trimmed.len() != rest.len() {
                self.pending_space = true;
            }
            rest = trimmed;
            if rest.is_empty() {
                return;
            }
            // The longest piece whose whitespace is already single spaces
            // is handed over as one borrowed slice.
            let end = borrowable_end(rest);
            let piece = &rest[..end];
            self.ensure_phrasing();
            self.flush_space();
            self.emit_text(piece);
            rest = &rest[end..];
        }
    }

    fn pre_text(&mut self, text: &'a str) {
        if text.is_empty() {
            return;
        }
        let ended_with_newline = text.ends_with('\n');
        if text.contains('&') {
            self.scratch.clear();
            decode_into(text, &mut self.scratch);
            let owned = std::mem::take(&mut self.scratch);
            self.emit(Event::Text(Cow::Owned(owned)));
        } else {
            self.emit(Event::Text(Cow::Borrowed(text)));
        }
        for open in self.stack.iter_mut().rev() {
            if let Node::Pre {
                ended_with_newline: flag,
            } = &mut open.node
            {
                *flag = ended_with_newline;
                break;
            }
        }
    }

    /// A word of text, entities decoded.
    fn emit_text(&mut self, word: &'a str) {
        if word.contains('&') {
            self.scratch.clear();
            decode_into(word, &mut self.scratch);
            let owned = std::mem::take(&mut self.scratch);
            self.emit(Event::Text(Cow::Owned(owned)));
        } else {
            self.emit(Event::Text(Cow::Borrowed(word)));
        }
        self.at_start = false;
    }

    fn flush_space(&mut self) {
        if self.pending_space && !self.at_start {
            self.emit(Event::Text(Cow::Borrowed(" ")));
        }
        self.pending_space = false;
    }

    fn close_code(&mut self) {
        self.stack.pop();
        self.close_code_events();
    }

    fn close_code_events(&mut self) {
        let text = std::mem::take(&mut self.scratch);
        self.emit(Event::Code(Cow::Owned(text)));
        self.at_start = false;
    }

    // ----- tables -----

    fn finish_table(&mut self) {
        let Some(table) = self.table.take() else {
            return;
        };
        let mut alignments = table.alignments;
        let columns = alignments.len().max(1);
        alignments.resize(columns, Alignment::None);
        self.sink.event(Event::Start(Tag::Table(alignments)));
        let mut had_head = false;
        for event in table.events {
            if let Event::Start(Tag::TableHead) = event {
                had_head = true;
            }
            self.sink.event(event);
        }
        if !had_head {
            self.sink.event(Event::Start(Tag::TableHead));
            self.sink.event(Event::End(TagEnd::TableHead));
        }
        self.sink.event(Event::End(TagEnd::Table));
    }

    fn emit(&mut self, event: Event<'a>) {
        match self.table.as_mut() {
            Some(table) => table.events.push(event),
            None => self.sink.event(event),
        }
    }

    // ----- lookahead -----

    /// After `<pre>`: the language of an inner `<code class="language-x">`.
    fn peek_code_language(&self) -> &'a str {
        let rest = self.html[self.position..].trim_start();
        let Some(after) = rest.strip_prefix("<code") else {
            return "";
        };
        let tag_end = after.find('>').unwrap_or(after.len());
        let tag = &after[..tag_end];
        let Some(class_at) = tag.find("class=") else {
            return "";
        };
        let value = &tag[class_at + 6..];
        let quote = value.chars().next().unwrap_or(' ');
        let value = if quote == '"' || quote == '\'' {
            let inner = &value[1..];
            &inner[..inner.find(quote).unwrap_or(inner.len())]
        } else {
            &value[..value.find(char::is_whitespace).unwrap_or(value.len())]
        };
        for class in value.split_whitespace() {
            if let Some(language) = class.strip_prefix("language-") {
                return language;
            }
        }
        ""
    }

    /// After `<ul>` or `<ol>`: whether the first item holds a `<p>` of its
    /// own (not inside a nested list), which is what a loose list is.
    fn peek_loose_list(&self) -> bool {
        let rest = self.html[self.position..].trim_start();
        let Some(after_li) = strip_prefix_ignore_case(rest, "<li") else {
            return false;
        };
        let mut depth = 0usize;
        let mut cursor = after_li;
        while let Some(at) = cursor.find('<') {
            let tag = &cursor[at + 1..];
            let (closing, body) = match tag.strip_prefix('/') {
                Some(body) => (true, body),
                None => (false, tag),
            };
            let name_end = body
                .find(|ch: char| ch.is_ascii_whitespace() || ch == '>' || ch == '/')
                .unwrap_or(body.len().min(16));
            let lower = LowerName::of(&body[..name_end]);
            let is_container = matches!(lower.as_str(), "ul" | "ol" | "table" | "blockquote");
            if is_container && !closing {
                depth += 1;
            } else if is_container {
                if depth == 0 {
                    return false;
                }
                depth -= 1;
            } else if depth == 0 && lower.is("p") && !closing {
                return true;
            } else if depth == 0 && lower.is("li") {
                return false;
            }
            cursor = tag;
        }
        false
    }

    /// Inside `<sup class="footnote-ref">`: the label of its `<a href="#fn-...">`.
    fn peek_footnote_label(&self) -> &'a str {
        let rest = &self.html[self.position..];
        let end = rest.find("</sup>").unwrap_or(rest.len());
        let inner = &rest[..end];
        let Some(at) = inner.find("#fn-") else {
            return "";
        };
        let label = &inner[at + 4..];
        &label[..label.find('"').unwrap_or(label.len())]
    }
}

/// A tag name compared without case, without allocating for the common
/// lowercase case.
struct LowerName<'a> {
    original: &'a str,
    lower: [u8; 16],
    length: usize,
}

impl<'a> LowerName<'a> {
    fn of(name: &'a str) -> LowerName<'a> {
        let mut lower = [0u8; 16];
        let length = name.len().min(16);
        for (index, byte) in name.as_bytes()[..length].iter().enumerate() {
            lower[index] = byte.to_ascii_lowercase();
        }
        LowerName {
            original: name,
            lower,
            length,
        }
    }

    fn as_str(&self) -> &str {
        if self.original.len() > 16 {
            return self.original;
        }
        std::str::from_utf8(&self.lower[..self.length]).unwrap_or(self.original)
    }

    fn is(&self, name: &str) -> bool {
        self.as_str() == name
    }
}

/// Parses attributes up to and including the tag's `>`; returns them, the
/// self-closing flag, and how many bytes were consumed.
fn parse_attributes(text: &str) -> (Attributes<'_>, bool, usize) {
    let mut attributes = Vec::new();
    let mut rest = text;
    let mut self_closing = false;
    loop {
        rest = rest.trim_start_matches(is_html_space);
        if rest.is_empty() {
            break;
        }
        if let Some(after) = rest.strip_prefix('>') {
            rest = after;
            break;
        }
        if let Some(after) = rest.strip_prefix("/>") {
            self_closing = true;
            rest = after;
            break;
        }
        if let Some(after) = rest.strip_prefix('/') {
            rest = after;
            continue;
        }
        let name_end = rest
            .find(|ch: char| is_html_space(ch) || ch == '=' || ch == '>' || ch == '/')
            .unwrap_or(rest.len());
        let name = &rest[..name_end];
        rest = &rest[name_end..];
        let after_name = rest.trim_start_matches(is_html_space);
        if let Some(after_equals) = after_name.strip_prefix('=') {
            let value_text = after_equals.trim_start_matches(is_html_space);
            let (value, remaining) = match value_text.chars().next() {
                Some(quote @ ('"' | '\'')) => {
                    let inner = &value_text[1..];
                    let end = inner.find(quote).unwrap_or(inner.len());
                    (&inner[..end], &inner[(end + 1).min(inner.len())..])
                }
                _ => {
                    let end = value_text
                        .find(|ch: char| is_html_space(ch) || ch == '>')
                        .unwrap_or(value_text.len());
                    (&value_text[..end], &value_text[end..])
                }
            };
            attributes.push((name, value));
            rest = remaining;
        } else {
            attributes.push((name, ""));
            rest = after_name;
        }
    }
    let consumed = text.len() - rest.len();
    (attributes, self_closing, consumed)
}

fn attribute<'a>(attributes: &[(&'a str, &'a str)], name: &str) -> Option<&'a str> {
    attributes
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| *value)
}

fn cell_alignment(attributes: &[(&str, &str)]) -> Alignment {
    let by_attribute = attribute(attributes, "align").map(|value| value.to_ascii_lowercase());
    let by_style = attribute(attributes, "style").and_then(|style| {
        let lower = style.to_ascii_lowercase();
        let at = lower.find("text-align")?;
        let value = lower[at + 10..].trim_start_matches([':', ' ']);
        Some(
            value[..value.find(';').unwrap_or(value.len())]
                .trim()
                .to_string(),
        )
    });
    match by_attribute.or(by_style).as_deref() {
        Some("left") => Alignment::Left,
        Some("center") => Alignment::Center,
        Some("right") => Alignment::Right,
        _ => Alignment::None,
    }
}

fn decode_attribute(value: &str) -> Cow<'_, str> {
    if value.contains('&') {
        let mut decoded = String::with_capacity(value.len());
        decode_into(value, &mut decoded);
        Cow::Owned(decoded)
    } else {
        Cow::Borrowed(value)
    }
}

/// Appends `text` with its character references decoded; an unknown or
/// malformed reference stays as written.
fn decode_into(text: &str, out: &mut String) {
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let end = after
            .find(|ch: char| ch == ';' || ch == '&' || is_html_space(ch) || ch == '<')
            .unwrap_or(after.len());
        let name = &after[..end];
        let terminated = after[end..].starts_with(';');
        let decoded = if let Some(number) = name.strip_prefix('#') {
            let code = if let Some(hex) = number.strip_prefix(['x', 'X']) {
                u32::from_str_radix(hex, 16).ok()
            } else {
                number.parse::<u32>().ok()
            };
            code.and_then(char::from_u32).map(|ch| {
                if ch == '\0' {
                    '\u{FFFD}'.to_string()
                } else {
                    ch.to_string()
                }
            })
        } else if name.is_empty() {
            None
        } else {
            entities::lookup(name).map(str::to_string)
        };
        match decoded {
            Some(decoded) if terminated => {
                out.push_str(&decoded);
                rest = &after[end + 1..];
            }
            _ => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
}

/// The length of the prefix of `text` (which starts with a non-space)
/// in which every whitespace run is one plain space between words.
fn borrowable_end(text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        if !is_html_space_byte(bytes[index]) {
            index += 1;
            continue;
        }
        let run_start = index;
        while index < bytes.len() && is_html_space_byte(bytes[index]) {
            index += 1;
        }
        let single_space = index - run_start == 1 && bytes[run_start] == b' ';
        if !single_space || index == bytes.len() {
            return run_start;
        }
    }
    bytes.len()
}

fn is_html_space_byte(byte: u8) -> bool {
    matches!(byte, b' ' | b'\n' | b'\t' | b'\r' | 0x0C)
}

/// Elements that end a code span or a paragraph when they start.
fn is_block_name(name: &str) -> bool {
    matches!(
        name,
        "p" | "div"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "ul"
            | "ol"
            | "li"
            | "table"
            | "tr"
            | "td"
            | "th"
            | "pre"
            | "blockquote"
            | "hr"
            | "section"
            | "article"
    )
}

fn is_html_space(ch: char) -> bool {
    matches!(ch, ' ' | '\n' | '\t' | '\r' | '\u{0C}')
}

fn strip_prefix_ignore_case<'t>(text: &'t str, prefix: &str) -> Option<&'t str> {
    if text.len() < prefix.len() || !text.is_char_boundary(prefix.len()) {
        return None;
    }
    if text[..prefix.len()].eq_ignore_ascii_case(prefix) {
        Some(&text[prefix.len()..])
    } else {
        None
    }
}
