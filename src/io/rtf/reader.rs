//! RTF into the document model. The source is a tree of brace groups; each
//! group starts with the formatting of the one around it and may name a
//! destination (the font table, a footnote, a picture) that decides what its
//! text is. Text in the body becomes runs of the current flow (the body, a
//! footnote, a header, a comment, a text box), paragraphs end at `\par`,
//! `\cell`, and `\row`, and tables are rebuilt from RTF's row-by-row form.
//!
//! What it reads: the font, colour, style, list, and revision tables;
//! paragraph and character formatting; lists (Word's list tables and the
//! older `\pn` paragraphs); tables with merges, borders, shading, and
//! nesting; sections with page setup, columns, headers, and footers;
//! fields (links, page numbers); footnotes; comments; tracked changes;
//! pictures (PNG, JPEG, EMF, WMF, device-independent bitmaps); text boxes;
//! the page colour. Text in 8-bit code pages is decoded through the single-
//! byte tables in `codepage`; Unicode escapes (`\uN`) are taken as written.

use std::collections::HashMap;

use super::codepage;
use super::lexer::{Lexer, Token};
use crate::document::CharacterStyle;
use crate::document::{
    Alignment, Baseline, Block, Border, Caps, Cell, CellBorders, CellMargins, Color, Comment,
    Document, FloatingContent, FloatingObject, Id, Inline, InlineImage, LineSpacing, ListItem,
    ListLabel, ListLevel, ListStyle, Media, Merge, Note, NumberFormat, NumberKind, PageSetup,
    PageVariants, Paragraph, ParagraphBorder, ParagraphProperties, ParagraphStyle, Placement,
    Revision, RevisionKind, Row, Run, RunProperties, Section, SectionStart, ShapeGeometry, StyleId,
    TabAlignment, TabStop, Table, TableBorders, TextWrap, VerticalAlignment,
};

/// Why a file could not be read as RTF.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RtfError {
    /// The file does not start with `{\rtf`.
    NotRtf,
}

impl std::fmt::Display for RtfError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RtfError::NotRtf => write!(formatter, "not an RTF document (no {{\\rtf header)"),
        }
    }
}

impl std::error::Error for RtfError {}

/// Reads an RTF document into the document model.
pub fn read_rtf(bytes: &[u8]) -> Result<Document, RtfError> {
    let start = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(0);
    if !bytes[start..].starts_with(b"{\\rtf") {
        return Err(RtfError::NotRtf);
    }
    let mut reader = Reader::new(&bytes[start..]);
    reader.run();
    Ok(reader.finish())
}

/// Twentieths of a point to points.
fn twips(value: i32) -> f32 {
    value as f32 / 20.0
}

// ----- per-group state -----

/// What a group's text is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Destination {
    /// Document text: runs of the current flow.
    Body,
    /// Ignored, with everything inside it.
    Skip,
    /// Ignored text, but `\ud`, `\result`, and `\shppict` inside it are read
    /// as the destination around it.
    Wrapper(WrapperOf),
    FontTable,
    ColorTable,
    StyleSheet,
    ListTable,
    ListOverrideTable,
    RevisionTable,
    /// Text kept for the action the group runs when it closes (a font or
    /// style name, a field instruction, a comment's author).
    Collect,
    /// Hex digits of picture data.
    Picture,
    /// A shape's instructions: its position words and properties.
    Shape,
}

/// What a wrapper group returns to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WrapperOf {
    Body,
    Collect,
}

/// What a group does when it closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OnClose {
    StyleEntry,
    List,
    ListLevel,
    LevelText,
    LevelNumbers,
    ListOverride,
    Field,
    FieldInstruction,
    Picture,
    Footnote,
    Header(bool, PartKind),
    Annotation,
    AnnotationAuthor,
    AnnotationInitials,
    AnnotationDate,
    AnnotationReference,
    RangeStart,
    RangeEnd,
    Shape,
    ShapeProperty,
    ShapeName,
    ShapeValue,
    ShapeText,
    OldList,
    OldListBefore,
    OldListAfter,
    Background,
    ListText,
    PictureProperties,
    AnnotationParent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PartKind {
    Both,
    Left,
    Right,
    First,
}

/// Which lines the border words that follow describe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BorderTarget {
    None,
    Paragraph(Side),
    Cell(Side),
    Table(TableSide),
    Character,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Top,
    Bottom,
    Left,
    Right,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TableSide {
    Top,
    Bottom,
    Left,
    Right,
    InsideHorizontal,
    InsideVertical,
}

/// Character formatting in effect.
#[derive(Debug, Clone, Copy, Default)]
struct Chars {
    props: RunProperties,
    /// The font number (`\fN`), for its code page.
    font: Option<i32>,
    style: Option<StyleId>,
    link: Option<Id>,
    inserted: bool,
    deleted: bool,
    author: Option<i32>,
    date: Option<i32>,
    deletion_author: Option<i32>,
    deletion_date: Option<i32>,
}

/// Paragraph formatting in effect.
#[derive(Debug, Clone, Default)]
struct Para {
    props: ParagraphProperties,
    /// `\fi`, relative to the left indent.
    first_relative: Option<f32>,
    /// `\sl` and `\slmult`.
    line: Option<i32>,
    line_multiple: bool,
    tabs: Vec<TabStop>,
    tab_alignment: Option<TabAlignment>,
    tab_leader: Option<char>,
    style: Option<StyleId>,
    /// `\lsN` and `\ilvlN`.
    list: Option<i32>,
    level: u8,
    /// An old-style (`\pn`) list: its style, level, and start.
    old_list: Option<(StyleId, u8, u32)>,
    in_table: bool,
    /// Table nesting depth (`\itapN`), 1 for a top-level table.
    depth: usize,
    border_sides: [bool; 4],
    border: Option<Border>,
}

#[derive(Debug, Clone)]
struct Group {
    destination: Destination,
    chars: Chars,
    para: Para,
    /// How many characters stand in for each `\uN` (`\ucN`).
    unicode_skip: usize,
    /// Characters still to skip after the last `\uN`.
    unicode_skip_pending: usize,
    border_target: BorderTarget,
    on_close: Option<OnClose>,
    /// The collector this group's text goes to.
    collector: Option<usize>,
    /// A shape's property value being read (`\sv`): a picture in it is the
    /// shape's picture.
    in_shape_value: bool,
}

impl Group {
    fn root() -> Group {
        Group {
            destination: Destination::Body,
            chars: Chars::default(),
            para: Para::default(),
            unicode_skip: 1,
            unicode_skip_pending: 0,
            border_target: BorderTarget::None,
            on_close: None,
            collector: None,
            in_shape_value: false,
        }
    }
}

// ----- tables -----

/// One cell's definition in a row (`\cellx` and the words before it).
#[derive(Debug, Clone, Default)]
struct CellDef {
    right: i32,
    background: Option<Color>,
    borders: CellBorders,
    vertical_alignment: Option<VerticalAlignment>,
    margins: [Option<f32>; 4],
    merge_first: bool,
    merge_continue: bool,
    vertical_first: bool,
    vertical_continue: bool,
}

/// A row's definition (`\trowd` and what follows).
#[derive(Debug, Clone, Default)]
struct RowDef {
    left: i32,
    gap: Option<i32>,
    height: Option<i32>,
    header: bool,
    alignment: Option<Alignment>,
    cells: Vec<CellDef>,
    borders: TableBorders,
    padding: [Option<i32>; 4],
}

#[derive(Debug, Default)]
struct TableBuilder {
    rows: Vec<(RowDef, Vec<Vec<Block>>)>,
    cells: Vec<Vec<Block>>,
    cell_blocks: Vec<Block>,
}

// ----- flows -----

/// Where text goes: a list of blocks being built, with the paragraph and
/// tables in progress.
#[derive(Debug, Default)]
struct Flow {
    blocks: Vec<Block>,
    runs: Vec<Run>,
    tables: Vec<TableBuilder>,
}

// ----- the destinations' own state -----

#[derive(Debug, Default, Clone)]
struct Font {
    name: String,
    code_page: Option<u16>,
    symbol: bool,
}

#[derive(Debug, Default, Clone)]
struct LevelDef {
    format: i32,
    text: Vec<u16>,
    numbers: Vec<u8>,
    start: Option<i32>,
    first_relative: Option<f32>,
    left: Option<f32>,
    font: Option<i32>,
}

#[derive(Debug, Default, Clone)]
struct ListDef {
    id: i32,
    levels: Vec<LevelDef>,
}

#[derive(Debug, Default, Clone)]
struct OverrideDef {
    list: i32,
    number: i32,
    starts: Vec<(usize, i32)>,
}

#[derive(Debug, Default)]
struct PictureState {
    data: Vec<u8>,
    pending_nibble: Option<u8>,
    kind: Option<&'static str>,
    width: Option<i32>,
    height: Option<i32>,
    goal_width: Option<i32>,
    goal_height: Option<i32>,
    scale_x: Option<i32>,
    scale_y: Option<i32>,
    crop: [i32; 4],
    description: Option<String>,
}

#[derive(Debug, Default)]
struct ShapeState {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    from_margin_x: bool,
    from_margin_y: bool,
    follows_text: bool,
    wrap: Option<i32>,
    behind: bool,
    properties: Vec<(String, String)>,
    name: String,
    value: String,
    blocks: Option<Vec<Block>>,
    picture: Option<(usize, f32, f32)>,
}

#[derive(Debug, Default)]
struct FieldState {
    instruction: String,
}

#[derive(Debug, Default, Clone)]
struct OldListState {
    level: u8,
    bullet: bool,
    kind: Option<NumberKind>,
    start: Option<i32>,
    before: String,
    after: String,
    indent: Option<f32>,
    /// The font of its label text (`\pnfN`).
    font: Option<i32>,
}

#[derive(Debug, Default, Clone)]
struct SectionState {
    page: PageSetup,
    columns: u16,
    column_gap: Option<f32>,
    column_widths: Vec<(f32, f32)>,
    pending_column: Option<f32>,
    start: SectionStart,
    title_page: bool,
    headers: PageVariants,
    footers: PageVariants,
}

/// A comment being read: its author, initials, date, and the range it is on.
#[derive(Debug, Default)]
struct AnnotationState {
    author: Option<String>,
    initials: Option<String>,
    date: Option<String>,
    reference: Option<String>,
    /// The range of the comment this one replies to (`\atnparent`).
    parent: Option<String>,
}

struct Reader<'a> {
    lexer: Lexer<'a>,
    document: Document,
    groups: Vec<Group>,
    collectors: Vec<String>,
    flows: Vec<Flow>,

    default_code_page: u16,
    default_font: Option<i32>,
    fonts: HashMap<i32, Font>,
    font_entry: (Option<i32>, Font),
    colors: Vec<Option<Color>>,
    color_entry: (Option<u8>, Option<u8>, Option<u8>),

    paragraph_styles: HashMap<i32, StyleId>,
    character_styles: HashMap<i32, StyleId>,
    style_entry: StyleEntry,
    style_parents: Vec<(bool, StyleId, i32)>,
    style_runs: HashMap<StyleId, RunProperties>,

    lists: Vec<ListDef>,
    list_entry: ListDef,
    level_entry: LevelDef,
    overrides: Vec<OverrideDef>,
    override_entry: OverrideDef,
    list_styles: HashMap<i32, StyleId>,
    seen_lists: Vec<i32>,
    old_list: OldListState,
    old_lists: HashMap<String, StyleId>,

    row_defs: Vec<RowDef>,
    cell_def: CellDef,

    fields: Vec<FieldState>,
    pictures: Vec<PictureState>,
    shapes: Vec<ShapeState>,
    in_background: bool,

    revision_authors: Vec<String>,
    revision_entry: String,
    revisions: HashMap<(bool, Option<i32>, Option<i32>), Id>,

    annotation: AnnotationState,
    ranges: HashMap<String, Id>,
    pending_initials: Option<String>,
    pending_author: Option<String>,

    document_page: PageSetup,
    section: SectionState,
    facing_pages: bool,

    run_ids: HashMap<RunKey, Id>,
    paragraph_ids: HashMap<ParagraphKey, Id>,

    high_surrogate: Option<u16>,
    pending_lead: Option<u8>,
    /// The rendered label (`\listtext`) of the paragraph being read.
    list_label: Option<String>,
    /// A note's own mark was just read: the space after it is not text.
    after_note_mark: bool,
}

#[derive(Debug, Default)]
struct StyleEntry {
    number: Option<i32>,
    character: bool,
    skip: bool,
    parent: Option<i32>,
}

