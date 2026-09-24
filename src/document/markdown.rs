//! The document model as a Markdown event stream, so every Markdown
//! renderer (Markdown, HTML, plain text, events as JSON) can take a
//! document. Markdown has no pages: headers, footers, page setup, fonts,
//! sizes, and colors are left out; text boxes follow the body in page
//! order; footnotes come last as definitions.

use std::borrow::Cow;

use super::from_events::{StyleKind, is_code_character_style, paragraph_kind_of};
use super::{
    Block, Document, FloatingContent, FloatingObject, Inline, Merge, Paragraph,
    ParagraphProperties, Run, RunProperties, Table, mathml_text,
};
use crate::io::markdown::{Alignment, CodeBlockKind, Event, EventSink, Tag, TagEnd};

/// Points of indent per block quote level, as the builder writes them.
const QUOTE_INDENT: f32 = 36.0;

/// Pushes `document` into `sink` as Markdown events.
pub fn emit_events<'a>(document: &'a Document, sink: &mut dyn EventSink<'a>) {
    let mut emitter = Emitter {
        document,
        sink,
        paragraph_runs: vec![None; document.styles.paragraph.len()],
        paragraph_formats: vec![None; document.styles.paragraph.len()],
        character_runs: vec![None; document.styles.character.len()],
        paragraph_kinds: document
            .styles
            .paragraph
            .iter()
            .map(|style| paragraph_kind_of(&style.name))
            .collect(),
        code_characters: document
            .styles
            .character
            .iter()
            .map(|style| is_code_character_style(&style.name))
            .collect(),
        run_scratch: Vec::new(),
        lists: Vec::new(),
        quote_depth: 0,
    };
    for section in &document.sections {
        emitter.blocks(&section.blocks);
    }
    emitter.close_lists(0);
    emitter.close_quotes(0);
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
        emitter.close_quotes(0);
    }
    for (index, note) in document.footnotes.iter().enumerate() {
        let label = Cow::Owned((index + 1).to_string());
        emitter
            .sink
            .event(Event::Start(Tag::FootnoteDefinition(label)));
        emitter.blocks(&note.blocks);
        emitter.close_lists(0);
        emitter.close_quotes(0);
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
    /// What each paragraph style stands for (code, quote, rule), and which
    /// character styles mark code, by the names in `from_events`.
    paragraph_kinds: Vec<StyleKind>,
    code_characters: Vec<bool>,
    /// One entry per open list, outermost first: whether it is ordered.
    /// Every open list has an open item.
    lists: Vec<bool>,
    /// Block quotes open around the paragraphs being emitted.
    quote_depth: u32,
}

/// Inline formatting a run carries.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
struct Formatting {
    strong: bool,
    emphasis: bool,
    strike: bool,
}

impl Formatting {
    fn has(self, mark: Mark) -> bool {
        match mark {
            Mark::Strong => self.strong,
            Mark::Emphasis => self.emphasis,
            Mark::Strike => self.strike,
        }
    }

    fn common(self, other: Formatting) -> Formatting {
        Formatting {
            strong: self.strong && other.strong,
            emphasis: self.emphasis && other.emphasis,
            strike: self.strike && other.strike,
        }
    }

    fn from_marks(marks: &[Mark]) -> Formatting {
        Formatting {
            strong: marks.contains(&Mark::Strong),
            emphasis: marks.contains(&Mark::Emphasis),
            strike: marks.contains(&Mark::Strike),
        }
    }
}

/// One open formatting tag; the tags nest in the order they were opened.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    Strong,
    Emphasis,
    Strike,
}

impl Mark {
    fn start(self) -> Event<'static> {
        match self {
            Mark::Strong => Event::Start(Tag::Strong),
            Mark::Emphasis => Event::Start(Tag::Emphasis),
            Mark::Strike => Event::Start(Tag::Strikethrough),
        }
    }

    fn end(self) -> Event<'static> {
        match self {
            Mark::Strong => Event::End(TagEnd::Strong),
            Mark::Emphasis => Event::End(TagEnd::Emphasis),
            Mark::Strike => Event::End(TagEnd::Strikethrough),
        }
    }
}

