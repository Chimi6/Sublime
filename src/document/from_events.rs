//! Builds a document from the Markdown event stream: the bridge from every
//! text-shaped input (Markdown, and later HTML and plain text) into the
//! document model, and so into Word.
//!
//! Markdown structure becomes named styles Word users know: `Heading 1`
//! to `Heading 6` with outline levels, `Quote`, `Source Code` (one
//! paragraph per block, lines joined by line breaks), `Source Text` for
//! code spans, `Horizontal Line` for a rule, and two list styles. The
//! projection in `markdown.rs` recognizes the same names, so a document
//! built here reads back to the Markdown it came from.

use crate::document::{
    Alignment, Block, Cell, CharacterStyle, Document, Inline, InlineImage, ListItem, ListLabel,
    ListLevel, ListStyle, Media, Note, NumberFormat, NumberKind, Paragraph, ParagraphProperties,
    ParagraphStyle, Placement, Row, Run, RunProperties, Section, StyleId, Table,
};
use crate::io::markdown::{Alignment as ColumnAlignment, Event, EventSink, Tag, TagEnd};

/// Points of indent per quote or list level.
const LEVEL_INDENT: f32 = 36.0;
/// The text width of a Letter page with one-inch margins, shared by a
/// table's columns.
const TEXT_WIDTH: f32 = 468.0;

pub struct DocumentBuilder {
    document: Document,
    styles: BuiltInStyles,
    /// Where finished blocks go: the body, then a table cell or a footnote
    /// definition while one is open.
    targets: Vec<Vec<Block>>,
    /// The paragraph being filled.
    current: Option<Paragraph>,
    /// The block the current paragraph belongs to.
    current_kind: ParagraphKind,
    quote_depth: u32,
    lists: Vec<ListState>,
    /// The list item the next paragraph opens; taken by the first
    /// paragraph of the item.
    pending_item: Option<ListItem>,
    tables: Vec<TableState>,
    /// Footnote labels to note ids, in order of first sight.
    notes: Vec<(String, usize)>,
    /// The notes whose definitions are open, innermost last.
    open_notes: Vec<usize>,
    /// Inline state.
    strong: u32,
    emphasis: u32,
    strike: u32,
    link: Option<u32>,
    /// An image being read: its destination and the alt text so far.
    image: Option<(String, String)>,
    /// Text of the code block being read, split into lines at the end.
    code_text: String,
    /// Interned ids of the eight bold/italic/strike combinations, so a
    /// run's formatting is one table entry however many text events it
    /// spans.
    property_ids: [Option<Option<u32>>; 8],
    dropped_html: bool,
    other_images: bool,
}

struct BuiltInStyles {
    headings: [StyleId; 6],
    quote: StyleId,
    code: StyleId,
    rule: StyleId,
    code_span: StyleId,
    bullets: StyleId,
    numbers: StyleId,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ParagraphKind {
    Plain,
    Code,
}

struct ListState {
    ordered: bool,
    start: u32,
    first_item: bool,
}

struct TableState {
    alignments: Vec<ColumnAlignment>,
    rows: Vec<Row>,
    row: Vec<Cell>,
    header_rows: u32,
    in_head: bool,
}

impl Default for DocumentBuilder {
    fn default() -> Self {
        DocumentBuilder::new()
    }
}

impl DocumentBuilder {
    pub fn new() -> DocumentBuilder {
        let mut document = Document::default();
        let styles = built_in_styles(&mut document);
        DocumentBuilder {
            document,
            styles,
            targets: vec![Vec::new()],
            current: None,
            current_kind: ParagraphKind::Plain,
            quote_depth: 0,
            lists: Vec::new(),
            pending_item: None,
            tables: Vec::new(),
            notes: Vec::new(),
            open_notes: Vec::new(),
            strong: 0,
            emphasis: 0,
            strike: 0,
            link: None,
            image: None,
            code_text: String::new(),
            property_ids: [None; 8],
            dropped_html: false,
            other_images: false,
        }
    }