type RunKey = [u32; 16];
type ParagraphKey = [u32; 24];

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Reader<'a> {
        let mut document = Document::default();
        document.text.reserve(bytes.len() / 2);
        Reader {
            lexer: Lexer::new(bytes),
            document,
            groups: vec![Group::root()],
            collectors: Vec::new(),
            flows: vec![Flow::default()],
            default_code_page: 1252,
            default_font: None,
            fonts: HashMap::new(),
            font_entry: (None, Font::default()),
            colors: Vec::new(),
            color_entry: (None, None, None),
            paragraph_styles: HashMap::new(),
            character_styles: HashMap::new(),
            style_entry: StyleEntry::default(),
            style_parents: Vec::new(),
            style_runs: HashMap::new(),
            lists: Vec::new(),
            list_entry: ListDef::default(),
            level_entry: LevelDef::default(),
            overrides: Vec::new(),
            override_entry: OverrideDef::default(),
            list_styles: HashMap::new(),
            seen_lists: Vec::new(),
            old_list: OldListState::default(),
            old_lists: HashMap::new(),
            row_defs: Vec::new(),
            cell_def: CellDef::default(),
            fields: Vec::new(),
            pictures: Vec::new(),
            shapes: Vec::new(),
            in_background: false,
            revision_authors: Vec::new(),
            revision_entry: String::new(),
            revisions: HashMap::new(),
            annotation: AnnotationState::default(),
            ranges: HashMap::new(),
            pending_initials: None,
            pending_author: None,
            document_page: PageSetup::default(),
            section: SectionState {
                columns: 1,
                ..SectionState::default()
            },
            facing_pages: false,
            run_ids: HashMap::new(),
            paragraph_ids: HashMap::new(),
            high_surrogate: None,
            pending_lead: None,
            list_label: None,
            after_note_mark: false,
        }
    }

    fn group(&self) -> &Group {
        // The root group is never popped.
        &self.groups[self.groups.len() - 1]
    }

    fn group_mut(&mut self) -> &mut Group {
        let last = self.groups.len() - 1;
        &mut self.groups[last]
    }

    fn run(&mut self) {
        while let Some(token) = self.lexer.next() {
            match token {
                Token::Open => self.open(),
                Token::Close => {
                    if self.groups.len() <= 1 {
                        // The document's own closing brace: anything after
                        // it is not part of the document.
                        break;
                    }
                    self.close();
                }
                Token::Word { name, parameter } => self.word(name, parameter),
                Token::Symbol(symbol) => self.symbol(symbol),
                Token::Byte(byte) => {
                    if self.skip_one() {
                        continue;
                    }
                    self.bytes(&[byte]);
                }
                Token::Text(bytes) => {
                    let bytes = self.skip_bytes(bytes);
                    if !bytes.is_empty() {
                        self.bytes(bytes);
                    }
                }
                Token::Binary(data) => {
                    if self.group().destination == Destination::Picture
                        && let Some(picture) = self.pictures.last_mut()
                    {
                        picture.data.extend_from_slice(data);
                    }
                }
            }
        }
    }

    // ----- groups -----

    fn open(&mut self) {
        let mut child = self.group().clone();
        child.on_close = None;
        child.unicode_skip_reset();
        self.groups.push(child);
    }

    fn close(&mut self) {
        let Some(group) = self.groups.pop() else {
            return;
        };
        if let Some(action) = group.on_close {
            self.on_close(action, &group);
        }
    }

    /// Starts a collector for this group's text.
    fn collect(&mut self, on_close: OnClose) {
        self.collectors.push(String::new());
        let index = self.collectors.len() - 1;
        let group = self.group_mut();
        group.destination = Destination::Collect;
        group.collector = Some(index);
        group.on_close = Some(on_close);
    }

    /// The text collected by a closing group.
    fn collected(&mut self, group: &Group) -> String {
        match group.collector {
            Some(index) if index + 1 == self.collectors.len() => {
                self.collectors.pop().unwrap_or_default()
            }
            Some(index) => self.collectors.get(index).cloned().unwrap_or_default(),
            None => String::new(),
        }
    }

    /// Starts a new flow for this group's text (a footnote, a header).
    fn push_flow(&mut self, on_close: OnClose) {
        self.flows.push(Flow::default());
        let group = self.group_mut();
        group.destination = Destination::Body;
        group.on_close = Some(on_close);
        // A new story starts with default paragraph formatting.
        group.para = Para::default();
    }

    /// Ends the current flow and returns its blocks.
    fn pop_flow(&mut self) -> Vec<Block> {
        if self.flows.len() <= 1 {
            return Vec::new();
        }
        self.end_story();
        self.flows.pop().map(|flow| flow.blocks).unwrap_or_default()
    }

    // ----- unicode skipping -----

    fn skip_one(&mut self) -> bool {
        let group = self.group_mut();
        if group.unicode_skip_pending > 0 {
            group.unicode_skip_pending -= 1;
            return true;
        }
        false
    }

    fn skip_bytes<'b>(&mut self, bytes: &'b [u8]) -> &'b [u8] {
        let group = self.group_mut();
        let skip = group.unicode_skip_pending.min(bytes.len());
        group.unicode_skip_pending -= skip;
        &bytes[skip..]
    }

    // ----- text -----

    fn code_page(&self) -> (u16, bool) {
        let font = self.group().chars.font.or(self.default_font);
        match font.and_then(|number| self.fonts.get(&number)) {
            Some(font) if font.symbol => (0, true),
            Some(font) => (font.code_page.unwrap_or(self.default_code_page), false),
            None => (self.default_code_page, false),
        }
    }

    /// Decodes bytes in the current code page and routes the text.
    fn bytes(&mut self, bytes: &[u8]) {
        if bytes.is_ascii() && self.pending_lead.is_none() {
            // Most text: no decoding needed.
            let text = std::str::from_utf8(bytes).unwrap_or("");
            self.text(text);
            return;
        }
        let (code_page, symbol) = self.code_page();
        let mut text = String::with_capacity(bytes.len());
        for &byte in bytes {
            if let Some(lead) = self.pending_lead.take() {
                // A double-byte character the single-byte tables cannot
                // decode; Word writes `\uN` for these, so this is rare.
                let _ = (lead, byte);
                text.push('\u{FFFD}');
                continue;
            }
            if byte < 0x80 {
                if symbol && byte >= 0x20 {
                    text.push(char::from_u32(0xF000 + u32::from(byte)).unwrap_or(' '));
                } else {
                    text.push(byte as char);
                }
                continue;
            }
            if symbol {
                text.push(char::from_u32(0xF000 + u32::from(byte)).unwrap_or(' '));
                continue;
            }
            if is_lead_byte(code_page, byte) {
                self.pending_lead = Some(byte);
                continue;
            }
            let character =
                match codepage::upper_half(code_page).or_else(|| codepage::upper_half(1252)) {
                    Some(table) => char::from_u32(u32::from(table[usize::from(byte - 0x80)]))
                        .unwrap_or('\u{FFFD}'),
                    None => byte as char,
                };
            text.push(character);
        }
        self.text(&text);
    }

    /// Routes text by the current destination.
    fn text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        match self.group().destination {
            Destination::Body => self.body_text(text),
            Destination::Collect => {
                if let Some(index) = self.group().collector
                    && let Some(buffer) = self.collectors.get_mut(index)
                {
                    buffer.push_str(text);
                }
            }
            Destination::FontTable => self.font_table_text(text),
            Destination::ColorTable => {
                for _ in text.chars().filter(|character| *character == ';') {
                    self.end_color();
                }
            }
            Destination::Picture => self.picture_hex(text),
            Destination::RevisionTable => {
                for character in text.chars() {
                    if character == ';' {
                        let name = std::mem::take(&mut self.revision_entry);
                        self.revision_authors.push(name.trim().to_string());
                    } else {
                        self.revision_entry.push(character);
                    }
                }
            }
            Destination::Shape
            | Destination::Skip
            | Destination::Wrapper(_)
            | Destination::StyleSheet
            | Destination::ListTable
            | Destination::ListOverrideTable => {}
        }
    }

    fn character(&mut self, character: char) {
        let mut buffer = [0u8; 4];
        let text = character.encode_utf8(&mut buffer);
        self.text(text);
    }

    fn unicode(&mut self, value: i32) {
        let unit = if value < 0 { value + 65536 } else { value } as u32;
        let skip = self.group().unicode_skip;
        self.group_mut().unicode_skip_pending = skip;
        if (0xD800..0xDC00).contains(&unit) {
            self.high_surrogate = Some(unit as u16);
            return;
        }
        let character = if (0xDC00..0xE000).contains(&unit) {
            match self.high_surrogate.take() {
                Some(high) => {
                    let code = 0x10000 + ((u32::from(high) - 0xD800) << 10) + (unit - 0xDC00);
                    char::from_u32(code)
                }
                None => None,
            }
        } else {
            self.high_surrogate = None;
            char::from_u32(unit)
        };
        if let Some(character) = character {
            self.character(character);
        }
    }

    fn body_text(&mut self, text: &str) {
        // The line separator (Cocoa's line break) breaks the line.
        if text.contains('\u{2028}') {
            let mut first = true;
            for piece in text.split('\u{2028}') {
                if !first {
                    self.inline(Inline::LineBreak);
                }
                first = false;
                if !piece.is_empty() {
                    self.body_text(piece);
                }
            }
            return;
        }
        // A literal tab in the text is a tab.
        if text.contains('\t') {
            let mut first = true;
            for piece in text.split('\t') {
                if !first {
                    self.inline(Inline::Tab);
                }
                first = false;
                if !piece.is_empty() {
                    self.append_text(piece);
                }
            }
            return;
        }
        self.append_text(text);
    }

    fn append_text(&mut self, text: &str) {
        let text = if std::mem::take(&mut self.after_note_mark) {
            match text.strip_prefix(' ') {
                Some("") => return,
                Some(rest) => rest,
                None => text,
            }
        } else {
            text
        };
        let (style, properties, link, revision) = self.run_identity();
        let end = self.document.text.len() as u32;
        let flow = self.flows.last_mut();
        if let Some(flow) = flow
            && let Some(last) = flow.runs.last_mut()
            && let Inline::Text(span) = &mut last.content
            && span.end == end
            && last.style == style
            && last.properties == properties
            && last.link == link
            && last.revision == revision
        {
            self.document.text.push_str(text);
            span.end = self.document.text.len() as u32;
            return;
        }
        let span = self.document.push_text(text);
        if let Some(flow) = self.flows.last_mut() {
            flow.runs.push(Run {
                style,
                properties,
                link,
                revision,
                content: Inline::Text(span),
            });
        }
    }

    /// Adds a non-text run (a tab, a break, an image) at the current place.
    fn inline(&mut self, content: Inline) {
        if self.group().destination != Destination::Body {
            if content == Inline::Tab {
                self.text("\t");
            }
            return;
        }
        let (style, properties, link, revision) = self.run_identity();
        if let Some(flow) = self.flows.last_mut() {
            flow.runs.push(Run {
                style,
                properties,
                link,
                revision,
                content,
            });
        }
    }

    /// The current run's style, interned formatting, link, and revision.
    fn run_identity(&mut self) -> (Option<StyleId>, Option<Id>, Option<Id>, Option<Id>) {
        let chars = self.group().chars;
        let paragraph_style = self
            .group()
            .para
            .style
            .or(self.document.styles.default_paragraph);
        let mut props = chars.props;
        // Text that names no font is in the document's default font.
        if props.font.is_none()
            && let Some(number) = self.default_font
            && let Some(font) = self.fonts.get(&number)
            && !font.name.is_empty()
        {
            let name = font.name.clone();
            props.font = Some(self.document.intern_string(&name));
        }
        // RTF formatting is absolute: a style's character formatting
        // reaches text only where it is repeated on the text. Where the
        // paragraph's style sets something the text does not, the text has
        // RTF's default.
        if let Some(style) = paragraph_style {
            let style_run = self.style_run(style);
            fill_defaults(&mut props, &style_run);
            // What only repeats the style is the style's, not the text's.
            strip_repeats(&mut props, &style_run);
        }
        let properties = self.intern_run(props);
        let revision = self.revision_id(&chars);
        (chars.style, properties, chars.link, revision)
    }

    fn style_run(&mut self, style: StyleId) -> RunProperties {
        if let Some(run) = self.style_runs.get(&style) {
            return *run;
        }
        let run = if style < self.document.styles.paragraph.len() {
            self.document.paragraph_style_run(style)
        } else {
            RunProperties::default()
        };
        self.style_runs.insert(style, run);
        run
    }

    fn intern_run(&mut self, props: RunProperties) -> Option<Id> {
        if props.is_empty() {
            return None;
        }
        let key = run_key(&props);
        if let Some(id) = self.run_ids.get(&key) {
            return Some(*id);
        }
        let id = self.document.intern_run_properties(props)?;
        self.run_ids.insert(key, id);
        Some(id)
    }

    fn intern_paragraph(&mut self, props: ParagraphProperties) -> Option<Id> {
        if props.is_empty() {
            return None;
        }
        let key = paragraph_key(&props);
        if let Some(id) = self.paragraph_ids.get(&key) {
            return Some(*id);
        }
        let id = self.document.intern_paragraph_properties(props)?;
        self.paragraph_ids.insert(key, id);
        Some(id)
    }

    fn revision_id(&mut self, chars: &Chars) -> Option<Id> {
        let (deleted, author, date) = if chars.deleted {
            (
                true,
                chars.deletion_author.or(chars.author),
                chars.deletion_date.or(chars.date),
            )
        } else if chars.inserted {
            (false, chars.author, chars.date)
        } else {
            return None;
        };
        let key = (deleted, author, date);
        if let Some(id) = self.revisions.get(&key) {
            return Some(*id);
        }
        let author_name = author
            .and_then(|index| self.revision_authors.get(index as usize))
            .filter(|name| !name.is_empty() && name.as_str() != "Unknown")
            .cloned();
        let id = self.document.push_revision(Revision {
            kind: if deleted {
                RevisionKind::Deletion
            } else {
                RevisionKind::Insertion
            },
            author: author_name,
            date: date.and_then(packed_date),
        });
        self.revisions.insert(key, id);
        Some(id)
    }

    // ----- paragraphs -----

    /// Ends the paragraph in progress (`\par`, `\cell`).
    fn end_paragraph(&mut self) {
        if self.group().destination != Destination::Body {
            if self.group().destination == Destination::Collect {
                self.text("\n");
            }
            return;
        }
        let paragraph = self.build_paragraph();
        let para = &self.group().para;
        let depth = if para.in_table { para.depth.max(1) } else { 0 };
        let Some(flow) = self.flows.last_mut() else {
            return;
        };
        finish_tables_deeper_than(flow, depth, &self.row_defs);
        let block = Block::Paragraph(paragraph);
        if depth == 0 {
            flow.blocks.push(block);
        } else {
            ensure_tables(flow, depth);
            flow.tables[depth - 1].cell_blocks.push(block);
        }
    }

    fn build_paragraph(&mut self) -> Paragraph {
        let runs = self
            .flows
            .last_mut()
            .map(|flow| std::mem::take(&mut flow.runs))
            .unwrap_or_default();
        let para = self.group().para.clone();
        let mut props = para.props;
        if let Some(relative) = para.first_relative {
            let left = props.left_indent.or_else(|| {
                para.style.and_then(|style| {
                    (style < self.document.styles.paragraph.len())
                        .then(|| self.document.paragraph_style_properties(style).left_indent)
                        .flatten()
                })
            });
            props.first_line_indent = Some(left.unwrap_or(0.0) + relative);
        }
        props.line_spacing = line_spacing(para.line, para.line_multiple);
        if !para.tabs.is_empty() {
            props.tabs = Some(self.document.intern_tabs(para.tabs.clone()));
        }
        if let Some(line) = para.border
            && para.border_sides.iter().any(|side| *side)
        {
            props.border = Some(ParagraphBorder {
                top: para.border_sides[0],
                bottom: para.border_sides[1],
                left: para.border_sides[2],
                right: para.border_sides[3],
                line,
            });
        }
        let has_text = runs
            .iter()
            .any(|run| matches!(run.content, Inline::Text(_)));
        let mark = if has_text {
            None
        } else {
            let props = self.group().chars.props;
            self.intern_run(props)
        };
        let label = self.list_label.take();
        let list = match (para.list, para.old_list) {
            (Some(number), _) => self.list_item(number, para.level).map(|mut item| {
                // Some writers keep a list's real start only in its first
                // item's rendered label.
                if item.starts_list
                    && let Some(number) = label
                        .as_deref()
                        .and_then(|label| self.label_number(item.style, item.level, label))
                {
                    item.start = number;
                }
                item
            }),
            (None, Some((style, level, start))) => {
                let key = style as i32 + 1_000_000;
                let starts_list = !self.seen_lists.contains(&key);
                if starts_list {
                    self.seen_lists.push(key);
                }
                Some(ListItem {
                    style,
                    level,
                    starts_list,
                    start,
                })
            }
            _ => None,
        };
        Paragraph {
            // A paragraph that names no style is in style 0.
            style: para.style.or(self.document.styles.default_paragraph),
            properties: self.intern_paragraph(props),
            run_properties: None,
            mark,
            list,
            runs,
        }
    }

    /// Finishes everything open in the current flow at the end of a story.
    fn end_story(&mut self) {
        let has_runs = self.flows.last().is_some_and(|flow| !flow.runs.is_empty());
        if has_runs {
            // The last paragraph of a story need not end with `\par`.
            let saved = self.group().destination;
            self.group_mut().destination = Destination::Body;
            self.end_paragraph();
            self.group_mut().destination = saved;
        }
        if let Some(flow) = self.flows.last_mut() {
            finish_tables_deeper_than(flow, 0, &self.row_defs);
        }
    }

    // ----- tables -----

    fn end_cell(&mut self, nested: bool) {
        if self.group().destination != Destination::Body {
            return;
        }
        {
            let para = &mut self.group_mut().para;
            para.in_table = true;
            if nested && para.depth < 2 {
                para.depth = 2;
            }
        }
        self.end_paragraph();
        let depth = self.group().para.depth.max(1);
        let Some(flow) = self.flows.last_mut() else {
            return;
        };
        ensure_tables(flow, depth);
        let table = &mut flow.tables[depth - 1];
        let blocks = std::mem::take(&mut table.cell_blocks);
        table.cells.push(blocks);
    }

    fn end_row(&mut self, nested: bool) {
        if self.group().destination != Destination::Body {
            return;
        }
        let depth = if nested {
            self.group().para.depth.max(2)
        } else {
            self.group().para.depth.max(1)
        };
        let definition = self.row_defs.get(depth - 1).cloned().unwrap_or_default();
        let Some(flow) = self.flows.last_mut() else {
            return;
        };
        // Text after the last cell and before `\row` belongs to no cell.
        if !flow.runs.is_empty() {
            flow.runs.clear();
        }
        finish_tables_deeper_than(flow, depth, &self.row_defs);
        ensure_tables(flow, depth);
        let table = &mut flow.tables[depth - 1];
        let mut cells = std::mem::take(&mut table.cells);
        if !table.cell_blocks.is_empty() {
            cells.push(std::mem::take(&mut table.cell_blocks));
        }
        table.rows.push((definition, cells));
    }

    fn table_depth(&self) -> usize {
        self.group().para.depth.max(1)
    }

    fn row_def(&mut self) -> &mut RowDef {
        let depth = self.table_depth();
        while self.row_defs.len() < depth {
            self.row_defs.push(RowDef::default());
        }
        &mut self.row_defs[depth - 1]
    }

    // ----- lists -----

    fn list_item(&mut self, number: i32, level: u8) -> Option<ListItem> {
        let style = self.list_style(number)?;
        let starts_list = !self.seen_lists.contains(&number);
        if starts_list {
            self.seen_lists.push(number);
        }
        let override_def = self.overrides.iter().find(|entry| entry.number == number);
        let list_id = override_def.map(|entry| entry.list);
        let override_start = override_def.and_then(|entry| {
            entry
                .starts
                .iter()
                .find(|(at, _)| *at == usize::from(level))
                .map(|(_, start)| *start)
        });
        let level_start = list_id
            .and_then(|id| self.lists.iter().find(|list| list.id == id))
            .and_then(|list| list.levels.get(usize::from(level)))
            .and_then(|level| level.start);
        let start = override_start.or(level_start).unwrap_or(1).max(0) as u32;
        Some(ListItem {
            style,
            level,
            starts_list,
            start,
        })
    }

    /// The number a rendered list label shows, read in its level's style.
    fn label_number(&self, style: StyleId, level: u8, label: &str) -> Option<u32> {
        let level = self
            .document
            .styles
            .list
            .get(style)?
            .levels
            .get(usize::from(level))?;
        let ListLabel::Number(format) = &level.label else {
            return None;
        };
        let token: String = label
            .trim()
            .split(|character: char| !character.is_alphanumeric())
            .rfind(|piece| !piece.is_empty())?
            .to_string();
        parse_number(format.kind, &token)
    }

    /// The model list style for list override `number`, made on first use.
    fn list_style(&mut self, number: i32) -> Option<StyleId> {
        if let Some(style) = self.list_styles.get(&number) {
            return Some(*style);
        }
        let list_id = self
            .overrides
            .iter()
            .find(|entry| entry.number == number)
            .map(|entry| entry.list);
        let definition = list_id
            .and_then(|id| self.lists.iter().find(|list| list.id == id))
            .cloned()
            .unwrap_or_default();
        let mut levels = Vec::new();
        for level in &definition.levels {
            levels.push(self.list_level(level, levels.len()));
        }
        if levels.is_empty() {
            levels.push(ListLevel {
                label: ListLabel::Text("\u{2022}".to_string()),
                indent: 36.0,
                label_indent: 18.0,
            });
        }
        self.document.styles.list.push(ListStyle {
            name: format!("List {number}"),
            levels,
        });
        let style = self.document.styles.list.len() - 1;
        self.list_styles.insert(number, style);
        Some(style)
    }

    fn list_level(&self, level: &LevelDef, index: usize) -> ListLevel {
        let left = level.left.unwrap_or(36.0 * (index as f32 + 1.0));
        let first = level.first_relative.unwrap_or(-18.0);
        let symbol_font = level
            .font
            .and_then(|number| self.fonts.get(&number))
            .is_some_and(|font| font.symbol);
        // The level text: its length, then characters, with \'00..\'08
        // standing for the levels' numbers.
        let text: Vec<u16> = level.text.iter().skip(1).copied().collect();
        let label = match level.format {
            255 => ListLabel::None,
            23 => {
                let mut bullet = String::new();
                for unit in &text {
                    let unit = if symbol_font && *unit < 0x100 {
                        0xF000 + u32::from(*unit)
                    } else {
                        u32::from(*unit)
                    };
                    if let Some(character) = char::from_u32(unit) {
                        bullet.push(character);
                    }
                }
                let bullet = crate::io::docx::reader::symbol_bullet(&bullet);
                ListLabel::Text(if bullet.trim().is_empty() {
                    "\u{2022}".to_string()
                } else {
                    bullet
                })
            }
            format => {
                let kind = match format {
                    1 => NumberKind::UpperRoman,
                    2 => NumberKind::LowerRoman,
                    3 => NumberKind::UpperLetter,
                    4 => NumberKind::LowerLetter,
                    _ => NumberKind::Decimal,
                };
                let own = index as u16;
                let tiered = text.iter().any(|unit| *unit < own);
                let start = if tiered {
                    text.iter().position(|unit| *unit == own).unwrap_or(0)
                } else {
                    0
                };
                let mut pattern = String::new();
                for unit in &text[start..] {
                    if *unit < 9 {
                        pattern.push_str("%1");
                    } else if let Some(character) = char::from_u32(u32::from(*unit)) {
                        pattern.push(character);
                    }
                }
                if pattern.is_empty() {
                    pattern.push_str("%1.");
                }
                ListLabel::Number(NumberFormat {
                    kind,
                    pattern,
                    tiered,
                })
            }
        };
        ListLevel {
            label,
            indent: left,
            label_indent: left + first,
        }
    }

    // ----- control words -----

    fn word(&mut self, name: &str, parameter: Option<i32>) {
        // A control word counts as one character skipped after `\uN`.
        if name != "u" && self.skip_one() {
            return;
        }
        let value = parameter.unwrap_or(0);
        let on = parameter != Some(0);
        if self.destination_word(name, parameter) {
            return;
        }
        match self.group().destination {
            Destination::Skip => return,
            Destination::FontTable => {
                self.font_word(name, value);
                return;
            }
            Destination::ColorTable => {
                match name {
                    "red" => self.color_entry.0 = Some(value.clamp(0, 255) as u8),
                    "green" => self.color_entry.1 = Some(value.clamp(0, 255) as u8),
                    "blue" => self.color_entry.2 = Some(value.clamp(0, 255) as u8),
                    _ => {}
                }
                return;
            }
            Destination::Picture => {
                self.picture_word(name, value);
                return;
            }
            Destination::Shape => {
                self.shape_word(name, value);
                return;
            }
            _ => {}
        }
        if self.list_word(name, parameter) {
            return;
        }
        if self.character_word(name, value, on, parameter) {
            return;
        }
        if self.paragraph_word(name, value, on) {
            return;
        }
        if self.table_word(name, value) {
            return;
        }
        if self.section_word(name, value) {
            return;
        }
        self.special_word(name, value);
    }

    /// Words that start a destination; returns whether `name` was one.
    fn destination_word(&mut self, name: &str, parameter: Option<i32>) -> bool {
        let destination = self.group().destination;
        let inside_wrapper = matches!(destination, Destination::Wrapper(_));
        if destination == Destination::Skip {
            return true;
        }
        match name {
            "rtf" => {}
            "fonttbl" => self.group_mut().destination = Destination::FontTable,
            "colortbl" => self.group_mut().destination = Destination::ColorTable,
            "stylesheet" => self.group_mut().destination = Destination::StyleSheet,
            "listtable" => self.group_mut().destination = Destination::ListTable,
            "listoverridetable" => self.group_mut().destination = Destination::ListOverrideTable,
            "revtbl" => self.group_mut().destination = Destination::RevisionTable,
            "info" | "xmlnstbl" | "rsidtbl" | "generator" | "userprops" | "themedata"
            | "colorschememapping" | "latentstyles" | "datastore" | "mmathPr" | "pgdsctbl"
            | "listpicture" | "pnseclvl" | "template" | "docvar" | "ftnsep" | "ftnsepc"
            | "aftnsep" | "aftnsepc" | "txe" | "xe" | "tc" | "tcn" | "nonshppict" | "bkmkstart"
            | "bkmkend" | "objdata" | "objclass" | "objname" | "fontemb" | "fontfile" | "falt"
            | "panose" | "shprslt" | "do" | "expandedcolortbl" | "fldtype" | "datafield"
            | "formfield" | "blipuid" | "leveltemplateid" | "levelmarker" | "listname"
            | "listpicture_" | "annotation_" | "atnicn" | "atntime" | "wgrffmtfilter"
            | "pgptbl" | "protusertbl" | "oldcprops" | "oldpprops" | "oldsprops" | "oldtprops"
            | "mmath" | "ffdeftext" | "ffname" | "ffentrymcr" | "ffexitmcr" | "ffstattext"
            | "ffhelptext" | "ffformat" | "footnotes_" | "category" | "factoidname" | "htmltag"
            | "mhtmltag" | "comment" | "hlinkbase" | "passwordhash" | "password"
            | "writereservhash" | "xform" | "ebcstart" | "ebcend" | "nesttableprops_" => {
                self.group_mut().destination = Destination::Skip;
            }
            "listtext" | "pntext" => self.collect(OnClose::ListText),
            "nesttableprops" => {
                // Row properties of a nested table, read as the row's.
                self.group_mut().destination = Destination::Body;
            }
            "upr" => {
                let back = if destination == Destination::Collect {
                    WrapperOf::Collect
                } else {
                    WrapperOf::Body
                };
                self.group_mut().destination = Destination::Wrapper(back);
            }
            "object" => self.group_mut().destination = Destination::Wrapper(WrapperOf::Body),
            "ud" | "result" | "shppict" => {
                if let Destination::Wrapper(back) = destination {
                    self.group_mut().destination = match back {
                        WrapperOf::Body => Destination::Body,
                        WrapperOf::Collect => Destination::Collect,
                    };
                }
            }
            "f" if destination == Destination::StyleSheet => return false,
            "s" | "cs" | "ds" | "ts" | "tsrowd"
                if destination == Destination::StyleSheet && self.style_entry.number.is_none() =>
            {
                self.style_entry = StyleEntry {
                    number: parameter,
                    character: name == "cs",
                    skip: name == "ds" || name == "ts" || name == "tsrowd",
                    parent: None,
                };
                self.collect(OnClose::StyleEntry);
                // The entry's formatting words are read into its own state.
                let group = self.group_mut();
                group.chars = Chars::default();
                group.para = Para::default();
            }
            "list" if destination == Destination::ListTable => {
                self.list_entry = ListDef::default();
                self.group_mut().on_close = Some(OnClose::List);
            }
            "listlevel" => {
                self.level_entry = LevelDef::default();
                let group = self.group_mut();
                group.on_close = Some(OnClose::ListLevel);
                group.para = Para::default();
                group.chars = Chars::default();
            }
            "leveltext" => self.collect(OnClose::LevelText),
            "levelnumbers" => self.collect(OnClose::LevelNumbers),
            "listoverride" if destination == Destination::ListOverrideTable => {
                self.override_entry = OverrideDef::default();
                self.group_mut().on_close = Some(OnClose::ListOverride);
            }
            "field" => {
                self.fields.push(FieldState::default());
                self.group_mut().on_close = Some(OnClose::Field);
            }
            "fldinst" => self.collect(OnClose::FieldInstruction),
            "fldrslt" => self.field_result(),
            "picprop" if !self.pictures.is_empty() => {
                // The picture's properties, in shape-property form; the
                // description (alt text) is kept.
                self.shapes.push(ShapeState::default());
                let group = self.group_mut();
                group.destination = Destination::Shape;
                group.on_close = Some(OnClose::PictureProperties);
            }
            "picprop" => self.group_mut().destination = Destination::Skip,
            "pict" => {
                self.pictures.push(PictureState::default());
                let group = self.group_mut();
                group.destination = Destination::Picture;
                group.on_close = Some(OnClose::Picture);
            }
            "footnote" => self.push_flow(OnClose::Footnote),
            "header" => self.push_flow(OnClose::Header(false, PartKind::Both)),
            "headerl" => self.push_flow(OnClose::Header(false, PartKind::Left)),
            "headerr" => self.push_flow(OnClose::Header(false, PartKind::Right)),
            "headerf" => self.push_flow(OnClose::Header(false, PartKind::First)),
            "footer" => self.push_flow(OnClose::Header(true, PartKind::Both)),
            "footerl" => self.push_flow(OnClose::Header(true, PartKind::Left)),
            "footerr" => self.push_flow(OnClose::Header(true, PartKind::Right)),
            "footerf" => self.push_flow(OnClose::Header(true, PartKind::First)),
            "annotation" => {
                self.annotation = AnnotationState {
                    author: self.pending_author.take(),
                    initials: self.pending_initials.take(),
                    ..AnnotationState::default()
                };
                self.push_flow(OnClose::Annotation);
            }
            "atnauthor" => self.collect(OnClose::AnnotationAuthor),
            "atnid" => self.collect(OnClose::AnnotationInitials),
            "atndate" => self.collect(OnClose::AnnotationDate),
            "atnref" => self.collect(OnClose::AnnotationReference),
            "atnparent" => self.collect(OnClose::AnnotationParent),
            "atrfstart" => self.collect(OnClose::RangeStart),
            "atrfend" => self.collect(OnClose::RangeEnd),
            "shp" => {
                self.shapes.push(ShapeState::default());
                let group = self.group_mut();
                group.on_close = Some(OnClose::Shape);
            }
            "shpinst" => self.group_mut().destination = Destination::Shape,
            "sp" => self.group_mut().on_close = Some(OnClose::ShapeProperty),
            "sn" => self.collect(OnClose::ShapeName),
            "sv" => {
                self.collect(OnClose::ShapeValue);
                self.group_mut().in_shape_value = true;
            }
            "shptxt" => self.push_flow(OnClose::ShapeText),
            "background" => {
                self.in_background = true;
                self.group_mut().on_close = Some(OnClose::Background);
            }
            "pn" => {
                self.old_list = OldListState::default();
                let group = self.group_mut();
                group.on_close = Some(OnClose::OldList);
                group.destination = Destination::Shape;
            }
            "pntxtb" => self.collect(OnClose::OldListBefore),
            "pntxta" => self.collect(OnClose::OldListAfter),
            _ => {
                if inside_wrapper {
                    return true;
                }
                return false;
            }
        }
        true
    }

    fn field_result(&mut self) {
        let instruction = self
            .fields
            .last()
            .map(|field| field.instruction.trim().to_string())
            .unwrap_or_default();
        let keyword = instruction
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        let group = self.group_mut();
        group.destination = Destination::Body;
        match keyword.as_str() {
            "HYPERLINK" => {
                if let Some(target) = hyperlink_target(&instruction) {
                    let link = self.document.intern_link(&target);
                    self.group_mut().chars.link = Some(link);
                }
            }
            "PAGE" => {
                self.inline(Inline::PageNumber);
                self.group_mut().destination = Destination::Skip;
            }
            "NUMPAGES" | "SECTIONPAGES" => {
                self.inline(Inline::PageCount);
                self.group_mut().destination = Destination::Skip;
            }
            _ => {}
        }
    }

    fn font_word(&mut self, name: &str, value: i32) {
        match name {
            "f" => {
                if self.font_entry.0.is_some() && !self.font_entry.1.name.is_empty() {
                    self.end_font();
                }
                self.font_entry = (Some(value), Font::default());
            }
            "fcharset" => {
                self.font_entry.1.symbol = value == 2;
                self.font_entry.1.code_page = charset_code_page(value);
            }
            "cpg" => self.font_entry.1.code_page = u16::try_from(value).ok(),
            _ => {}
        }
    }

    fn font_table_text(&mut self, text: &str) {
        for character in text.chars() {
            if character == ';' {
                self.end_font();
            } else {
                self.font_entry.1.name.push(character);
            }
        }
    }

    fn end_font(&mut self) {
        let (number, mut font) = std::mem::take(&mut self.font_entry);
        if let Some(number) = number {
            font.name = font.name.trim().to_string();
            if font.name.eq_ignore_ascii_case("symbol") || font.name.starts_with("Wingdings") {
                font.symbol = true;
            }
            self.fonts.insert(number, font);
        }
    }

    fn end_color(&mut self) {
        let (red, green, blue) = std::mem::take(&mut self.color_entry);
        let color = match (red, green, blue) {
            (None, None, None) => None,
            _ => Some(Color {
                red: red.unwrap_or(0),
                green: green.unwrap_or(0),
                blue: blue.unwrap_or(0),
            }),
        };
        self.colors.push(color);
    }

    fn color(&self, index: i32) -> Option<Color> {
        usize::try_from(index)
            .ok()
            .and_then(|index| self.colors.get(index))
            .copied()
            .flatten()
    }

    fn list_word(&mut self, name: &str, parameter: Option<i32>) -> bool {
        let value = parameter.unwrap_or(0);
        let destination = self.group().destination;
        match name {
            "listid" if destination == Destination::ListTable => self.list_entry.id = value,
            "listid" if destination == Destination::ListOverrideTable => {
                self.override_entry.list = value;
            }
            "ls" if destination == Destination::ListOverrideTable => {
                self.override_entry.number = value;
            }
            "levelnfc" => self.level_entry.format = value,
            "levelnfcn" => {}
            "levelstartat" => {
                if self.group().on_close == Some(OnClose::ListLevel)
                    || self
                        .groups
                        .iter()
                        .any(|g| g.on_close == Some(OnClose::ListLevel))
                {
                    self.level_entry.start = Some(value);
                } else {
                    let level = self.override_entry.starts.len();
                    self.override_entry.starts.push((level, value));
                }
            }
            "listoverridestartat" | "lfolevel" => {}
            "ls" => {
                let para = &mut self.group_mut().para;
                para.list = Some(value);
            }
            "ilvl" => self.group_mut().para.level = value.clamp(0, 8) as u8,
            // Old-style list paragraphs.
            "pnlvl" => self.old_list.level = value.clamp(1, 9) as u8 - 1,
            "pnlvlblt" => self.old_list.bullet = true,
            "pnlvlbody" | "pnlvlcont" => {}
            "pndec" => self.old_list.kind = Some(NumberKind::Decimal),
            "pnucltr" => self.old_list.kind = Some(NumberKind::UpperLetter),
            "pnlcltr" => self.old_list.kind = Some(NumberKind::LowerLetter),
            "pnucrm" => self.old_list.kind = Some(NumberKind::UpperRoman),
            "pnlcrm" => self.old_list.kind = Some(NumberKind::LowerRoman),
            "pnstart" => self.old_list.start = Some(value),
            "pnindent" => self.old_list.indent = Some(twips(value)),
            "pnf" => self.old_list.font = Some(value),
            _ => return false,
        }
        true
    }

    fn character_word(&mut self, name: &str, value: i32, on: bool, parameter: Option<i32>) -> bool {
        let destination = self.group().destination;
        if name == "f" && destination == Destination::Shape {
            return true;
        }
        let color = self.color(value);
        let font_name = (name == "f")
            .then(|| self.fonts.get(&value).map(|font| font.name.clone()))
            .flatten();
        let font_id = font_name
            .filter(|font| !font.is_empty())
            .map(|font| self.document.intern_string(&font));
        let character_style = (name == "cs")
            .then(|| self.character_styles.get(&value).copied())
            .flatten();
        let language = (name == "lang").then(|| lcid_tag(value)).flatten();
        let language_id = language.map(|tag| self.document.intern_string(tag));
        let group = self.group_mut();
        let chars = &mut group.chars;
        let props = &mut chars.props;
        match name {
            "plain" => {
                let link = chars.link;
                *chars = Chars::default();
                chars.link = link;
            }
            "b" => props.bold = Some(on),
            "i" => props.italic = Some(on),
            "ul" | "uld" | "uldash" | "uldashd" | "uldashdd" | "uldb" | "ulth" | "ulw"
            | "ulwave" | "ulhwave" | "ululdbwave" | "ulthd" | "ulthdash" | "ulldash"
            | "ulthldash" | "ulthdashd" | "ulthdashdd" => props.underline = Some(on),
            "ulnone" => props.underline = Some(false),
            "strike" | "striked" => props.strike = Some(on),
            "f" => {
                chars.font = Some(value);
                props.font = font_id;
            }
            "fs" => props.size = (value > 0).then(|| value as f32 / 2.0),
            "cf" => props.color = color,
            "cb" | "highlight" | "chcbpat" => props.highlight = color,
            "super" => props.baseline = Some(Baseline::Superscript),
            "sub" => props.baseline = Some(Baseline::Subscript),
            "nosupersub" => props.baseline = None,
            "up" => props.shift = Some(parameter.unwrap_or(6) as f32 / 2.0),
            "dn" => props.shift = Some(-(parameter.unwrap_or(6) as f32) / 2.0),
            "caps" => props.caps = on.then_some(Caps::All),
            "scaps" => props.caps = on.then_some(Caps::Small),
            "v" => props.hidden = Some(on),
            "lang" => props.language = language_id.or(props.language),
            "cs" => chars.style = character_style,
            "revised" => chars.inserted = on,
            "deleted" => chars.deleted = on,
            "revauth" => chars.author = Some(value),
            "revdttm" => chars.date = Some(value),
            "revauthdel" => chars.deletion_author = Some(value),
            "revdttmdel" => chars.deletion_date = Some(value),
            "chbrdr" => group.border_target = BorderTarget::Character,
            "expndtw" => props.letter_spacing = (value != 0).then(|| twips(value)),
            "expnd" => props.letter_spacing = (value != 0).then(|| value as f32 / 4.0),
            "charscalex" => {
                props.width_scale = (value > 0 && value != 100).then_some(value as f32);
            }
            _ => return false,
        }
        true
    }

    fn paragraph_word(&mut self, name: &str, value: i32, on: bool) -> bool {
        let style = (name == "s")
            .then(|| self.paragraph_styles.get(&value).copied())
            .flatten();
        let color = self.color(value);
        let group = self.group_mut();
        let para = &mut group.para;
        let props = &mut para.props;
        match name {
            "pard" => {
                *para = Para::default();
                group.border_target = BorderTarget::None;
            }
            "s" => para.style = style,
            "ql" => props.alignment = Some(Alignment::Left),
            "qc" => props.alignment = Some(Alignment::Center),
            "qr" => props.alignment = Some(Alignment::Right),
            "qj" | "qd" => props.alignment = Some(Alignment::Justify),
            "li" | "lin" => props.left_indent = Some(twips(value)),
            "ri" | "rin" => props.right_indent = Some(twips(value)),
            "fi" => para.first_relative = Some(twips(value)),
            "sb" => props.space_before = Some(twips(value)),
            "sa" => props.space_after = Some(twips(value)),
            "sl" => para.line = Some(value),
            "slmult" => para.line_multiple = on,
            "keepn" => props.keep_with_next = Some(on),
            "keep" => props.keep_lines_together = Some(on),
            "widctlpar" => props.widow_control = Some(true),
            "nowidctlpar" => props.widow_control = Some(false),
            "outlinelevel" => props.outline_level = Some(value.clamp(0, 9) as u8),
            "pagebb" => props.page_break_before = Some(on),
            "contextualspace" => props.contextual_spacing = Some(on),
            "cbpat" => props.background = color,
            "intbl" => {
                para.in_table = true;
                if para.depth == 0 {
                    para.depth = 1;
                }
            }
            "itap" => {
                para.depth = value.max(0) as usize;
                para.in_table = value > 0;
            }
            "tqr" => para.tab_alignment = Some(TabAlignment::Right),
            "tqc" => para.tab_alignment = Some(TabAlignment::Center),
            "tqdec" => para.tab_alignment = Some(TabAlignment::Decimal),
            "tldot" => para.tab_leader = Some('.'),
            "tlmdot" => para.tab_leader = Some('\u{b7}'),
            "tlhyph" => para.tab_leader = Some('-'),
            "tlul" | "tlth" => para.tab_leader = Some('_'),
            "tleq" => para.tab_leader = Some('='),
            "tx" => {
                let stop = TabStop {
                    position: twips(value),
                    alignment: para.tab_alignment.take().unwrap_or(TabAlignment::Left),
                    leader: para.tab_leader.take(),
                };
                para.tabs.push(stop);
            }
            "tb" => {
                para.tab_alignment = None;
                para.tab_leader = None;
            }
            "brdrt" => group.border_target = BorderTarget::Paragraph(Side::Top),
            "brdrb" => group.border_target = BorderTarget::Paragraph(Side::Bottom),
            "brdrl" => group.border_target = BorderTarget::Paragraph(Side::Left),
            "brdrr" => group.border_target = BorderTarget::Paragraph(Side::Right),
            "box" => group.border_target = BorderTarget::Paragraph(Side::All),
            _ => return self.border_word(name, value),
        }
        true
    }

    /// Line words (`\brdrs`, `\brdrwN`, `\brdrcfN`, `\brdrnone`) for the
    /// border named last.
    fn border_word(&mut self, name: &str, value: i32) -> bool {
        let is_style = matches!(
            name,
            "brdrs"
                | "brdrth"
                | "brdrsh"
                | "brdrdb"
                | "brdrdot"
                | "brdrdash"
                | "brdrhair"
                | "brdrdashsm"
                | "brdrdashd"
                | "brdrdashdd"
                | "brdrtriple"
                | "brdrthtnsg"
                | "brdrtnthsg"
                | "brdrwavy"
                | "brdrinset"
                | "brdroutset"
                | "brdremboss"
                | "brdrengrave"
                | "brdrframe"
                | "brdrdashdotstr"
                | "brdrsingle"
        );
        let is_none = matches!(name, "brdrnone" | "brdrnil" | "brdrtbl");
        let width = (name == "brdrw").then(|| twips(value));
        let color = (name == "brdrcf").then(|| self.color(value)).flatten();
        if !is_style && !is_none && width.is_none() && name != "brdrcf" {
            return false;
        }
        let target = self.group().border_target;
        let update = |line: &mut Option<Border>| {
            if is_none {
                *line = None;
                return;
            }
            let mut border = line.unwrap_or(Border {
                width: 0.5,
                color: None,
            });
            if let Some(width) = width {
                border.width = width;
            }
            if name == "brdrcf" {
                border.color = color;
            }
            *line = Some(border);
        };
        match target {
            BorderTarget::None | BorderTarget::Character => {}
            BorderTarget::Paragraph(side) => {
                let para = &mut self.group_mut().para;
                if is_none {
                    match side {
                        Side::All => para.border_sides = [false; 4],
                        side => para.border_sides[side_index(side)] = false,
                    }
                    return true;
                }
                match side {
                    Side::All => para.border_sides = [true; 4],
                    side => para.border_sides[side_index(side)] = true,
                }
                update(&mut para.border);
            }
            BorderTarget::Cell(side) => {
                let borders = &mut self.cell_def.borders;
                let slot = match side {
                    Side::Top => &mut borders.top,
                    Side::Bottom => &mut borders.bottom,
                    Side::Left => &mut borders.left,
                    Side::Right | Side::All => &mut borders.right,
                };
                let mut line = slot.flatten();
                update(&mut line);
                *slot = Some(line);
            }
            BorderTarget::Table(side) => {
                let borders = &mut self.row_def().borders;
                let slot = match side {
                    TableSide::Top => &mut borders.top,
                    TableSide::Bottom => &mut borders.bottom,
                    TableSide::Left => &mut borders.left,
                    TableSide::Right => &mut borders.right,
                    TableSide::InsideHorizontal => &mut borders.inside_horizontal,
                    TableSide::InsideVertical => &mut borders.inside_vertical,
                };
                update(slot);
            }
        }
        true
    }

    fn table_word(&mut self, name: &str, value: i32) -> bool {
        let color = self.color(value);
        match name {
            "trowd" => {
                *self.row_def() = RowDef::default();
                self.cell_def = CellDef::default();
            }
            "trleft" => self.row_def().left = value,
            "trgaph" => self.row_def().gap = Some(value),
            "trrh" => self.row_def().height = (value != 0).then_some(value),
            "trhdr" => self.row_def().header = true,
            "trql" => self.row_def().alignment = Some(Alignment::Left),
            "trqc" => self.row_def().alignment = Some(Alignment::Center),
            "trqr" => self.row_def().alignment = Some(Alignment::Right),
            "trpaddl" => self.row_def().padding[2] = Some(value),
            "trpaddr" => self.row_def().padding[3] = Some(value),
            "trpaddt" => self.row_def().padding[0] = Some(value),
            "trpaddb" => self.row_def().padding[1] = Some(value),
            "trbrdrt" => self.group_mut().border_target = BorderTarget::Table(TableSide::Top),
            "trbrdrb" => self.group_mut().border_target = BorderTarget::Table(TableSide::Bottom),
            "trbrdrl" => self.group_mut().border_target = BorderTarget::Table(TableSide::Left),
            "trbrdrr" => self.group_mut().border_target = BorderTarget::Table(TableSide::Right),
            "trbrdrh" => {
                self.group_mut().border_target = BorderTarget::Table(TableSide::InsideHorizontal);
            }
            "trbrdrv" => {
                self.group_mut().border_target = BorderTarget::Table(TableSide::InsideVertical);
            }
            "clbrdrt" => self.group_mut().border_target = BorderTarget::Cell(Side::Top),
            "clbrdrb" => self.group_mut().border_target = BorderTarget::Cell(Side::Bottom),
            "clbrdrl" => self.group_mut().border_target = BorderTarget::Cell(Side::Left),
            "clbrdrr" => self.group_mut().border_target = BorderTarget::Cell(Side::Right),
            "clcbpat" => self.cell_def.background = color,
            "clvertalt" => self.cell_def.vertical_alignment = Some(VerticalAlignment::Top),
            "clvertalc" => self.cell_def.vertical_alignment = Some(VerticalAlignment::Center),
            "clvertalb" => self.cell_def.vertical_alignment = Some(VerticalAlignment::Bottom),
            // Word writes left padding as `\clpadt` and top as `\clpadl`.
            "clpadt" => self.cell_def.margins[2] = Some(twips(value)),
            "clpadl" => self.cell_def.margins[0] = Some(twips(value)),
            "clpadb" => self.cell_def.margins[1] = Some(twips(value)),
            "clpadr" => self.cell_def.margins[3] = Some(twips(value)),
            "clmgf" => self.cell_def.merge_first = true,
            "clmrg" => self.cell_def.merge_continue = true,
            "clvmgf" => self.cell_def.vertical_first = true,
            "clvmrg" => self.cell_def.vertical_continue = true,
            "cellx" => {
                let mut cell = std::mem::take(&mut self.cell_def);
                cell.right = value;
                self.row_def().cells.push(cell);
            }
            "cell" => self.end_cell(false),
            "nestcell" => self.end_cell(true),
            "row" => self.end_row(false),
            "nestrow" => self.end_row(true),
            "lastrow" | "ltrrow" | "rtlrow" | "trautofit" | "trpaddfl" | "trpaddfr"
            | "trpaddft" | "trpaddfb" | "clpadfl" | "clpadfr" | "clpadft" | "clpadfb"
            | "clftsWidth" | "clwWidth" | "trftsWidth" | "trwWidth" | "tblind" | "tblindtype"
            | "clshdrawnil" | "gaph" | "taflags" => {}
            _ => return false,
        }
        true
    }

    fn section_word(&mut self, name: &str, value: i32) -> bool {
        let points = twips(value);
        match name {
            "paperw" => {
                self.document_page.width = points;
                self.section.page.width = points;
            }
            "paperh" => {
                self.document_page.height = points;
                self.section.page.height = points;
            }
            "margl" => {
                self.document_page.margin_left = points;
                self.section.page.margin_left = points;
            }
            "margr" => {
                self.document_page.margin_right = points;
                self.section.page.margin_right = points;
            }
            "margt" => {
                self.document_page.margin_top = points;
                self.section.page.margin_top = points;
            }
            "margb" => {
                self.document_page.margin_bottom = points;
                self.section.page.margin_bottom = points;
            }
            "pgwsxn" => self.section.page.width = points,
            "pghsxn" => self.section.page.height = points,
            "marglsxn" => self.section.page.margin_left = points,
            "margrsxn" => self.section.page.margin_right = points,
            "margtsxn" => self.section.page.margin_top = points,
            "margbsxn" => self.section.page.margin_bottom = points,
            "headery" => self.section.page.header_distance = points,
            "footery" => self.section.page.footer_distance = points,
            "pgnstarts" => {
                if self.section.page.page_number_start.is_some() {
                    self.section.page.page_number_start = Some(value.max(0) as u32);
                }
            }
            "pgnrestart" => {
                self.section.page.page_number_start = Some(1);
            }
            "cols" => self.section.columns = value.clamp(1, 45) as u16,
            "colsx" => self.section.column_gap = Some(points),
            "colw" => self.section.pending_column = Some(points),
            "colsr" => {
                let width = self.section.pending_column.take().unwrap_or(0.0);
                self.section.column_widths.push((width, points));
            }
            "colno" => {
                if let Some(width) = self.section.pending_column.take() {
                    self.section.column_widths.push((width, 0.0));
                }
            }
            "sbknone" => self.section.start = SectionStart::Continuous,
            "sbkpage" | "sbkodd" | "sbkeven" | "sbkcol" => {
                self.section.start = SectionStart::NewPage;
            }
            "titlepg" => self.section.title_page = true,
            "facingp" => self.facing_pages = true,
            "sectd" => {
                let headers = std::mem::take(&mut self.section.headers);
                let footers = std::mem::take(&mut self.section.footers);
                self.section = SectionState {
                    page: self.document_page.clone(),
                    columns: 1,
                    headers,
                    footers,
                    ..SectionState::default()
                };
            }
            "sect" => self.end_section(),
            "ansi" => self.default_code_page = 1252,
            "mac" => self.default_code_page = 10000,
            "pc" => self.default_code_page = 437,
            "pca" => self.default_code_page = 850,
            "ansicpg" => self.default_code_page = u16::try_from(value).unwrap_or(1252),
            "deff" => self.default_font = Some(value),
            _ => return false,
        }
        true
    }

    fn special_word(&mut self, name: &str, value: i32) {
        match name {
            "par" => self.end_paragraph(),
            "line" | "lbr" => self.inline(Inline::LineBreak),
            "tab" => self.inline(Inline::Tab),
            "page" => self.inline(Inline::PageBreak),
            "column" => self.inline(Inline::ColumnBreak),
            "chpgn" => self.inline(Inline::PageNumber),
            "chftn" => self.after_note_mark = self.flows.len() > 1,
            "u" => self.unicode(value),
            "uc" => {
                let group = self.group_mut();
                group.unicode_skip = value.max(0) as usize;
            }
            "emdash" => self.character('\u{2014}'),
            "endash" => self.character('\u{2013}'),
            "emspace" => self.character('\u{2003}'),
            "enspace" => self.character('\u{2002}'),
            "qmspace" => self.character('\u{2005}'),
            "bullet" => self.character('\u{2022}'),
            "lquote" => self.character('\u{2018}'),
            "rquote" => self.character('\u{2019}'),
            "ldblquote" => self.character('\u{201C}'),
            "rdblquote" => self.character('\u{201D}'),
            "zwj" => self.character('\u{200D}'),
            "zwnj" => self.character('\u{200C}'),
            "zwbo" => self.character('\u{200B}'),
            "ltrmark" => self.character('\u{200E}'),
            "rtlmark" => self.character('\u{200F}'),
            _ => {}
        }
    }

    fn symbol(&mut self, symbol: u8) {
        if symbol == b'*' {
            // `\*` marks a destination a reader may skip: skip it unless the
            // word that follows is one this reader knows.
            self.starred_destination();
            return;
        }
        if self.skip_one() {
            return;
        }
        match symbol {
            b'\\' | b'{' | b'}' => self.bytes(&[symbol]),
            b'~' => self.character('\u{A0}'),
            b'_' => self.character('\u{2011}'),
            b'-' => {}
            b'\n' => self.end_paragraph(),
            b'\t' => self.inline(Inline::Tab),
            _ => {}
        }
    }

    /// After `\*`: reads the destination word and keeps the group only if
    /// the word is known.
    fn starred_destination(&mut self) {
        let known = [
            "picprop",
            "listtable",
            "listoverridetable",
            "revtbl",
            "fldinst",
            "shppict",
            "shpinst",
            "shptxt",
            "atnid",
            "atnauthor",
            "annotation",
            "atnparent",
            "atndate",
            "atnref",
            "atrfstart",
            "atrfend",
            "ud",
            "pn",
            "background",
            "nesttableprops",
            "footnote",
            "headerl",
            "headerr",
            "headerf",
            "footerl",
            "footerr",
            "footerf",
            "cs",
            "ts",
            "ds",
            "levelmarker",
            "listname",
            "leveltemplateid",
        ];
        let Some(token) = self.lexer.next() else {
            return;
        };
        match token {
            Token::Word { name, parameter } if known.contains(&name) => {
                self.word(name, parameter);
            }
            Token::Word { .. } => self.group_mut().destination = Destination::Skip,
            other => {
                // Not a word after all: treat the token normally.
                self.group_mut().destination = Destination::Skip;
                if other == Token::Close {
                    self.close();
                }
            }
        }
    }

    // ----- pictures -----

    fn picture_word(&mut self, name: &str, value: i32) {
        let Some(picture) = self.pictures.last_mut() else {
            return;
        };
        match name {
            "pngblip" => picture.kind = Some("png"),
            "jpegblip" => picture.kind = Some("jpg"),
            "emfblip" => picture.kind = Some("emf"),
            "wmetafile" => picture.kind = Some("wmf"),
            "dibitmap" => picture.kind = Some("bmp"),
            "macpict" | "pmmetafile" | "wbitmap" => picture.kind = None,
            "picw" => picture.width = Some(value),
            "pich" => picture.height = Some(value),
            "picwgoal" => picture.goal_width = Some(value),
            "pichgoal" => picture.goal_height = Some(value),
            "picscalex" => picture.scale_x = Some(value),
            "picscaley" => picture.scale_y = Some(value),
            "piccropl" => picture.crop[0] = value,
            "piccropt" => picture.crop[1] = value,
            "piccropr" => picture.crop[2] = value,
            "piccropb" => picture.crop[3] = value,
            _ => {}
        }
    }

    fn picture_hex(&mut self, text: &str) {
        let Some(picture) = self.pictures.last_mut() else {
            return;
        };
        for byte in text.bytes() {
            let Some(nibble) = (byte as char).to_digit(16) else {
                continue;
            };
            let nibble = nibble as u8;
            match picture.pending_nibble.take() {
                Some(high) => picture.data.push(high << 4 | nibble),
                None => picture.pending_nibble = Some(nibble),
            }
        }
    }

    fn end_picture(&mut self, group: &Group) {
        let Some(picture) = self.pictures.pop() else {
            return;
        };
        let Some(kind) = picture.kind else {
            return;
        };
        if picture.data.is_empty() {
            return;
        }
        let bytes = if kind == "bmp" {
            match bitmap_file(&picture.data) {
                Some(bytes) => bytes,
                None => return,
            }
        } else {
            picture.data
        };
        let scale_x = picture.scale_x.unwrap_or(100).max(1) as f32 / 100.0;
        let scale_y = picture.scale_y.unwrap_or(100).max(1) as f32 / 100.0;
        let natural = |pixels: Option<i32>| -> f32 {
            let value = pixels.unwrap_or(0).max(0) as f32;
            if matches!(kind, "emf" | "wmf") {
                // Hundredths of a millimetre.
                value / 100.0 / 25.4 * 72.0
            } else {
                value * 0.75
            }
        };
        let width = picture
            .goal_width
            .map(twips)
            .unwrap_or_else(|| natural(picture.width))
            * scale_x;
        let height = picture
            .goal_height
            .map(twips)
            .unwrap_or_else(|| natural(picture.height))
            * scale_y;
        // A picture repeated in the file is one picture.
        let media_index = match self
            .document
            .media
            .iter()
            .position(|media| media.bytes == bytes)
        {
            Some(index) => index,
            None => {
                let index = self.document.media.len();
                self.document.media.push(Media {
                    name: format!("image{}.{}", index + 1, kind),
                    bytes,
                });
                index
            }
        };
        let crop = if picture.crop.iter().any(|value| *value != 0) {
            let full_width = picture.goal_width.unwrap_or(1).max(1) as f32;
            let full_height = picture.goal_height.unwrap_or(1).max(1) as f32;
            Some([
                picture.crop[0] as f32 / full_width,
                picture.crop[1] as f32 / full_height,
                picture.crop[2] as f32 / full_width,
                picture.crop[3] as f32 / full_height,
            ])
        } else {
            None
        };
        if group.in_shape_value
            && let Some(shape) = self.shapes.last_mut()
        {
            shape.picture = Some((media_index, width, height));
            return;
        }
        if self.in_background {
            return;
        }
        let image = self.document.push_image(InlineImage {
            media: media_index,
            width,
            height,
            description: picture.description,
            placement: Placement::Inline,
            crop,
        });
        self.inline(Inline::Image(image));
    }

    // ----- shapes -----

    fn shape_word(&mut self, name: &str, value: i32) {
        if name == "pnlvl" || name.starts_with("pn") {
            self.list_word(name, Some(value));
            return;
        }
        let Some(shape) = self.shapes.last_mut() else {
            return;
        };
        match name {
            "shpleft" => shape.left = value,
            "shptop" => shape.top = value,
            "shpright" => shape.right = value,
            "shpbottom" => shape.bottom = value,
            "shpbxmargin" | "shpbxcolumn" => shape.from_margin_x = true,
            "shpbxpage" => shape.from_margin_x = false,
            "shpbymargin" => shape.from_margin_y = true,
            "shpbypage" => shape.from_margin_y = false,
            "shpbypara" => shape.follows_text = true,
            "shpwr" => shape.wrap = Some(value),
            "shpfblwtxt" => shape.behind = value != 0,
            _ => {}
        }
    }

    fn end_shape(&mut self) {
        let Some(shape) = self.shapes.pop() else {
            return;
        };
        let property = |name: &str| -> Option<&str> {
            shape
                .properties
                .iter()
                .rev()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
        };
        let number = |name: &str| -> Option<i64> { property(name)?.trim().parse().ok() };
        let color_of = |value: i64| Color {
            red: (value & 0xFF) as u8,
            green: ((value >> 8) & 0xFF) as u8,
            blue: ((value >> 16) & 0xFF) as u8,
        };
        if self.in_background {
            if number("fFilled") != Some(0)
                && let Some(fill) = number("fillColor")
            {
                self.document.page_color = Some(color_of(fill));
            }
            return;
        }
        let width = twips(shape.right - shape.left).max(0.0);
        let height = twips(shape.bottom - shape.top).max(0.0);
        let mut x = twips(shape.left);
        let mut y = twips(shape.top);
        if shape.from_margin_x {
            x += self.section.page.margin_left;
        }
        if shape.from_margin_y && !shape.follows_text {
            y += self.section.page.margin_top;
        }
        let wrap = match shape.wrap {
            Some(1) => TextWrap::TopAndBottom,
            Some(3) => TextWrap::None,
            _ => TextWrap::Around,
        };
        let filled = number("fFilled") != Some(0);
        let lined = number("fLine") != Some(0);
        let content = if let Some((media, _, _)) = shape.picture {
            FloatingContent::Image(media)
        } else {
            let has_fill = number("fillColor").is_some() && filled;
            let has_line = number("lineColor").is_some() && lined;
            let blocks = match shape.blocks {
                Some(blocks) => blocks,
                // A shape without text is kept when it draws something (a
                // rule, a panel).
                None if has_fill || has_line => Vec::new(),
                None => return,
            };
            let fill = if filled {
                Some(number("fillColor").map_or(
                    Color {
                        red: 255,
                        green: 255,
                        blue: 255,
                    },
                    color_of,
                ))
            } else {
                None
            };
            let line = lined.then(|| Border {
                width: number("lineWidth").map_or(0.75, |emu| emu as f32 / 12700.0),
                color: Some(number("lineColor").map_or(
                    Color {
                        red: 0,
                        green: 0,
                        blue: 0,
                    },
                    color_of,
                )),
            });
            let geometry = match number("shapeType") {
                Some(0) => match custom_path(
                    property("pVerticies"),
                    property("pSegmentInfo"),
                    number("geoRight"),
                    number("geoBottom"),
                    width,
                    height,
                ) {
                    Some(path) => {
                        self.document.paths.push(path);
                        ShapeGeometry::Path((self.document.paths.len() - 1) as u32)
                    }
                    None => ShapeGeometry::Rectangle,
                },
                Some(2) => ShapeGeometry::RoundedRectangle,
                Some(3) => ShapeGeometry::Ellipse,
                Some(4) => ShapeGeometry::Diamond,
                Some(5) => ShapeGeometry::Triangle,
                Some(6) => ShapeGeometry::RightTriangle,
                Some(20) => ShapeGeometry::Line,
                _ => ShapeGeometry::Rectangle,
            };
            FloatingContent::TextBox {
                blocks,
                fill,
                line,
                geometry,
                flip: (false, false),
                ends: (None, None),
            }
        };
        self.document.floating.push(FloatingObject {
            page: 0,
            x,
            y,
            width,
            height,
            content,
            follows_text: true,
            wrap,
            repeats: None,
            behind: shape.behind,
        });
        let id = (self.document.floating.len() - 1) as Id;
        self.inline(Inline::Anchor(id));
    }

    // ----- closing actions -----

    fn on_close(&mut self, action: OnClose, group: &Group) {
        match action {
            OnClose::StyleEntry => self.end_style(group),
            OnClose::List => {
                let list = std::mem::take(&mut self.list_entry);
                self.lists.push(list);
            }
            OnClose::ListLevel => {
                let mut level = std::mem::take(&mut self.level_entry);
                level.left = group.para.props.left_indent;
                level.first_relative = group.para.first_relative;
                level.font = group.chars.font;
                self.list_entry.levels.push(level);
            }
            OnClose::LevelText => {
                let text = self.collected(group);
                let text = text.trim_end_matches(';');
                self.level_entry.text = text.encode_utf16().collect();
            }
            OnClose::LevelNumbers => {
                let text = self.collected(group);
                self.level_entry.numbers = text.bytes().filter(|byte| *byte < 10).collect();
            }
            OnClose::ListOverride => {
                let entry = std::mem::take(&mut self.override_entry);
                self.overrides.push(entry);
            }
            OnClose::Field => {
                self.fields.pop();
            }
            OnClose::FieldInstruction => {
                let text = self.collected(group);
                if let Some(field) = self.fields.last_mut() {
                    field.instruction.push_str(&text);
                }
            }
            OnClose::Picture => self.end_picture(group),
            OnClose::Footnote => {
                let blocks = self.pop_flow();
                self.document.footnotes.push(Note { blocks });
                let note = self.document.footnotes.len() - 1;
                self.inline(Inline::Footnote(note));
            }
            OnClose::Header(footer, kind) => {
                let blocks = self.pop_flow();
                let variants = if footer {
                    &mut self.section.footers
                } else {
                    &mut self.section.headers
                };
                match kind {
                    PartKind::Both | PartKind::Right => variants.default = Some(blocks),
                    PartKind::Left => variants.even = Some(blocks),
                    PartKind::First => variants.first = Some(blocks),
                }
            }
            OnClose::Annotation => self.end_annotation(),
            OnClose::AnnotationAuthor => {
                let text = self.collected(group);
                self.pending_author = Some(text.trim().to_string());
            }
            OnClose::AnnotationInitials => {
                let text = self.collected(group);
                self.pending_initials = Some(text.trim().to_string());
            }
            OnClose::AnnotationDate => {
                let text = self.collected(group);
                self.annotation.date =
                    text.trim().parse::<i64>().ok().and_then(|value| {
                        packed_date(i32::try_from(value).unwrap_or(value as i32))
                    });
            }
            OnClose::AnnotationParent => {
                let text = self.collected(group);
                self.annotation.parent = Some(text.trim().to_string());
            }
            OnClose::AnnotationReference => {
                let text = self.collected(group);
                self.annotation.reference = Some(text.trim().to_string());
            }
            OnClose::RangeStart => {
                let key = self.collected(group).trim().to_string();
                self.document.comments.push(Comment::default());
                let id = (self.document.comments.len() - 1) as Id;
                self.ranges.insert(key, id);
                self.inline(Inline::CommentStart(id));
            }
            OnClose::RangeEnd => {
                let key = self.collected(group).trim().to_string();
                if let Some(id) = self.ranges.get(&key).copied() {
                    self.inline(Inline::CommentEnd(id));
                }
            }
            OnClose::Shape => self.end_shape(),
            OnClose::ShapeProperty => {
                if let Some(shape) = self.shapes.last_mut() {
                    let name = std::mem::take(&mut shape.name);
                    let value = std::mem::take(&mut shape.value);
                    shape.properties.push((name, value));
                }
            }
            OnClose::ShapeName => {
                let text = self.collected(group);
                if let Some(shape) = self.shapes.last_mut() {
                    shape.name = text.trim().to_string();
                }
            }
            OnClose::ShapeValue => {
                let text = self.collected(group);
                if let Some(shape) = self.shapes.last_mut() {
                    shape.value = text.trim().to_string();
                }
            }
            OnClose::ShapeText => {
                let blocks = self.pop_flow();
                if let Some(shape) = self.shapes.last_mut() {
                    shape.blocks = Some(blocks);
                }
            }
            OnClose::OldList => self.end_old_list(),
            OnClose::OldListBefore => self.old_list.before = self.collected(group),
            OnClose::OldListAfter => self.old_list.after = self.collected(group),
            OnClose::Background => self.in_background = false,
            OnClose::ListText => self.list_label = Some(self.collected(group)),
            OnClose::PictureProperties => {
                if let Some(shape) = self.shapes.pop()
                    && let Some(picture) = self.pictures.last_mut()
                {
                    picture.description = shape
                        .properties
                        .iter()
                        .find(|(name, _)| name == "wzDescription")
                        .map(|(_, value)| value.clone())
                        .filter(|value| !value.is_empty());
                }
            }
        }
    }

    fn end_style(&mut self, group: &Group) {
        let name = self.collected(group);
        let entry = std::mem::take(&mut self.style_entry);
        if entry.skip {
            return;
        }
        let name = name.trim_end_matches(';').trim().to_string();
        let Some(number) = entry.number else {
            return;
        };
        let mut paragraph = group.para.props;
        if let Some(relative) = group.para.first_relative {
            paragraph.first_line_indent = Some(paragraph.left_indent.unwrap_or(0.0) + relative);
        }
        paragraph.line_spacing = line_spacing(group.para.line, group.para.line_multiple);
        // Word's built-in headings carry their outline level; writers that
        // leave it out still name them.
        if paragraph.outline_level.is_none()
            && let Some(level) = heading_number(&name)
        {
            paragraph.outline_level = Some(level - 1);
        }
        if entry.character {
            self.document.styles.character.push(CharacterStyle {
                name,
                parent: None,
                run: group.chars.props,
            });
            let id = self.document.styles.character.len() - 1;
            self.character_styles.insert(number, id);
            if let Some(parent) = entry.parent {
                self.style_parents.push((true, id, parent));
            }
        } else {
            self.document.styles.paragraph.push(ParagraphStyle {
                name,
                parent: None,
                paragraph,
                run: group.chars.props,
            });
            let id = self.document.styles.paragraph.len() - 1;
            self.paragraph_styles.insert(number, id);
            if number == 0 {
                self.document.styles.default_paragraph = Some(id);
            }
            if let Some(parent) = entry.parent {
                self.style_parents.push((false, id, parent));
            }
        }
    }

    fn end_annotation(&mut self) {
        let blocks = self.pop_flow();
        let mut texts = Vec::new();
        let mut scratch = Document::default();
        std::mem::swap(&mut scratch.text, &mut self.document.text);
        for block in &blocks {
            if let Block::Paragraph(paragraph) = block {
                texts.push(scratch.paragraph_text(paragraph));
            }
        }
        std::mem::swap(&mut scratch.text, &mut self.document.text);
        let text = texts.join("\n").trim().to_string();
        let annotation = std::mem::take(&mut self.annotation);
        let parent = annotation
            .parent
            .as_ref()
            .and_then(|key| self.ranges.get(key))
            .copied();
        let comment = Comment {
            author: annotation.author.unwrap_or_default(),
            initials: annotation.initials,
            date: annotation.date,
            text,
            reply_to: parent,
        };
        if parent.is_some() && annotation.reference.is_none() {
            // A reply shares its parent's range and has no marks of its own.
            self.document.comments.push(comment);
            return;
        }
        let ranged = annotation
            .reference
            .as_ref()
            .and_then(|key| self.ranges.get(key))
            .copied();
        match ranged {
            Some(id) => {
                if let Some(slot) = self.document.comments.get_mut(id as usize) {
                    *slot = comment;
                }
            }
            None => {
                self.document.comments.push(comment);
                let id = (self.document.comments.len() - 1) as Id;
                self.inline(Inline::CommentStart(id));
                self.inline(Inline::CommentEnd(id));
            }
        }
    }

    fn end_old_list(&mut self) {
        let mut state = std::mem::take(&mut self.old_list);
        let symbol = state
            .font
            .and_then(|number| self.fonts.get(&number))
            .is_some_and(|font| font.symbol);
        if symbol {
            // The label's bytes are in the symbol font, not the text's
            // code page.
            state.before = state
                .before
                .chars()
                .map(|character| match character as u32 {
                    code @ 0x20..=0xFF => char::from_u32(0xF000 + code).unwrap_or(character),
                    _ => character,
                })
                .collect();
        }
        let label = if state.bullet || state.kind.is_none() {
            let bullet = crate::io::docx::reader::symbol_bullet(&state.before);
            ListLabel::Text(if bullet.trim().is_empty() {
                "\u{2022}".to_string()
            } else {
                bullet.trim().to_string()
            })
        } else {
            ListLabel::Number(NumberFormat {
                kind: state.kind.unwrap_or(NumberKind::Decimal),
                pattern: format!("{}%1{}", state.before, state.after),
                tiered: false,
            })
        };
        let key = format!("{label:?}");
        let style = match self.old_lists.get(&key) {
            Some(style) => *style,
            None => {
                let indent = state.indent.unwrap_or(18.0);
                let levels = (0..9)
                    .map(|level| ListLevel {
                        label: label.clone(),
                        indent: indent * (level as f32 + 1.0) + 18.0,
                        label_indent: indent * level as f32 + 18.0,
                    })
                    .collect();
                self.document.styles.list.push(ListStyle {
                    name: format!("Old list {}", self.old_lists.len() + 1),
                    levels,
                });
                let style = self.document.styles.list.len() - 1;
                self.old_lists.insert(key, style);
                style
            }
        };
        let start = state.start.unwrap_or(1).max(0) as u32;
        // The `\pn` group sits in the paragraph's formatting: the paragraph
        // is the group around it.
        self.group_mut().para.old_list = Some((style, state.level, start));
    }

    // ----- sections and the end -----

    fn end_section(&mut self) {
        let has_runs = self.flows.first().is_some_and(|flow| !flow.runs.is_empty());
        if has_runs && self.flows.len() == 1 {
            self.end_paragraph();
        }
        if let Some(flow) = self.flows.first_mut() {
            finish_tables_deeper_than(flow, 0, &self.row_defs);
        }
        let blocks = self
            .flows
            .first_mut()
            .map(|flow| std::mem::take(&mut flow.blocks))
            .unwrap_or_default();
        let section = self.section_from_state(blocks);
        self.document.sections.push(section);
        // Properties carry on unless the next section resets them.
        self.section.headers = PageVariants::default();
        self.section.footers = PageVariants::default();
    }

    fn section_from_state(&self, blocks: Vec<Block>) -> Section {
        let state = &self.section;
        // The last column's width has no spacing after it.
        let mut column_widths = state.column_widths.clone();
        if let Some(width) = state.pending_column {
            column_widths.push((width, 0.0));
        }
        let mut headers = state.headers.clone();
        let mut footers = state.footers.clone();
        if !state.title_page {
            headers.first = None;
            footers.first = None;
        }
        if !self.facing_pages {
            headers.even = None;
            footers.even = None;
        }
        Section {
            page: state.page.clone(),
            columns: state.columns.max(1),
            column_gap: state.column_gap,
            column_widths: if column_widths.len() > 1 {
                column_widths
            } else {
                Vec::new()
            },
            start: state.start,
            headers,
            footers,
            blocks,
        }
    }

    fn finish(mut self) -> Document {
        // Close whatever the file left open.
        while self.groups.len() > 1 {
            self.close();
        }
        while self.flows.len() > 1 {
            let _ = self.pop_flow();
        }
        self.groups[0].destination = Destination::Body;
        self.end_story();
        let blocks = self
            .flows
            .first_mut()
            .map(|flow| std::mem::take(&mut flow.blocks))
            .unwrap_or_default();
        if !blocks.is_empty() || self.document.sections.is_empty() {
            let section = self.section_from_state(blocks);
            self.document.sections.push(section);
        }
        // A section without headers of its own shows the previous one's.
        for index in 1..self.document.sections.len() {
            let (before, after) = self.document.sections.split_at_mut(index);
            let previous = &before[index - 1];
            let current = &mut after[0];
            if current.headers.is_empty() {
                current.headers = previous.headers.clone();
            }
            if current.footers.is_empty() {
                current.footers = previous.footers.clone();
            }
        }
        for (character, id, parent) in std::mem::take(&mut self.style_parents) {
            if character {
                let parent = self.character_styles.get(&parent).copied();
                if let Some(style) = self.document.styles.character.get_mut(id) {
                    style.parent = parent.filter(|parent| *parent != id);
                }
            } else {
                let parent = self.paragraph_styles.get(&parent).copied();
                if let Some(style) = self.document.styles.paragraph.get_mut(id) {
                    style.parent = parent.filter(|parent| *parent != id);
                }
            }
        }
        // Comments whose range never got its text keep a placeholder author.
        for comment in &mut self.document.comments {
            if comment.author.is_empty() {
                comment.author = "Unknown".to_string();
            }
        }
        self.document
    }
}