impl<'a> Emitter<'a, '_> {
    fn blocks(&mut self, blocks: &'a [Block]) {
        for block in blocks {
            match block {
                Block::Paragraph(paragraph) => self.paragraph(paragraph),
                Block::Table(table) => {
                    self.close_lists(0);
                    self.close_quotes(0);
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
        let kind = paragraph
            .style
            .and_then(|style| self.paragraph_kinds.get(style))
            .copied()
            .unwrap_or(StyleKind::Plain);
        if kind == StyleKind::Rule {
            self.close_lists(0);
            self.close_quotes(0);
            self.sink.event(Event::Rule);
            return;
        }
        if !has_content && kind != StyleKind::Code && paragraph.list.is_none() {
            // An empty paragraph ends a list but is nothing in Markdown.
            self.close_lists(0);
            return;
        }
        // A quote's depth is its indent in quote steps, at least one.
        let wanted_depth = if kind == StyleKind::Quote {
            let indent = self
                .effective_paragraph(paragraph)
                .left_indent
                .unwrap_or(0.0);
            ((indent / QUOTE_INDENT).round() as u32).max(1)
        } else {
            0
        };
        if wanted_depth != self.quote_depth {
            self.close_lists(0);
            self.close_quotes(wanted_depth);
            while self.quote_depth < wanted_depth {
                self.sink.event(Event::Start(Tag::BlockQuote));
                self.quote_depth += 1;
            }
        }
        if kind == StyleKind::Code {
            self.close_lists(0);
            self.code_block(paragraph);
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
                if !has_content {
                    // An empty item: the marker alone.
                    return;
                }
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

    /// Closes open block quotes until `depth` remain.
    fn close_quotes(&mut self, depth: u32) {
        while self.quote_depth > depth {
            self.sink.event(Event::End(TagEnd::BlockQuote));
            self.quote_depth -= 1;
        }
    }

    /// A code-styled paragraph as a fenced code block: its text lines,
    /// joined at line breaks, with the block's final newline.
    fn code_block(&mut self, paragraph: &'a Paragraph) {
        let mut text = String::new();
        for run in &paragraph.runs {
            if self.document.is_deleted(run) {
                continue;
            }
            match run.content {
                Inline::Text(span) => text.push_str(self.document.text(span)),
                Inline::LineBreak => text.push('\n'),
                Inline::Tab => text.push('\t'),
                _ => {}
            }
        }
        if !text.is_empty() {
            text.push('\n');
        }
        self.sink
            .event(Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(
                Cow::Borrowed(""),
            ))));
        self.sink.transient(Event::Text(Cow::Borrowed(&text)));
        self.sink.event(Event::End(TagEnd::CodeBlock));
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
        // A break at the very end of a paragraph breaks nothing.
        while runs
            .last()
            .is_some_and(|(run, _)| matches!(run.content, Inline::LineBreak))
        {
            runs.pop();
        }
        let mut open_link: Option<&'a str> = None;
        let mut open: Vec<Mark> = Vec::with_capacity(3);
        for (index, (run, wanted)) in runs.iter().enumerate() {
            let wanted = *wanted;
            let link = self.document.link(run);
            let next = runs.get(index + 1);
            // What the next run keeps of this run's formatting, so that a
            // trailing space stays inside it.
            let kept = match next {
                Some((next_run, next_wanted)) if self.document.link(next_run) == link => {
                    wanted.common(*next_wanted)
                }
                _ => Formatting::default(),
            };
            let closes_after = kept != wanted
                || next.is_none_or(|(next_run, _)| self.document.link(next_run) != link);
            let is_code = run
                .style
                .and_then(|style| self.code_characters.get(style))
                .copied()
                .unwrap_or(false);
            let (leading, core, trailing) = match run.content {
                Inline::Text(span)
                    if !is_code && (wanted != Formatting::default() || link.is_some()) =>
                {
                    let text = self.document.text(span);
                    let core = text.trim_matches(is_space);
                    let start = text.len() - text.trim_start_matches(is_space).len();
                    (&text[..start], core, &text[start + core.len()..])
                }
                _ => ("", "", ""),
            };
            let opens_before = link != open_link || wanted != Formatting::from_marks(&open);
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
            self.open_formatting(&mut open, wanted, kept);
            match run.content {
                Inline::Text(span) if is_code => {
                    self.sink
                        .event(Event::Code(Cow::Borrowed(self.document.text(span))));
                }
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
                let link_closes =
                    next.is_none_or(|(next_run, _)| self.document.link(next_run) != link);
                let keep = if link_closes {
                    Formatting::default()
                } else {
                    kept
                };
                self.close_formatting(&mut open, keep);
                if link_closes && open_link.is_some() {
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
    /// Closes open tags, innermost first, until every one left is wanted.
    fn close_formatting(&mut self, open: &mut Vec<Mark>, wanted: Formatting) {
        while open.iter().any(|mark| !wanted.has(*mark)) {
            let Some(mark) = open.pop() else {
                break;
            };
            self.sink.event(mark.end());
        }
    }

    /// Opens what is wanted and not yet open, inside what is: first the
    /// marks the next run keeps, so they end up outermost and survive the
    /// change, then the rest.
    fn open_formatting(&mut self, open: &mut Vec<Mark>, wanted: Formatting, kept: Formatting) {
        const ORDER: [Mark; 3] = [Mark::Strong, Mark::Emphasis, Mark::Strike];
        for mark in ORDER {
            if wanted.has(mark) && kept.has(mark) && !open.contains(&mark) {
                self.sink.event(mark.start());
                open.push(mark);
            }
        }
        for mark in ORDER {
            if wanted.has(mark) && !open.contains(&mark) {
                self.sink.event(mark.start());
                open.push(mark);
            }
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
/// The GFM autolink rules for an email address: letters, digits, and a
/// few marks in the user part; letters, digits, hyphens, underscores, and
/// dots in the host, which ends in a letter or digit.
fn is_email_user(user: &str) -> bool {
    !user.is_empty()
        && user
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_' | '+'))
}

fn is_email_host(host: &str) -> bool {
    host.contains('.')
        && host
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'))
        && host.ends_with(|ch: char| ch.is_ascii_alphanumeric())
}

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
                .is_some_and(|(user, host)| is_email_user(user) && is_email_host(host));
        if (is_web && trimmed.len() > 8) || is_email {
            let end = start + trimmed.len();
            return Some((&text[..start], &text[start..end], &text[end..]));
        }
    }
    None
}