    /// The document, with everything read so far closed.
    pub fn finish(mut self) -> Document {
        self.close_paragraph();
        let body = self.targets.swap_remove(0);
        self.document.sections.push(Section {
            blocks: body,
            ..Section::default()
        });
        self.document
    }

    /// Whether raw HTML was dropped on the way.
    pub fn dropped_html(&self) -> bool {
        self.dropped_html
    }

    /// Whether an image not given as a data URI became a link.
    pub fn other_images(&self) -> bool {
        self.other_images
    }

    // ----- blocks -----

    fn open_paragraph(&mut self, style: Option<StyleId>, kind: ParagraphKind) {
        self.close_paragraph();
        // A plain paragraph inside a quote takes the quote style; its
        // depth is carried by the indent.
        let style = match style {
            None if self.quote_depth > 0 && kind == ParagraphKind::Plain => Some(self.styles.quote),
            other => other,
        };
        let mut paragraph = Paragraph {
            style,
            ..Paragraph::default()
        };
        let mut properties = ParagraphProperties::default();
        let mut indent_levels = self.quote_depth;
        if let Some(item) = self.pending_item.take() {
            paragraph.list = Some(item);
        } else if !self.lists.is_empty() {
            // A later paragraph of a list item sits under the item's text.
            indent_levels += self.lists.len() as u32;
        }
        if indent_levels > 0 {
            properties.left_indent = Some(LEVEL_INDENT * indent_levels as f32);
        }
        if let Some(table) = self.tables.last() {
            let column = table.row.len();
            properties.alignment = match table.alignments.get(column) {
                Some(ColumnAlignment::Center) => Some(Alignment::Center),
                Some(ColumnAlignment::Right) => Some(Alignment::Right),
                Some(ColumnAlignment::Left) => Some(Alignment::Left),
                _ => None,
            };
        }
        paragraph.properties = self.document.intern_paragraph_properties(properties);
        self.current = Some(paragraph);
        self.current_kind = kind;
    }

    fn close_paragraph(&mut self) {
        let Some(mut paragraph) = self.current.take() else {
            return;
        };
        if self.current_kind == ParagraphKind::Code {
            self.push_code_lines(&mut paragraph);
        }
        if let Some(target) = self.targets.last_mut() {
            target.push(Block::Paragraph(paragraph));
        }
    }

    /// The code block's text as runs, one line break between lines; the
    /// block's final newline is not a line.
    fn push_code_lines(&mut self, paragraph: &mut Paragraph) {
        let text = std::mem::take(&mut self.code_text);
        let body = text.strip_suffix('\n').unwrap_or(&text);
        let mut first = true;
        for line in body.split('\n') {
            if !first {
                paragraph.runs.push(plain_run(Inline::LineBreak));
            }
            first = false;
            if !line.is_empty() {
                let span = self.document.push_text(line);
                paragraph.runs.push(plain_run(Inline::Text(span)));
            }
        }
    }

    /// A paragraph to put inline content in, opened when content arrives
    /// outside one (a table cell's text, for instance).
    fn ensure_paragraph(&mut self) {
        if self.current.is_none() {
            self.open_paragraph(None, ParagraphKind::Plain);
        }
    }

    fn push_block(&mut self, block: Block) {
        self.close_paragraph();
        if let Some(target) = self.targets.last_mut() {
            target.push(block);
        }
    }

    /// An item whose first block is not a paragraph (a nested list, a
    /// quote, a table) still needs its own paragraph to carry the marker.
    fn flush_pending_item(&mut self) {
        if let Some(item) = self.pending_item.take() {
            let paragraph = Paragraph {
                list: Some(item),
                ..Paragraph::default()
            };
            self.push_block(Block::Paragraph(paragraph));
        }
    }

    // ----- inline -----