impl Group {
    fn unicode_skip_reset(&mut self) {
        self.unicode_skip_pending = 0;
    }
}

// ----- helpers -----

/// Opens table builders down to `depth`.
fn ensure_tables(flow: &mut Flow, depth: usize) {
    while flow.tables.len() < depth {
        flow.tables.push(TableBuilder::default());
    }
}

/// Finishes the tables nested deeper than `depth`, each into the cell of
/// the table around it (or the flow, at the top).
fn finish_tables_deeper_than(flow: &mut Flow, depth: usize, row_defs: &[RowDef]) {
    while flow.tables.len() > depth {
        let Some(mut builder) = flow.tables.pop() else {
            break;
        };
        // A row left without `\row` still counts.
        if !builder.cells.is_empty() || !builder.cell_blocks.is_empty() {
            let mut cells = std::mem::take(&mut builder.cells);
            if !builder.cell_blocks.is_empty() {
                cells.push(std::mem::take(&mut builder.cell_blocks));
            }
            let definition = row_defs.get(flow.tables.len()).cloned().unwrap_or_default();
            builder.rows.push((definition, cells));
        }
        let Some(table) = build_table(builder) else {
            continue;
        };
        let block = Block::Table(Box::new(table));
        match flow.tables.last_mut() {
            Some(parent) => parent.cell_blocks.push(block),
            None => flow.blocks.push(block),
        }
    }
}

