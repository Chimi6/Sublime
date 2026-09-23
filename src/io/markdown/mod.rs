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
mod inline;
mod scan;

use std::borrow::Cow;

pub use block::Options;

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

/// Pull parser over one document. Iterate it to receive events. Inline
/// content is parsed one top-level block at a time as events are pulled,
/// so memory stays proportional to the input rather than to the event
/// stream.
pub struct Parser<'a> {
    document: block::Document,
    next_block: usize,
    pending: std::vec::IntoIter<Event<'a>>,
}

impl<'a> Parser<'a> {
    pub fn new(text: &'a str) -> Parser<'a> {
        Parser::new_with_options(text, Options::default())
    }

    pub fn new_with_options(text: &'a str, options: Options) -> Parser<'a> {
        let document = block::parse(text, options);
        Parser {
            document,
            next_block: 0,
            pending: Vec::new().into_iter(),
        }
    }
}

impl<'a> Iterator for Parser<'a> {
    type Item = Event<'a>;

    fn next(&mut self) -> Option<Event<'a>> {
        loop {
            if let Some(event) = self.pending.next() {
                return Some(event);
            }
            let root_children = &self.document.nodes[0].children;
            let block = *root_children.get(self.next_block)?;
            self.next_block += 1;
            let events = inline::render_block(&self.document, block);
            self.pending = events.into_iter();
        }
    }
}