    fn run_properties(&mut self) -> Option<u32> {
        let bold = self.strong > 0;
        let italic = self.emphasis > 0;
        let strike = self.strike > 0;
        let key = usize::from(bold) | (usize::from(italic) << 1) | (usize::from(strike) << 2);
        if let Some(id) = self.property_ids[key] {
            return id;
        }
        let properties = RunProperties {
            bold: bold.then_some(true),
            italic: italic.then_some(true),
            strike: strike.then_some(true),
            ..RunProperties::default()
        };
        let id = self.document.intern_run_properties(properties);
        self.property_ids[key] = Some(id);
        id
    }

    fn push_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if let Some((_, alt)) = &mut self.image {
            alt.push_str(text);
            return;
        }
        if self.current_kind == ParagraphKind::Code && self.current.is_some() {
            self.code_text.push_str(text);
            return;
        }
        self.ensure_paragraph();
        let properties = self.run_properties();
        let link = self.link;
        let arena_end = self.document.text.len() as u32;
        let Some(paragraph) = self.current.as_mut() else {
            return;
        };
        // Extend the last run when nothing about it changed.
        if let Some(Run {
            style: None,
            properties: last_properties,
            link: last_link,
            revision: None,
            content: Inline::Text(span),
        }) = paragraph.runs.last_mut()
            && *last_properties == properties
            && *last_link == link
            && span.end == arena_end
        {
            self.document.text.push_str(text);
            span.end = self.document.text.len() as u32;
            return;
        }
        let span = self.document.push_text(text);
        paragraph.runs.push(Run {
            style: None,
            properties,
            link,
            revision: None,
            content: Inline::Text(span),
        });
    }

    fn push_inline(&mut self, content: Inline) {
        self.ensure_paragraph();
        let properties = self.run_properties();
        let link = self.link;
        if let Some(paragraph) = self.current.as_mut() {
            paragraph.runs.push(Run {
                style: None,
                properties,
                link,
                revision: None,
                content,
            });
        }
    }

    fn push_code_span(&mut self, text: &str) {
        self.ensure_paragraph();
        let properties = self.run_properties();
        let link = self.link;
        let span = self.document.push_text(text);
        let style = Some(self.styles.code_span);
        if let Some(paragraph) = self.current.as_mut() {
            paragraph.runs.push(Run {
                style,
                properties,
                link,
                revision: None,
                content: Inline::Text(span),
            });
        }
    }

    fn note_id(&mut self, label: &str) -> usize {
        if let Some((_, id)) = self.notes.iter().find(|(known, _)| known == label) {
            return *id;
        }
        let id = self.document.footnotes.len();
        self.document.footnotes.push(Note::default());
        self.notes.push((label.to_string(), id));
        id
    }

    fn finish_image(&mut self) {
        let Some((destination, alt)) = self.image.take() else {
            return;
        };
        if let Some(media) = self.embed_data_uri(&destination) {
            let description = if alt.is_empty() { None } else { Some(alt) };
            let id = self.document.push_image(InlineImage {
                media,
                width: 0.0,
                height: 0.0,
                description,
                placement: Placement::Inline,
            });
            self.push_inline(Inline::Image(id));
            return;
        }
        // Anything else becomes a link to the image, named by its alt text.
        self.other_images = true;
        let saved_link = self.link;
        self.link = Some(self.document.intern_link(&destination));
        let text = if alt.is_empty() {
            destination.clone()
        } else {
            alt
        };
        self.push_text(&text);
        self.link = saved_link;
    }

