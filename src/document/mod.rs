//! The document model: the structure and formatting that document formats
//! share. Readers (Pages, later HTML and Word) fill it; writers (Word,
//! later PDF) render it; the Markdown event stream is a projection of it.
//! It is the hub between document formats, kept to what those formats can
//! all express, with every property optional so "not set" means "inherit".
//!
//! Units: points for lengths and font sizes, sRGB bytes for colors.

pub mod markdown;

/// Index of a style in its table.
pub type StyleId = usize;
/// Index of a footnote in `Document::footnotes`.
pub type NoteId = usize;
/// Index of a media file in `Document::media`.
pub type MediaId = usize;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Document {
    pub styles: StyleTable,
    pub sections: Vec<Section>,
    pub footnotes: Vec<Note>,
    pub media: Vec<Media>,
    /// Objects placed on pages rather than in the text flow.
    pub floating: Vec<FloatingObject>,
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
    /// Direct formatting on top of the style.
    pub properties: ParagraphProperties,
    /// Direct character formatting set on the whole paragraph, under
    /// each run's own.
    pub run_properties: RunProperties,
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

#[derive(Debug, Clone, PartialEq, Default)]
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

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RunProperties {
    pub font: Option<String>,
    pub size: Option<f32>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strike: Option<bool>,
    pub color: Option<Color>,
    pub highlight: Option<Color>,
    pub baseline: Option<Baseline>,
    pub caps: Option<Caps>,
    pub language: Option<String>,
}

impl RunProperties {
    /// `other` on top of `self`: set fields win.
    pub fn overlay(&mut self, other: &RunProperties) {
        let RunProperties {
            font,
            size,
            bold,
            italic,
            underline,
            strike,
            color,
            highlight,
            baseline,
            caps,
            language,
        } = other;
        if font.is_some() {
            self.font.clone_from(font);
        }
        if size.is_some() {
            self.size = *size;
        }
        if bold.is_some() {
            self.bold = *bold;
        }
        if italic.is_some() {
            self.italic = *italic;
        }
        if underline.is_some() {
            self.underline = *underline;
        }
        if strike.is_some() {
            self.strike = *strike;
        }
        if color.is_some() {
            self.color = *color;
        }
        if highlight.is_some() {
            self.highlight = *highlight;
        }
        if baseline.is_some() {
            self.baseline = *baseline;
        }
        if caps.is_some() {
            self.caps = *caps;
        }
        if language.is_some() {
            self.language.clone_from(language);
        }
    }

    pub fn is_empty(&self) -> bool {
        *self == RunProperties::default()
    }
}

impl ParagraphProperties {
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
    /// Direct formatting on top of the style and the paragraph's.
    pub properties: RunProperties,
    /// The run is part of a link to this target.
    pub link: Option<String>,
    /// The run is a tracked change.
    pub revision: Option<Revision>,
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

#[derive(Debug, Clone, PartialEq)]
pub enum Inline {
    Text(String),
    LineBreak,
    Tab,
    /// A page break; usually the only content of its paragraph.
    PageBreak,
    /// An equation, as MathML.
    Math(String),
    Footnote(NoteId),
    Image(InlineImage),
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

    /// What a run looks like: paragraph style, then character style, then
    /// the run's own formatting.
    pub fn effective_run(&self, paragraph: &Paragraph, run: &Run) -> RunProperties {
        let mut properties = match paragraph.style {
            Some(style) => self.paragraph_style_run(style),
            None => RunProperties::default(),
        };
        properties.overlay(&paragraph.run_properties);
        if let Some(style) = run.style {
            properties.overlay(&self.character_style_run(style));
        }
        properties.overlay(&run.properties);
        properties
    }

    /// Every paragraph's text, one string per paragraph, for tests and
    /// simple projections. Footnotes and tables are included in order.
    pub fn paragraph_texts(&self) -> Vec<String> {
        let mut texts = Vec::new();
        for section in &self.sections {
            collect_texts(&section.blocks, &mut texts);
        }
        texts
    }
}

fn collect_texts(blocks: &[Block], texts: &mut Vec<String>) {
    for block in blocks {
        match block {
            Block::Paragraph(paragraph) => texts.push(paragraph.text()),
            Block::Table(table) => {
                for row in &table.rows {
                    for cell in &row.cells {
                        collect_texts(&cell.blocks, texts);
                    }
                }
            }
        }
    }
}

impl Paragraph {
    /// The text with tracked changes accepted: deleted runs left out.
    pub fn text(&self) -> String {
        let mut text = String::new();
        for run in &self.runs {
            let deleted = run
                .revision
                .as_ref()
                .is_some_and(|revision| revision.kind == RevisionKind::Deletion);
            if deleted {
                continue;
            }
            match &run.content {
                Inline::Text(piece) => text.push_str(piece),
                Inline::LineBreak => text.push('\n'),
                Inline::Tab => text.push('\t'),
                Inline::Math(mathml) => text.push_str(&mathml_text(mathml)),
                Inline::PageBreak
                | Inline::Footnote(_)
                | Inline::Image(_)
                | Inline::PageNumber
                | Inline::PageCount => {}
            }
        }
        text
    }
}

/// The plain text of a MathML equation: its element text in order,
/// which reads as the equation on one line (`E = mc2`).
pub fn mathml_text(mathml: &str) -> String {
    let mut text = String::new();
    for event in crate::io::xml::XmlReader::new(mathml) {
        if let crate::io::xml::XmlEvent::Text(piece) = event {
            text.push_str(&piece);
        }
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