/// A model table from rows of cells and their definitions: the grid is
/// every cell edge any row names.
fn build_table(builder: TableBuilder) -> Option<Table> {
    if builder.rows.is_empty() {
        return None;
    }
    // Each row's cell edges, from its definition (or even widths when a row
    // has more cells than edges).
    let mut row_edges: Vec<Vec<i32>> = Vec::new();
    for (definition, cells) in &builder.rows {
        let left = definition.left;
        let mut edges = vec![left];
        let mut last = left;
        for cell in &definition.cells {
            let right = cell.right.max(last + 1);
            edges.push(right);
            last = right;
        }
        while edges.len() < cells.len() + 1 {
            last += 1440;
            edges.push(last);
        }
        row_edges.push(edges);
    }
    let mut grid: Vec<i32> = row_edges.iter().flatten().copied().collect();
    grid.sort_unstable();
    // Edges a few twips apart are the same edge.
    let mut merged: Vec<i32> = Vec::new();
    for edge in grid {
        if merged.last().is_none_or(|last| edge - last > 10) {
            merged.push(edge);
        }
    }
    let grid = merged;
    let column_at = |edge: i32| -> usize {
        grid.iter()
            .enumerate()
            .min_by_key(|(_, at)| (**at - edge).abs())
            .map_or(0, |(index, _)| index)
    };
    let columns: Vec<f32> = grid
        .windows(2)
        .map(|pair| twips(pair[1] - pair[0]))
        .collect();
    let column_count = columns.len().max(1);

    let first = &builder.rows[0].0;
    let mut table = Table {
        rows: Vec::new(),
        header_rows: 0,
        columns,
        borders: None,
        cell_margins: None,
        alignment: first.alignment,
        indent: (first.left != 0).then(|| twips(first.left)),
    };
    let borders = first.borders;
    if borders != TableBorders::default() {
        table.borders = Some(borders);
    }
    let padding = first.padding;
    let gap = first.gap.map(twips);
    if padding.iter().any(Option::is_some) || gap.is_some() {
        table.cell_margins = Some(CellMargins {
            top: padding[0].map_or(0.0, twips),
            bottom: padding[1].map_or(0.0, twips),
            left: padding[2].map_or(gap.unwrap_or(5.4), twips),
            right: padding[3].map_or(gap.unwrap_or(5.4), twips),
        });
    }
    let mut counting_headers = true;
    for ((definition, cells), edges) in builder.rows.into_iter().zip(row_edges) {
        if counting_headers && definition.header {
            table.header_rows += 1;
        } else {
            counting_headers = false;
        }
        let mut row = Row {
            cells: vec![Cell::default(); column_count],
            height: definition.height.map(|height| twips(height.abs())),
        };
        for slot in &mut row.cells {
            slot.merge = Merge::Left;
            slot.column_span = 0;
        }
        let mut cells = cells.into_iter();
        let mut previous_origin: Option<usize> = None;
        for (index, cell_def) in definition
            .cells
            .iter()
            .cloned()
            .chain(std::iter::repeat_with(CellDef::default))
            .enumerate()
        {
            if index + 1 >= edges.len() {
                break;
            }
            let blocks = cells.next().unwrap_or_default();
            let start = column_at(edges[index]);
            let end = column_at(edges[index + 1]).max(start + 1).min(column_count);
            if start >= column_count {
                break;
            }
            if cell_def.merge_continue
                && let Some(origin) = previous_origin
            {
                // An old-style horizontal merge: this cell joins the one on
                // its left.
                let span = end - origin;
                row.cells[origin].column_span = span as u32;
                for column in start..end {
                    row.cells[column] = Cell {
                        merge: Merge::Left,
                        ..Cell::default()
                    };
                }
                continue;
            }
            let cell = Cell {
                blocks: if blocks.is_empty() {
                    vec![Block::Paragraph(Paragraph::default())]
                } else {
                    blocks
                },
                column_span: (end - start) as u32,
                row_span: 1,
                background: cell_def.background,
                merge: if cell_def.vertical_continue {
                    Merge::Above
                } else {
                    Merge::Origin
                },
                borders: cell_def.borders,
                vertical_alignment: cell_def.vertical_alignment,
                margins: cell_def.margins,
            };
            row.cells[start] = cell;
            for column in start + 1..end {
                row.cells[column] = Cell {
                    merge: Merge::Left,
                    ..Cell::default()
                };
            }
            previous_origin = Some(start);
        }
        table.rows.push(row);
    }
    // Vertical merges: each origin spans the rows of `Above` cells under it.
    for column in 0..column_count {
        for row in 0..table.rows.len() {
            if table.rows[row].cells[column].merge != Merge::Origin {
                continue;
            }
            let mut span = 1;
            while row + span < table.rows.len()
                && table.rows[row + span].cells[column].merge == Merge::Above
            {
                span += 1;
            }
            table.rows[row].cells[column].row_span = span as u32;
        }
    }
    Some(table)
}