    /// `data:<type>;base64,<bytes>` as a media entry.
    fn embed_data_uri(&mut self, destination: &str) -> Option<usize> {
        let rest = destination.strip_prefix("data:")?;
        let (header, payload) = rest.split_once(',')?;
        let (media_type, encoding) = header.split_once(';').unwrap_or((header, ""));
        if encoding != "base64" {
            return None;
        }
        let extension = match media_type {
            "image/png" => "png",
            "image/jpeg" => "jpg",
            "image/gif" => "gif",
            "image/bmp" => "bmp",
            "image/tiff" => "tiff",
            "image/svg+xml" => "svg",
            _ => return None,
        };
        let mut bytes = Vec::with_capacity(payload.len() / 4 * 3);
        crate::io::base64::decode(payload.trim(), &mut bytes)?;
        let id = self.document.media.len();
        self.document.media.push(Media {
            name: format!("image{}.{extension}", id + 1),
            bytes,
        });
        Some(id)
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => self.open_paragraph(None, ParagraphKind::Plain),
            Tag::Heading(level) => {
                let index = usize::from(level.clamp(1, 6)) - 1;
                let style = self.styles.headings[index];
                self.open_paragraph(Some(style), ParagraphKind::Plain);
            }
            Tag::BlockQuote => {
                self.close_paragraph();
                self.flush_pending_item();
                self.quote_depth += 1;
            }
            Tag::CodeBlock(_) => {
                let style = self.styles.code;
                self.open_paragraph(Some(style), ParagraphKind::Code);
                self.code_text.clear();
            }
            Tag::HtmlBlock => {
                self.close_paragraph();
                self.dropped_html = true;
            }
            Tag::List { start, .. } => {
                self.close_paragraph();
                self.flush_pending_item();
                self.lists.push(ListState {
                    ordered: start.is_some(),
                    start: start.unwrap_or(1).min(u64::from(u32::MAX)) as u32,
                    first_item: true,
                });
            }
            Tag::Item => {
                self.close_paragraph();
                let level = (self.lists.len().max(1) - 1).min(8) as u8;
                if let Some(list) = self.lists.last_mut() {
                    let style = if list.ordered {
                        self.styles.numbers
                    } else {
                        self.styles.bullets
                    };
                    self.pending_item = Some(ListItem {
                        style,
                        level,
                        starts_list: list.first_item,
                        start: list.start,
                    });
                    list.first_item = false;
                }
            }
            Tag::FootnoteDefinition(label) => {
                self.close_paragraph();
                let id = self.note_id(&label);
                self.targets.push(Vec::new());
                self.open_notes.push(id);
            }
            Tag::Table(alignments) => {
                self.close_paragraph();
                self.flush_pending_item();
                self.tables.push(TableState {
                    alignments,
                    rows: Vec::new(),
                    row: Vec::new(),
                    header_rows: 0,
                    in_head: false,
                });
            }
            Tag::TableHead => {
                if let Some(table) = self.tables.last_mut() {
                    table.in_head = true;
                    table.row.clear();
                }
            }
            Tag::TableRow => {
                if let Some(table) = self.tables.last_mut() {
                    table.row.clear();
                }
            }
            Tag::TableCell => {
                self.close_paragraph();
                self.targets.push(Vec::new());
            }
            Tag::Emphasis => self.emphasis += 1,
            Tag::Strong => self.strong += 1,
            Tag::Strikethrough => self.strike += 1,
            Tag::Link { destination, .. } => {
                self.link = Some(self.document.intern_link(&destination));
            }
            Tag::Image { destination, .. } => {
                self.ensure_paragraph();
                self.image = Some((destination.into_owned(), String::new()));
            }
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::CodeBlock => self.close_paragraph(),
            TagEnd::HtmlBlock => {}
            TagEnd::BlockQuote => {
                self.close_paragraph();
                self.quote_depth = self.quote_depth.saturating_sub(1);
            }
            TagEnd::List(_) => {
                self.close_paragraph();
                self.lists.pop();
            }
            TagEnd::Item => {
                self.close_paragraph();
                // An item with no content still counts as an item.
                if let Some(item) = self.pending_item.take() {
                    let paragraph = Paragraph {
                        list: Some(item),
                        ..Paragraph::default()
                    };
                    self.push_block(Block::Paragraph(paragraph));
                }
            }
            TagEnd::FootnoteDefinition => {
                self.close_paragraph();
                let blocks = self.targets.pop().unwrap_or_default();
                if let Some(id) = self.open_notes.pop()
                    && let Some(note) = self.document.footnotes.get_mut(id)
                {
                    note.blocks = blocks;
                }
            }
            TagEnd::Table => {
                self.close_paragraph();
                let Some(table) = self.tables.pop() else {
                    return;
                };
                let columns = table
                    .rows
                    .iter()
                    .map(|row| row.cells.len())
                    .max()
                    .unwrap_or(0)
                    .max(table.alignments.len());
                let width = if columns == 0 {
                    TEXT_WIDTH
                } else {
                    TEXT_WIDTH / columns as f32
                };
                let mut rows = table.rows;
                for row in &mut rows {
                    while row.cells.len() < columns {
                        row.cells.push(empty_cell());
                    }
                }
                self.push_block(Block::Table(Table {
                    rows,
                    header_rows: table.header_rows,
                    columns: vec![width; columns],
                }));
            }
            TagEnd::TableHead => {
                self.close_paragraph();
                if let Some(table) = self.tables.last_mut() {
                    let cells = std::mem::take(&mut table.row);
                    table.rows.push(Row {
                        cells,
                        height: None,
                    });
                    table.header_rows = 1;
                    table.in_head = false;
                }
            }
            TagEnd::TableRow => {
                self.close_paragraph();
                if let Some(table) = self.tables.last_mut() {
                    let cells = std::mem::take(&mut table.row);
                    table.rows.push(Row {
                        cells,
                        height: None,
                    });
                }
            }
            TagEnd::TableCell => {
                self.close_paragraph();
                let blocks = self.targets.pop().unwrap_or_default();
                if let Some(table) = self.tables.last_mut() {
                    table.row.push(Cell {
                        blocks,
                        column_span: 1,
                        row_span: 1,
                        background: None,
                        merge: crate::document::Merge::Origin,
                    });
                }
            }
            TagEnd::Emphasis => self.emphasis = self.emphasis.saturating_sub(1),
            TagEnd::Strong => self.strong = self.strong.saturating_sub(1),
            TagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
            TagEnd::Link => self.link = None,
            TagEnd::Image => self.finish_image(),
        }
    }
}

