//! The document model: the structure and formatting that document formats
//! share. Readers (Pages, later HTML and Word) fill it; writers (Word,
//! later PDF) render it; the Markdown event stream is a projection of it.
//! It is the hub between document formats, kept to what those formats can
//! all express, with every property optional so "not set" means "inherit".
//!
//! Layout: the document owns the tables, and paragraphs and runs hold
//! indices into them. Text is one arena the runs point into by byte range;
//! run and paragraph properties, links, revisions, images, and strings
//! (fonts, languages) are interned once and shared, so a run is a few
//! words and a document of 170,000 runs is a few megabytes.
//!
//! Units: points for lengths and font sizes, sRGB bytes for colors.

use std::collections::HashMap;

pub mod markdown;

/// Index of a style in its table.
pub type StyleId = usize;
/// Index of a footnote in `Document::footnotes`.
pub type NoteId = usize;
/// Index of a media file in `Document::media`.
pub type MediaId = usize;
/// Index into one of the document's interned tables.
pub type Id = u32;

/// A byte range of `Document::text`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

/// Not `Clone` or comparable as a whole: a document is built once and
/// read; the parts that tests compare carry their own derives.
#[derive(Default)]
pub struct Document {
    pub styles: StyleTable,
    pub sections: Vec<Section>,
    pub footnotes: Vec<Note>,
    pub media: Vec<Media>,
    /// Objects placed on pages rather than in the text flow.
    pub floating: Vec<FloatingObject>,
    /// Every run's text, in reading order; runs hold spans of it.
    pub text: String,
    /// Interned strings: font names and language tags.
    pub strings: Vec<String>,
    string_ids: HashMap<String, usize>,
    /// Link targets, interned.
    pub links: Vec<String>,
    link_ids: HashMap<String, usize>,
    pub revisions: Vec<Revision>,
    pub images: Vec<InlineImage>,
    /// Direct run formatting, interned; `None` on a run means none.
    pub run_properties: Vec<RunProperties>,
    pub paragraph_properties: Vec<ParagraphProperties>,
}

