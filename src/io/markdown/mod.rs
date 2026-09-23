//! Markdown parser: CommonMark 0.31.2 plus the GitHub Flavored Markdown
//! extensions (tables, strikethrough, task lists, autolink literals, the raw
//! HTML tag filter) and footnotes.
//!
//! The parser yields a stream of [`Event`]s. Renderers consume the stream;
//! the parser never knows what it is feeding. Block structure is parsed
//! first over the whole document (link reference definitions may appear
//! after their use), then inline content is parsed one block at a time as
//! events are pulled.

mod block;
pub mod entities;
pub mod events_json;
mod inline;
mod scan;
pub mod writer;

use std::borrow::Cow;

pub use block::Options;
pub use writer::{MarkdownWriter, push_markdown};

/// Text alignment of a table column, from the delimiter row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alignment {
    None,
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeBlockKind<'a> {
    Indented,
    /// The info string after the opening fence, entity- and escape-decoded.
    Fenced(Cow<'a, str>),
}

/// Start of a container. Every `Start(tag)` is followed, after its content,
/// by the matching `End(tag_end)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tag<'a> {
    Paragraph,
    Heading(u8),
    BlockQuote,
    CodeBlock(CodeBlockKind<'a>),
    HtmlBlock,
    /// `start` is `Some` for an ordered list. A tight list renders its
    /// items' paragraphs without `<p>` tags.
    List {
        start: Option<u64>,
        tight: bool,
    },
    Item,
    FootnoteDefinition(Cow<'a, str>),
    Table(Vec<Alignment>),
    TableHead,
    TableRow,
    TableCell,
    Emphasis,
    Strong,
    Strikethrough,
    Link {
        destination: Cow<'a, str>,
        title: Cow<'a, str>,
    },
    Image {
        destination: Cow<'a, str>,
        title: Cow<'a, str>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagEnd {
    Paragraph,
    Heading(u8),
    BlockQuote,
    CodeBlock,
    HtmlBlock,
    List(bool),
    Item,
    FootnoteDefinition,
    Table,
    TableHead,
    TableRow,
    TableCell,
    Emphasis,
    Strong,
    Strikethrough,
    Link,
    Image,
}

impl Tag<'_> {
    pub fn end(&self) -> TagEnd {
        match self {
            Tag::Paragraph => TagEnd::Paragraph,
            Tag::Heading(level) => TagEnd::Heading(*level),
            Tag::BlockQuote => TagEnd::BlockQuote,
            Tag::CodeBlock(_) => TagEnd::CodeBlock,
            Tag::HtmlBlock => TagEnd::HtmlBlock,
            Tag::List { start, .. } => TagEnd::List(start.is_some()),
            Tag::Item => TagEnd::Item,
            Tag::FootnoteDefinition(_) => TagEnd::FootnoteDefinition,
            Tag::Table(_) => TagEnd::Table,
            Tag::TableHead => TagEnd::TableHead,
            Tag::TableRow => TagEnd::TableRow,
            Tag::TableCell => TagEnd::TableCell,
            Tag::Emphasis => TagEnd::Emphasis,
            Tag::Strong => TagEnd::Strong,
            Tag::Strikethrough => TagEnd::Strikethrough,
            Tag::Link { .. } => TagEnd::Link,
            Tag::Image { .. } => TagEnd::Image,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event<'a> {
    Start(Tag<'a>),
    End(TagEnd),
    /// Text with escapes and entities already resolved.
    Text(Cow<'a, str>),
    /// Contents of a code span, normalized per the spec.
    Code(Cow<'a, str>),
    /// Raw HTML from an HTML block (one line, including its newline).
    Html(Cow<'a, str>),
    /// Raw inline HTML.
    InlineHtml(Cow<'a, str>),
    SoftBreak,
    HardBreak,
    Rule,
    FootnoteReference(Cow<'a, str>),
    /// `[ ]` or `[x]` at the start of a list item.
    TaskListMarker(bool),
}

/// Receives events as the parser produces them. Push-mode counterpart of
/// iterating a [`Parser`]: nothing is buffered between the parser and the
/// sink, so a renderer can consume each event straight from the source.
pub trait EventSink<'a> {
    /// An event whose text borrows from the source document.
    fn event(&mut self, event: Event<'a>);

    /// An event whose text borrows from storage that only lives for this
    /// call, such as lines joined across a list item. A sink that keeps
    /// events must copy it; a sink that renders immediately need not.
    fn transient(&mut self, event: Event<'_>) {
        self.event(inline::into_static(event));
    }
}

impl<'a> EventSink<'a> for Vec<Event<'a>> {
    fn event(&mut self, event: Event<'a>) {
        self.push(event);
    }
}

/// Parses `text` and pushes every event into `sink`. Inline content is
/// parsed one top-level block at a time, so memory stays proportional to
/// the input rather than to the event stream.
pub fn parse_into<'a>(text: &'a str, options: Options, sink: &mut dyn EventSink<'a>) {
    let document = block::parse(text, options);
    let mut scratch = inline::InlineScratch::default();
    let mut block = document.nodes[0].first_child;
    while block != block::NONE {
        let index = block as usize;
        block = document.nodes[index].next_sibling;
        inline::render_block(&document, text, index, &mut scratch, sink);
    }
}

/// Pull parser over one document. Iterate it to receive events. Inline
/// content is parsed one top-level block at a time as events are pulled,
/// so memory stays proportional to the input rather than to the event
/// stream.
pub struct Parser<'a> {
    text: &'a str,
    document: block::Document,
    /// Next top-level block to render, or `block::NONE`.
    next_block: u32,
    /// Events of the current block; `pending_index` is the next to hand out.
    /// Keeps its capacity across blocks.
    pending: Vec<Event<'a>>,
    pending_index: usize,
    scratch: inline::InlineScratch,
}

impl<'a> Parser<'a> {
    pub fn new(text: &'a str) -> Parser<'a> {
        Parser::new_with_options(text, Options::default())
    }

    pub fn new_with_options(text: &'a str, options: Options) -> Parser<'a> {
        let document = block::parse(text, options);
        let first_block = document.nodes[0].first_child;
        Parser {
            text,
            document,
            next_block: first_block,
            pending: Vec::new(),
            pending_index: 0,
            scratch: inline::InlineScratch::default(),
        }
    }
}

impl<'a> Iterator for Parser<'a> {
    type Item = Event<'a>;

    fn next(&mut self) -> Option<Event<'a>> {
        loop {
            if self.pending_index < self.pending.len() {
                let event =
                    std::mem::replace(&mut self.pending[self.pending_index], Event::SoftBreak);
                self.pending_index += 1;
                return Some(event);
            }
            if self.next_block == block::NONE {
                return None;
            }
            let block = self.next_block as usize;
            self.next_block = self.document.nodes[block].next_sibling;
            self.pending.clear();
            self.pending_index = 0;
            inline::render_block(
                &self.document,
                self.text,
                block,
                &mut self.scratch,
                &mut self.pending,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_reported() {
        eprintln!(
            "sizes: Event {} Tag {} Node {} Line {}",
            std::mem::size_of::<Event<'_>>(),
            std::mem::size_of::<Tag<'_>>(),
            std::mem::size_of::<block::Node>(),
            std::mem::size_of::<block::Line>()
        );
    }
}