impl<'a> EventSink<'a> for DocumentBuilder {
    fn event(&mut self, event: Event<'a>) {
        self.transient(event);
    }

    fn transient(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => self.push_text(&text),
            Event::Code(text) => self.push_code_span(&text),
            Event::Html(_) | Event::InlineHtml(_) => self.dropped_html = true,
            Event::SoftBreak => self.push_text(" "),
            Event::HardBreak => {
                if self.current_kind == ParagraphKind::Code {
                    self.code_text.push('\n');
                } else {
                    self.push_inline(Inline::LineBreak);
                }
            }
            Event::Rule => {
                let style = self.styles.rule;
                self.open_paragraph(Some(style), ParagraphKind::Plain);
                self.close_paragraph();
            }
            Event::FootnoteReference(label) => {
                let id = self.note_id(&label);
                self.push_inline(Inline::Footnote(id));
            }
            Event::TaskListMarker(checked) => {
                let marker = if checked { "\u{2612} " } else { "\u{2610} " };
                self.push_text(marker);
            }
        }
    }
}

fn plain_run(content: Inline) -> Run {
    Run {
        style: None,
        properties: None,
        link: None,
        revision: None,
        content,
    }
}

fn empty_cell() -> Cell {
    Cell {
        blocks: Vec::new(),
        column_span: 1,
        row_span: 1,
        background: None,
        merge: crate::document::Merge::Origin,
    }
}