/// A text box, shape with text, or image placed on a page.
#[derive(Debug, Clone, PartialEq)]
pub struct FloatingObject {
    /// Zero-based page of the document.
    pub page: u32,
    /// Position of the top-left corner from the page's top-left, in points.
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub content: FloatingContent,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FloatingContent {
    Image(MediaId),
    TextBox {
        blocks: Vec<Block>,
        fill: Option<Color>,
    },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct StyleTable {
    pub paragraph: Vec<ParagraphStyle>,
    pub character: Vec<CharacterStyle>,
    pub list: Vec<ListStyle>,
}

impl StyleTable {
    pub fn paragraph_named(&self, name: &str) -> Option<StyleId> {
        self.paragraph.iter().position(|style| style.name == name)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParagraphStyle {
    pub name: String,
    pub parent: Option<StyleId>,
    pub paragraph: ParagraphProperties,
    pub run: RunProperties,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CharacterStyle {
    pub name: String,
    pub parent: Option<StyleId>,
    pub run: RunProperties,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ListStyle {
    pub name: String,
    /// One entry per nesting level, outermost first.
    pub levels: Vec<ListLevel>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ListLevel {
    pub label: ListLabel,
    /// Indent of the paragraph text from the margin.
    pub indent: f32,
    /// Indent of the label; usually less than `indent`.
    pub label_indent: f32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum ListLabel {
    #[default]
    None,
    /// A literal marker such as a bullet character.
    Text(String),
    Number(NumberFormat),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumberFormat {
    pub kind: NumberKind,
    /// Text around the number, with `%1` standing for it, e.g. `%1.`.
    pub pattern: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberKind {
    Decimal,
    LowerLetter,
    UpperLetter,
    LowerRoman,
    UpperRoman,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Section {
    pub page: PageSetup,
    pub columns: u16,
    /// How the section begins relative to the previous one.
    pub start: SectionStart,
    pub headers: PageVariants,
    pub footers: PageVariants,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SectionStart {
    /// On a new page.
    #[default]
    NewPage,
    /// Where the previous section ended, as for a column change.
    Continuous,
}

/// A header or footer, with the pages it may differ on. `default` is the
/// odd pages when `even` is set, and every page otherwise.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PageVariants {
    pub default: Option<Vec<Block>>,
    pub first: Option<Vec<Block>>,
    pub even: Option<Vec<Block>>,
}

impl PageVariants {
    pub fn is_empty(&self) -> bool {
        self.default.is_none() && self.first.is_none() && self.even.is_none()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PageSetup {
    pub width: f32,
    pub height: f32,
    pub margin_top: f32,
    pub margin_bottom: f32,
    pub margin_left: f32,
    pub margin_right: f32,
    /// Distance of the header from the top edge, and of the footer from
    /// the bottom edge.
    pub header_distance: f32,
    pub footer_distance: f32,
}

impl Default for PageSetup {
    /// US Letter with one-inch margins.
    fn default() -> Self {
        PageSetup {
            width: 612.0,
            height: 792.0,
            margin_top: 72.0,
            margin_bottom: 72.0,
            margin_left: 72.0,
            margin_right: 72.0,
            header_distance: 36.0,
            footer_distance: 36.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Paragraph(Paragraph),
    Table(Table),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Paragraph {
    pub style: Option<StyleId>,
    /// Direct formatting on top of the style (`Document::paragraph_properties`).
    pub properties: Option<Id>,
    /// Direct character formatting set on the whole paragraph, under
    /// each run's own (`Document::run_properties`).
    pub run_properties: Option<Id>,
    pub list: Option<ListItem>,
    pub runs: Vec<Run>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListItem {
    pub style: StyleId,
    /// Nesting level, zero-based.
    pub level: u8,
    /// This item starts a new list (numbering restarts).
    pub starts_list: bool,
    /// The number the list starts at when this item starts it.
    pub start: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ParagraphProperties {
    pub alignment: Option<Alignment>,
    /// From the left margin, as Pages measures it.
    pub first_line_indent: Option<f32>,
    pub left_indent: Option<f32>,
    pub right_indent: Option<f32>,
    pub space_before: Option<f32>,
    pub space_after: Option<f32>,
    pub line_spacing: Option<LineSpacing>,
    pub keep_with_next: Option<bool>,
    pub keep_lines_together: Option<bool>,
    pub widow_control: Option<bool>,
    pub outline_level: Option<u8>,
    pub background: Option<Color>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alignment {
    Left,
    Center,
    Right,
    Justify,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LineSpacing {
    /// A multiple of the font's line height.
    Relative(f32),
    /// Points, at least.
    Minimum(f32),
    /// Points, exactly.
    Exact(f32),
}

/// Character formatting. Fonts and languages are interned strings
/// (`Document::string`), so the whole struct is a few words and copies.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RunProperties {
    pub font: Option<Id>,
    pub size: Option<f32>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strike: Option<bool>,
    pub color: Option<Color>,
    pub highlight: Option<Color>,
    pub baseline: Option<Baseline>,
    pub caps: Option<Caps>,
    pub language: Option<Id>,
}

impl RunProperties {
    /// `other` on top of `self`: set fields win.
    #[inline(never)]
    pub fn overlay(&mut self, other: &RunProperties) {
        macro_rules! take {
            ($($field:ident),*) => {
                $( if other.$field.is_some() { self.$field = other.$field; } )*
            };
        }
        take!(
            font, size, bold, italic, underline, strike, color, highlight, baseline, caps, language
        );
    }

    pub fn is_empty(&self) -> bool {
        *self == RunProperties::default()
    }
}

impl ParagraphProperties {
    #[inline(never)]
    pub fn overlay(&mut self, other: &ParagraphProperties) {
        macro_rules! take {
            ($($field:ident),*) => {
                $( if other.$field.is_some() { self.$field = other.$field; } )*
            };
        }
        take!(
            alignment,
            first_line_indent,
            left_indent,
            right_indent,
            space_before,
            space_after,
            line_spacing,
            keep_with_next,
            keep_lines_together,
            widow_control,
            outline_level,
            background
        );
    }

    pub fn is_empty(&self) -> bool {
        *self == ParagraphProperties::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Baseline {
    Superscript,
    Subscript,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Caps {
    All,
    Small,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

impl Color {
    pub fn hex(&self) -> String {
        format!("{:02X}{:02X}{:02X}", self.red, self.green, self.blue)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    /// Named character style, if any.
    pub style: Option<StyleId>,
    /// Direct formatting on top of the style and the paragraph's
    /// (`Document::run_properties`).
    pub properties: Option<Id>,
    /// The run is part of a link to this target (`Document::links`).
    pub link: Option<Id>,
    /// The run is a tracked change (`Document::revisions`).
    pub revision: Option<Id>,
    pub content: Inline,
}

/// A tracked change on a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision {
    pub kind: RevisionKind,
    pub author: Option<String>,
    /// When the change was made, as `YYYY-MM-DDTHH:MM:SSZ`.
    pub date: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevisionKind {
    Insertion,
    Deletion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inline {
    Text(Span),
    LineBreak,
    Tab,
    /// A page break; usually the only content of its paragraph.
    PageBreak,
    /// An equation, as MathML (a span of the text arena).
    Math(Span),
    Footnote(NoteId),
    /// An image (`Document::images`).
    Image(Id),
    /// The current page number, as a field.
    PageNumber,
    /// The number of pages, as a field.
    PageCount,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InlineImage {
    pub media: MediaId,
    pub width: f32,
    pub height: f32,
    pub description: Option<String>,
    pub placement: Placement,
}

/// Where an image sits: in the text line, or floating beside it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Placement {
    #[default]
    Inline,
    /// Anchored to the paragraph, with text wrapping around it.
    Floating {
        horizontal: Anchor,
        vertical: Anchor,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Anchor {
    pub from: AnchorBase,
    /// Offset in points from `from`.
    pub offset: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorBase {
    Page,
    Margin,
    /// The line the image is anchored in (vertical only).
    Line,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Note {
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Media {
    /// File name inside the package, unique.
    pub name: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Table {
    pub rows: Vec<Row>,
    pub header_rows: u32,
    /// Column widths in points.
    pub columns: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Row {
    /// One cell per grid column, merged ones included.
    pub cells: Vec<Cell>,
    /// Minimum height in points.
    pub height: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Cell {
    pub blocks: Vec<Block>,
    /// Columns covered, at least one, when this cell is not `Merge::Left`.
    pub column_span: u32,
    pub row_span: u32,
    pub background: Option<Color>,
    pub merge: Merge,
}

/// A cell's part in a merged region: the grid keeps every cell, and the
/// covered ones say which way their origin lies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Merge {
    /// A cell of its own, or the top-left of a merged region.
    #[default]
    Origin,
    /// Covered by a cell to the left in the same row.
    Left,
    /// Covered by a cell above; the first column of a region's lower rows.
    Above,
}

impl Document {
    // ----- the arena and the interned tables -----

    /// Appends text to the arena and returns its span.
    pub fn push_text(&mut self, text: &str) -> Span {
        let start = self.text.len() as u32;
        self.text.push_str(text);
        Span {
            start,
            end: self.text.len() as u32,
        }
    }

    pub fn text(&self, span: Span) -> &str {
        self.text
            .get(span.start as usize..span.end as usize)
            .unwrap_or("")
    }

    /// Interns a font name or language tag.
    pub fn intern_string(&mut self, text: &str) -> Id {
        intern(&mut self.strings, &mut self.string_ids, text)
    }

    pub fn string(&self, id: Id) -> &str {
        self.strings.get(id as usize).map_or("", String::as_str)
    }

    pub fn intern_link(&mut self, target: &str) -> Id {
        intern(&mut self.links, &mut self.link_ids, target)
    }

    pub fn link(&self, run: &Run) -> Option<&str> {
        run.link
            .and_then(|id| self.links.get(id as usize))
            .map(String::as_str)
    }

    pub fn push_revision(&mut self, revision: Revision) -> Id {
        self.revisions.push(revision);
        (self.revisions.len() - 1) as Id
    }

    pub fn revision(&self, run: &Run) -> Option<&Revision> {
        run.revision.and_then(|id| self.revisions.get(id as usize))
    }

    /// The run is deleted text under tracked changes.
    pub fn is_deleted(&self, run: &Run) -> bool {
        self.revision(run)
            .is_some_and(|revision| revision.kind == RevisionKind::Deletion)
    }

    pub fn push_image(&mut self, image: InlineImage) -> Id {
        self.images.push(image);
        (self.images.len() - 1) as Id
    }

    pub fn image(&self, id: Id) -> Option<&InlineImage> {
        self.images.get(id as usize)
    }

    /// Interns direct run formatting; empty formatting is `None`.
    pub fn intern_run_properties(&mut self, properties: RunProperties) -> Option<Id> {
        if properties.is_empty() {
            return None;
        }
        self.run_properties.push(properties);
        Some((self.run_properties.len() - 1) as Id)
    }

    pub fn intern_paragraph_properties(&mut self, properties: ParagraphProperties) -> Option<Id> {
        if properties.is_empty() {
            return None;
        }
        self.paragraph_properties.push(properties);
        Some((self.paragraph_properties.len() - 1) as Id)
    }

    /// A run's own direct formatting.
    pub fn run_properties(&self, run: &Run) -> RunProperties {
        self.run_properties_at(run.properties)
    }

    /// A paragraph's direct paragraph formatting.
    pub fn paragraph_properties(&self, paragraph: &Paragraph) -> ParagraphProperties {
        paragraph
            .properties
            .and_then(|id| self.paragraph_properties.get(id as usize))
            .copied()
            .unwrap_or_default()
    }

    /// A paragraph's direct character formatting, under its runs' own.
    pub fn paragraph_run_properties(&self, paragraph: &Paragraph) -> RunProperties {
        self.run_properties_at(paragraph.run_properties)
    }

    fn run_properties_at(&self, id: Option<Id>) -> RunProperties {
        id.and_then(|id| self.run_properties.get(id as usize))
            .copied()
            .unwrap_or_default()
    }

    // ----- styles -----

    /// A paragraph style's run properties with its ancestors applied,
    /// nearest last.
    pub fn paragraph_style_run(&self, style: StyleId) -> RunProperties {
        let mut chain = Vec::new();
        let mut current = Some(style);
        while let Some(id) = current {
            let style = &self.styles.paragraph[id];
            chain.push(id);
            current = style.parent;
            if chain.len() > 64 {
                break;
            }
        }
        let mut run = RunProperties::default();
        for id in chain.into_iter().rev() {
            run.overlay(&self.styles.paragraph[id].run);
        }
        run
    }

    /// A paragraph style's paragraph properties with its ancestors applied.
    pub fn paragraph_style_properties(&self, style: StyleId) -> ParagraphProperties {
        let mut chain = Vec::new();
        let mut current = Some(style);
        while let Some(id) = current {
            chain.push(id);
            current = self.styles.paragraph[id].parent;
            if chain.len() > 64 {
                break;
            }
        }
        let mut properties = ParagraphProperties::default();
        for id in chain.into_iter().rev() {
            properties.overlay(&self.styles.paragraph[id].paragraph);
        }
        properties
    }

    /// A character style's run properties with its ancestors applied.
    pub fn character_style_run(&self, style: StyleId) -> RunProperties {
        let mut chain = Vec::new();
        let mut current = Some(style);
        while let Some(id) = current {
            chain.push(id);
            current = self.styles.character[id].parent;
            if chain.len() > 64 {
                break;
            }
        }
        let mut run = RunProperties::default();
        for id in chain.into_iter().rev() {
            run.overlay(&self.styles.character[id].run);
        }
        run
    }

    /// A paragraph's effective paragraph formatting: its style's, then its
    /// own.
    pub fn effective_paragraph(&self, paragraph: &Paragraph) -> ParagraphProperties {
        let mut properties = match paragraph.style {
            Some(style) => self.paragraph_style_properties(style),
            None => ParagraphProperties::default(),
        };
        properties.overlay(&self.paragraph_properties(paragraph));
        properties
    }

    /// What a run looks like: paragraph style, then the paragraph's own
    /// character formatting, then character style, then the run's own.
    #[inline(never)]
    pub fn effective_run(&self, paragraph: &Paragraph, run: &Run) -> RunProperties {
        let mut properties = match paragraph.style {
            Some(style) => self.paragraph_style_run(style),
            None => RunProperties::default(),
        };
        properties.overlay(&self.paragraph_run_properties(paragraph));
        if let Some(style) = run.style {
            properties.overlay(&self.character_style_run(style));
        }
        properties.overlay(&self.run_properties(run));
        properties
    }

    // ----- text -----

    /// A paragraph's text with tracked changes accepted: deleted runs left
    /// out.
    #[inline(never)]
    pub fn paragraph_text(&self, paragraph: &Paragraph) -> String {
        let mut text = String::new();
        for run in &paragraph.runs {
            if self.is_deleted(run) {
                continue;
            }
            match run.content {
                Inline::Text(span) => text.push_str(self.text(span)),
                Inline::LineBreak => text.push('\n'),
                Inline::Tab => text.push('\t'),
                Inline::Math(span) => text.push_str(&mathml_text(self.text(span))),
                Inline::PageBreak
                | Inline::Footnote(_)
                | Inline::Image(_)
                | Inline::PageNumber
                | Inline::PageCount => {}
            }
        }
        text
    }

    /// Every paragraph's text, one string per paragraph, for tests and
    /// simple projections. Tables are included in order.
    pub fn paragraph_texts(&self) -> Vec<String> {
        let mut texts = Vec::new();
        for section in &self.sections {
            self.collect_texts(&section.blocks, &mut texts);
        }
        texts
    }

    fn collect_texts(&self, blocks: &[Block], texts: &mut Vec<String>) {
        for block in blocks {
            match block {
                Block::Paragraph(paragraph) => texts.push(self.paragraph_text(paragraph)),
                Block::Table(table) => {
                    for row in &table.rows {
                        for cell in &row.cells {
                            self.collect_texts(&cell.blocks, texts);
                        }
                    }
                }
            }
        }
    }
}

/// The plain text of a MathML equation: its element text in order,
/// which reads as the equation on one line (`E=mc2`).
pub fn mathml_text(mathml: &str) -> String {
    let mut text = String::new();
    for event in crate::io::xml::XmlReader::new(mathml) {
        if let crate::io::xml::XmlEvent::Text(piece) = event {
            // Indentation between elements is not part of the formula.
            if piece.trim().is_empty() {
                continue;
            }
            text.push_str(&piece);
        }
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Adds `text` to `table` once and returns its index.
#[inline(never)]
fn intern(table: &mut Vec<String>, ids: &mut HashMap<String, usize>, text: &str) -> Id {
    if let Some(id) = ids.get(text) {
        return *id as Id;
    }
    let id = table.len();
    table.push(text.to_string());
    ids.insert(text.to_string(), id);
    id as Id
}