fn side_index(side: Side) -> usize {
    match side {
        Side::Top | Side::All => 0,
        Side::Bottom => 1,
        Side::Left => 2,
        Side::Right => 3,
    }
}

fn line_spacing(line: Option<i32>, multiple: bool) -> Option<LineSpacing> {
    let line = line?;
    if line == 0 {
        return None;
    }
    if multiple {
        return Some(LineSpacing::Relative(line as f32 / 240.0));
    }
    if line > 0 {
        Some(LineSpacing::Minimum(twips(line)))
    } else {
        Some(LineSpacing::Exact(twips(-line)))
    }
}

/// Fills formatting the text leaves unset where its paragraph style sets
/// it, with RTF's defaults.
fn fill_defaults(props: &mut RunProperties, style: &RunProperties) {
    macro_rules! off {
        ($($field:ident),*) => {
            $( if props.$field.is_none() && style.$field.is_some() { props.$field = Some(false); } )*
        };
    }
    off!(bold, italic, underline, strike, hidden);
    if props.size.is_none() && style.size.is_some() {
        props.size = Some(12.0);
    }
}

/// Unsets the formatting `props` shares with its paragraph style.
fn strip_repeats(props: &mut RunProperties, style: &RunProperties) {
    macro_rules! same {
        ($($field:ident),*) => {
            $( if props.$field.is_some() && props.$field == style.$field { props.$field = None; } )*
        };
    }
    same!(
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
        hidden,
        shift,
        letter_spacing,
        width_scale
    );
}