/// The named styles Markdown structure maps onto, added to the document's
/// style table once.
fn built_in_styles(document: &mut Document) -> BuiltInStyles {
    let monospace = document.intern_string("Courier New");
    let heading_sizes = [20.0, 16.0, 14.0, 12.0, 11.0, 11.0];
    let mut headings = [0; 6];
    for (index, size) in heading_sizes.iter().enumerate() {
        headings[index] = document.styles.paragraph.len();
        document.styles.paragraph.push(ParagraphStyle {
            name: format!("Heading {}", index + 1),
            parent: None,
            paragraph: ParagraphProperties {
                outline_level: Some(index as u8),
                space_before: Some(12.0),
                space_after: Some(4.0),
                keep_with_next: Some(true),
                ..ParagraphProperties::default()
            },
            run: RunProperties {
                size: Some(*size),
                bold: Some(true),
                italic: (index == 5).then_some(true),
                ..RunProperties::default()
            },
        });
    }
    let quote = document.styles.paragraph.len();
    document.styles.paragraph.push(ParagraphStyle {
        name: "Quote".to_string(),
        parent: None,
        paragraph: ParagraphProperties {
            right_indent: Some(LEVEL_INDENT),
            ..ParagraphProperties::default()
        },
        run: RunProperties::default(),
    });
    let code = document.styles.paragraph.len();
    document.styles.paragraph.push(ParagraphStyle {
        name: "Source Code".to_string(),
        parent: None,
        paragraph: ParagraphProperties {
            space_after: Some(8.0),
            ..ParagraphProperties::default()
        },
        run: RunProperties {
            font: Some(monospace),
            size: Some(10.0),
            ..RunProperties::default()
        },
    });
    let rule = document.styles.paragraph.len();
    document.styles.paragraph.push(ParagraphStyle {
        name: "Horizontal Line".to_string(),
        parent: None,
        paragraph: ParagraphProperties {
            space_before: Some(6.0),
            space_after: Some(6.0),
            ..ParagraphProperties::default()
        },
        run: RunProperties::default(),
    });
    let code_span = document.styles.character.len();
    document.styles.character.push(CharacterStyle {
        name: "Source Text".to_string(),
        parent: None,
        run: RunProperties {
            font: Some(monospace),
            ..RunProperties::default()
        },
    });
    let bullets = document.styles.list.len();
    let markers = ["\u{2022}", "\u{25E6}", "\u{25AA}"];
    let bullet_levels: Vec<ListLevel> = (0..9)
        .map(|level| ListLevel {
            label: ListLabel::Text(markers[level % 3].to_string()),
            indent: LEVEL_INDENT * (level as f32 + 1.0),
            label_indent: LEVEL_INDENT * (level as f32 + 1.0) - 18.0,
        })
        .collect();
    document.styles.list.push(ListStyle {
        name: "Bullets".to_string(),
        levels: bullet_levels,
    });
    let numbers = document.styles.list.len();
    let number_levels: Vec<ListLevel> = (0..9)
        .map(|level| ListLevel {
            label: ListLabel::Number(NumberFormat {
                kind: NumberKind::Decimal,
                pattern: "%1.".to_string(),
            }),
            indent: LEVEL_INDENT * (level as f32 + 1.0),
            label_indent: LEVEL_INDENT * (level as f32 + 1.0) - 18.0,
        })
        .collect();
    document.styles.list.push(ListStyle {
        name: "Numbers".to_string(),
        levels: number_levels,
    });
    BuiltInStyles {
        headings,
        quote,
        code,
        rule,
        code_span,
        bullets,
        numbers,
    }
}

/// Which Markdown block a paragraph style stands for, by the names Word,
/// LibreOffice, and this builder use.
pub fn paragraph_kind_of(name: &str) -> StyleKind {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "source code" | "code" | "html preformatted" | "preformatted text" | "code block"
        | "source_code" => StyleKind::Code,
        "quote" | "block text" | "intense quote" | "quotations" | "block quote" => StyleKind::Quote,
        "horizontal line" | "horizontal rule" => StyleKind::Rule,
        _ => StyleKind::Plain,
    }
}

/// Whether a character style name marks code.
pub fn is_code_character_style(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "source text" | "code" | "html code" | "verbatim char" | "code char" | "source_text"
    )
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StyleKind {
    Plain,
    Code,
    Quote,
    Rule,
}

/// A convenience for callers with the text in hand.
pub fn build_from_markdown(text: &str) -> Document {
    let mut builder = DocumentBuilder::new();
    crate::io::markdown::parse_into(text, crate::io::markdown::Options::default(), &mut builder);
    builder.finish()
}
