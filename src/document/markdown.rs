//! The document model as a Markdown event stream, so every Markdown
//! renderer (Markdown, HTML, plain text, events as JSON) can take a
//! document. Markdown has no pages: headers, footers, page setup, fonts,
//! sizes, and colors are left out; text boxes follow the body in page
//! order; footnotes come last as definitions.

use std::borrow::Cow;

use super::{
    Block, Document, FloatingContent, FloatingObject, Inline, Merge, Paragraph,
    ParagraphProperties, Run, RunProperties, Table, mathml_text,
};
use crate::io::markdown::{Alignment, Event, EventSink, Tag, TagEnd};

/// Pushes `document` into `sink` as Markdown events.
pub fn emit_events<'a>(document: &'a Document, sink: &mut dyn EventSink<'a>) {
    let mut emitter = Emitter {
        document,
        sink,
        paragraph_runs: vec![None; document.styles.paragraph.len()],
        paragraph_formats: vec![None; document.styles.paragraph.len()],
        character_runs: vec![None; document.styles.character.len()],
        run_scratch: Vec::new(),
        lists: Vec::new(),
    };
    for section in &document.sections {
        emitter.blocks(&section.blocks);
    }
    emitter.close_lists(0);
    let mut floating: Vec<&FloatingObject> = document.floating.iter().collect();
    floating.sort_by(|left, right| {
        (left.page, left.y, left.x)
            .partial_cmp(&(right.page, right.y, right.x))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for object in floating {
        match &object.content {
            FloatingContent::TextBox { blocks, .. } => emitter.blocks(blocks),
            FloatingContent::Image(media) => {
                emitter.sink.event(Event::Start(Tag::Paragraph));
                emitter.image(*media, None);
                emitter.sink.event(Event::End(TagEnd::Paragraph));
            }
        }
        emitter.close_lists(0);
    }
    for (index, note) in document.footnotes.iter().enumerate() {
        let label = Cow::Owned((index + 1).to_string());
        emitter
            .sink
            .event(Event::Start(Tag::FootnoteDefinition(label)));
        emitter.blocks(&note.blocks);
        emitter.close_lists(0);
        emitter.sink.event(Event::End(TagEnd::FootnoteDefinition));
    }
}

struct Emitter<'a, 's> {
    document: &'a Document,
    sink: &'s mut dyn EventSink<'a>,
    /// Style chains resolved once per style: run formatting and paragraph
    /// formatting of paragraph styles, run formatting of character styles.
    paragraph_runs: Vec<Option<RunProperties>>,
    paragraph_formats: Vec<Option<ParagraphProperties>>,
    character_runs: Vec<Option<RunProperties>>,
    /// The runs of the paragraph being emitted with their formatting,
    /// reused across paragraphs.
    run_scratch: Vec<(&'a Run, Formatting)>,
    /// One entry per open list, outermost first: whether it is ordered.
    /// Every open list has an open item.
    lists: Vec<bool>,
}

/// Inline formatting a run carries, in the order the tags nest.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
struct Formatting {
    strong: bool,
    emphasis: bool,
    strike: bool,
}

impl<'a> Emitter<'a, '_> {
    fn blocks(&mut self, blocks: &'a [Block]) {
        for block in blocks {
            match block {
                Block::Paragraph(paragraph) => self.paragraph(paragraph),
                Block::Table(table) => {
                    self.close_lists(0);
                    self.table(table);
                }
            }
        }
    }

    fn paragraph(&mut self, paragraph: &'a Paragraph) {
        let has_content = paragraph.runs.iter().any(|run| {
            !matches!(
                run.content,
                Inline::PageBreak | Inline::PageNumber | Inline::PageCount
            )
        });
        if !has_content {
            // An empty paragraph ends a list but is nothing in Markdown.
            self.close_lists(0);
            return;
        }
        if let Some(level) = self.heading_level(paragraph) {
            self.close_lists(0);
            self.sink.event(Event::Start(Tag::Heading(level)));
            self.inlines(paragraph, true);
            self.sink.event(Event::End(TagEnd::Heading(level)));
            return;
        }
        match paragraph.list {
            Some(item) => {
                let ordered = self
                    .document
                    .styles
                    .list
                    .get(item.style)
                    .and_then(|style| style.levels.get(usize::from(item.level)))
                    .is_some_and(|level| matches!(level.label, super::ListLabel::Number(_)));
                self.open_item(
                    usize::from(item.level),
                    ordered,
                    item.starts_list,
                    item.start,
                );
            }
            None => self.close_lists(0),
        }
        self.sink.event(Event::Start(Tag::Paragraph));
        self.inlines(paragraph, false);
        self.sink.event(Event::End(TagEnd::Paragraph));
    }

    /// Headings by outline level, or the `Title` style as the first level.
    fn heading_level(&mut self, paragraph: &Paragraph) -> Option<u8> {
        let properties = self.effective_paragraph(paragraph);
        if let Some(level) = properties.outline_level {
            return Some((level + 1).min(6));
        }
        let name = paragraph
            .style
            .map(|style| self.document.styles.paragraph[style].name.as_str())
            .unwrap_or("");
        if name == "Title" {
            return Some(1);
        }
        None
    }

    /// Opens the item a list paragraph belongs to, closing and opening
    /// lists as the level changes.
    fn open_item(&mut self, level: usize, ordered: bool, starts_list: bool, start_at: u32) {
        self.close_lists(level + 1);
        if self.lists.len() == level + 1 {
            if starts_list {
                self.close_lists(level);
            } else {
                self.sink.event(Event::End(TagEnd::Item));
                self.sink.event(Event::Start(Tag::Item));
                return;
            }
        }
        while self.lists.len() < level + 1 {
            let start = if ordered {
                Some(u64::from(start_at))
            } else {
                None
            };
            self.sink
                .event(Event::Start(Tag::List { start, tight: true }));
            self.sink.event(Event::Start(Tag::Item));
            self.lists.push(ordered);
        }
    }

    /// Closes open lists until `depth` remain.
    fn close_lists(&mut self, depth: usize) {
        while self.lists.len() > depth {
            let ordered = self.lists.pop().unwrap_or(false);
            self.sink.event(Event::End(TagEnd::Item));
            self.sink.event(Event::End(TagEnd::List(ordered)));
        }
    }

    /// The runs of a paragraph, with links and formatting opened and
    /// closed as they change between runs. Whitespace at a run's edges
    /// stays outside the tags, where Markdown needs it. With
    /// `own_formatting_only`, what the paragraph style sets is not marked
    /// up: a heading is not bold on top of being a heading.
    fn inlines(&mut self, paragraph: &'a Paragraph, own_formatting_only: bool) {
        // The paragraph's share of every run's formatting, computed once.
        let mut base = match paragraph.style {
            Some(style) => self.paragraph_run(style),
            None => RunProperties::default(),
        };
        base.overlay(&self.document.paragraph_run_properties(paragraph));
        let mut runs = std::mem::take(&mut self.run_scratch);
        runs.clear();
        for run in &paragraph.runs {
            if self.document.is_deleted(run) {
                continue;
            }
            let mut effective = if own_formatting_only {
                RunProperties::default()
            } else {
                base
            };
            if let Some(style) = run.style {
                effective.overlay(&self.character_run(style));
            }
            effective.overlay(&self.document.run_properties(run));
            let wanted = Formatting {
                strong: effective.bold == Some(true),
                emphasis: effective.italic == Some(true),
                strike: effective.strike == Some(true),
            };
            runs.push((run, wanted));
        }
        let mut open_link: Option<&'a str> = None;
        let mut open = Formatting::default();
        for (index, (run, wanted)) in runs.iter().enumerate() {
            let wanted = *wanted;
            let link = self.document.link(run);
            let next = runs.get(index + 1);
            let closes_after = next.is_none_or(|(next_run, next_wanted)| {
                *next_wanted != wanted || self.document.link(next_run) != link
            });
            let (leading, core, trailing) = match run.content {
                Inline::Text(span) if wanted != Formatting::default() || link.is_some() => {
                    let text = self.document.text(span);
                    let core = text.trim_matches(is_space);
                    let start = text.len() - text.trim_start_matches(is_space).len();
                    (&text[..start], core, &text[start + core.len()..])
                }
                _ => ("", "", ""),
            };
            let opens_before = link != open_link || wanted != open;
            if opens_before && !leading.is_empty() {
                self.sink.event(Event::Text(Cow::Borrowed(leading)));
            }
            if link != open_link {
                self.close_formatting(&mut open, Formatting::default());
                if open_link.is_some() {
                    self.sink.event(Event::End(TagEnd::Link));
                }
                open_link = link;
                if let Some(target) = link {
                    self.sink.event(Event::Start(Tag::Link {
                        destination: Cow::Borrowed(target),
                        title: Cow::Borrowed(""),
                    }));
                }
            }
            self.close_formatting(&mut open, wanted);
            self.open_formatting(&mut open, wanted);
            match run.content {
                Inline::Text(_) if wanted != Formatting::default() || link.is_some() => {
                    if !opens_before && !leading.is_empty() {
                        self.sink.event(Event::Text(Cow::Borrowed(leading)));
                    }
                    if !core.is_empty() {
                        self.sink.event(Event::Text(Cow::Borrowed(core)));
                    }
                    if !closes_after && !trailing.is_empty() {
                        self.sink.event(Event::Text(Cow::Borrowed(trailing)));
                    }
                }
                _ => self.run(run),
            }
            if closes_after {
                self.close_formatting(&mut open, Formatting::default());
                if open_link.is_some() {
                    self.sink.event(Event::End(TagEnd::Link));
                    open_link = None;
                }
                if !trailing.is_empty() {
                    self.sink.event(Event::Text(Cow::Borrowed(trailing)));
                }
            }
        }
        self.run_scratch = runs;
    }

    fn paragraph_run(&mut self, style: usize) -> RunProperties {
        if let Some(Some(cached)) = self.paragraph_runs.get(style) {
            return *cached;
        }
        let resolved = self.document.paragraph_style_run(style);
        if let Some(slot) = self.paragraph_runs.get_mut(style) {
            *slot = Some(resolved);
        }
        resolved
    }

    fn character_run(&mut self, style: usize) -> RunProperties {
        if let Some(Some(cached)) = self.character_runs.get(style) {
            return *cached;
        }
        let resolved = self.document.character_style_run(style);
        if let Some(slot) = self.character_runs.get_mut(style) {
            *slot = Some(resolved);
        }
        resolved
    }

    /// A paragraph's effective paragraph formatting, with the style's
    /// share cached.
    fn effective_paragraph(&mut self, paragraph: &Paragraph) -> ParagraphProperties {
        let mut properties = match paragraph.style {
            Some(style) => {
                if let Some(Some(cached)) = self.paragraph_formats.get(style) {
                    *cached
                } else {
                    let resolved = self.document.paragraph_style_properties(style);
                    if let Some(slot) = self.paragraph_formats.get_mut(style) {
                        *slot = Some(resolved);
                    }
                    resolved
                }
            }
            None => ParagraphProperties::default(),
        };
        properties.overlay(&self.document.paragraph_properties(paragraph));
        properties
    }

    /// Closes the formatting tags that `wanted` no longer has, innermost
    /// first, along with anything nested inside them.
    fn close_formatting(&mut self, open: &mut Formatting, wanted: Formatting) {
        let keep_strong = open.strong && wanted.strong;
        let keep_emphasis = keep_strong && open.emphasis && wanted.emphasis;
        let keep_strike = keep_emphasis && open.strike && wanted.strike;
        if open.strike && !keep_strike {
            self.sink.event(Event::End(TagEnd::Strikethrough));
            open.strike = false;
        }
        if open.emphasis && !keep_emphasis {
            self.sink.event(Event::End(TagEnd::Emphasis));
            open.emphasis = false;
        }
        if open.strong && !keep_strong {
            self.sink.event(Event::End(TagEnd::Strong));
            open.strong = false;
        }
        // Formatting that stays open only if everything outside it does.
        if !open.strong && open.emphasis {
            self.sink.event(Event::End(TagEnd::Emphasis));
            open.emphasis = false;
        }
        if !open.emphasis && open.strike {
            self.sink.event(Event::End(TagEnd::Strikethrough));
            open.strike = false;
        }
    }

    fn open_formatting(&mut self, open: &mut Formatting, wanted: Formatting) {
        if wanted.strong && !open.strong {
            self.sink.event(Event::Start(Tag::Strong));
            open.strong = true;
        }
        if wanted.emphasis && !open.emphasis {
            self.sink.event(Event::Start(Tag::Emphasis));
            open.emphasis = true;
        }
        if wanted.strike && !open.strike {
            self.sink.event(Event::Start(Tag::Strikethrough));
            open.strike = true;
        }
    }

    fn run(&mut self, run: &'a Run) {
        match run.content {
            Inline::Text(span) if run.link.is_none() => {
                self.text_with_bare_links(self.document.text(span));
            }
            Inline::Text(span) => self
                .sink
                .event(Event::Text(Cow::Borrowed(self.document.text(span)))),
            Inline::LineBreak => self.sink.event(Event::HardBreak),
            Inline::Tab => self.sink.event(Event::Text(Cow::Borrowed("\t"))),
            Inline::Footnote(note) => {
                self.sink
                    .event(Event::FootnoteReference(Cow::Owned((note + 1).to_string())));
            }
            Inline::Image(id) => {
                if let Some(image) = self.document.image(id) {
                    self.image(image.media, image.description.as_deref());
                }
            }
            Inline::Math(span) => self.sink.event(Event::Code(Cow::Owned(mathml_text(
                self.document.text(span),
            )))),
            Inline::PageBreak | Inline::PageNumber | Inline::PageCount => {}
        }
    }

    /// Text whose bare web addresses and email addresses become links,
    /// as a reader would expect them to be clickable.
    fn text_with_bare_links(&mut self, text: &'a str) {
        let mut rest = text;
        while let Some((before, address, after)) = next_bare_address(rest) {
            if !before.is_empty() {
                self.sink.event(Event::Text(Cow::Borrowed(before)));
            }
            let destination = if address.contains('@') {
                Cow::Owned(format!("mailto:{address}"))
            } else if address.starts_with("www.") {
                Cow::Owned(format!("http://{address}"))
            } else {
                Cow::Borrowed(address)
            };
            self.sink.event(Event::Start(Tag::Link {
                destination,
                title: Cow::Borrowed(""),
            }));
            self.sink.event(Event::Text(Cow::Borrowed(address)));
            self.sink.event(Event::End(TagEnd::Link));
            rest = after;
        }
        if !rest.is_empty() {
            self.sink.event(Event::Text(Cow::Borrowed(rest)));
        }
    }

    /// An image by its media file name, with the description as its text.
    fn image(&mut self, media: usize, description: Option<&'a str>) {
        let Some(file) = self.document.media.get(media) else {
            return;
        };
        self.sink.event(Event::Start(Tag::Image {
            destination: Cow::Borrowed(file.name.as_str()),
            title: Cow::Borrowed(""),
        }));
        if let Some(description) = description {
            self.sink.event(Event::Text(Cow::Borrowed(description)));
        }
        self.sink.event(Event::End(TagEnd::Image));
    }

    /// A table as a Markdown table: the header rows (or the first row when
    /// there is none) as the head, merged cells as empty cells to keep the
    /// grid, each cell's paragraphs on one line.
    fn table(&mut self, table: &'a Table) {
        let Some(first) = table.rows.first() else {
            return;
        };
        let columns = table
            .rows
            .iter()
            .map(|row| row.cells.len())
            .max()
            .unwrap_or(0)
            .max(table.columns.len());
        let alignments: Vec<Alignment> = (0..columns)
            .map(|column| {
                let alignment = first
                    .cells
                    .get(column)
                    .and_then(|cell| cell.blocks.first())
                    .and_then(|block| match block {
                        Block::Paragraph(paragraph) => {
                            self.document.effective_paragraph(paragraph).alignment
                        }
                        Block::Table(_) => None,
                    });
                match alignment {
                    Some(super::Alignment::Center) => Alignment::Center,
                    Some(super::Alignment::Right) => Alignment::Right,
                    Some(super::Alignment::Left | super::Alignment::Justify) => Alignment::Left,
                    None => Alignment::None,
                }
            })
            .collect();
        self.sink.event(Event::Start(Tag::Table(alignments)));
        let head_rows = (table.header_rows as usize).clamp(1, table.rows.len());
        self.sink.event(Event::Start(Tag::TableHead));
        for row in &table.rows[..head_rows] {
            if head_rows > 1 {
                self.sink.event(Event::Start(Tag::TableRow));
            }
            self.cells(&row.cells, columns, true);
            if head_rows > 1 {
                self.sink.event(Event::End(TagEnd::TableRow));
            }
        }
        self.sink.event(Event::End(TagEnd::TableHead));
        for row in &table.rows[head_rows..] {
            self.sink.event(Event::Start(Tag::TableRow));
            self.cells(&row.cells, columns, false);
            self.sink.event(Event::End(TagEnd::TableRow));
        }
        self.sink.event(Event::End(TagEnd::Table));
    }

    fn cells(&mut self, cells: &'a [super::Cell], columns: usize, header: bool) {
        for column in 0..columns {
            self.sink.event(Event::Start(Tag::TableCell));
            if let Some(cell) = cells.get(column)
                && cell.merge == Merge::Origin
            {
                let mut first = true;
                for block in &cell.blocks {
                    if let Block::Paragraph(paragraph) = block {
                        if !first {
                            self.sink.event(Event::Text(Cow::Borrowed(" ")));
                        }
                        first = false;
                        self.inlines(paragraph, header);
                    }
                }
            }
            self.sink.event(Event::End(TagEnd::TableCell));
        }
    }
}

/// Whitespace Markdown does not want inside a delimiter run.
fn is_space(ch: char) -> bool {
    ch == ' ' || ch == '\t' || ch == '\u{a0}'
}

/// The first bare web or email address in `text`: the text before it, the
/// address, and the text after it. An address is a whitespace-delimited
/// word starting with `http://`, `https://`, or `www.`, or holding one `@`
/// with a dot after it, less any trailing punctuation.
fn next_bare_address(text: &str) -> Option<(&str, &str, &str)> {
    let mut position = 0usize;
    for word in text.split(|ch: char| ch.is_whitespace() || ch == '|' || ch == '(' || ch == '<') {
        let start = position;
        position += word.len() + 1;
        let trimmed = word.trim_end_matches(['.', ',', ';', ':', ')', '>', '!', '?', '"', '\'']);
        if trimmed.is_empty() {
            continue;
        }
        let is_web = trimmed.starts_with("http://")
            || trimmed.starts_with("https://")
            || trimmed.starts_with("www.");
        let is_email = !is_web
            && trimmed.matches('@').count() == 1
            && trimmed
                .split_once('@')
                .is_some_and(|(user, host)| !user.is_empty() && host.contains('.'));
        if (is_web && trimmed.len() > 8) || is_email {
            let end = start + trimmed.len();
            return Some((&text[..start], &text[start..end], &text[end..]));
        }
    }
    None
}