fn run_key(props: &RunProperties) -> RunKey {
    let color = |color: Option<Color>| {
        color.map_or(u32::MAX, |color| {
            u32::from(color.red) << 16 | u32::from(color.green) << 8 | u32::from(color.blue)
        })
    };
    let flag = |value: Option<bool>| value.map_or(2, u32::from);
    [
        props.font.map_or(u32::MAX, |id| id),
        props.size.map_or(u32::MAX, f32::to_bits),
        flag(props.bold),
        flag(props.italic),
        flag(props.underline),
        flag(props.strike),
        color(props.color),
        color(props.highlight),
        props.baseline.map_or(0, |baseline| baseline as u32 + 1),
        props.caps.map_or(0, |caps| caps as u32 + 1),
        props.language.map_or(u32::MAX, |id| id),
        flag(props.hidden),
        props.shift.map_or(u32::MAX, f32::to_bits),
        props.letter_spacing.map_or(u32::MAX, f32::to_bits),
        props.width_scale.map_or(u32::MAX, f32::to_bits),
        0,
    ]
}

fn paragraph_key(props: &ParagraphProperties) -> ParagraphKey {
    let float = |value: Option<f32>| value.map_or(u32::MAX, f32::to_bits);
    let flag = |value: Option<bool>| value.map_or(2, u32::from);
    let (line_kind, line_value) = match props.line_spacing {
        None => (0, 0),
        Some(LineSpacing::Relative(value)) => (1, value.to_bits()),
        Some(LineSpacing::Minimum(value)) => (2, value.to_bits()),
        Some(LineSpacing::Exact(value)) => (3, value.to_bits()),
    };
    let border = props.border.map_or([u32::MAX; 4], |border| {
        [
            u32::from(border.top)
                | u32::from(border.bottom) << 1
                | u32::from(border.left) << 2
                | u32::from(border.right) << 3,
            border.line.width.to_bits(),
            border.line.color.map_or(u32::MAX, |color| {
                u32::from(color.red) << 16 | u32::from(color.green) << 8 | u32::from(color.blue)
            }),
            1,
        ]
    });
    [
        props.alignment.map_or(0, |alignment| alignment as u32 + 1),
        float(props.first_line_indent),
        float(props.left_indent),
        float(props.right_indent),
        float(props.space_before),
        float(props.space_after),
        line_kind,
        line_value,
        flag(props.keep_with_next),
        flag(props.keep_lines_together),
        flag(props.widow_control),
        props.outline_level.map_or(u32::MAX, u32::from),
        props.background.map_or(u32::MAX, |color| {
            u32::from(color.red) << 16 | u32::from(color.green) << 8 | u32::from(color.blue)
        }),
        flag(props.contextual_spacing),
        border[0],
        border[1],
        border[2],
        border[3],
        props.tabs.unwrap_or(u32::MAX),
        flag(props.page_break_before),
        0,
        0,
        0,
        0,
    ]
}

/// The code page a font's `\fcharsetN` names.
fn charset_code_page(charset: i32) -> Option<u16> {
    Some(match charset {
        0 => 1252,
        77 => 10000,
        128 => 932,
        129 => 949,
        134 => 936,
        136 => 950,
        161 => 1253,
        162 => 1254,
        163 => 1258,
        177 => 1255,
        178 => 1256,
        186 => 1257,
        204 => 1251,
        222 => 874,
        238 => 1250,
        254 => 437,
        255 => 850,
        _ => return None,
    })
}

/// Whether `byte` starts a two-byte character in a double-byte code page.
fn is_lead_byte(code_page: u16, byte: u8) -> bool {
    match code_page {
        932 => matches!(byte, 0x81..=0x9F | 0xE0..=0xFC),
        936 | 949 | 950 => (0x81..=0xFE).contains(&byte),
        _ => false,
    }
}

/// A `HYPERLINK "target" \l "anchor"` instruction's target.
fn hyperlink_target(instruction: &str) -> Option<String> {
    let rest = instruction.trim_start().get("HYPERLINK".len()..)?.trim();
    let mut target = String::new();
    let mut anchor = None;
    let mut parts = quoted_parts(rest).into_iter();
    while let Some(part) = parts.next() {
        match part.as_str() {
            "\\l" => anchor = parts.next(),
            switch if switch.starts_with('\\') => {
                // Switches with an argument (\o "tip", \t "frame").
                if matches!(switch, "\\o" | "\\t" | "\\m") {
                    parts.next();
                }
            }
            _ if target.is_empty() => target = part,
            _ => {}
        }
    }
    match (target.is_empty(), anchor) {
        (true, Some(anchor)) => Some(format!("#{anchor}")),
        (false, Some(anchor)) => Some(format!("{target}#{anchor}")),
        (false, None) => Some(target),
        (true, None) => None,
    }
}

/// Splits a field instruction into words, keeping quoted words whole.
fn quoted_parts(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for character in text.chars() {
        match character {
            '"' => {
                if quoted {
                    parts.push(std::mem::take(&mut current));
                }
                quoted = !quoted;
            }
            character if character.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    parts.push(std::mem::take(&mut current));
                }
            }
            character => current.push(character),
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

/// A custom outline from Word's shape properties: `pVerticies`
/// (`8;n;(x,y);...`) and `pSegmentInfo` (`2;n;segment;...`), in the shape's
/// own coordinate space (`geoRight` by `geoBottom`), scaled to its size.
fn custom_path(
    vertices: Option<&str>,
    segments: Option<&str>,
    geo_right: Option<i64>,
    geo_bottom: Option<i64>,
    width: f32,
    height: f32,
) -> Option<crate::document::ShapePath> {
    use crate::document::{PathStep, ShapePath};
    let points: Vec<(f32, f32)> = vertices?
        .split(';')
        .skip(2)
        .filter_map(|pair| {
            let pair = pair.trim().trim_start_matches('(').trim_end_matches(')');
            let (x, y) = pair.split_once(',')?;
            Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
        })
        .collect();
    if points.is_empty() {
        return None;
    }
    let scale_x = width / geo_right.unwrap_or(21600).max(1) as f32;
    let scale_y = height / geo_bottom.unwrap_or(21600).max(1) as f32;
    let point = |index: usize| -> Option<(f32, f32)> {
        let (x, y) = points.get(index)?;
        Some((x * scale_x, y * scale_y))
    };
    let segments: Vec<u32> = match segments {
        Some(text) => text
            .split(';')
            .skip(2)
            .filter_map(|value| value.trim().parse().ok())
            .collect(),
        // Without segments the points are one open polyline.
        None => std::iter::once(0x4000)
            .chain(std::iter::repeat_n(1, points.len() - 1))
            .collect(),
    };
    let mut steps = Vec::new();
    let mut at = 0usize;
    for segment in segments {
        let count = (segment & 0x1FFF).max(1) as usize;
        match segment >> 13 {
            // Line to, `count` times.
            0 => {
                for _ in 0..count {
                    let (x, y) = point(at)?;
                    steps.push(PathStep::Line(x, y));
                    at += 1;
                }
            }
            // Cubic curve to, `count` times.
            1 => {
                for _ in 0..count {
                    let controls = [point(at)?, point(at + 1)?, point(at + 2)?];
                    steps.push(PathStep::Curve(controls));
                    at += 3;
                }
            }
            2 => {
                let (x, y) = point(at)?;
                steps.push(PathStep::Move(x, y));
                at += 1;
            }
            3 => steps.push(PathStep::Close),
            4 => break,
            // Escapes and other kinds carry no points this reads.
            _ => {}
        }
    }
    (!steps.is_empty()).then_some(ShapePath {
        width,
        height,
        steps,
    })
}

/// The level of a built-in heading style's name (`heading 2`, `Heading 2`).
fn heading_number(name: &str) -> Option<u8> {
    let lower = name.trim().to_ascii_lowercase();
    let rest = lower.strip_prefix("heading")?.trim();
    let level: u8 = rest.parse().ok()?;
    (1..=9).contains(&level).then_some(level)
}

/// A list number written in `kind`: 12, l, L, xii, XII.
fn parse_number(kind: NumberKind, text: &str) -> Option<u32> {
    match kind {
        NumberKind::Decimal => text.parse().ok(),
        NumberKind::LowerLetter | NumberKind::UpperLetter => {
            let first = text.chars().next()?.to_ascii_lowercase();
            if !first.is_ascii_lowercase() || !text.chars().all(|c| c.eq_ignore_ascii_case(&first))
            {
                return None;
            }
            Some((first as u32 - 'a' as u32 + 1) + 26 * (text.len() as u32 - 1))
        }
        NumberKind::LowerRoman | NumberKind::UpperRoman => {
            let mut total = 0i64;
            let mut previous = 0i64;
            for character in text.chars().rev() {
                let value = match character.to_ascii_lowercase() {
                    'i' => 1,
                    'v' => 5,
                    'x' => 10,
                    'l' => 50,
                    'c' => 100,
                    'd' => 500,
                    'm' => 1000,
                    _ => return None,
                };
                if value < previous {
                    total -= value;
                } else {
                    total += value;
                    previous = value;
                }
            }
            u32::try_from(total).ok().filter(|value| *value > 0)
        }
    }
}

/// A packed RTF date (`\revdttm`, `\atndate`): minutes, hours, day, month,
/// and years since 1900 in bit fields, as ISO 8601.
fn packed_date(value: i32) -> Option<String> {
    let value = value as u32;
    if value == 0 {
        return None;
    }
    let minute = value & 0x3F;
    let hour = (value >> 6) & 0x1F;
    let day = (value >> 11) & 0x1F;
    let month = (value >> 16) & 0x0F;
    let year = ((value >> 20) & 0x1FF) + 1900;
    if month == 0 || day == 0 || month > 12 || hour > 23 || minute > 59 {
        return None;
    }
    Some(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:00Z"
    ))
}

/// A language tag for the common Windows language ids.
fn lcid_tag(lcid: i32) -> Option<&'static str> {
    Some(match lcid {
        1025 => "ar-SA",
        1028 => "zh-TW",
        1029 => "cs-CZ",
        1030 => "da-DK",
        1031 => "de-DE",
        1032 => "el-GR",
        1033 => "en-US",
        1034 | 3082 => "es-ES",
        1035 => "fi-FI",
        1036 => "fr-FR",
        1037 => "he-IL",
        1038 => "hu-HU",
        1040 => "it-IT",
        1041 => "ja-JP",
        1042 => "ko-KR",
        1043 => "nl-NL",
        1044 => "nb-NO",
        1045 => "pl-PL",
        1046 => "pt-BR",
        1049 => "ru-RU",
        1053 => "sv-SE",
        1055 => "tr-TR",
        1058 => "uk-UA",
        2052 => "zh-CN",
        2057 => "en-GB",
        2070 => "pt-PT",
        3081 => "en-AU",
        4105 => "en-CA",
        _ => return None,
    })
}

/// A BMP file from a device-independent bitmap: the file header added.
fn bitmap_file(dib: &[u8]) -> Option<Vec<u8>> {
    let header_size = u32::from_le_bytes(dib.get(0..4)?.try_into().ok()?) as usize;
    let bit_count = u16::from_le_bytes(dib.get(14..16)?.try_into().ok()?);
    let colors_used = if header_size >= 40 {
        u32::from_le_bytes(dib.get(32..36)?.try_into().ok()?) as usize
    } else {
        0
    };
    let palette = if colors_used > 0 {
        colors_used
    } else if bit_count <= 8 {
        1usize << bit_count
    } else {
        0
    };
    let offset = 14 + header_size + palette * 4;
    let size = 14 + dib.len();
    let mut file = Vec::with_capacity(size);
    file.extend_from_slice(b"BM");
    file.extend_from_slice(&(size as u32).to_le_bytes());
    file.extend_from_slice(&[0, 0, 0, 0]);
    file.extend_from_slice(&(offset as u32).to_le_bytes());
    file.extend_from_slice(dib);
    Some(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(source: &str) -> Document {
        read_rtf(source.as_bytes()).expect("rtf")
    }

    #[test]
    fn paragraphs_and_formatting() {
        let document = read(
            "{\\rtf1\\ansi{\\fonttbl\\f0\\fswiss Helvetica;}\\f0\\fs24 Plain {\\b bold} text\\par\\pard\\qc Centred\\par}",
        );
        assert_eq!(
            document.paragraph_texts(),
            vec!["Plain bold text", "Centred"]
        );
        let Block::Paragraph(first) = &document.sections[0].blocks[0] else {
            panic!("paragraph");
        };
        let bold = document.run_properties(&first.runs[1]);
        assert_eq!(bold.bold, Some(true));
        assert_eq!(bold.size, Some(12.0));
        assert_eq!(document.string(bold.font.expect("font")), "Helvetica");
    }

    #[test]
    fn code_pages_and_unicode() {
        let document = read("{\\rtf1\\ansi\\ansicpg1252 caf\\'e9 \\uc1\\u8364?uro\\par}");
        assert_eq!(document.paragraph_texts(), vec!["café €uro"]);
        let mac = read("{\\rtf1\\mac caf\\'8e\\par}");
        assert_eq!(mac.paragraph_texts(), vec!["café"]);
    }

    #[test]
    fn cocoa_line_ends_and_last_paragraph() {
        let document = read("{\\rtf1\\ansi First\\\nSecond}");
        assert_eq!(document.paragraph_texts(), vec!["First", "Second"]);
    }

    #[test]
    fn a_table_with_merged_cells() {
        let document = read(concat!(
            "{\\rtf1\\ansi",
            "\\trowd\\cellx2000\\cellx4000\\cellx6000",
            "\\pard\\intbl A\\cell B\\cell C\\cell\\row",
            "\\trowd\\cellx4000\\cellx6000",
            "\\pard\\intbl Wide\\cell D\\cell\\row",
            "\\pard After\\par}"
        ));
        let Block::Table(table) = &document.sections[0].blocks[0] else {
            panic!("table");
        };
        assert_eq!(table.columns.len(), 3);
        assert_eq!(table.rows[1].cells[0].column_span, 2);
        assert_eq!(table.rows[1].cells[1].merge, Merge::Left);
        assert_eq!(
            document.paragraph_texts(),
            vec!["A", "B", "C", "Wide", "D", "After"]
        );
    }

    #[test]
    fn links_footnotes_and_page_numbers() {
        let document = read(concat!(
            "{\\rtf1\\ansi See {\\field{\\*\\fldinst HYPERLINK \"https://example.com\"}{\\fldrslt here}}",
            "{\\super\\chftn}{\\footnote\\pard{\\super\\chftn} A note.}",
            " page {\\field{\\*\\fldinst PAGE}{\\fldrslt 1}}\\par}"
        ));
        let Block::Paragraph(paragraph) = &document.sections[0].blocks[0] else {
            panic!("paragraph");
        };
        let link = paragraph
            .runs
            .iter()
            .find_map(|run| document.link(run))
            .expect("link");
        assert_eq!(link, "https://example.com");
        assert!(
            paragraph
                .runs
                .iter()
                .any(|run| run.content == Inline::Footnote(0))
        );
        assert!(
            paragraph
                .runs
                .iter()
                .any(|run| run.content == Inline::PageNumber)
        );
        assert_eq!(document.footnotes.len(), 1);
    }

    #[test]
    fn word_lists() {
        let document = read(concat!(
            "{\\rtf1\\ansi{\\*\\listtable{\\list\\listtemplateid1",
            "{\\listlevel\\levelnfc0\\levelstartat1{\\leveltext\\'02\\'00.;}{\\levelnumbers\\'01;}\\fi-360\\li720}",
            "\\listid7}}",
            "{\\*\\listoverridetable{\\listoverride\\listid7\\listoverridecount0\\ls1}}",
            "\\pard\\ls1\\ilvl0{\\listtext 1.\\tab}One\\par",
            "{\\listtext 2.\\tab}Two\\par}"
        ));
        assert_eq!(document.paragraph_texts(), vec!["One", "Two"]);
        let Block::Paragraph(first) = &document.sections[0].blocks[0] else {
            panic!("paragraph");
        };
        let item = first.list.expect("list item");
        assert!(item.starts_list);
        let level = &document.styles.list[item.style].levels[0];
        assert_eq!(
            level.label,
            ListLabel::Number(NumberFormat {
                kind: NumberKind::Decimal,
                pattern: "%1.".to_string(),
                tiered: false,
            })
        );
    }

    #[test]
    fn packed_dates() {
        // 2024-03-15 10:30.
        let value = 30 | 10 << 6 | 15 << 11 | 3 << 16 | (124 << 20);
        assert_eq!(packed_date(value).as_deref(), Some("2024-03-15T10:30:00Z"));
    }
}
