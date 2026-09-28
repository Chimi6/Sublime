//! Reads a Word document (WordprocessingML) into the document model.
//!
//! The package is a ZIP of XML parts. `styles.xml` gives the style table,
//! `numbering.xml` the list styles, `footnotes.xml` and `endnotes.xml` the
//! notes, the header and footer parts the page furniture, and
//! `document.xml` the body: paragraphs, tables, sections. Every part is
//! walked once with the pull reader in `io::xml`, straight into the
//! model's arenas; nothing is held as a tree.

// The element readers pull events in a `while let` so they can hand the
// same reader to the reader of a nested element mid-loop.
#![allow(clippy::while_let_on_iterator)]

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;

use crate::document::{
    Alignment, Anchor, AnchorBase, Baseline, Block, Border, Caps, Cell, CellBorders, CellMargins,
    CharacterStyle, Chart, Color, Document, FloatingContent, FloatingObject, Id, Inline,
    InlineImage, LineSpacing, ListItem, ListLabel, ListLevel, ListStyle, Media, MediaId, Merge,
    NoteId, NumberFormat, NumberKind, PageSetup, PageVariants, Paragraph, ParagraphProperties,
    ParagraphStyle, Placement, Revision, RevisionKind, Row, Run, RunProperties, Section,
    SectionStart, ShapeGeometry, Span, StyleId, Table, TableBorders, VerticalAlignment,
};
use crate::io::xml::{XmlEvent, XmlReader};
use crate::io::zip::{ZipArchive, ZipError};

#[derive(Debug)]
pub enum DocxError {
    Zip(ZipError),
    /// An OLE compound file: a password-protected (encrypted) `.docx` or a
    /// legacy binary `.doc`, neither of which is a Word package.
    Compound,
    /// A required part is missing or not text.
    Part(&'static str),
}

impl fmt::Display for DocxError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DocxError::Zip(error) => write!(formatter, "not a Word package: {error:?}"),
            DocxError::Compound => write!(
                formatter,
                "this is a password-protected .docx or a legacy .doc (an OLE compound file), not a Word package; remove the password or save it as .docx in Word first"
            ),
            DocxError::Part(name) => write!(formatter, "Word package has no readable {name}"),
        }
    }
}

impl std::error::Error for DocxError {}

const OFFICE_DOCUMENT_REL: &str = "officeDocument/2006/relationships/officeDocument";

/// Reads the document out of a `.docx` file's bytes.
pub fn read_docx(bytes: &[u8]) -> Result<Document, DocxError> {
    const COMPOUND_FILE: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    if bytes.starts_with(&COMPOUND_FILE) {
        return Err(DocxError::Compound);
    }
    let archive = ZipArchive::parse(bytes).map_err(DocxError::Zip)?;
    let document_part = main_part(&archive);
    let base = part_directory(&document_part);
    let mut reader = Reader {
        archive: &archive,
        base,
        document: Document::default(),
        paragraph_styles: HashMap::new(),
        character_styles: HashMap::new(),
        numbering_styles: HashMap::new(),
        default_run: RunProperties::default(),
        default_paragraph: ParagraphProperties::default(),
        list_starts: Vec::new(),
        nums: Vec::new(),
        seen_nums: Vec::new(),
        footnotes: Vec::new(),
        media: Vec::new(),
        rels: Vec::new(),
        page: 0,
        even_headers: false,
        shows_background: false,
        pending_floating: Vec::new(),
        anchoring: true,
        comments: Vec::new(),
        ranged_comments: Vec::new(),
        page_part: None,
        drawing_layout: DrawingLayout::default(),
        floating_layouts: Vec::new(),
        pending_blocks: Vec::new(),
        theme_colors: HashMap::new(),
        theme_fonts: [None, None],
        table_style_borders: HashMap::new(),
        after_note_mark: false,
    };
    // The theme first: styles name its fonts and colours.
    reader.rels = reader.relationships(&document_part);
    reader.read_theme();
    reader.read_styles();
    reader.read_numbering();
    reader.read_settings();
    reader.read_notes("footnotes.xml", "w:footnote");
    reader.read_notes("endnotes.xml", "w:endnote");
    reader.read_comments();
    let body = reader
        .part_text(&document_part)
        .ok_or(DocxError::Part("document part"))?;
    reader.read_body(&body);
    Ok(reader.document)
}

/// The main part named by the package relationships, or `word/document.xml`.
fn main_part(archive: &ZipArchive<'_>) -> String {
    if let Some(text) = read_text(archive, "_rels/.rels") {
        for relationship in parse_relationships(&text) {
            if relationship.kind.ends_with(OFFICE_DOCUMENT_REL) {
                return relationship.target.trim_start_matches('/').to_string();
            }
        }
    }
    "word/document.xml".to_string()
}

fn part_directory(part: &str) -> String {
    match part.rfind('/') {
        Some(index) => part[..=index].to_string(),
        None => String::new(),
    }
}

fn read_text(archive: &ZipArchive<'_>, name: &str) -> Option<String> {
    let entry = archive.find(name)?;
    let mut bytes = Vec::with_capacity(entry.uncompressed_size as usize);
    archive.read(entry, &mut bytes).ok()?;
    String::from_utf8(bytes).ok()
}

struct Relationship {
    id: String,
    kind: String,
    target: String,
}

fn parse_relationships(text: &str) -> Vec<Relationship> {
    let mut relationships = Vec::new();
    for event in XmlReader::new(text) {
        let XmlEvent::Start {
            name, attributes, ..
        } = event
        else {
            continue;
        };
        if name != "Relationship" {
            continue;
        }
        let mut relationship = Relationship {
            id: String::new(),
            kind: String::new(),
            target: String::new(),
        };
        for (key, value) in attributes {
            match key {
                "Id" => relationship.id = value.into_owned(),
                "Type" => relationship.kind = value.into_owned(),
                "Target" => relationship.target = value.into_owned(),
                _ => {}
            }
        }
        relationships.push(relationship);
    }
    relationships
}

/// The state of a complex field (`w:fldChar`) while its runs go by.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    None,
    /// Between `begin` and `separate`: the runs hold the instruction.
    Instruction,
    /// Between `separate` and `end`: the runs hold the result.
    Result,
}

/// What a paragraph's properties said, beyond the properties themselves.
#[derive(Default)]
struct ParagraphHeader {
    properties: ParagraphProperties,
    style: Option<StyleId>,
    /// (numId, level) from `w:numPr`.
    numbering: Option<(i64, u8)>,
    mark: RunProperties,
    section: Option<SectionHeader>,
    /// A frame (`w:framePr`) the paragraph is set in, apart from the text.
    frame: Option<Frame>,
}

/// A text frame: where it is placed from, its offset or alignment on each
/// axis, and its size, in points (a size of `None` fits the text).
#[derive(Clone, Copy, Default)]
struct Frame {
    horizontal: Option<AnchorBase>,
    vertical: Option<AnchorBase>,
    x: f32,
    y: f32,
    align: (Option<f32>, Option<f32>),
    width: Option<f32>,
    height: Option<f32>,
}

/// A section's properties as read; the header and footer parts are
/// loaded when the section closes.
#[derive(Default)]
struct SectionHeader {
    page: PageSetup,
    columns: u16,
    column_gap: Option<f32>,
    column_widths: Vec<(f32, f32)>,
    continuous: bool,
    title_page: bool,
    /// (element, page kind, relationship id)
    references: Vec<(bool, String, String)>,
}

struct Reader<'a> {
    archive: &'a ZipArchive<'a>,
    /// The directory of the main part (`word/`), which relationship
    /// targets are relative to.
    base: String,
    document: Document,
    paragraph_styles: HashMap<String, usize>,
    character_styles: HashMap<String, usize>,
    /// Numbering styles (`w:type="numbering"`) by style id, to the `numId`
    /// their paragraph properties name.
    numbering_styles: HashMap<String, usize>,
    default_run: RunProperties,
    default_paragraph: ParagraphProperties,
    /// Per list style, the starting number of each level.
    list_starts: Vec<Vec<u32>>,
    /// `w:num` instances: (numId, list style, level 0 start override).
    nums: Vec<(i64, StyleId, Option<u32>)>,
    seen_nums: Vec<i64>,
    /// Note ids (`w:id`) to the model's note ids, footnotes then endnotes.
    footnotes: Vec<(bool, i64, NoteId)>,
    /// Media parts already loaded, by package path.
    media: Vec<(String, MediaId)>,
    /// The relationships of the part being read.
    rels: Vec<Relationship>,
    /// Pages begun so far, by explicit breaks, for floating objects.
    page: u32,
    even_headers: bool,
    /// Word draws the page colour (it is kept only then).
    shows_background: bool,
    /// Floating objects of the section being read, by index, with the bases
    /// their offsets are measured from.
    pending_floating: Vec<(usize, AnchorBase, AnchorBase)>,
    /// Blocks a paragraph's drawings leave to follow the paragraph.
    pending_blocks: Vec<Block>,
    /// The header or footer being read, for the drawings it holds.
    page_part: Option<crate::document::PagePart>,
    /// How the drawing being read is placed and sized relative to the page.
    drawing_layout: DrawingLayout,
    /// Floating objects placed that way: (index, layout).
    floating_layouts: Vec<(usize, DrawingLayout)>,
    /// Comment ids (`w:id`) to the model's comments.
    comments: Vec<(i64, Id)>,
    /// Comments whose range has begun (their reference adds no markers).
    ranged_comments: Vec<Id>,
    /// Reading the body's own text, where a drawing placed from its
    /// paragraph is anchored in the text (not in headers, notes, or boxes).
    anchoring: bool,
    /// The theme's colour scheme (dk1, lt1, accent1, ...), for colours Word
    /// gives by scheme name.
    theme_colors: HashMap<String, Color>,
    /// The theme's Latin heading (major) and body (minor) fonts.
    theme_fonts: [Option<String>; 2],
    /// Table styles' borders by style id, with the style they are based on.
    table_style_borders: HashMap<String, (TableSides, Option<String>)>,
    /// A note mark was just read; the space Word puts after it is not text.
    after_note_mark: bool,
}

impl Reader<'_> {
    // ----- parts and relationships -----

    fn part_text(&self, name: &str) -> Option<String> {
        read_text(self.archive, name)
    }

    fn relationships(&self, part: &str) -> Vec<Relationship> {
        let directory = part_directory(part);
        let file = &part[directory.len()..];
        let rels_part = format!("{directory}_rels/{file}.rels");
        match self.part_text(&rels_part) {
            Some(text) => parse_relationships(&text),
            None => Vec::new(),
        }
    }

    fn target(&self, id: &str) -> Option<String> {
        let relationship = self
            .rels
            .iter()
            .find(|relationship| relationship.id == id)?;
        if relationship.target.starts_with('/') {
            return Some(relationship.target.trim_start_matches('/').to_string());
        }
        Some(format!("{}{}", self.base, relationship.target))
    }

    /// Reads the theme's colour scheme through the document's theme relationship.
    fn read_theme(&mut self) {
        let Some(id) = self
            .rels
            .iter()
            .find(|relationship| relationship.kind.ends_with("/theme"))
            .map(|relationship| relationship.id.clone())
        else {
            return;
        };
        let Some(part) = self.target(&id) else {
            return;
        };
        let Some(text) = self.part_text(&part) else {
            return;
        };
        let mut reader = XmlReader::new(&text);
        let mut in_scheme = false;
        let mut slot: Option<String> = None;
        let mut font_slot: Option<usize> = None;
        while let Some(event) = reader.next() {
            match event {
                XmlEvent::Start {
                    name: "a:clrScheme",
                    ..
                } => in_scheme = true,
                XmlEvent::End {
                    name: "a:clrScheme",
                } => in_scheme = false,
                XmlEvent::Start {
                    name: "a:majorFont",
                    ..
                } => font_slot = Some(0),
                XmlEvent::Start {
                    name: "a:minorFont",
                    ..
                } => font_slot = Some(1),
                XmlEvent::End {
                    name: "a:majorFont" | "a:minorFont",
                } => font_slot = None,
                XmlEvent::Start {
                    name: "a:latin",
                    attributes,
                    ..
                } if font_slot.is_some() => {
                    if let (Some(index), Some(face)) =
                        (font_slot, attribute(&attributes, "typeface"))
                        && !face.is_empty()
                    {
                        self.theme_fonts[index] = Some(face.to_string());
                    }
                }
                XmlEvent::End {
                    name: "a:fontScheme",
                } => break,
                XmlEvent::Start {
                    name, attributes, ..
                } if in_scheme => match name {
                    "a:srgbClr" => {
                        if let (Some(key), Some(color)) = (
                            slot.take(),
                            attribute(&attributes, "val").and_then(parse_color),
                        ) {
                            self.theme_colors.insert(key, color);
                        }
                    }
                    "a:sysClr" => {
                        if let (Some(key), Some(color)) = (
                            slot.take(),
                            attribute(&attributes, "lastClr").and_then(parse_color),
                        ) {
                            self.theme_colors.insert(key, color);
                        }
                    }
                    other => slot = other.strip_prefix("a:").map(str::to_string),
                },
                _ => {}
            }
        }
    }

    /// A scheme colour by name (with Word's aliases bg1/tx1/bg2/tx2), after its
    /// luminance, shade, and tint modifiers.
    fn theme_color(&self, name: &str, modifiers: &[(String, f32)]) -> Option<Color> {
        let key = match name {
            "bg1" => "lt1",
            "tx1" => "dk1",
            "bg2" => "lt2",
            "tx2" => "dk2",
            other => other,
        };
        let base = *self.theme_colors.get(key)?;
        Some(apply_color_modifiers(base, modifiers))
    }

    fn hyperlink_target(&self, id: &str) -> Option<String> {
        let relationship = self
            .rels
            .iter()
            .find(|relationship| relationship.id == id)?;
        Some(relationship.target.clone())
    }

    // ----- styles.xml -----

    fn read_styles(&mut self) {
        let part = format!("{}styles.xml", self.base);
        let Some(text) = self.part_text(&part) else {
            return;
        };
        let mut reader = XmlReader::new(&text);
        let mut paragraph_parents: Vec<Option<String>> = Vec::new();
        let mut character_parents: Vec<Option<String>> = Vec::new();
        while let Some(event) = reader.next() {
            let XmlEvent::Start {
                name,
                attributes,
                self_closing,
            } = event
            else {
                continue;
            };
            match name {
                "w:docDefaults" if !self_closing => self.read_defaults(&mut reader),
                "w:style" if !self_closing => {
                    let kind = attribute(&attributes, "w:type").unwrap_or("paragraph");
                    let id = attribute(&attributes, "w:styleId")
                        .unwrap_or("")
                        .to_string();
                    let style = self.read_style(&mut reader);
                    match kind {
                        "paragraph" => {
                            let index = self.document.styles.paragraph.len();
                            let default = attribute(&attributes, "w:default");
                            if default.is_some()
                                && toggle(default)
                                && self.document.styles.default_paragraph.is_none()
                            {
                                self.document.styles.default_paragraph = Some(index);
                            }
                            self.document.styles.paragraph.push(ParagraphStyle {
                                name: style.name.unwrap_or_else(|| id.clone()),
                                parent: None,
                                paragraph: style.paragraph,
                                run: style.run,
                            });
                            paragraph_parents.push(style.based_on);
                            self.paragraph_styles.insert(id, index);
                        }
                        "table" => {
                            if let Some(borders) = style.table_borders {
                                self.table_style_borders
                                    .insert(id.clone(), (borders, style.based_on.clone()));
                            }
                        }
                        "character" => {
                            let index = self.document.styles.character.len();
                            self.document.styles.character.push(CharacterStyle {
                                name: style.name.unwrap_or_else(|| id.clone()),
                                parent: None,
                                run: style.run,
                            });
                            character_parents.push(style.based_on);
                            self.character_styles.insert(id, index);
                        }
                        "numbering" => {
                            if let Some(num) = style.num_id {
                                self.numbering_styles.insert(id, num);
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        for (index, based_on) in paragraph_parents.iter().enumerate() {
            let parent = based_on
                .as_deref()
                .and_then(|id| self.paragraph_styles.get(id))
                .copied()
                .filter(|parent| *parent != index);
            self.document.styles.paragraph[index].parent = parent;
            if parent.is_none() {
                // Root styles sit on the document defaults.
                let mut paragraph = self.default_paragraph;
                paragraph.overlay(&self.document.styles.paragraph[index].paragraph);
                self.document.styles.paragraph[index].paragraph = paragraph;
                let mut run = self.default_run;
                run.overlay(&self.document.styles.paragraph[index].run);
                self.document.styles.paragraph[index].run = run;
            }
        }
        for (index, based_on) in character_parents.iter().enumerate() {
            let parent = based_on
                .as_deref()
                .and_then(|id| self.character_styles.get(id))
                .copied()
                .filter(|parent| *parent != index);
            self.document.styles.character[index].parent = parent;
        }
    }

    fn read_defaults(&mut self, reader: &mut XmlReader<'_>) {
        while let Some(event) = reader.next() {
            match event {
                XmlEvent::Start {
                    name: "w:rPr",
                    self_closing: false,
                    ..
                } => {
                    let run = self.read_run_properties(reader);
                    self.default_run = run.properties;
                }
                XmlEvent::Start {
                    name: "w:pPr",
                    self_closing: false,
                    ..
                } => {
                    let header = self.read_paragraph_properties(reader);
                    self.default_paragraph = header.properties;
                }
                XmlEvent::End {
                    name: "w:docDefaults",
                } => return,
                _ => {}
            }
        }
    }

    fn read_style(&mut self, reader: &mut XmlReader<'_>) -> StyleRead {
        let mut style = StyleRead::default();
        while let Some(event) = reader.next() {
            match event {
                XmlEvent::Start {
                    name, attributes, ..
                } => match name {
                    "w:name" => style.name = attribute(&attributes, "w:val").map(str::to_string),
                    "w:basedOn" => {
                        style.based_on = attribute(&attributes, "w:val").map(str::to_string);
                    }
                    "w:pPr" => {
                        let header = self.read_paragraph_properties(reader);
                        style.paragraph = header.properties;
                        style.num_id = header
                            .numbering
                            .filter(|(id, _)| *id >= 0)
                            .map(|(id, _)| id as usize);
                    }
                    "w:rPr" => {
                        let run = self.read_run_properties(reader);
                        style.run = run.properties;
                    }
                    "w:tblPr" => {
                        let (_, borders) = read_table_properties(reader);
                        style.table_borders = Some(borders);
                    }
                    // Conditional formats (first row, banding...) carry their own
                    // table properties; only the style's own count here.
                    "w:tblStylePr" => skip_element(reader, name),
                    _ => {}
                },
                XmlEvent::End { name: "w:style" } => break,
                _ => {}
            }
        }
        style
    }

    // ----- numbering.xml -----

    fn read_numbering(&mut self) {
        let part = format!("{}numbering.xml", self.base);
        let Some(text) = self.part_text(&part) else {
            return;
        };
        let mut reader = XmlReader::new(&text);
        // (abstractNumId, style, starts, numStyleLink, styleLink)
        let mut abstracts: Vec<AbstractNumbering> = Vec::new();
        let mut nums: Vec<(i64, i64, Option<u32>)> = Vec::new();
        while let Some(event) = reader.next() {
            let XmlEvent::Start {
                name,
                attributes,
                self_closing,
            } = event
            else {
                continue;
            };
            match name {
                "w:abstractNum" if !self_closing => {
                    let id = attribute(&attributes, "w:abstractNumId")
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(-1);
                    let numbering = read_abstract_numbering(&mut reader, id);
                    abstracts.push(numbering);
                }
                "w:num" if !self_closing => {
                    let id = attribute(&attributes, "w:numId")
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(-1);
                    let (abstract_id, start) = read_num(&mut reader);
                    nums.push((id, abstract_id, start));
                }
                _ => {}
            }
        }
        // A `numStyleLink` borrows the levels of the abstract numbering
        // that a numbering style links back to.
        let mut resolved_levels: Vec<Option<usize>> = vec![None; abstracts.len()];
        for (index, numbering) in abstracts.iter().enumerate() {
            if !numbering.levels.is_empty() {
                resolved_levels[index] = Some(index);
                continue;
            }
            let Some(link) = &numbering.num_style_link else {
                resolved_levels[index] = Some(index);
                continue;
            };
            let by_style_link = abstracts
                .iter()
                .position(|other| other.style_link.as_deref() == Some(link));
            let by_numbering_style = self.numbering_styles.get(link).and_then(|num_id| {
                let (_, abstract_id, _) = nums.iter().find(|(id, _, _)| *id == *num_id as i64)?;
                abstracts.iter().position(|other| other.id == *abstract_id)
            });
            resolved_levels[index] = by_style_link.or(by_numbering_style).or(Some(index));
        }
        for (index, numbering) in abstracts.iter().enumerate() {
            let source = resolved_levels[index].unwrap_or(index);
            let levels = abstracts[source].levels.clone();
            let starts = abstracts[source].starts.clone();
            let name = numbering
                .style_link
                .clone()
                .unwrap_or_else(|| format!("List {}", numbering.id));
            self.document.styles.list.push(ListStyle { name, levels });
            self.list_starts.push(starts);
        }
        for (num_id, abstract_id, start) in nums {
            let Some(style) = abstracts
                .iter()
                .position(|numbering| numbering.id == abstract_id)
            else {
                continue;
            };
            self.nums.push((num_id, style, start));
        }
    }

    fn read_settings(&mut self) {
        let part = format!("{}settings.xml", self.base);
        if let Some(text) = self.part_text(&part) {
            self.even_headers = text.contains("<w:evenAndOddHeaders");
            self.shows_background = text.contains("<w:displayBackgroundShape");
        }
    }

    // ----- notes -----

    fn read_notes(&mut self, file: &str, element: &str) {
        let part = format!("{}{file}", self.base);
        let Some(text) = self.part_text(&part) else {
            return;
        };
        let part_rels = self.relationships(&part);
        let saved_rels = std::mem::replace(&mut self.rels, part_rels);
        let mut reader = XmlReader::new(&text);
        while let Some(event) = reader.next() {
            let XmlEvent::Start {
                name,
                attributes,
                self_closing,
            } = event
            else {
                continue;
            };
            if name != element || self_closing {
                continue;
            }
            let id: i64 = attribute(&attributes, "w:id")
                .and_then(|value| value.parse().ok())
                .unwrap_or(-1);
            let is_separator = attribute(&attributes, "w:type").is_some();
            let anchoring = std::mem::replace(&mut self.anchoring, false);
            let blocks = self.read_blocks(&mut reader, element);
            self.anchoring = anchoring;
            if is_separator || id < 0 {
                continue;
            }
            let note = self.document.footnotes.len();
            self.document
                .footnotes
                .push(crate::document::Note { blocks });
            self.footnotes.push((element == "w:endnote", id, note));
        }
        self.rels = saved_rels;
    }

    // ----- comments -----

    /// The comments part: each comment's author, date, and text.
    fn read_comments(&mut self) {
        let part = format!("{}comments.xml", self.base);
        let Some(text) = self.part_text(&part) else {
            return;
        };
        let mut reader = XmlReader::new(&text);
        while let Some(event) = reader.next() {
            let XmlEvent::Start {
                name: "w:comment",
                attributes,
                self_closing: false,
            } = event
            else {
                continue;
            };
            let id: i64 = attribute(&attributes, "w:id")
                .and_then(|value| value.parse().ok())
                .unwrap_or(-1);
            let comment = crate::document::Comment {
                author: attribute(&attributes, "w:author")
                    .unwrap_or_default()
                    .to_string(),
                initials: attribute(&attributes, "w:initials").map(str::to_string),
                date: attribute(&attributes, "w:date").map(str::to_string),
                text: String::new(),
            };
            let anchoring = std::mem::replace(&mut self.anchoring, false);
            let blocks = self.read_blocks(&mut reader, "w:comment");
            self.anchoring = anchoring;
            let text = blocks
                .iter()
                .filter_map(|block| match block {
                    Block::Paragraph(paragraph) => Some(self.document.paragraph_text(paragraph)),
                    Block::Table(_) => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            self.comments.push((id, self.document.comments.len() as Id));
            self.document
                .comments
                .push(crate::document::Comment { text, ..comment });
        }
    }

    /// The model comment a `w:id` attribute names.
    fn comment_for(&self, attributes: &[(&str, Cow<'_, str>)]) -> Option<Id> {
        let id: i64 = attribute(attributes, "w:id")?.parse().ok()?;
        self.comments
            .iter()
            .find(|(word, _)| *word == id)
            .map(|(_, comment)| *comment)
    }

    // ----- the body -----

    fn read_body(&mut self, text: &str) {
        let mut reader = XmlReader::new(text);
        let mut blocks: Vec<Block> = Vec::new();
        let mut in_body = false;
        while let Some(event) = reader.next() {
            match event {
                XmlEvent::Start {
                    name: "w:body",
                    self_closing: false,
                    ..
                } => in_body = true,
                // The page colour, stated before the body.
                XmlEvent::Start {
                    name: "w:background",
                    attributes,
                    ..
                } if !in_body && self.shows_background => {
                    self.document.page_color = attribute(&attributes, "w:color")
                        .and_then(parse_color)
                        .filter(|color| (color.red, color.green, color.blue) != (255, 255, 255));
                }
                XmlEvent::Start {
                    name: "w:p",
                    self_closing,
                    ..
                } if in_body => {
                    let section = if self_closing {
                        blocks.push(Block::Paragraph(Paragraph::default()));
                        None
                    } else {
                        self.read_paragraph(&mut reader, &mut blocks)
                    };
                    if let Some(header) = section {
                        let section_blocks = std::mem::take(&mut blocks);
                        self.finish_section(header, section_blocks);
                    }
                }
                XmlEvent::Start {
                    name: "w:tbl",
                    self_closing: false,
                    ..
                } if in_body => {
                    let table = self.read_table(&mut reader);
                    blocks.push(Block::Table(table));
                }
                XmlEvent::Start {
                    name: "w:sectPr",
                    self_closing,
                    ..
                } if in_body => {
                    let header = if self_closing {
                        SectionHeader::default()
                    } else {
                        self.read_section_properties(&mut reader)
                    };
                    let section_blocks = std::mem::take(&mut blocks);
                    self.finish_section(header, section_blocks);
                }
                XmlEvent::Start {
                    name: "w:sdtPr", ..
                } => skip_element(&mut reader, "w:sdtPr"),
                XmlEvent::Start {
                    name: "mc:Fallback",
                    self_closing: false,
                    ..
                } => skip_element(&mut reader, "mc:Fallback"),
                XmlEvent::End { name: "w:body" } => break,
                _ => {}
            }
        }
        if !blocks.is_empty() || self.document.sections.is_empty() {
            self.finish_section(SectionHeader::default(), blocks);
        }
    }

    fn finish_section(&mut self, header: SectionHeader, blocks: Vec<Block>) {
        let mut headers = PageVariants::default();
        let mut footers = PageVariants::default();
        for (is_header, kind, id) in &header.references {
            let Some(part) = self.target(id) else {
                continue;
            };
            let Some(text) = self.part_text(&part) else {
                continue;
            };
            let part_rels = self.relationships(&part);
            let saved_rels = std::mem::replace(&mut self.rels, part_rels);
            let mut reader = XmlReader::new(&text);
            let root = if *is_header { "w:hdr" } else { "w:ftr" };
            let anchoring = std::mem::replace(&mut self.anchoring, false);
            let used = match kind.as_str() {
                "first" => header.title_page,
                "even" => self.even_headers,
                _ => true,
            };
            self.page_part = Some(crate::document::PagePart {
                footer: !*is_header,
                pages: match kind.as_str() {
                    "first" => crate::document::PageKind::First,
                    "even" => crate::document::PageKind::Even,
                    _ => crate::document::PageKind::Default,
                },
            });
            let floating_before = self.document.floating.len();
            let part_blocks = self.read_blocks(&mut reader, root);
            self.page_part = None;
            // Drawings of a variant Word does not show are not shown.
            if !used {
                self.document.floating.truncate(floating_before);
                self.pending_floating
                    .retain(|(index, _, _)| *index < floating_before);
            }
            self.anchoring = anchoring;
            self.rels = saved_rels;
            let variants = if *is_header {
                &mut headers
            } else {
                &mut footers
            };
            match kind.as_str() {
                "first" if header.title_page => variants.first = Some(part_blocks),
                "even" if self.even_headers => variants.even = Some(part_blocks),
                "default" => variants.default = Some(part_blocks),
                _ => {}
            }
        }
        let start = if header.continuous {
            SectionStart::Continuous
        } else {
            SectionStart::NewPage
        };
        if start == SectionStart::NewPage && !self.document.sections.is_empty() {
            self.page += 1;
        }
        // Offsets from the margin (or, approximately, from the anchoring
        // paragraph, taken as the top of the text area) become page
        // coordinates now that the section's margins are known.
        let page = &header.page;
        let layouts = std::mem::take(&mut self.floating_layouts);
        let text_width = page.width - page.margin_left - page.margin_right;
        let text_height = page.height - page.margin_top - page.margin_bottom;
        for (index, horizontal, vertical) in std::mem::take(&mut self.pending_floating) {
            if let Some(object) = self.document.floating.get_mut(index) {
                let layout = layouts
                    .iter()
                    .find(|(at, _)| *at == index)
                    .map(|(_, layout)| *layout)
                    .unwrap_or_default();
                let across = match horizontal {
                    AnchorBase::Page => page.width,
                    _ => text_width,
                };
                let down = match vertical {
                    AnchorBase::Page => page.height,
                    _ => text_height,
                };
                // A size given as a share of the page (or margins).
                if let Some((share, of_page)) = layout.size.0 {
                    object.width = share * if of_page { page.width } else { text_width };
                }
                if let Some((share, of_page)) = layout.size.1 {
                    object.height = share * if of_page { page.height } else { text_height };
                }
                // An offset given as a share, or an alignment, within what
                // it is placed from.
                if let Some(share) = layout.share.0 {
                    object.x += share * across;
                }
                if let Some(share) = layout.share.1 {
                    object.y += share * down;
                }
                if let Some(factor) = layout.align.0 {
                    object.x += factor * (across - object.width);
                }
                if let Some(factor) = layout.align.1 {
                    object.y += factor * (down - object.height);
                }
                if horizontal != AnchorBase::Page {
                    object.x += page.margin_left;
                }
                // One in a header or footer placed from its paragraph sits
                // from the header's (or footer's) first line.
                match (vertical, object.repeats) {
                    (AnchorBase::Page, _) => {}
                    (AnchorBase::Line, Some(part)) if part.footer => {
                        object.y += page.height - page.footer_distance - FOOTER_LINE;
                    }
                    (AnchorBase::Line, Some(_)) => object.y += page.header_distance,
                    _ if !object.follows_text => object.y += page.margin_top,
                    _ => {}
                }
            }
        }
        let columns = header.columns.max(1);
        // Explicit widths only where they are unequal columns of the section.
        let column_widths = if header.column_widths.len() == usize::from(columns) && columns > 1 {
            header.column_widths
        } else {
            Vec::new()
        };
        self.document.sections.push(Section {
            page: header.page,
            columns,
            column_gap: header.column_gap,
            column_widths,
            start,
            headers,
            footers,
            blocks,
        });
    }

    /// Paragraphs and tables until the end tag of `until`.
    fn read_blocks(&mut self, reader: &mut XmlReader<'_>, until: &str) -> Vec<Block> {
        let mut blocks = Vec::new();
        while let Some(event) = reader.next() {
            match event {
                XmlEvent::Start {
                    name: "w:p",
                    self_closing,
                    ..
                } => {
                    if self_closing {
                        blocks.push(Block::Paragraph(Paragraph::default()));
                    } else {
                        self.read_paragraph(reader, &mut blocks);
                    }
                }
                XmlEvent::Start {
                    name: "w:tbl",
                    self_closing: false,
                    ..
                } => {
                    let table = self.read_table(reader);
                    blocks.push(Block::Table(table));
                }
                XmlEvent::Start {
                    name: "w:sdtPr", ..
                } => skip_element(reader, "w:sdtPr"),
                XmlEvent::Start {
                    name: "mc:Fallback",
                    self_closing: false,
                    ..
                } => skip_element(reader, "mc:Fallback"),
                XmlEvent::End { name } if name == until => break,
                _ => {}
            }
        }
        blocks
    }

    // ----- paragraphs and runs -----

    /// Reads a paragraph into `blocks` and returns its section properties
    /// when it carries them (it is then the last paragraph of a section).
    #[inline(never)]
    fn read_paragraph(
        &mut self,
        reader: &mut XmlReader<'_>,
        blocks: &mut Vec<Block>,
    ) -> Option<SectionHeader> {
        let mut paragraph = Paragraph::default();
        let mut section = None;
        let mut frame: Option<Frame> = None;
        let mut link: Option<u32> = None;
        let mut revision: Option<u32> = None;
        let mut fields = FieldState::default();
        while let Some(event) = reader.next() {
            match event {
                XmlEvent::Start {
                    name,
                    attributes,
                    self_closing,
                } => match name {
                    "w:pPr" if !self_closing => {
                        let header = self.read_paragraph_properties(reader);
                        frame = header.frame;
                        paragraph.style = header.style;
                        paragraph.list = header
                            .numbering
                            .filter(|(id, _)| *id > 0)
                            .and_then(|(id, level)| self.list_item(id, level));
                        paragraph.properties =
                            self.document.intern_paragraph_properties(header.properties);
                        paragraph.mark = self.document.intern_run_properties(header.mark);
                        section = header.section;
                    }
                    "w:r" if !self_closing => {
                        let run_link = fields.link.or(link);
                        self.read_run(reader, &mut paragraph, run_link, revision, &mut fields);
                    }
                    "w:hyperlink" if !self_closing => {
                        let target = attribute(&attributes, "r:id")
                            .and_then(|id| self.hyperlink_target(id))
                            .or_else(|| {
                                attribute(&attributes, "w:anchor")
                                    .map(|anchor| format!("#{anchor}"))
                            });
                        link = target.map(|target| self.document.intern_link(&target));
                    }
                    "w:commentRangeStart" | "w:commentRangeEnd" => {
                        if let Some(comment) = self.comment_for(&attributes) {
                            let start = name == "w:commentRangeStart";
                            if start {
                                self.ranged_comments.push(comment);
                            }
                            paragraph.runs.push(Run {
                                style: None,
                                properties: None,
                                link: None,
                                revision: None,
                                content: if start {
                                    Inline::CommentStart(comment)
                                } else {
                                    Inline::CommentEnd(comment)
                                },
                            });
                        }
                    }
                    "w:ins" if !self_closing => {
                        revision = Some(self.push_revision(RevisionKind::Insertion, &attributes));
                    }
                    "w:del" if !self_closing => {
                        revision = Some(self.push_revision(RevisionKind::Deletion, &attributes));
                    }
                    "w:fldSimple" if !self_closing => {
                        let instr = attribute(&attributes, "w:instr").unwrap_or("");
                        let (target, page) = self.field_meaning(instr, &mut paragraph);
                        if page {
                            skip_element(reader, "w:fldSimple");
                        } else {
                            fields.link = target;
                        }
                    }
                    "m:oMathPara" | "m:oMath" if !self_closing => {
                        self.read_math(reader, name, &mut paragraph, link, revision);
                    }
                    "w:sdtPr" | "w:pPrChange" | "w:rPrChange" if !self_closing => {
                        skip_element(reader, name);
                    }
                    "mc:Fallback" if !self_closing => skip_element(reader, "mc:Fallback"),
                    _ => {}
                },
                XmlEvent::End { name } => match name {
                    "w:p" => break,
                    "w:hyperlink" => link = None,
                    "w:ins" | "w:del" => revision = None,
                    "w:fldSimple" => fields.link = None,
                    _ => {}
                },
                XmlEvent::Text(_) => {}
            }
        }
        // A framed paragraph in a header or footer (a page number beside the
        // text, say) is a box of its own there, drawn on each of its pages.
        if let (Some(frame), Some(part)) = (frame, self.page_part) {
            self.frame_paragraph(paragraph, frame, part);
            blocks.append(&mut self.pending_blocks);
            return section;
        }
        blocks.push(Block::Paragraph(paragraph));
        // What the paragraph's drawings hold that is not inline (a chart's
        // data) follows it.
        blocks.append(&mut self.pending_blocks);
        section
    }

    /// A framed paragraph as a text box on the pages of its header or
    /// footer, sized to its text unless the frame says otherwise.
    fn frame_paragraph(
        &mut self,
        paragraph: Paragraph,
        frame: Frame,
        part: crate::document::PagePart,
    ) {
        let size = paragraph
            .runs
            .first()
            .and_then(|run| self.document.effective_run(&paragraph, run).size)
            .unwrap_or(11.0);
        // Page fields show a number or two; text runs show themselves.
        let characters: usize = paragraph
            .runs
            .iter()
            .map(|run| match run.content {
                Inline::Text(span) => self.document.text(span).chars().count(),
                Inline::PageNumber | Inline::PageCount => 3,
                _ => 1,
            })
            .sum();
        let width = frame
            .width
            .unwrap_or(characters.max(1) as f32 * size * 0.6 + 8.0);
        let height = frame.height.unwrap_or(size * 1.3 + 8.0);
        let index = self.document.floating.len();
        self.document.floating.push(FloatingObject {
            page: self.page,
            x: frame.x,
            y: frame.y,
            width,
            height,
            content: FloatingContent::TextBox {
                blocks: vec![Block::Paragraph(paragraph)],
                fill: None,
                line: None,
                geometry: Default::default(),
                flip: (false, false),
                ends: (None, None),
            },
            follows_text: false,
            wrap: crate::document::TextWrap::None,
            repeats: Some(part),
        });
        self.pending_floating.push((
            index,
            frame.horizontal.unwrap_or(AnchorBase::Margin),
            frame.vertical.unwrap_or(AnchorBase::Line),
        ));
        self.floating_layouts.push((
            index,
            DrawingLayout {
                align: frame.align,
                ..DrawingLayout::default()
            },
        ));
    }

    /// What a field instruction means for the runs that follow: a link
    /// target, or a page field (which becomes a run of its own here).
    fn field_meaning(
        &mut self,
        instruction: &str,
        paragraph: &mut Paragraph,
    ) -> (Option<u32>, bool) {
        let mut words = instruction.split_whitespace();
        match words.next() {
            Some("HYPERLINK") => {
                let mut is_bookmark = false;
                let mut target = "";
                for word in words {
                    if word == "\\l" {
                        is_bookmark = true;
                    } else if !word.starts_with('\\') && target.is_empty() {
                        target = word.trim_matches('"');
                    }
                }
                if target.is_empty() {
                    return (None, false);
                }
                let link = if is_bookmark {
                    self.document.intern_link(&format!("#{target}"))
                } else {
                    self.document.intern_link(target)
                };
                (Some(link), false)
            }
            Some("PAGE") => {
                paragraph.runs.push(plain_run(Inline::PageNumber));
                (None, true)
            }
            Some("NUMPAGES") | Some("SECTIONPAGES") => {
                paragraph.runs.push(plain_run(Inline::PageCount));
                (None, true)
            }
            _ => (None, false),
        }
    }

    fn push_revision(&mut self, kind: RevisionKind, attributes: &[(&str, Cow<'_, str>)]) -> u32 {
        let author = attribute(attributes, "w:author").map(str::to_string);
        let date = attribute(attributes, "w:date").map(str::to_string);
        self.document.push_revision(Revision { kind, author, date })
    }

    /// One `w:r`: its properties, then each content element as a run of
    /// the paragraph. Field characters move the paragraph's field state as
    /// they come (a whole field may sit in one run).
    #[inline(never)]
    fn read_run(
        &mut self,
        reader: &mut XmlReader<'_>,
        paragraph: &mut Paragraph,
        link: Option<u32>,
        revision: Option<u32>,
        fields: &mut FieldState,
    ) {
        let mut style: Option<StyleId> = None;
        let mut properties: Option<u32> = None;
        let mut hidden = fields.hidden();
        // A legacy form checkbox (`FORMCHECKBOX`): Word draws the box itself,
        // with no result text, so its state is kept as a box character.
        let mut checkbox: Option<bool> = None;
        let mut seen_checked = false;
        while let Some(event) = reader.next() {
            match event {
                XmlEvent::Start {
                    name,
                    attributes,
                    self_closing,
                } => match name {
                    "w:rPr" if !self_closing => {
                        let read = self.read_run_properties(reader);
                        style = read.style;
                        properties = self.document.intern_run_properties(read.properties);
                    }
                    "w:footnoteRef" | "w:endnoteRef" => self.after_note_mark = true,
                    "w:t" | "w:delText" if !self_closing => {
                        let mut span = self.read_text_element(reader, name);
                        if self.after_note_mark {
                            self.after_note_mark = false;
                            if self.document.text(span).starts_with(' ') {
                                span.start += 1;
                            }
                        }
                        if !hidden && span.start != span.end {
                            paragraph.runs.push(Run {
                                style,
                                properties,
                                link,
                                revision,
                                content: Inline::Text(span),
                            });
                        }
                    }
                    "w:instrText" if !self_closing => {
                        let span = self.read_text_element(reader, name);
                        fields.instruction.push_str(self.document.text(span));
                    }
                    "w:fldChar" => {
                        match attribute(&attributes, "w:fldCharType") {
                            Some("begin") => {
                                fields.field = Field::Instruction;
                                fields.instruction.clear();
                                fields.page_field = false;
                            }
                            Some("separate") => {
                                fields.field = Field::Result;
                                let (target, page) =
                                    self.field_meaning(&fields.instruction, paragraph);
                                fields.link = target;
                                fields.page_field = page;
                            }
                            Some("end") => {
                                if fields.field == Field::Instruction {
                                    // No result: a bare page field still counts.
                                    self.field_meaning(&fields.instruction, paragraph);
                                }
                                fields.field = Field::None;
                                fields.link = None;
                                fields.page_field = false;
                            }
                            _ => {}
                        }
                        hidden = fields.hidden();
                    }
                    "w:tab" | "w:ptab" if !hidden => {
                        paragraph.runs.push(Run {
                            style,
                            properties,
                            link,
                            revision,
                            content: Inline::Tab,
                        });
                    }
                    "w:br" if !hidden => {
                        let content = match attribute(&attributes, "w:type") {
                            Some("page") => {
                                self.page += 1;
                                Inline::PageBreak
                            }
                            Some("column") => Inline::ColumnBreak,
                            _ => Inline::LineBreak,
                        };
                        paragraph.runs.push(Run {
                            style,
                            properties,
                            link,
                            revision,
                            content,
                        });
                    }
                    "w:cr" if !hidden => {
                        paragraph.runs.push(Run {
                            style,
                            properties,
                            link,
                            revision,
                            content: Inline::LineBreak,
                        });
                    }
                    "w:noBreakHyphen" if !hidden => {
                        let span = self.document.push_text("\u{2011}");
                        paragraph.runs.push(Run {
                            style,
                            properties,
                            link,
                            revision,
                            content: Inline::Text(span),
                        });
                    }
                    "w:sym" if !hidden => {
                        if let Some(text) = symbol_text(attribute(&attributes, "w:char")) {
                            let span = self.document.push_text(&text);
                            paragraph.runs.push(Run {
                                style,
                                properties,
                                link,
                                revision,
                                content: Inline::Text(span),
                            });
                        }
                    }
                    // A comment without a range is on the point it is made at.
                    "w:commentReference" => {
                        if let Some(comment) = self.comment_for(&attributes)
                            && !self.ranged_comments.contains(&comment)
                        {
                            for content in
                                [Inline::CommentStart(comment), Inline::CommentEnd(comment)]
                            {
                                paragraph.runs.push(Run {
                                    style: None,
                                    properties: None,
                                    link: None,
                                    revision: None,
                                    content,
                                });
                            }
                        }
                    }
                    "w:footnoteReference" | "w:endnoteReference" if !hidden => {
                        let id: i64 = attribute(&attributes, "w:id")
                            .and_then(|value| value.parse().ok())
                            .unwrap_or(-1);
                        if let Some((_, _, note)) =
                            self.footnotes.iter().find(|(endnote, note_id, _)| {
                                // Footnotes and endnotes number apart.
                                *endnote == (name == "w:endnoteReference") && *note_id == id
                            })
                        {
                            paragraph.runs.push(Run {
                                style,
                                properties,
                                link,
                                revision,
                                content: Inline::Footnote(*note),
                            });
                        }
                    }
                    "w:drawing" if !self_closing => {
                        for content in self.read_drawing(reader) {
                            match content {
                                Drawn::Image(image) => {
                                    let id = self.document.push_image(image);
                                    paragraph.runs.push(Run {
                                        style,
                                        properties,
                                        link,
                                        revision,
                                        content: Inline::Image(id),
                                    });
                                }
                                Drawn::Blocks(mut drawn) => self.pending_blocks.append(&mut drawn),
                                Drawn::Floating(mut object, horizontal, vertical) => {
                                    let index = self.document.floating.len();
                                    object.repeats = self.page_part;
                                    self.floating_layouts.push((index, self.drawing_layout));
                                    // One placed from its paragraph moves with
                                    // the text: anchored here by a run.
                                    if vertical == AnchorBase::Line && self.anchoring {
                                        object.follows_text = true;
                                        paragraph.runs.push(Run {
                                            style,
                                            properties,
                                            link,
                                            revision,
                                            content: Inline::Anchor(index as crate::document::Id),
                                        });
                                    }
                                    self.pending_floating.push((index, horizontal, vertical));
                                    self.document.floating.push(object);
                                }
                            }
                        }
                    }
                    "w:checkBox" if !self_closing => checkbox = Some(false),
                    // `w:checked` (the state) overrides `w:default` (the initial one).
                    "w:default" | "w:checked" if checkbox.is_some() => {
                        let on = !matches!(attribute(&attributes, "w:val"), Some("0" | "false"));
                        if name == "w:checked" || !seen_checked {
                            checkbox = Some(on);
                        }
                        seen_checked |= name == "w:checked";
                    }
                    "w:pict" | "w:object" if !self_closing => {
                        match self.read_pict(reader, name) {
                            Pict::Rule(line) => {
                                // A horizontal rule: the paragraph's bottom border.
                                let mut own = self.document.paragraph_properties(paragraph);
                                own.border = Some(crate::document::ParagraphBorder {
                                    top: false,
                                    bottom: true,
                                    left: false,
                                    right: false,
                                    line,
                                });
                                paragraph.properties =
                                    self.document.intern_paragraph_properties(own);
                            }
                            Pict::Image(image) => {
                                let id = self.document.push_image(image);
                                paragraph.runs.push(Run {
                                    style,
                                    properties,
                                    link,
                                    revision,
                                    content: Inline::Image(id),
                                });
                            }
                            Pict::Blocks(mut blocks) => self.pending_blocks.append(&mut blocks),
                            Pict::None => {}
                        }
                    }
                    "mc:Fallback" | "w:rPrChange" if !self_closing => {
                        skip_element(reader, name);
                    }
                    _ => {}
                },
                XmlEvent::End { name: "w:checkBox" } => {
                    if let Some(checked) = checkbox.take() {
                        let span =
                            self.document
                                .push_text(if checked { "\u{2612}" } else { "\u{2610}" });
                        paragraph.runs.push(Run {
                            style,
                            properties,
                            link,
                            revision,
                            content: Inline::Text(span),
                        });
                    }
                }
                XmlEvent::End { name: "w:r" } => break,
                _ => {}
            }
        }
    }

    /// The text of a `w:t`-like element, pushed to the arena.
    fn read_text_element(&mut self, reader: &mut XmlReader<'_>, element: &str) -> Span {
        let start = self.document.text.len() as u32;
        while let Some(event) = reader.next() {
            match event {
                XmlEvent::Text(piece) => self.document.text.push_str(&piece),
                XmlEvent::End { name } if name == element => break,
                _ => {}
            }
        }
        Span {
            start,
            end: self.document.text.len() as u32,
        }
    }

    /// Office Math comes out as its text, one run.
    fn read_math(
        &mut self,
        reader: &mut XmlReader<'_>,
        element: &str,
        paragraph: &mut Paragraph,
        link: Option<u32>,
        revision: Option<u32>,
    ) {
        let start = self.document.text.len() as u32;
        let mut depth = 1usize;
        let mut in_text = false;
        while let Some(event) = reader.next() {
            match event {
                XmlEvent::Start {
                    name, self_closing, ..
                } => {
                    if name == "m:t" && !self_closing {
                        in_text = true;
                    }
                    if !self_closing {
                        depth += 1;
                    }
                }
                XmlEvent::End { name } => {
                    if name == "m:t" {
                        in_text = false;
                    }
                    depth -= 1;
                    if depth == 0 || name == element {
                        break;
                    }
                }
                XmlEvent::Text(piece) if in_text => self.document.text.push_str(&piece),
                XmlEvent::Text(_) => {}
            }
        }
        let end = self.document.text.len() as u32;
        if start != end {
            paragraph.runs.push(Run {
                style: None,
                properties: None,
                link,
                revision,
                content: Inline::Text(Span { start, end }),
            });
        }
    }

    // ----- properties -----

    #[inline(never)]
    fn read_paragraph_properties(&mut self, reader: &mut XmlReader<'_>) -> ParagraphHeader {
        let mut header = ParagraphHeader::default();
        let mut first_relative: Option<f32> = None;
        let mut level: u8 = 0;
        let mut num_id: Option<i64> = None;
        while let Some(event) = reader.next() {
            match event {
                XmlEvent::Start {
                    name,
                    attributes,
                    self_closing,
                } => {
                    let value = attribute(&attributes, "w:val");
                    match name {
                        "w:framePr" => {
                            let points =
                                |key: &str| attribute(&attributes, key).and_then(twips_to_points);
                            let base = |key: &str| match attribute(&attributes, key) {
                                Some("page") => Some(AnchorBase::Page),
                                Some("text") => Some(AnchorBase::Line),
                                Some(_) => Some(AnchorBase::Margin),
                                None => None,
                            };
                            let align = |key: &str| match attribute(&attributes, key) {
                                Some("center") => Some(0.5),
                                Some("right" | "bottom" | "outside") => Some(1.0),
                                Some("left" | "top" | "inside") => Some(0.0),
                                _ => None,
                            };
                            header.frame = Some(Frame {
                                horizontal: base("w:hAnchor"),
                                vertical: base("w:vAnchor"),
                                x: points("w:x").unwrap_or(0.0),
                                y: points("w:y").unwrap_or(0.0),
                                align: (align("w:xAlign"), align("w:yAlign")),
                                width: points("w:w").filter(|width| *width > 0.0),
                                height: points("w:h").filter(|height| *height > 0.0),
                            });
                        }
                        "w:pStyle" => {
                            header.style =
                                value.and_then(|id| self.paragraph_styles.get(id)).copied();
                        }
                        "w:ilvl" => level = value.and_then(|v| v.parse().ok()).unwrap_or(0),
                        "w:numId" => num_id = value.and_then(|v| v.parse().ok()),
                        "w:jc" => {
                            header.properties.alignment = match value {
                                Some("center") => Some(Alignment::Center),
                                Some("right") | Some("end") => Some(Alignment::Right),
                                Some("both") | Some("distribute") => Some(Alignment::Justify),
                                Some(_) => Some(Alignment::Left),
                                None => None,
                            };
                        }
                        "w:ind" => {
                            let left = attribute(&attributes, "w:left")
                                .or_else(|| attribute(&attributes, "w:start"));
                            let right = attribute(&attributes, "w:right")
                                .or_else(|| attribute(&attributes, "w:end"));
                            if let Some(points) = left.and_then(twips_to_points) {
                                header.properties.left_indent = Some(points);
                            }
                            if let Some(points) = right.and_then(twips_to_points) {
                                header.properties.right_indent = Some(points);
                            }
                            // Word measures the first line from the left indent;
                            // the model from the margin (resolved when the
                            // properties close, once the left indent is known).
                            if let Some(points) =
                                attribute(&attributes, "w:firstLine").and_then(twips_to_points)
                            {
                                first_relative = Some(points);
                            }
                            if let Some(points) =
                                attribute(&attributes, "w:hanging").and_then(twips_to_points)
                            {
                                first_relative = Some(-points);
                            }
                        }
                        "w:tabs" if !self_closing => {
                            let tabs = read_tabs(reader);
                            if !tabs.is_empty() {
                                header.properties.tabs = Some(self.document.intern_tabs(tabs));
                            }
                        }
                        "w:pBdr" if !self_closing => {
                            header.properties.border = read_paragraph_border(reader);
                        }
                        "w:pageBreakBefore" => {
                            header.properties.page_break_before = Some(toggle(value));
                        }
                        "w:contextualSpacing" => {
                            header.properties.contextual_spacing = Some(toggle(value));
                        }
                        "w:spacing" => {
                            if let Some(points) =
                                attribute(&attributes, "w:before").and_then(twips_to_points)
                            {
                                header.properties.space_before = Some(points);
                            }
                            if let Some(points) =
                                attribute(&attributes, "w:after").and_then(twips_to_points)
                            {
                                header.properties.space_after = Some(points);
                            }
                            if let Some(line) = attribute(&attributes, "w:line")
                                .and_then(|line| line.parse::<f32>().ok())
                            {
                                header.properties.line_spacing =
                                    Some(match attribute(&attributes, "w:lineRule") {
                                        Some("exact") => LineSpacing::Exact(line / 20.0),
                                        Some("atLeast") => LineSpacing::Minimum(line / 20.0),
                                        _ => LineSpacing::Relative(line / 240.0),
                                    });
                            }
                        }
                        "w:keepNext" => header.properties.keep_with_next = Some(toggle(value)),
                        "w:keepLines" => {
                            header.properties.keep_lines_together = Some(toggle(value))
                        }
                        "w:widowControl" => header.properties.widow_control = Some(toggle(value)),
                        "w:outlineLvl" => {
                            let outline: Option<u8> = value.and_then(|v| v.parse().ok());
                            header.properties.outline_level = outline.filter(|level| *level < 9);
                        }
                        "w:shd" => {
                            header.properties.background =
                                attribute(&attributes, "w:fill").and_then(parse_color);
                        }
                        "w:rPr" if !self_closing => {
                            let run = self.read_run_properties(reader);
                            header.mark = run.properties;
                        }
                        "w:sectPr" if !self_closing => {
                            header.section = Some(self.read_section_properties(reader));
                        }
                        "w:sectPr" => header.section = Some(SectionHeader::default()),
                        "w:pPrChange" if !self_closing => skip_element(reader, name),
                        _ => {}
                    }
                }
                XmlEvent::End { name: "w:pPr" } => {
                    if let Some(relative) = first_relative {
                        let left = header.properties.left_indent.or_else(|| {
                            header.style.and_then(|style| {
                                self.document.paragraph_style_properties(style).left_indent
                            })
                        });
                        header.properties.first_line_indent = Some(left.unwrap_or(0.0) + relative);
                    }
                    break;
                }
                _ => {}
            }
        }
        header.numbering = num_id.map(|id| (id, level));
        header
    }

    fn list_item(&mut self, num_id: i64, level: u8) -> Option<ListItem> {
        let (_, style, start_override) = *self.nums.iter().find(|(id, _, _)| *id == num_id)?;
        let starts_list = !self.seen_nums.contains(&num_id);
        if starts_list {
            self.seen_nums.push(num_id);
        }
        let level_start = self
            .list_starts
            .get(style)
            .and_then(|starts| starts.get(usize::from(level)))
            .copied()
            .unwrap_or(1);
        let start = match start_override {
            Some(start) if level == 0 => start,
            _ => level_start,
        };
        Some(ListItem {
            style,
            level,
            starts_list,
            start,
        })
    }

    #[inline(never)]
    fn read_run_properties(&mut self, reader: &mut XmlReader<'_>) -> RunRead {
        let mut read = RunRead::default();
        while let Some(event) = reader.next() {
            match event {
                XmlEvent::Start {
                    name,
                    attributes,
                    self_closing,
                } => {
                    let value = attribute(&attributes, "w:val");
                    match name {
                        "w:rStyle" => {
                            read.style =
                                value.and_then(|id| self.character_styles.get(id)).copied();
                        }
                        "w:rFonts" => {
                            // A theme font wins over a named one, as in Word. The complex-script
                            // font (`w:cs`) is not the Latin text's, so it is left out.
                            let theme = |key: &str| {
                                attribute(&attributes, key).and_then(|slot| {
                                    let index = if slot.starts_with("major") { 0 } else { 1 };
                                    self.theme_fonts[index].clone()
                                })
                            };
                            let font = theme("w:asciiTheme")
                                .or_else(|| attribute(&attributes, "w:ascii").map(str::to_string))
                                .or_else(|| theme("w:hAnsiTheme"))
                                .or_else(|| attribute(&attributes, "w:hAnsi").map(str::to_string));
                            if let Some(font) = font {
                                read.properties.font = Some(self.document.intern_string(&font));
                            }
                        }
                        "w:sz" => {
                            read.properties.size = value
                                .and_then(|v| v.parse::<f32>().ok())
                                .map(|half| half / 2.0);
                        }
                        "w:b" => read.properties.bold = Some(toggle(value)),
                        "w:i" => read.properties.italic = Some(toggle(value)),
                        "w:u" => read.properties.underline = Some(!matches!(value, Some("none"))),
                        "w:strike" | "w:dstrike" => read.properties.strike = Some(toggle(value)),
                        "w:color" => read.properties.color = value.and_then(parse_color),
                        "w:highlight" => {
                            read.properties.highlight = value.and_then(highlight_color)
                        }
                        "w:shd" => {
                            if read.properties.highlight.is_none() {
                                read.properties.highlight =
                                    attribute(&attributes, "w:fill").and_then(parse_color);
                            }
                        }
                        "w:vertAlign" => {
                            read.properties.baseline = match value {
                                Some("superscript") => Some(Baseline::Superscript),
                                Some("subscript") => Some(Baseline::Subscript),
                                _ => None,
                            };
                        }
                        "w:vanish" => read.properties.hidden = Some(toggle(value)),
                        "w:position" => {
                            read.properties.shift = value
                                .and_then(|v| v.parse::<f32>().ok())
                                .map(|half| half / 2.0)
                                .filter(|shift| *shift != 0.0);
                        }
                        "w:caps" => {
                            read.properties.caps =
                                if toggle(value) { Some(Caps::All) } else { None };
                        }
                        "w:smallCaps" => {
                            if toggle(value) {
                                read.properties.caps = Some(Caps::Small);
                            }
                        }
                        "w:lang" => {
                            if let Some(language) = value {
                                read.properties.language =
                                    Some(self.document.intern_string(language));
                            }
                        }
                        "w:rPrChange" if !self_closing => skip_element(reader, name),
                        _ => {}
                    }
                }
                XmlEvent::End { name: "w:rPr" } => break,
                _ => {}
            }
        }
        read
    }

    fn read_section_properties(&mut self, reader: &mut XmlReader<'_>) -> SectionHeader {
        let mut header = SectionHeader::default();
        while let Some(event) = reader.next() {
            match event {
                XmlEvent::Start {
                    name, attributes, ..
                } => match name {
                    "w:headerReference" | "w:footerReference" => {
                        let kind = attribute(&attributes, "w:type").unwrap_or("default");
                        let id = attribute(&attributes, "r:id").unwrap_or("");
                        header.references.push((
                            name == "w:headerReference",
                            kind.to_string(),
                            id.to_string(),
                        ));
                    }
                    "w:pgSz" => {
                        if let Some(width) = attribute(&attributes, "w:w").and_then(twips_to_points)
                        {
                            header.page.width = width;
                        }
                        if let Some(height) =
                            attribute(&attributes, "w:h").and_then(twips_to_points)
                        {
                            header.page.height = height;
                        }
                    }
                    "w:pgNumType" => {
                        header.page.page_number_start =
                            attribute(&attributes, "w:start").and_then(|start| start.parse().ok());
                    }
                    "w:pgMar" => {
                        let fields: [(&str, &mut f32); 6] = [
                            ("w:top", &mut header.page.margin_top),
                            ("w:bottom", &mut header.page.margin_bottom),
                            ("w:left", &mut header.page.margin_left),
                            ("w:right", &mut header.page.margin_right),
                            ("w:header", &mut header.page.header_distance),
                            ("w:footer", &mut header.page.footer_distance),
                        ];
                        for (key, slot) in fields {
                            if let Some(points) =
                                attribute(&attributes, key).and_then(twips_to_points)
                            {
                                *slot = points.abs();
                            }
                        }
                    }
                    "w:cols" => {
                        header.columns = attribute(&attributes, "w:num")
                            .and_then(|num| num.parse().ok())
                            .unwrap_or(1);
                        header.column_gap =
                            attribute(&attributes, "w:space").and_then(twips_to_points);
                    }
                    // A column of its own width (in `w:cols` without equal widths).
                    "w:col" => {
                        if let Some(width) = attribute(&attributes, "w:w").and_then(twips_to_points)
                        {
                            let gap = attribute(&attributes, "w:space")
                                .and_then(twips_to_points)
                                .unwrap_or(0.0);
                            header.column_widths.push((width, gap));
                        }
                    }
                    "w:type" => {
                        header.continuous = attribute(&attributes, "w:val") == Some("continuous");
                    }
                    "w:titlePg" => header.title_page = toggle(attribute(&attributes, "w:val")),
                    "w:sectPrChange" => skip_element(reader, name),
                    _ => {}
                },
                XmlEvent::End { name: "w:sectPr" } => break,
                _ => {}
            }
        }
        header
    }

    // ----- tables -----

    #[inline(never)]
    fn read_table(&mut self, reader: &mut XmlReader<'_>) -> Table {
        let mut table = Table::default();
        let mut table_style: Option<String> = None;
        let mut own_borders = TableSides::default();
        // Per row, per cell as read: (cell, grid span, vertical merge)
        let mut rows: Vec<(Row, Vec<(u32, VerticalMerge)>)> = Vec::new();
        // Per row, the grid columns its cells occupy: [skipped before, end).
        let mut occupied: Vec<(usize, usize)> = Vec::new();
        let mut header_rows = 0u32;
        let mut leading = true;
        while let Some(event) = reader.next() {
            match event {
                XmlEvent::Start {
                    name,
                    attributes,
                    self_closing,
                } => match name {
                    "w:gridCol" => {
                        let width = attribute(&attributes, "w:w")
                            .and_then(twips_to_points)
                            .unwrap_or(0.0);
                        table.columns.push(width);
                    }
                    "w:tblPr" if !self_closing => {
                        let (style, borders) = read_table_properties(reader);
                        table_style = style;
                        own_borders = borders;
                    }
                    "w:tr" if !self_closing => {
                        let (mut row, mut spans, is_header, before) = self.read_row(reader);
                        if is_header && leading {
                            header_rows += 1;
                        } else {
                            leading = false;
                        }
                        // Grid columns a row skips (`w:gridBefore`) hold no cell.
                        for _ in 0..before {
                            row.cells.insert(0, ghost_cell());
                            spans.insert(0, (1, VerticalMerge::None));
                        }
                        let end = spans.iter().map(|(span, _)| *span as usize).sum();
                        occupied.push((before, end));
                        rows.push((row, spans));
                    }
                    _ => {}
                },
                XmlEvent::End { name: "w:tbl" } => break,
                _ => {}
            }
        }
        table.header_rows = header_rows;
        table.rows = resolve_merges(rows);
        // A row may stop short of the grid (`w:gridAfter`, or simply fewer
        // cells): the rest is empty space, not cells.
        let width = table
            .rows
            .iter()
            .map(|row| row.cells.len())
            .max()
            .unwrap_or(0);
        for row in &mut table.rows {
            row.cells
                .resize_with(width.max(row.cells.len()), ghost_cell);
        }
        // The table's own borders over its style's (and that style's bases');
        // with neither, Word draws none.
        let mut sides = own_borders;
        let mut next = table_style;
        for _ in 0..16 {
            let Some(id) = next else { break };
            let Some((style_sides, based_on)) = self.table_style_borders.get(&id) else {
                break;
            };
            sides = sides.or(*style_sides);
            next = based_on.clone();
        }
        let borders = sides.resolve();
        table.borders = Some(borders);
        table.cell_margins = Some(sides.cell_margins());
        table.alignment = sides.alignment;
        table.indent = sides.indent;
        outline_ghost_cells(&mut table.rows, &occupied, borders);
        // A grid may declare more columns than any row reaches (Google Docs and
        // some templates pad tblGrid); Word lays out only the occupied ones, so
        // the unused trailing columns are dropped rather than rendered empty.
        let used = table
            .rows
            .iter()
            .map(|row| row.cells.len())
            .max()
            .unwrap_or(0);
        if used > 0 && used < table.columns.len() {
            table.columns.truncate(used);
        }
        table
    }

    /// A row's cells as read, with each cell's grid span and vertical
    /// merge, and whether the row repeats as a header.
    fn read_row(
        &mut self,
        reader: &mut XmlReader<'_>,
    ) -> (Row, Vec<(u32, VerticalMerge)>, bool, usize) {
        let mut row = Row::default();
        let mut spans = Vec::new();
        let mut is_header = false;
        let mut before = 0usize;
        while let Some(event) = reader.next() {
            match event {
                XmlEvent::Start {
                    name,
                    attributes,
                    self_closing,
                } => match name {
                    "w:tblHeader" => is_header = toggle(attribute(&attributes, "w:val")),
                    "w:gridBefore" => {
                        before = attribute(&attributes, "w:val")
                            .and_then(|value| value.parse().ok())
                            .unwrap_or(0);
                    }
                    "w:trHeight" => {
                        row.height = attribute(&attributes, "w:val").and_then(twips_to_points);
                    }
                    "w:tblPrEx" if !self_closing => skip_element(reader, name),
                    "w:tc" if !self_closing => {
                        let (cell, span, merge) = self.read_cell(reader);
                        row.cells.push(cell);
                        spans.push((span, merge));
                    }
                    _ => {}
                },
                XmlEvent::End { name: "w:tr" } => break,
                _ => {}
            }
        }
        (row, spans, is_header, before)
    }

    fn read_cell(&mut self, reader: &mut XmlReader<'_>) -> (Cell, u32, VerticalMerge) {
        let mut cell = Cell {
            column_span: 1,
            row_span: 1,
            ..Cell::default()
        };
        let mut span = 1u32;
        let mut merge = VerticalMerge::None;
        while let Some(event) = reader.next() {
            match event {
                XmlEvent::Start {
                    name, self_closing, ..
                } => match name {
                    "w:tcPr" if !self_closing => {
                        let read = read_cell_properties(reader);
                        span = read.span;
                        merge = read.merge;
                        cell.background = read.background;
                        cell.borders = read.borders;
                        cell.vertical_alignment = read.vertical;
                        cell.margins = read.margins;
                    }
                    "w:p" => {
                        if self_closing {
                            cell.blocks.push(Block::Paragraph(Paragraph::default()));
                        } else {
                            self.read_paragraph(reader, &mut cell.blocks);
                        }
                    }
                    "w:tbl" if !self_closing => {
                        let table = self.read_table(reader);
                        cell.blocks.push(Block::Table(table));
                    }
                    "w:sdtPr" if !self_closing => skip_element(reader, name),
                    "mc:Fallback" if !self_closing => skip_element(reader, name),
                    _ => {}
                },
                XmlEvent::End { name: "w:tc" } => break,
                _ => {}
            }
        }
        (cell, span, merge)
    }

    // ----- drawings -----

    /// A drawing: a picture, inline or anchored, or text boxes (floating
    /// objects) — every text box of a group, placed where the group puts it.
    /// A group without text yields its first picture.
    #[inline(never)]
    fn read_drawing(&mut self, reader: &mut XmlReader<'_>) -> Vec<Drawn> {
        let mut width = 0.0f32;
        let mut height = 0.0f32;
        let mut description: Option<String> = None;
        let mut anchored = false;
        let mut wrap = crate::document::TextWrap::Around;
        self.drawing_layout = DrawingLayout::default();
        let mut horizontal = Anchor {
            from: AnchorBase::Margin,
            offset: 0.0,
        };
        let mut vertical = Anchor {
            from: AnchorBase::Line,
            offset: 0.0,
        };
        let mut position_axis: Option<bool> = None;
        let mut size_axis: Option<(bool, bool)> = None;
        let mut media: Option<MediaId> = None;
        let mut shapes: Vec<ShapeRead> = Vec::new();
        let mut chart: Option<String> = None;
        let mut crop: Option<[f32; 4]> = None;
        // The shape being read (a `wps:wsp`).
        let mut shape = ShapeRead::default();
        let mut shape_frame: Option<[f32; 4]> = None;
        // The group's frame and its children's coordinate space (EMU).
        let mut group: Option<([f32; 4], [f32; 4])> = None;
        let mut in_group_properties = false;
        let mut frame = [0.0f32; 4];
        let mut child_frame = [0.0f32; 4];
        let mut fill: Option<Color> = None;
        let mut in_shape_properties = false;
        // A shape's own fill (in spPr, outside its outline) or, failing that,
        // its style's fill reference; theme colours resolve through the theme.
        let mut in_line = false;
        let mut no_fill = false;
        let mut in_fill_ref = false;
        let mut style_fill: Option<Color> = None;
        // Its outline: its own (`a:ln`) or its style's line reference.
        let mut line_width: Option<f32> = None;
        let mut line_color: Option<Color> = None;
        let mut no_line = false;
        let mut in_line_ref = false;
        let mut style_line: Option<Color> = None;
        let mut scheme: Option<PendingSchemeColor> = None;
        while let Some(event) = reader.next() {
            match event {
                XmlEvent::Start {
                    name,
                    attributes,
                    self_closing,
                } => match name {
                    "wp:anchor" => anchored = true,
                    "wp:wrapNone" => wrap = crate::document::TextWrap::None,
                    "wp:wrapTopAndBottom" => wrap = crate::document::TextWrap::TopAndBottom,
                    "wp:extent" => {
                        width = attribute(&attributes, "cx")
                            .and_then(emu_to_points)
                            .unwrap_or(0.0);
                        height = attribute(&attributes, "cy")
                            .and_then(emu_to_points)
                            .unwrap_or(0.0);
                    }
                    "wp:docPr" => {
                        description = attribute(&attributes, "descr")
                            .filter(|text| !text.is_empty())
                            .map(str::to_string);
                    }
                    "wp:positionH" | "wp:positionV" => {
                        let is_horizontal = name == "wp:positionH";
                        let from = match attribute(&attributes, "relativeFrom") {
                            Some("page") => AnchorBase::Page,
                            Some("line") | Some("paragraph") if !is_horizontal => AnchorBase::Line,
                            _ => AnchorBase::Margin,
                        };
                        if is_horizontal {
                            horizontal.from = from;
                        } else {
                            vertical.from = from;
                        }
                        position_axis = Some(is_horizontal);
                    }
                    "wp:posOffset" if !self_closing => {
                        let text = read_element_text(reader, name);
                        let offset = emu_to_points(&text).unwrap_or(0.0);
                        match position_axis {
                            Some(true) => horizontal.offset = offset,
                            Some(false) => vertical.offset = offset,
                            None => {}
                        }
                    }
                    // An offset as a share of what it is measured from, in
                    // thousandths of a percent (Word 2010's relative position).
                    "wp14:pctPosHOffset" | "wp14:pctPosVOffset" if !self_closing => {
                        let share = read_element_text(reader, name)
                            .trim()
                            .parse::<f32>()
                            .ok()
                            .map(|value| value / 100_000.0);
                        if name == "wp14:pctPosHOffset" {
                            self.drawing_layout.share.0 = share;
                        } else {
                            self.drawing_layout.share.1 = share;
                        }
                    }
                    // Aligned in what it is placed from, rather than offset.
                    "wp:align" if !self_closing => {
                        let factor = match read_element_text(reader, name).trim() {
                            "center" => Some(0.5),
                            "right" | "bottom" | "outside" => Some(1.0),
                            "left" | "top" | "inside" => Some(0.0),
                            _ => None,
                        };
                        match position_axis {
                            Some(true) => self.drawing_layout.align.0 = factor,
                            Some(false) => self.drawing_layout.align.1 = factor,
                            None => {}
                        }
                    }
                    // A size as a share of the page or its margins.
                    "wp14:sizeRelH" | "wp14:sizeRelV" if !self_closing => {
                        let of_page = attribute(&attributes, "relativeFrom") == Some("page");
                        size_axis = Some((name == "wp14:sizeRelH", of_page));
                    }
                    "wp14:pctWidth" | "wp14:pctHeight" if !self_closing => {
                        let share = read_element_text(reader, name)
                            .trim()
                            .parse::<f32>()
                            .ok()
                            .map(|value| value / 100_000.0)
                            .filter(|share| *share > 0.0);
                        if let (Some(share), Some((horizontal, of_page))) = (share, size_axis) {
                            if horizontal {
                                self.drawing_layout.size.0 = Some((share, of_page));
                            } else {
                                self.drawing_layout.size.1 = Some((share, of_page));
                            }
                        }
                    }
                    // A cropped picture: thousandths of a percent cut per edge.
                    "a:srcRect" => {
                        let edge = |key: &str| {
                            attribute(&attributes, key)
                                .and_then(|value| value.parse::<f32>().ok())
                                .map_or(0.0, |value| value / 100_000.0)
                        };
                        let edges = [edge("l"), edge("t"), edge("r"), edge("b")];
                        if edges.iter().any(|edge| *edge != 0.0) {
                            crop = Some(edges);
                        }
                    }
                    "c:chart" => {
                        chart = attribute(&attributes, "r:id").map(str::to_string);
                    }
                    // A drawing may offer its picture twice (a PDF or
                    // metafile, then a PNG); the common raster is kept.
                    "a:blip" => {
                        let raster = |document: &Document, id: MediaId| {
                            document
                                .media
                                .get(id)
                                .is_some_and(|file| is_raster(&file.name))
                        };
                        if media.is_none_or(|id| !raster(&self.document, id)) {
                            let offered = attribute(&attributes, "r:embed")
                                .and_then(|id| self.load_media(id));
                            if media.is_none()
                                || offered.is_some_and(|id| raster(&self.document, id))
                            {
                                media = offered.or(media);
                            }
                        }
                    }
                    "wps:spPr" if !self_closing => in_shape_properties = true,
                    "wps:wsp" if !self_closing => {
                        shape = ShapeRead::default();
                        shape_frame = None;
                        fill = None;
                        style_fill = None;
                        no_fill = false;
                        line_width = None;
                        line_color = None;
                        no_line = false;
                        style_line = None;
                    }
                    "a:headEnd" if in_line && in_shape_properties => {
                        shape.ends.0 = line_end(attribute(&attributes, "type"));
                    }
                    "a:tailEnd" if in_line && in_shape_properties => {
                        shape.ends.1 = line_end(attribute(&attributes, "type"));
                    }
                    // A custom outline: its path, in its own units.
                    "a:path" if in_shape_properties && !self_closing => {
                        let size = (
                            attribute(&attributes, "w").and_then(emu_to_points),
                            attribute(&attributes, "h").and_then(emu_to_points),
                        );
                        if let Some(path) = read_custom_path(reader, size) {
                            self.document.paths.push(path);
                            shape.geometry =
                                ShapeGeometry::Path((self.document.paths.len() - 1) as u32);
                        }
                    }
                    "a:prstGeom" if in_shape_properties => {
                        shape.geometry =
                            preset_geometry(attribute(&attributes, "prst").unwrap_or(""));
                    }
                    "a:xfrm" if in_shape_properties => {
                        let flag =
                            |key: &str| matches!(attribute(&attributes, key), Some("1" | "true"));
                        shape.flip = (flag("flipH"), flag("flipV"));
                    }
                    "wpg:grpSpPr" if !self_closing && group.is_none() => {
                        in_group_properties = true;
                    }
                    "a:off" | "a:ext" | "a:chOff" | "a:chExt"
                        if in_group_properties || in_shape_properties =>
                    {
                        let (first, second) = if name.ends_with("Off") || name.ends_with("off") {
                            ("x", "y")
                        } else {
                            ("cx", "cy")
                        };
                        let value = |key: &str| {
                            attribute(&attributes, key)
                                .and_then(|value| value.parse::<f32>().ok())
                                .unwrap_or(0.0)
                        };
                        let pair = [value(first), value(second)];
                        let (target, at) = match name {
                            "a:off" => (&mut frame, 0),
                            "a:ext" => (&mut frame, 2),
                            "a:chOff" => (&mut child_frame, 0),
                            _ => (&mut child_frame, 2),
                        };
                        target[at..at + 2].copy_from_slice(&pair);
                        if in_shape_properties && name == "a:ext" {
                            shape_frame = Some(frame);
                        }
                    }
                    "a:ln" => {
                        in_line = !self_closing;
                        if in_shape_properties {
                            line_width = attribute(&attributes, "w").and_then(emu_to_points);
                        }
                    }
                    "a:noFill" if in_shape_properties && in_line => no_line = true,
                    "a:noFill" if in_shape_properties && !in_line => no_fill = true,
                    "a:fillRef" if !self_closing => {
                        in_fill_ref = attribute(&attributes, "idx").is_some_and(|idx| idx != "0");
                    }
                    "a:lnRef" if !self_closing => {
                        in_line_ref = attribute(&attributes, "idx").is_some_and(|idx| idx != "0");
                    }
                    "a:srgbClr" | "a:prstClr"
                        if in_shape_properties || in_fill_ref || in_line_ref =>
                    {
                        let color = match name {
                            "a:prstClr" => {
                                preset_color(attribute(&attributes, "val").unwrap_or(""))
                            }
                            _ => attribute(&attributes, "val").and_then(parse_color),
                        };
                        let target = match (in_shape_properties, in_line) {
                            (true, true) => &mut line_color,
                            (true, false) => &mut fill,
                            _ if in_fill_ref => &mut style_fill,
                            _ => &mut style_line,
                        };
                        if target.is_none() {
                            *target = color;
                        }
                    }
                    "a:schemeClr" if in_shape_properties || in_fill_ref || in_line_ref => {
                        let target = match (in_shape_properties, in_line) {
                            (true, true) => ColorTarget::Line,
                            (true, false) => ColorTarget::Fill,
                            _ if in_fill_ref => ColorTarget::StyleFill,
                            _ => ColorTarget::StyleLine,
                        };
                        let name = attribute(&attributes, "val").unwrap_or("").to_string();
                        if self_closing {
                            let color = self.theme_color(&name, &[]);
                            let slot = match target {
                                ColorTarget::Fill => &mut fill,
                                ColorTarget::StyleFill => &mut style_fill,
                                ColorTarget::Line => &mut line_color,
                                ColorTarget::StyleLine => &mut style_line,
                            };
                            if slot.is_none() {
                                *slot = color;
                            }
                        } else {
                            scheme = Some((name, Vec::new(), target));
                        }
                    }
                    "a:lumMod" | "a:lumOff" | "a:shade" | "a:tint" if scheme.is_some() => {
                        if let (Some((_, modifiers, _)), Some(value)) = (
                            scheme.as_mut(),
                            attribute(&attributes, "val").and_then(|v| v.parse::<f32>().ok()),
                        ) {
                            modifiers.push((
                                name.trim_start_matches("a:").to_string(),
                                value / 100_000.0,
                            ));
                        }
                    }
                    "w:txbxContent" if !self_closing => {
                        let anchoring = std::mem::replace(&mut self.anchoring, false);
                        let blocks = self.read_blocks(reader, "w:txbxContent");
                        self.anchoring = anchoring;
                        shape.blocks.extend(blocks);
                    }
                    "mc:Fallback" if !self_closing => skip_element(reader, name),
                    _ => {}
                },
                XmlEvent::End { name } => match name {
                    "w:drawing" => break,
                    "wps:spPr" => in_shape_properties = false,
                    "wpg:grpSpPr" if in_group_properties => {
                        in_group_properties = false;
                        group = Some((frame, child_frame));
                    }
                    "wps:wsp" => {
                        let mut done = std::mem::take(&mut shape);
                        done.frame = shape_frame;
                        done.fill = if no_fill { None } else { fill.or(style_fill) };
                        let color = line_color.or(style_line);
                        done.line =
                            (!no_line && (color.is_some() || line_width.is_some())).then(|| {
                                Border {
                                    width: line_width.unwrap_or(0.75).max(0.25),
                                    color,
                                }
                            });
                        // A shape that draws nothing and holds no text leaves no trace.
                        if !done.blocks.is_empty() || done.fill.is_some() || done.line.is_some() {
                            shapes.push(done);
                        }
                    }
                    "a:ln" => in_line = false,
                    "a:fillRef" => in_fill_ref = false,
                    "a:lnRef" => in_line_ref = false,
                    "a:schemeClr" => {
                        if let Some((name, modifiers, target)) = scheme.take() {
                            let color = self.theme_color(&name, &modifiers);
                            let slot = match target {
                                ColorTarget::Fill => &mut fill,
                                ColorTarget::StyleFill => &mut style_fill,
                                ColorTarget::Line => &mut line_color,
                                ColorTarget::StyleLine => &mut style_line,
                            };
                            if slot.is_none() {
                                *slot = color;
                            }
                        }
                    }
                    _ => {}
                },
                XmlEvent::Text(_) => {}
            }
        }
        // A chart keeps its data: one floating on the page stays a chart;
        // one in the text line becomes its title and a table of its series.
        if let Some((title, chart)) = chart.and_then(|id| self.read_chart(&id)) {
            if anchored {
                return vec![Drawn::Floating(
                    FloatingObject {
                        page: self.page,
                        x: horizontal.offset,
                        y: vertical.offset,
                        width,
                        height,
                        content: FloatingContent::Chart(chart),
                        follows_text: false,
                        wrap,
                        repeats: None,
                    },
                    horizontal.from,
                    vertical.from,
                )];
            }
            return vec![Drawn::Blocks(self.chart_blocks(&title, &chart, width))];
        }
        if !shapes.is_empty() {
            // A child's frame maps from the group's child space onto the
            // drawing's extent; a lone shape fills the drawing.
            let place = |child: Option<[f32; 4]>| match (group, child) {
                (Some((_, space)), Some([x, y, w, h])) if space[2] > 0.0 && space[3] > 0.0 => {
                    let (scale_x, scale_y) = (width / space[2], height / space[3]);
                    [
                        (x - space[0]) * scale_x,
                        (y - space[1]) * scale_y,
                        w * scale_x,
                        h * scale_y,
                    ]
                }
                _ => [0.0, 0.0, width, height],
            };
            // A lone shape in the text line stays there; the shapes of an
            // inline group keep their places, apart from the text.
            let wrap = match (anchored, shapes.len()) {
                (true, _) => wrap,
                (false, 1) => crate::document::TextWrap::Inline,
                (false, _) => crate::document::TextWrap::TopAndBottom,
            };
            return shapes
                .into_iter()
                .map(|shape| {
                    let [x, y, box_width, box_height] = place(shape.frame);
                    Drawn::Floating(
                        FloatingObject {
                            page: self.page,
                            x: horizontal.offset + x,
                            y: vertical.offset + y,
                            width: box_width,
                            height: box_height,
                            content: FloatingContent::TextBox {
                                blocks: shape.blocks,
                                fill: shape.fill,
                                line: shape.line,
                                geometry: shape.geometry,
                                flip: shape.flip,
                                ends: shape.ends,
                            },
                            follows_text: false,
                            wrap,
                            repeats: None,
                        },
                        horizontal.from,
                        vertical.from,
                    )
                })
                .collect();
        }
        let Some(media) = media else {
            return Vec::new();
        };
        // A picture positioned on the page (both axes) is page furniture,
        // not part of the text flow; the writer puts floating objects
        // there.
        if anchored && horizontal.from == AnchorBase::Page && vertical.from == AnchorBase::Page {
            return vec![Drawn::Floating(
                FloatingObject {
                    page: self.page,
                    x: horizontal.offset,
                    y: vertical.offset,
                    width,
                    height,
                    content: FloatingContent::Image(media),
                    follows_text: false,
                    wrap,
                    repeats: None,
                },
                AnchorBase::Page,
                AnchorBase::Page,
            )];
        }
        let placement = if anchored {
            Placement::Floating {
                horizontal,
                vertical,
            }
        } else {
            Placement::Inline
        };
        vec![Drawn::Image(InlineImage {
            media,
            width,
            height,
            description,
            placement,
            crop,
        })]
    }

    /// A chart part's cached data: its title, and its series over their
    /// categories as a model chart.
    fn read_chart(&mut self, relationship_id: &str) -> Option<(String, Chart)> {
        use crate::document::ChartKind;
        let part = self.target(relationship_id)?;
        let text = self.part_text(&part)?;
        let mut reader = XmlReader::new(&text);
        let mut title = String::new();
        let mut kind: Option<ChartKind> = None;
        let mut in_title = false;
        // Per series: name, categories by index, values by index.
        let mut series: Vec<ChartSeries> = Vec::new();
        let mut part_of = "";
        let mut point: Option<usize> = None;
        while let Some(event) = reader.next() {
            match event {
                XmlEvent::Start {
                    name, attributes, ..
                } => match name {
                    "c:title" if series.is_empty() => in_title = true,
                    "c:barChart" | "c:bar3DChart" => kind = Some(ChartKind::Column),
                    "c:barDir" if attribute(&attributes, "val") == Some("bar") => {
                        kind = Some(ChartKind::Bar);
                    }
                    "c:lineChart" | "c:line3DChart" | "c:stockChart" => {
                        kind = Some(ChartKind::Line)
                    }
                    "c:areaChart" | "c:area3DChart" => kind = Some(ChartKind::Area),
                    "c:pieChart" | "c:pie3DChart" | "c:doughnutChart" | "c:ofPieChart" => {
                        kind = Some(ChartKind::Pie);
                    }
                    "c:scatterChart" | "c:bubbleChart" => kind = Some(ChartKind::Scatter),
                    "c:ser" => series.push((String::new(), Vec::new(), Vec::new())),
                    "c:tx" | "c:cat" | "c:val" | "c:xVal" | "c:yVal" if !series.is_empty() => {
                        part_of = name;
                    }
                    "c:pt" => {
                        point = attribute(&attributes, "idx").and_then(|idx| idx.parse().ok())
                    }
                    "a:t" if in_title => title.push_str(&read_element_text(&mut reader, name)),
                    "c:v" => {
                        let value = read_element_text(&mut reader, name);
                        if let (Some(current), Some(index)) = (series.last_mut(), point) {
                            match part_of {
                                "c:tx" => current.0 = value,
                                "c:cat" | "c:xVal" => current.1.push((index, value)),
                                "c:val" | "c:yVal" => current.2.push((index, value)),
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                },
                XmlEvent::End { name } => match name {
                    "c:title" => in_title = false,
                    "c:tx" | "c:cat" | "c:val" | "c:xVal" | "c:yVal" => part_of = "",
                    "c:pt" => point = None,
                    _ => {}
                },
                XmlEvent::Text(_) => {}
            }
        }
        if series.is_empty() {
            return None;
        }
        let count = series
            .iter()
            .flat_map(|(_, categories, values)| {
                categories
                    .iter()
                    .map(|(index, _)| *index)
                    .chain(values.iter().map(|(index, _)| *index))
            })
            .max()
            .map_or(0, |last| last + 1);
        let categories = (0..count)
            .map(|index| {
                series
                    .iter()
                    .find_map(|(_, categories, _)| {
                        categories
                            .iter()
                            .find(|(at, _)| *at == index)
                            .map(|(_, text)| text.clone())
                    })
                    .unwrap_or_default()
            })
            .collect();
        let series = series
            .into_iter()
            .map(|(name, _, values)| crate::document::ChartSeries {
                name,
                values: (0..count)
                    .map(|index| {
                        values
                            .iter()
                            .find(|(at, _)| *at == index)
                            .and_then(|(_, text)| text.parse::<f64>().ok())
                    })
                    .collect(),
            })
            .collect();
        let chart = Chart {
            kind: kind.unwrap_or(ChartKind::Column),
            categories,
            series,
        };
        Some((title.trim().to_string(), chart))
    }

    /// A chart as blocks for the text flow: its title, then a table of its
    /// series (a column each) by category (a row each).
    fn chart_blocks(&mut self, title: &str, chart: &Chart, width: f32) -> Vec<Block> {
        let mut cell = |text: &str| {
            let span = self.document.push_text(text);
            Cell {
                blocks: vec![Block::Paragraph(Paragraph {
                    runs: if text.is_empty() {
                        Vec::new()
                    } else {
                        vec![plain_run(Inline::Text(span))]
                    },
                    ..Paragraph::default()
                })],
                column_span: 1,
                row_span: 1,
                ..Cell::default()
            }
        };
        let mut rows = Vec::new();
        let mut header = vec![cell("")];
        for series in &chart.series {
            header.push(cell(&series.name));
        }
        rows.push(Row {
            cells: header,
            height: None,
        });
        for (index, category) in chart.categories.iter().enumerate() {
            let mut cells = vec![cell(category)];
            for series in &chart.series {
                let value = series
                    .values
                    .get(index)
                    .copied()
                    .flatten()
                    .map(|value| general_number(&value.to_string()))
                    .unwrap_or_default();
                cells.push(cell(&value));
            }
            rows.push(Row {
                cells,
                height: None,
            });
        }
        let columns = chart.series.len() + 1;
        let line = Some(Border {
            width: 0.5,
            color: None,
        });
        let table = Table {
            columns: vec![(width.max(144.0) / columns as f32).max(36.0); columns],
            rows,
            header_rows: 1,
            borders: Some(TableBorders {
                top: line,
                bottom: line,
                left: line,
                right: line,
                inside_horizontal: line,
                inside_vertical: line,
            }),
            ..Table::default()
        };
        let mut blocks = Vec::new();
        if !title.is_empty() {
            let span = self.document.push_text(title);
            blocks.push(Block::Paragraph(Paragraph {
                runs: vec![plain_run(Inline::Text(span))],
                ..Paragraph::default()
            }));
        }
        blocks.push(Block::Table(table));
        blocks
    }

    /// A legacy VML drawing (`w:pict`, or an embedded object's `w:object`):
    /// a horizontal rule, its picture (or an object's preview), or the text
    /// of its text boxes.
    fn read_pict(&mut self, reader: &mut XmlReader<'_>, element: &str) -> Pict {
        let mut rule: Option<Border> = None;
        let mut media: Option<MediaId> = None;
        let mut size: Option<(f32, f32)> = None;
        let mut blocks: Vec<Block> = Vec::new();
        while let Some(event) = reader.next() {
            match event {
                XmlEvent::Start {
                    name,
                    attributes,
                    self_closing,
                } => {
                    let style = attribute(&attributes, "style").unwrap_or("");
                    if name.starts_with("v:") && name != "v:shapetype" && size.is_none() {
                        let (width, height) =
                            (vml_length(style, "width"), vml_length(style, "height"));
                        if let (Some(width), Some(height)) = (width, height) {
                            size = Some((width, height));
                        }
                    }
                    match name {
                        "v:rect" if attribute(&attributes, "o:hr") == Some("t") => {
                            rule = Some(Border {
                                width: vml_length(style, "height").unwrap_or(1.0).clamp(0.25, 6.0),
                                color: attribute(&attributes, "fillcolor")
                                    .map(|color| color.trim_start_matches('#'))
                                    .and_then(parse_color),
                            });
                        }
                        "v:imagedata" if media.is_none() => {
                            media =
                                attribute(&attributes, "r:id").and_then(|id| self.load_media(id));
                        }
                        "w:txbxContent" if !self_closing => {
                            let anchoring = std::mem::replace(&mut self.anchoring, false);
                            blocks.extend(self.read_blocks(reader, "w:txbxContent"));
                            self.anchoring = anchoring;
                        }
                        _ => {}
                    }
                }
                XmlEvent::End { name } if name == element => break,
                _ => {}
            }
        }
        if let Some(line) = rule {
            return Pict::Rule(line);
        }
        if !blocks.is_empty() {
            return Pict::Blocks(blocks);
        }
        match (media, size) {
            (Some(media), Some((width, height))) => Pict::Image(InlineImage {
                media,
                width,
                height,
                description: None,
                placement: Placement::Inline,
                crop: None,
            }),
            _ => Pict::None,
        }
    }

    fn load_media(&mut self, relationship_id: &str) -> Option<MediaId> {
        let part = self.target(relationship_id)?;
        if let Some((_, id)) = self.media.iter().find(|(name, _)| *name == part) {
            return Some(*id);
        }
        let entry = self.archive.find(&part)?;
        let mut bytes = Vec::with_capacity(entry.uncompressed_size as usize);
        self.archive.read(entry, &mut bytes).ok()?;
        let name = part.rsplit('/').next().unwrap_or(&part).to_string();
        let id = self.document.media.len();
        self.document.media.push(Media { name, bytes });
        self.media.push((part, id));
        Some(id)
    }
}

// ----- small readers and helpers -----

#[derive(Default)]
struct StyleRead {
    /// A table style's own borders (its top-level `w:tblPr`).
    table_borders: Option<TableSides>,
    name: Option<String>,
    based_on: Option<String>,
    paragraph: ParagraphProperties,
    run: RunProperties,
    num_id: Option<usize>,
}

#[derive(Default)]
struct RunRead {
    properties: RunProperties,
    style: Option<StyleId>,
}

/// A shape read from a drawing: its text, its frame (x, y, width, height in
/// the group's coordinates, EMU), and its look once the shape closes.
#[derive(Default)]
struct ShapeRead {
    blocks: Vec<Block>,
    frame: Option<[f32; 4]>,
    fill: Option<Color>,
    line: Option<Border>,
    geometry: ShapeGeometry,
    flip: (bool, bool),
    ends: (
        Option<crate::document::LineEnd>,
        Option<crate::document::LineEnd>,
    ),
}

/// A DrawingML line end (`a:headEnd`, `a:tailEnd`) by its type.
fn line_end(kind: Option<&str>) -> Option<crate::document::LineEnd> {
    use crate::document::LineEnd;
    match kind? {
        "triangle" | "stealth" => Some(LineEnd::Arrow),
        "arrow" => Some(LineEnd::OpenArrow),
        "diamond" => Some(LineEnd::Diamond),
        "oval" => Some(LineEnd::Circle),
        _ => None,
    }
}

/// Where a DrawingML colour being read goes.
#[derive(Clone, Copy)]
enum ColorTarget {
    Fill,
    StyleFill,
    Line,
    StyleLine,
}

/// A Word preset shape's geometry; one Pages cannot draw is a rectangle.
fn preset_geometry(preset: &str) -> ShapeGeometry {
    match preset {
        "roundRect" | "snipRoundRect" | "round2SameRect" => ShapeGeometry::RoundedRectangle,
        "ellipse" | "flowChartConnector" => ShapeGeometry::Ellipse,
        "triangle" | "flowChartExtract" => ShapeGeometry::Triangle,
        "rtTriangle" => ShapeGeometry::RightTriangle,
        "diamond" | "flowChartDecision" => ShapeGeometry::Diamond,
        "pentagon" | "homePlate" => ShapeGeometry::Pentagon,
        "hexagon" => ShapeGeometry::Hexagon,
        "octagon" => ShapeGeometry::Octagon,
        "star4" | "star5" | "star6" | "star7" | "star8" => ShapeGeometry::Star,
        "rightArrow" | "notchedRightArrow" | "stripedRightArrow" => ShapeGeometry::RightArrow,
        "leftArrow" => ShapeGeometry::LeftArrow,
        "upArrow" => ShapeGeometry::UpArrow,
        "downArrow" => ShapeGeometry::DownArrow,
        "line" | "straightConnector1" | "bentConnector2" | "bentConnector3" => ShapeGeometry::Line,
        _ => ShapeGeometry::Rectangle,
    }
}

/// A DrawingML preset colour by name (the common ones Word writes).
fn preset_color(name: &str) -> Option<Color> {
    let (red, green, blue) = match name {
        "black" => (0, 0, 0),
        "white" => (255, 255, 255),
        "red" => (255, 0, 0),
        "green" => (0, 128, 0),
        "blue" => (0, 0, 255),
        "yellow" => (255, 255, 0),
        "gray" | "grey" => (128, 128, 128),
        _ => return None,
    };
    Some(Color { red, green, blue })
}

/// A paragraph's complex field (`w:fldChar`) as its runs go by: where in the
/// field they are, its instruction, whether it is a page number (whose
/// cached result is not text), and the link a hyperlink field gives.
struct FieldState {
    field: Field,
    instruction: String,
    page_field: bool,
    link: Option<u32>,
}

impl Default for FieldState {
    fn default() -> Self {
        FieldState {
            field: Field::None,
            instruction: String::new(),
            page_field: false,
            link: None,
        }
    }
}

impl FieldState {
    /// Runs here are not text: the instruction, or a page field's result.
    fn hidden(&self) -> bool {
        self.field == Field::Instruction || (self.field == Field::Result && self.page_field)
    }
}

enum Drawn {
    Image(InlineImage),
    /// A page-positioned object, with what its x and y offsets are measured
    /// from (converted to page coordinates once the section's margins are known).
    Floating(FloatingObject, AnchorBase, AnchorBase),
    /// Blocks standing for the drawing after its paragraph (a chart's data).
    Blocks(Vec<Block>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum VerticalMerge {
    None,
    Restart,
    Continue,
}

struct AbstractNumbering {
    id: i64,
    levels: Vec<ListLevel>,
    starts: Vec<u32>,
    num_style_link: Option<String>,
    style_link: Option<String>,
}

fn read_abstract_numbering(reader: &mut XmlReader<'_>, id: i64) -> AbstractNumbering {
    let mut numbering = AbstractNumbering {
        id,
        levels: Vec::new(),
        starts: Vec::new(),
        num_style_link: None,
        style_link: None,
    };
    while let Some(event) = reader.next() {
        match event {
            XmlEvent::Start {
                name,
                attributes,
                self_closing,
            } => match name {
                "w:numStyleLink" => {
                    numbering.num_style_link = attribute(&attributes, "w:val").map(str::to_string);
                }
                "w:styleLink" => {
                    numbering.style_link = attribute(&attributes, "w:val").map(str::to_string);
                }
                "w:lvl" if !self_closing => {
                    let level_index: usize = attribute(&attributes, "w:ilvl")
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(numbering.levels.len());
                    let (level, start) = read_level(reader, level_index);
                    while numbering.levels.len() <= level_index {
                        numbering.levels.push(ListLevel::default());
                        numbering.starts.push(1);
                    }
                    numbering.levels[level_index] = level;
                    numbering.starts[level_index] = start;
                }
                _ => {}
            },
            XmlEvent::End {
                name: "w:abstractNum",
            } => break,
            _ => {}
        }
    }
    numbering
}

fn read_level(reader: &mut XmlReader<'_>, index: usize) -> (ListLevel, u32) {
    let mut format = "decimal";
    let mut text = String::new();
    let mut start = 1u32;
    let mut left = 0.0f32;
    let mut hanging = 0.0f32;
    let mut in_run_properties = false;
    while let Some(event) = reader.next() {
        match event {
            XmlEvent::Start {
                name,
                attributes,
                self_closing,
            } => match name {
                "w:start" => {
                    start = attribute(&attributes, "w:val")
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(1);
                }
                "w:numFmt" => {
                    format = match attribute(&attributes, "w:val") {
                        Some("bullet") => "bullet",
                        Some("none") => "none",
                        Some("lowerLetter") => "lowerLetter",
                        Some("upperLetter") => "upperLetter",
                        Some("lowerRoman") => "lowerRoman",
                        Some("upperRoman") => "upperRoman",
                        _ => "decimal",
                    };
                }
                "w:lvlText" => {
                    text = attribute(&attributes, "w:val").unwrap_or("").to_string();
                }
                "w:ind" if !in_run_properties => {
                    left = attribute(&attributes, "w:left")
                        .or_else(|| attribute(&attributes, "w:start"))
                        .and_then(twips_to_points)
                        .unwrap_or(0.0);
                    hanging = attribute(&attributes, "w:hanging")
                        .and_then(twips_to_points)
                        .unwrap_or(0.0);
                }
                "w:rPr" if !self_closing => in_run_properties = true,
                _ => {}
            },
            XmlEvent::End { name } => match name {
                "w:lvl" => break,
                "w:rPr" => in_run_properties = false,
                _ => {}
            },
            _ => {}
        }
    }
    let label = match format {
        "none" => ListLabel::None,
        "bullet" => ListLabel::Text(symbol_bullet(&text)),
        _ => {
            let kind = match format {
                "lowerLetter" => NumberKind::LowerLetter,
                "upperLetter" => NumberKind::UpperLetter,
                "lowerRoman" => NumberKind::LowerRoman,
                "upperRoman" => NumberKind::UpperRoman,
                _ => NumberKind::Decimal,
            };
            // A level that names its parents' numbers (`%1.%2.`) is tiered;
            // the model keeps only its own part, naming its number `%1`.
            let own = format!("%{}", index + 1);
            let tiered = (1..=index).any(|parent| text.contains(&format!("%{parent}")));
            let mut pattern = match text.find(&own) {
                Some(at) if tiered => text[at..].to_string(),
                _ => text,
            };
            for level in (1..=9).rev() {
                pattern = pattern.replace(&format!("%{level}"), "%1");
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
    let level = ListLevel {
        label,
        indent: left,
        label_indent: left - hanging,
    };
    (level, start)
}

fn read_num(reader: &mut XmlReader<'_>) -> (i64, Option<u32>) {
    let mut abstract_id = -1i64;
    let mut start: Option<u32> = None;
    let mut level = 0usize;
    while let Some(event) = reader.next() {
        match event {
            XmlEvent::Start {
                name, attributes, ..
            } => match name {
                "w:abstractNumId" => {
                    abstract_id = attribute(&attributes, "w:val")
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(-1);
                }
                "w:lvlOverride" => {
                    level = attribute(&attributes, "w:ilvl")
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(0);
                }
                "w:startOverride" if level == 0 => {
                    start = attribute(&attributes, "w:val").and_then(|value| value.parse().ok());
                }
                _ => {}
            },
            XmlEvent::End { name: "w:num" } => break,
            _ => {}
        }
    }
    (abstract_id, start)
}

/// A cell's `w:tcPr` as read.
struct CellProperties {
    span: u32,
    merge: VerticalMerge,
    background: Option<Color>,
    borders: CellBorders,
    vertical: Option<VerticalAlignment>,
    /// Top, bottom, left, right.
    margins: [Option<f32>; 4],
}

fn read_cell_properties(reader: &mut XmlReader<'_>) -> CellProperties {
    let mut span = 1u32;
    let mut merge = VerticalMerge::None;
    let mut background = None;
    let mut borders = CellBorders::default();
    let mut in_borders = false;
    let mut vertical = None;
    let mut margins = [None; 4];
    let mut in_margins = false;
    while let Some(event) = reader.next() {
        match event {
            XmlEvent::Start {
                name,
                attributes,
                self_closing,
            } => match name {
                "w:gridSpan" => {
                    span = attribute(&attributes, "w:val")
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(1);
                }
                "w:vMerge" => {
                    merge = match attribute(&attributes, "w:val") {
                        Some("restart") => VerticalMerge::Restart,
                        _ => VerticalMerge::Continue,
                    };
                }
                "w:shd" => background = attribute(&attributes, "w:fill").and_then(parse_color),
                "w:tcBorders" if !self_closing => in_borders = true,
                "w:tcMar" if !self_closing => in_margins = true,
                side if in_margins => {
                    let index = match side {
                        "w:top" => 0,
                        "w:bottom" => 1,
                        "w:left" | "w:start" => 2,
                        "w:right" | "w:end" => 3,
                        _ => continue,
                    };
                    if matches!(attribute(&attributes, "w:type"), None | Some("dxa")) {
                        margins[index] = attribute(&attributes, "w:w").and_then(twips_to_points);
                    }
                }
                "w:vAlign" => {
                    vertical = match attribute(&attributes, "w:val") {
                        Some("top") => Some(VerticalAlignment::Top),
                        Some("center") => Some(VerticalAlignment::Center),
                        Some("bottom") => Some(VerticalAlignment::Bottom),
                        _ => None,
                    };
                }
                "w:top" if in_borders => borders.top = Some(border_line(&attributes)),
                "w:bottom" if in_borders => borders.bottom = Some(border_line(&attributes)),
                "w:left" | "w:start" if in_borders => borders.left = Some(border_line(&attributes)),
                "w:right" | "w:end" if in_borders => borders.right = Some(border_line(&attributes)),
                _ => {}
            },
            XmlEvent::End {
                name: "w:tcBorders",
            } => in_borders = false,
            XmlEvent::End { name: "w:tcMar" } => in_margins = false,
            XmlEvent::End { name: "w:tcPr" } => break,
            _ => {}
        }
    }
    CellProperties {
        span: span.max(1),
        merge,
        background,
        borders,
        vertical,
        margins,
    }
}

/// A grid position no cell occupies: empty, with no lines of its own.
fn ghost_cell() -> Cell {
    Cell {
        column_span: 1,
        row_span: 1,
        borders: CellBorders {
            top: Some(None),
            bottom: Some(None),
            left: Some(None),
            right: Some(None),
        },
        ..Cell::default()
    }
}

/// Word draws a row's real cells in full up to where the row stops: the
/// edges they share with empty grid space keep the lines the table gives
/// them (its outer lines at the row's ends), which the empty space's own
/// "no line" would otherwise suppress.
fn outline_ghost_cells(rows: &mut [Row], occupied: &[(usize, usize)], borders: TableBorders) {
    let ghost = |row: usize, column: usize| {
        occupied
            .get(row)
            .is_none_or(|(before, end)| column < *before || column >= *end)
    };
    // (Every row is as wide as the grid by now.)
    let count = rows.len();
    for (r, row) in rows.iter_mut().enumerate() {
        let last = row.cells.len();
        for (c, cell) in row.cells.iter_mut().enumerate() {
            if ghost(r, c) {
                continue;
            }
            let sides = &mut cell.borders;
            if c > 0 && ghost(r, c - 1) {
                sides.left.get_or_insert(borders.left);
            }
            if c + 1 < last && ghost(r, c + 1) {
                sides.right.get_or_insert(borders.right);
            }
            if r > 0 && ghost(r - 1, c) {
                sides.top.get_or_insert(borders.inside_horizontal);
            }
            if r + 1 < count && ghost(r + 1, c) {
                sides.bottom.get_or_insert(borders.inside_horizontal);
            }
        }
    }
}

/// Lays the cells as read onto the grid: a spanning cell is followed by
/// covered cells, and vertically merged cells point up at their origin.
fn resolve_merges(rows: Vec<(Row, Vec<(u32, VerticalMerge)>)>) -> Vec<Row> {
    // Each grid cell's (row, column) as read, expanded by column span.
    let mut grid: Vec<Row> = Vec::with_capacity(rows.len());
    let mut merges: Vec<Vec<VerticalMerge>> = Vec::with_capacity(rows.len());
    for (row, spans) in rows {
        let mut expanded = Row {
            cells: Vec::new(),
            height: row.height,
        };
        let mut row_merges = Vec::new();
        for (cell, (span, merge)) in row.cells.into_iter().zip(spans) {
            let mut origin = cell;
            origin.column_span = span;
            // Covered cells take the origin's edges, so the merged region's
            // outline is the origin's.
            let borders = origin.borders;
            expanded.cells.push(origin);
            row_merges.push(merge);
            for _ in 1..span {
                expanded.cells.push(Cell {
                    column_span: 0,
                    row_span: 0,
                    merge: Merge::Left,
                    borders,
                    ..Cell::default()
                });
                row_merges.push(merge);
            }
        }
        grid.push(expanded);
        merges.push(row_merges);
    }
    for row_index in 0..grid.len() {
        for column in 0..grid[row_index].cells.len() {
            if merges[row_index][column] != VerticalMerge::Restart
                || grid[row_index].cells[column].merge != Merge::Origin
            {
                continue;
            }
            let span = grid[row_index].cells[column].column_span;
            let mut below = row_index + 1;
            while below < grid.len()
                && merges[below].get(column) == Some(&VerticalMerge::Continue)
                && grid[below].cells[column].merge == Merge::Origin
            {
                let covered = &mut grid[below].cells[column];
                covered.merge = Merge::Above;
                covered.column_span = span;
                covered.row_span = 0;
                below += 1;
            }
            grid[row_index].cells[column].row_span = (below - row_index) as u32;
        }
    }
    grid
}

/// Skips to the end tag of `name`, whose start tag was just read.
fn skip_element(reader: &mut XmlReader<'_>, name: &str) {
    let mut depth = 1usize;
    while let Some(event) = reader.next() {
        match event {
            XmlEvent::Start {
                name: inner,
                self_closing: false,
                ..
            } if inner == name => depth += 1,
            XmlEvent::End { name: inner } if inner == name => {
                depth -= 1;
                if depth == 0 {
                    return;
                }
            }
            _ => {}
        }
    }
}

fn read_element_text(reader: &mut XmlReader<'_>, name: &str) -> String {
    let mut text = String::new();
    while let Some(event) = reader.next() {
        match event {
            XmlEvent::Text(piece) => text.push_str(&piece),
            XmlEvent::End { name: inner } if inner == name => break,
            _ => {}
        }
    }
    text
}

fn attribute<'e>(attributes: &'e [(&str, Cow<'_, str>)], name: &str) -> Option<&'e str> {
    attributes
        .iter()
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.as_ref())
}

/// An on/off property: absent value means on.
fn toggle(value: Option<&str>) -> bool {
    !matches!(value, Some("0") | Some("false") | Some("off"))
}

fn twips_to_points(value: &str) -> Option<f32> {
    let twips: f32 = value.parse().ok()?;
    Some(twips / 20.0)
}

fn emu_to_points(value: &str) -> Option<f32> {
    let emu: f32 = value.parse().ok()?;
    Some(emu / 12_700.0)
}

/// How a drawing is placed and sized relative to what it is anchored to,
/// beyond fixed offsets: shares of it, alignments in it (0 start, 0.5
/// centre, 1 end), and sizes as shares of the page (`true`) or margins.
#[derive(Clone, Copy, Default)]
struct DrawingLayout {
    share: (Option<f32>, Option<f32>),
    align: (Option<f32>, Option<f32>),
    size: (Option<RelativeSize>, Option<RelativeSize>),
}

/// A share of the page (`true`) or of its margins.
type RelativeSize = (f32, bool);

/// Whether a picture file is a common raster (PNG, JPEG, GIF) every
/// target shows.
fn is_raster(name: &str) -> bool {
    let extension = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "gif")
}

/// A footer line's height, to place what sits on it from the page foot.
const FOOTER_LINE: f32 = 14.0;

fn parse_color(value: &str) -> Option<Color> {
    let hex = value.trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let number = u32::from_str_radix(hex, 16).ok()?;
    Some(Color {
        red: (number >> 16) as u8,
        green: (number >> 8) as u8,
        blue: number as u8,
    })
}

fn highlight_color(name: &str) -> Option<Color> {
    let hex = match name {
        "yellow" => "FFFF00",
        "green" => "00FF00",
        "cyan" => "00FFFF",
        "magenta" => "FF00FF",
        "blue" => "0000FF",
        "red" => "FF0000",
        "darkBlue" => "000080",
        "darkCyan" => "008080",
        "darkGreen" => "008000",
        "darkMagenta" => "800080",
        "darkRed" => "800000",
        "darkYellow" => "808000",
        "darkGray" => "808080",
        "lightGray" => "C0C0C0",
        "black" => "000000",
        "white" => "FFFFFF",
        _ => return None,
    };
    parse_color(hex)
}

/// The character of a `w:sym`, given as hex; the private-use range the
/// symbol fonts use is folded to the same code in the basic plane.
fn symbol_text(code: Option<&str>) -> Option<String> {
    let code = u32::from_str_radix(code?, 16).ok()?;
    let code = if (0xF000..=0xF0FF).contains(&code) {
        code - 0xF000
    } else {
        code
    };
    let ch = char::from_u32(code)?;
    Some(ch.to_string())
}

/// A bullet as its Unicode character: Word draws the common bullets from the
/// Symbol and Wingdings fonts, whose private-use codes show as boxes in any
/// other font.
fn symbol_bullet(text: &str) -> String {
    text.chars()
        .map(|character| match character as u32 {
            0xF0B7 => '\u{2022}',          // Symbol: bullet
            0xF0A7 | 0xF06E => '\u{25AA}', // Wingdings: small square
            0xF0A8 => '\u{25E6}',          // Wingdings: white bullet
            0xF0D8 => '\u{27A2}',          // Wingdings: arrowhead
            0xF0FC => '\u{2713}',          // Wingdings: check mark
            0xF076 => '\u{2756}',          // Wingdings: diamond
            0xF06C => '\u{25CF}',          // Wingdings: black circle
            0xF0E0 => '\u{27A4}',          // Wingdings: arrow
            _ => character,
        })
        .collect()
}

/// A DrawingML custom path (`a:path`): its moves, lines, curves (quadratic
/// ones raised to cubic), and closes, in points of its own box.
fn read_custom_path(
    reader: &mut XmlReader<'_>,
    size: (Option<f32>, Option<f32>),
) -> Option<crate::document::ShapePath> {
    use crate::document::{PathStep, ShapePath};
    let mut steps = Vec::new();
    let mut command = "";
    let mut points: Vec<(f32, f32)> = Vec::new();
    let mut last = (0.0f32, 0.0f32);
    while let Some(event) = reader.next() {
        match event {
            XmlEvent::Start {
                name, attributes, ..
            } => match name {
                "a:moveTo" | "a:lnTo" | "a:cubicBezTo" | "a:quadBezTo" => {
                    command = name;
                    points.clear();
                }
                "a:close" => steps.push(PathStep::Close),
                "a:pt" => {
                    let x = attribute(&attributes, "x")
                        .and_then(emu_to_points)
                        .unwrap_or(0.0);
                    let y = attribute(&attributes, "y")
                        .and_then(emu_to_points)
                        .unwrap_or(0.0);
                    points.push((x, y));
                }
                _ => {}
            },
            XmlEvent::End { name } => match name {
                "a:path" => break,
                "a:moveTo" | "a:lnTo" | "a:cubicBezTo" | "a:quadBezTo" => {
                    let step = match (command, points.as_slice()) {
                        ("a:moveTo", [p, ..]) => Some(PathStep::Move(p.0, p.1)),
                        ("a:lnTo", [p, ..]) => Some(PathStep::Line(p.0, p.1)),
                        ("a:cubicBezTo", [a, b, c, ..]) => Some(PathStep::Curve([*a, *b, *c])),
                        ("a:quadBezTo", [c, e, ..]) => {
                            // Raised to a cubic: controls two-thirds toward the quadratic's.
                            let toward = |from: (f32, f32)| {
                                (
                                    from.0 + 2.0 / 3.0 * (c.0 - from.0),
                                    from.1 + 2.0 / 3.0 * (c.1 - from.1),
                                )
                            };
                            Some(PathStep::Curve([toward(last), toward(*e), *e]))
                        }
                        _ => None,
                    };
                    if let Some(step) = step {
                        if let Some(end) = points.last() {
                            last = *end;
                        }
                        steps.push(step);
                    }
                }
                _ => {}
            },
            XmlEvent::Text(_) => {}
        }
    }
    if steps.is_empty() {
        return None;
    }
    let (width, height) = match size {
        (Some(width), Some(height)) => (width, height),
        _ => {
            // Without a stated size, the path's own extent.
            let (mut w, mut h) = (0.0f32, 0.0f32);
            for step in &steps {
                let points: Vec<(f32, f32)> = match *step {
                    PathStep::Move(x, y) | PathStep::Line(x, y) => vec![(x, y)],
                    PathStep::Curve(points) => points.to_vec(),
                    PathStep::Close => Vec::new(),
                };
                for (x, y) in points {
                    w = w.max(x);
                    h = h.max(y);
                }
            }
            (w, h)
        }
    };
    Some(ShapePath {
        width,
        height,
        steps,
    })
}

/// What a legacy VML drawing holds.
enum Pict {
    /// A horizontal rule, drawn with this line.
    Rule(Border),
    Image(InlineImage),
    /// Text box contents, to follow the paragraph.
    Blocks(Vec<Block>),
    None,
}

/// A length from a VML style (`width:72pt`, `height:1.5in`), in points.
fn vml_length(style: &str, key: &str) -> Option<f32> {
    let value = style.split(';').find_map(|part| {
        let (name, value) = part.split_once(':')?;
        (name.trim() == key).then(|| value.trim())
    })?;
    let number_end = value
        .find(|character: char| {
            !(character.is_ascii_digit() || character == '.' || character == '-')
        })
        .unwrap_or(value.len());
    let number: f32 = value[..number_end].parse().ok()?;
    let factor = match &value[number_end..] {
        "pt" => 1.0,
        "in" => 72.0,
        "cm" => 72.0 / 2.54,
        "mm" => 72.0 / 25.4,
        "pc" => 12.0,
        "px" | "" => 0.75,
        _ => return None,
    };
    Some(number * factor)
}

/// A chart series as read: its name, and its categories and values by index.
type ChartSeries = (String, Vec<(usize, String)>, Vec<(usize, String)>);

/// A cached chart value as Word's General format shows it: a number
/// without its binary noise (4.4000000000000004 is 4.4).
fn general_number(value: &str) -> String {
    let Ok(number) = value.trim().parse::<f64>() else {
        return value.to_string();
    };
    let text = format!("{number:.10}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".to_string()
    } else {
        text.to_string()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_an_encrypted_or_legacy_file() {
        let mut bytes = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
        bytes.extend_from_slice(&[0; 504]);
        let Err(error) = read_docx(&bytes) else {
            panic!("an OLE file is not a Word package");
        };
        assert!(error.to_string().contains("password-protected"));
    }

    fn package(document: &str, styles: &str) -> Vec<u8> {
        let mut zip = crate::io::zip::ZipWriter::new(Vec::new());
        zip.add("word/document.xml", document.as_bytes()).unwrap();
        zip.add("word/styles.xml", styles.as_bytes()).unwrap();
        zip.finish().unwrap()
    }

    /// A page number in a frame beside a footer's text is a box of its own,
    /// right-aligned on the footer's line of every page, not a paragraph
    /// before the text.
    #[test]
    fn footer_frames_become_repeating_boxes() {
        let w = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
        let document = format!(
            r#"<w:document {w}><w:body><w:p><w:r><w:t>Body</w:t></w:r></w:p><w:sectPr><w:footerReference w:type="default" r:id="rIdF"/><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:bottom="1440" w:left="1440" w:right="1440" w:footer="720"/></w:sectPr></w:body></w:document>"#
        );
        let footer = format!(
            r#"<w:ftr {w}><w:p><w:pPr><w:framePr w:wrap="around" w:vAnchor="text" w:hAnchor="margin" w:xAlign="right" w:y="1"/></w:pPr><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText>PAGE</w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>7</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p><w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:r><w:t>Title</w:t></w:r></w:p></w:ftr>"#
        );
        let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdF" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer" Target="footer1.xml"/></Relationships>"#;
        let mut zip = crate::io::zip::ZipWriter::new(Vec::new());
        zip.add("word/document.xml", document.as_bytes()).unwrap();
        zip.add("word/footer1.xml", footer.as_bytes()).unwrap();
        zip.add("word/_rels/document.xml.rels", rels.as_bytes())
            .unwrap();
        let read = read_docx(&zip.finish().unwrap()).expect("reads");
        let footer_blocks = read.sections[0].footers.default.as_ref().expect("footer");
        assert_eq!(
            footer_blocks.len(),
            1,
            "only the title stays in the footer text"
        );
        let framed = read
            .floating
            .iter()
            .find(|object| object.repeats.is_some_and(|part| part.footer))
            .expect("the page number is a repeating box");
        // Right-aligned within the margins, on the footer's line.
        assert!((framed.x + framed.width - (612.0 - 72.0)).abs() < 0.5);
        assert!(framed.y > 792.0 - 36.0 - 20.0 && framed.y < 792.0 - 36.0);
    }

    #[test]
    fn form_checkboxes_keep_their_state_as_box_characters() {
        let w = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
        let checkbox = |inner: &str| {
            format!(
                r#"<w:r><w:fldChar w:fldCharType="begin"><w:ffData><w:checkBox><w:sizeAuto/>{inner}</w:checkBox></w:ffData></w:fldChar></w:r>
                <w:r><w:instrText> FORMCHECKBOX </w:instrText></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>"#
            )
        };
        let document = format!(
            r#"<w:document {w}><w:body><w:p>{}<w:r><w:t>a</w:t></w:r>{}<w:r><w:t>b</w:t></w:r>{}</w:p></w:body></w:document>"#,
            checkbox(r#"<w:default w:val="1"/>"#),
            checkbox(r#"<w:default w:val="0"/>"#),
            checkbox(r#"<w:default w:val="1"/><w:checked w:val="0"/>"#),
        );
        let read = read_docx(&package(&document, &format!("<w:styles {w}/>"))).unwrap();
        let Some(Block::Paragraph(paragraph)) = read.sections[0].blocks.first() else {
            panic!("a paragraph");
        };
        let text: String = paragraph
            .runs
            .iter()
            .filter_map(|run| match run.content {
                Inline::Text(span) => Some(read.text(span)),
                _ => None,
            })
            .collect();
        assert_eq!(text, "\u{2612}a\u{2610}b\u{2610}");
    }

    #[test]
    fn rows_short_of_the_grid_leave_empty_undrawn_space() {
        let w = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
        let document = format!(
            r#"<w:document {w}><w:body><w:tbl><w:tblPr><w:tblBorders>
            <w:right w:val="single" w:sz="8"/><w:insideV w:val="single" w:sz="4"/></w:tblBorders>
            <w:tblCellMar><w:top w:w="40" w:type="dxa"/><w:left w:w="200" w:type="dxa"/></w:tblCellMar></w:tblPr>
            <w:tblGrid><w:gridCol w:w="100"/><w:gridCol w:w="100"/><w:gridCol w:w="100"/></w:tblGrid>
            <w:tr><w:tc><w:p/></w:tc><w:tc><w:p/></w:tc><w:tc><w:p/></w:tc></w:tr>
            <w:tr><w:trPr><w:gridBefore w:val="1"/><w:gridAfter w:val="1"/></w:trPr><w:tc><w:p><w:r><w:t>mid</w:t></w:r></w:p></w:tc></w:tr>
            </w:tbl></w:body></w:document>"#
        );
        let read = read_docx(&package(&document, &format!("<w:styles {w}/>"))).unwrap();
        let Some(Block::Table(table)) = read.sections[0].blocks.first() else {
            panic!("a table");
        };
        let row = &table.rows[1];
        assert_eq!(row.cells.len(), 3, "the row spans the grid");
        assert!(row.cells[0].blocks.is_empty() && row.cells[2].blocks.is_empty());
        assert!(
            !row.cells[1].blocks.is_empty(),
            "the real cell sits in column 2"
        );
        assert_eq!(
            row.cells[0].borders.right,
            Some(None),
            "empty space draws no line"
        );
        // The real cell keeps the lines the table gives it at the row's ends.
        let width = |side: Option<Option<Border>>| side.flatten().map(|line| line.width);
        assert_eq!(
            width(row.cells[1].borders.left),
            None,
            "no left outer line stated"
        );
        assert_eq!(width(row.cells[1].borders.right), Some(1.0));
        let margins = table.cell_margins.expect("margins");
        assert_eq!((margins.top, margins.bottom), (2.0, 0.0));
        assert_eq!((margins.left, margins.right), (10.0, 5.4));
    }

    #[test]
    fn every_text_box_of_a_group_is_kept_where_the_group_puts_it() {
        let ns = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:wp="wp" xmlns:a="a" xmlns:wpg="wpg" xmlns:wps="wps""#;
        let shape = |x: u32, text: &str, fill: &str| {
            format!(
                r#"<wps:wsp><wps:spPr><a:xfrm><a:off x="{x}" y="100"/><a:ext cx="200" cy="100"/></a:xfrm>{fill}</wps:spPr>
                <wps:txbx><w:txbxContent><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:txbxContent></wps:txbx></wps:wsp>"#
            )
        };
        let document = format!(
            r#"<w:document {ns}><w:body><w:p><w:r><w:drawing><wp:anchor>
            <wp:positionH relativeFrom="page"><wp:posOffset>127000</wp:posOffset></wp:positionH>
            <wp:positionV relativeFrom="page"><wp:posOffset>254000</wp:posOffset></wp:positionV>
            <wp:extent cx="508000" cy="254000"/><a:graphic><a:graphicData><wpg:wgp>
            <wpg:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="508000" cy="254000"/><a:chOff x="0" y="0"/><a:chExt cx="400" cy="200"/></a:xfrm></wpg:grpSpPr>
            {}{}</wpg:wgp></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r></w:p></w:body></w:document>"#,
            shape(
                0,
                "first",
                r#"<a:solidFill><a:srgbClr val="FF0000"/></a:solidFill>"#
            ),
            shape(200, "second", ""),
        );
        let read = read_docx(&package(&document, "<w:styles/>")).unwrap();
        assert_eq!(read.floating.len(), 2, "both boxes survive");
        let (first, second) = (&read.floating[0], &read.floating[1]);
        // The group is 40 x 20 points over a 400 x 200 child space.
        assert_eq!(
            (first.x, first.y, first.width, first.height),
            (10.0, 30.0, 20.0, 10.0)
        );
        assert_eq!((second.x, second.width), (30.0, 20.0));
        let fill = |object: &FloatingObject| match &object.content {
            FloatingContent::TextBox { fill, .. } => *fill,
            _ => panic!("a text box"),
        };
        assert_eq!(
            fill(first),
            Some(Color {
                red: 255,
                green: 0,
                blue: 0
            })
        );
        assert_eq!(fill(second), None, "a shape's fill is its own");
    }

    #[test]
    fn a_multi_level_pattern_is_tiered() {
        let w = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
        let numbering = format!(
            r#"<w:numbering {w}><w:abstractNum w:abstractNumId="0">
            <w:lvl w:ilvl="0"><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/></w:lvl>
            <w:lvl w:ilvl="1"><w:numFmt w:val="decimal"/><w:lvlText w:val="%1.%2."/></w:lvl>
            </w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>"#
        );
        let document = format!(
            r#"<w:document {w}><w:body><w:p><w:pPr><w:numPr><w:ilvl w:val="1"/><w:numId w:val="1"/></w:numPr></w:pPr>
            <w:r><w:t>x</w:t></w:r></w:p></w:body></w:document>"#
        );
        let mut zip = crate::io::zip::ZipWriter::new(Vec::new());
        zip.add("word/document.xml", document.as_bytes()).unwrap();
        zip.add("word/styles.xml", format!("<w:styles {w}/>").as_bytes())
            .unwrap();
        zip.add("word/numbering.xml", numbering.as_bytes()).unwrap();
        let read = read_docx(&zip.finish().unwrap()).unwrap();
        let style = &read.styles.list[0];
        let ListLabel::Number(first) = &style.levels[0].label else {
            panic!("a number");
        };
        let ListLabel::Number(second) = &style.levels[1].label else {
            panic!("a number");
        };
        assert!(!first.tiered);
        assert!(second.tiered);
        assert_eq!(second.pattern, "%1.");
    }

    #[test]
    fn first_line_indents_are_measured_from_the_margin() {
        let w = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
        let paragraph = |ind: &str| format!(r#"<w:p><w:pPr><w:ind {ind}/></w:pPr></w:p>"#);
        let document = format!(
            r#"<w:document {w}><w:body>{}{}{}</w:body></w:document>"#,
            paragraph(r#"w:left="720" w:hanging="360""#),
            paragraph(r#"w:left="720" w:firstLine="720""#),
            paragraph(r#"w:firstLine="360""#),
        );
        let read = read_docx(&package(&document, &format!("<w:styles {w}/>"))).unwrap();
        let first: Vec<Option<f32>> = read.sections[0]
            .blocks
            .iter()
            .map(|block| match block {
                Block::Paragraph(paragraph) => {
                    read.paragraph_properties(paragraph).first_line_indent
                }
                Block::Table(_) => None,
            })
            .collect();
        assert_eq!(first, vec![Some(18.0), Some(72.0), Some(18.0)]);
    }

    #[test]
    fn chart_values_read_as_word_shows_them() {
        assert_eq!(general_number("4.4000000000000004"), "4.4");
        assert_eq!(general_number("2"), "2");
        assert_eq!(general_number("n/a"), "n/a");
    }

    #[test]
    fn tab_stops_are_read_with_their_leaders() {
        let w = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
        let document = format!(
            r#"<w:document {w}><w:body><w:p><w:pPr><w:tabs><w:tab w:val="right" w:leader="dot" w:pos="9000"/>
            <w:tab w:val="clear" w:pos="720"/><w:tab w:val="decimal" w:pos="4680"/></w:tabs></w:pPr></w:p></w:body></w:document>"#
        );
        let read = read_docx(&package(&document, &format!("<w:styles {w}/>"))).unwrap();
        let Some(Block::Paragraph(paragraph)) = read.sections[0].blocks.first() else {
            panic!("a paragraph");
        };
        let tabs = read.tab_set(read.paragraph_properties(paragraph).tabs);
        assert_eq!(tabs.len(), 2, "a cleared stop is left out");
        assert_eq!((tabs[0].position, tabs[0].leader), (450.0, Some('.')));
        assert_eq!(tabs[0].alignment, crate::document::TabAlignment::Right);
        assert_eq!(tabs[1].alignment, crate::document::TabAlignment::Decimal);
    }

    #[test]
    fn a_vml_horizontal_rule_is_the_paragraphs_bottom_border() {
        let w = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:v="v" xmlns:o="o""#;
        let document = format!(
            r##"<w:document {w}><w:body><w:p><w:r><w:pict><v:rect style="width:0;height:1.5pt" o:hr="t" fillcolor="#A0A0A0" stroked="f"/></w:pict></w:r></w:p></w:body></w:document>"##
        );
        let read = read_docx(&package(&document, &format!("<w:styles {w}/>"))).unwrap();
        let Some(Block::Paragraph(paragraph)) = read.sections[0].blocks.first() else {
            panic!("a paragraph");
        };
        let border = read.paragraph_properties(paragraph).border.expect("a rule");
        assert!(border.bottom && !border.top);
        assert_eq!(border.line.width, 1.5);
        assert_eq!(
            border.line.color,
            Some(Color {
                red: 0xA0,
                green: 0xA0,
                blue: 0xA0
            })
        );
        assert_eq!(vml_length("width:1in;height:2cm", "width"), Some(72.0));
    }

    #[test]
    fn a_field_inside_one_run_is_read() {
        let w = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
        let document = format!(
            r#"<w:document {w}><w:body><w:p><w:r><w:fldChar w:fldCharType="begin"/><w:instrText>PAGE</w:instrText>
            <w:fldChar w:fldCharType="separate"/><w:t>7</w:t><w:fldChar w:fldCharType="end"/></w:r></w:p></w:body></w:document>"#
        );
        let read = read_docx(&package(&document, &format!("<w:styles {w}/>"))).unwrap();
        let Some(Block::Paragraph(paragraph)) = read.sections[0].blocks.first() else {
            panic!("a paragraph");
        };
        let contents: Vec<&Inline> = paragraph.runs.iter().map(|run| &run.content).collect();
        assert!(
            matches!(contents.as_slice(), [Inline::PageNumber]),
            "{contents:?}"
        );
    }

    #[test]
    fn shapes_without_text_keep_their_outline_fill_and_line() {
        let ns = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:wp="wp" xmlns:a="a" xmlns:wps="wps""#;
        let drawing = |shape: &str| {
            format!(
                r#"<w:r><w:drawing><wp:anchor><wp:extent cx="254000" cy="127000"/><a:graphic><a:graphicData>
                <wps:wsp>{shape}</wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r>"#
            )
        };
        let ellipse = drawing(
            r#"<wps:spPr><a:prstGeom prst="ellipse"/><a:solidFill><a:srgbClr val="00FF00"/></a:solidFill>
            <a:ln w="25400"><a:solidFill><a:srgbClr val="0000FF"/></a:solidFill></a:ln></wps:spPr>"#,
        );
        let line = drawing(
            r#"<wps:spPr><a:xfrm flipV="1"/><a:prstGeom prst="line"/></wps:spPr>
            <wps:style><a:lnRef idx="1"><a:srgbClr val="FF0000"/></a:lnRef></wps:style>"#,
        );
        let invisible = drawing(r#"<wps:spPr><a:prstGeom prst="rect"/><a:noFill/></wps:spPr>"#);
        let document = format!(
            r#"<w:document {ns}><w:body><w:p>{ellipse}{line}{invisible}</w:p></w:body></w:document>"#
        );
        let read = read_docx(&package(&document, "<w:styles/>")).unwrap();
        assert_eq!(
            read.floating.len(),
            2,
            "a shape drawing nothing is left out"
        );
        let FloatingContent::TextBox {
            blocks,
            fill,
            line,
            geometry,
            ..
        } = &read.floating[0].content
        else {
            panic!("a shape");
        };
        assert!(blocks.is_empty());
        assert_eq!(*geometry, ShapeGeometry::Ellipse);
        assert_eq!(
            *fill,
            Some(Color {
                red: 0,
                green: 255,
                blue: 0
            })
        );
        let outline = line.expect("an outline");
        assert_eq!(outline.width, 2.0);
        assert_eq!(
            outline.color,
            Some(Color {
                red: 0,
                green: 0,
                blue: 255
            })
        );
        let FloatingContent::TextBox {
            line,
            geometry,
            flip,
            fill,
            ..
        } = &read.floating[1].content
        else {
            panic!("a line");
        };
        assert_eq!(*geometry, ShapeGeometry::Line);
        assert_eq!(*flip, (false, true));
        assert_eq!(*fill, None);
        assert_eq!(
            line.and_then(|line| line.color),
            Some(Color {
                red: 255,
                green: 0,
                blue: 0
            })
        );
    }

    #[test]
    fn table_borders_come_from_the_style_table_and_cells() {
        let w = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
        let styles = format!(
            r#"<w:styles {w}><w:style w:type="table" w:styleId="Grid"><w:tblPr><w:tblBorders>
            <w:top w:val="single" w:sz="4"/><w:bottom w:val="single" w:sz="4"/>
            <w:insideH w:val="single" w:sz="4"/><w:insideV w:val="single" w:sz="4"/>
            </w:tblBorders></w:tblPr></w:style></w:styles>"#
        );
        let document = format!(
            r#"<w:document {w}><w:body><w:tbl><w:tblPr><w:tblStyle w:val="Grid"/><w:tblBorders>
            <w:top w:val="double" w:sz="16" w:color="FF0000"/><w:bottom w:val="nil"/>
            </w:tblBorders></w:tblPr><w:tblGrid><w:gridCol w:w="100"/><w:gridCol w:w="100"/></w:tblGrid>
            <w:tr><w:tc><w:tcPr><w:tcBorders><w:right w:val="nil"/><w:bottom w:val="single" w:sz="8"/>
            </w:tcBorders><w:vAlign w:val="center"/></w:tcPr><w:p/></w:tc><w:tc><w:p/></w:tc></w:tr></w:tbl></w:body></w:document>"#
        );
        let read = read_docx(&package(&document, &styles)).unwrap();
        let Some(Block::Table(table)) = read.sections[0].blocks.first() else {
            panic!("a table");
        };
        let borders = table.borders.expect("stated borders");
        let top = borders.top.expect("the table's own top line");
        assert_eq!(top.width, 2.0);
        assert_eq!(
            top.color,
            Some(Color {
                red: 255,
                green: 0,
                blue: 0
            })
        );
        assert_eq!(
            borders.bottom, None,
            "the table removes the style's bottom line"
        );
        assert_eq!(borders.left, None, "unstated anywhere is no line");
        assert_eq!(borders.inside_vertical.map(|line| line.width), Some(0.5));
        let cell = table.rows[0].cells[0].borders;
        assert_eq!(cell.right, Some(None));
        assert_eq!(cell.bottom.flatten().map(|line| line.width), Some(1.0));
        assert_eq!(cell.top, None);
        assert_eq!(
            table.rows[0].cells[0].vertical_alignment,
            Some(VerticalAlignment::Center)
        );
        assert_eq!(table.rows[0].cells[1].vertical_alignment, None);
    }
}

/// A scheme colour being read: its name, its modifiers so far, and whether
/// it belongs to the shape's style (fill reference) rather than its own fill.
type PendingSchemeColor = (String, Vec<(String, f32)>, ColorTarget);

/// Applies DrawingML colour modifiers: luminance scale and offset (in HSL),
/// shade (toward black), and tint (toward white).
fn apply_color_modifiers(color: Color, modifiers: &[(String, f32)]) -> Color {
    let (mut hue, mut saturation, mut lightness) = rgb_to_hsl(color);
    for (name, value) in modifiers {
        match name.as_str() {
            "lumMod" => lightness = (lightness * value).clamp(0.0, 1.0),
            "lumOff" => lightness = (lightness + value).clamp(0.0, 1.0),
            "shade" | "tint" => {
                let current = hsl_to_rgb(hue, saturation, lightness);
                let mut rgb = [
                    current.red as f32,
                    current.green as f32,
                    current.blue as f32,
                ];
                for channel in &mut rgb {
                    *channel = if name == "shade" {
                        *channel * value
                    } else {
                        255.0 - (255.0 - *channel) * value
                    };
                }
                let adjusted = Color {
                    red: rgb[0].round().clamp(0.0, 255.0) as u8,
                    green: rgb[1].round().clamp(0.0, 255.0) as u8,
                    blue: rgb[2].round().clamp(0.0, 255.0) as u8,
                };
                (hue, saturation, lightness) = rgb_to_hsl(adjusted);
            }
            _ => {}
        }
    }
    hsl_to_rgb(hue, saturation, lightness)
}

fn rgb_to_hsl(color: Color) -> (f32, f32, f32) {
    let r = color.red as f32 / 255.0;
    let g = color.green as f32 / 255.0;
    let b = color.blue as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let lightness = (max + min) / 2.0;
    if (max - min).abs() < f32::EPSILON {
        return (0.0, 0.0, lightness);
    }
    let delta = max - min;
    let saturation = if lightness > 0.5 {
        delta / (2.0 - max - min)
    } else {
        delta / (max + min)
    };
    let hue = if (max - r).abs() < f32::EPSILON {
        ((g - b) / delta).rem_euclid(6.0)
    } else if (max - g).abs() < f32::EPSILON {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    } / 6.0;
    (hue, saturation, lightness)
}

fn hsl_to_rgb(hue: f32, saturation: f32, lightness: f32) -> Color {
    let channel = |t: f32| {
        let q = if lightness < 0.5 {
            lightness * (1.0 + saturation)
        } else {
            lightness + saturation - lightness * saturation
        };
        let p = 2.0 * lightness - q;
        let t = t.rem_euclid(1.0);
        let value = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        (value * 255.0).round().clamp(0.0, 255.0) as u8
    };
    if saturation == 0.0 {
        let gray = (lightness * 255.0).round().clamp(0.0, 255.0) as u8;
        return Color {
            red: gray,
            green: gray,
            blue: gray,
        };
    }
    Color {
        red: channel(hue + 1.0 / 3.0),
        green: channel(hue),
        blue: channel(hue - 1.0 / 3.0),
    }
}

/// A table's sides as read: each border unstated, stated as no line, or a
/// line; and each cell margin (top, bottom, left, right) unstated or stated.
#[derive(Debug, Clone, Copy, Default)]
struct TableSides {
    sides: [Option<Option<Border>>; 6],
    margins: [Option<f32>; 4],
    alignment: Option<Alignment>,
    indent: Option<f32>,
}

impl TableSides {
    /// These sides, with unstated ones taken from `base`.
    fn or(self, base: TableSides) -> TableSides {
        let mut sides = self.sides;
        for (side, fallback) in sides.iter_mut().zip(base.sides) {
            if side.is_none() {
                *side = fallback;
            }
        }
        let mut margins = self.margins;
        for (margin, fallback) in margins.iter_mut().zip(base.margins) {
            if margin.is_none() {
                *margin = fallback;
            }
        }
        TableSides {
            sides,
            margins,
            alignment: self.alignment.or(base.alignment),
            indent: self.indent.or(base.indent),
        }
    }

    /// The cell margins, with Word's defaults where nothing states them
    /// (none above and below, 0.075" at the sides).
    fn cell_margins(self) -> CellMargins {
        let [top, bottom, left, right] = self.margins;
        CellMargins {
            top: top.unwrap_or(0.0),
            bottom: bottom.unwrap_or(0.0),
            left: left.unwrap_or(5.4),
            right: right.unwrap_or(5.4),
        }
    }

    fn resolve(self) -> TableBorders {
        let [top, bottom, left, right, inside_horizontal, inside_vertical] =
            self.sides.map(Option::flatten);
        TableBorders {
            top,
            bottom,
            left,
            right,
            inside_horizontal,
            inside_vertical,
        }
    }
}

/// A table's `w:tblPr`: its style id and its own borders.
fn read_table_properties(reader: &mut XmlReader<'_>) -> (Option<String>, TableSides) {
    let mut style = None;
    let mut borders = TableSides::default();
    let mut in_borders = false;
    let mut in_margins = false;
    while let Some(event) = reader.next() {
        match event {
            XmlEvent::Start {
                name,
                attributes,
                self_closing,
            } => match name {
                "w:tblStyle" => style = attribute(&attributes, "w:val").map(str::to_string),
                "w:tblBorders" if !self_closing => in_borders = true,
                "w:tblCellMar" if !self_closing => in_margins = true,
                "w:jc" if !in_margins && !in_borders => {
                    borders.alignment = match attribute(&attributes, "w:val") {
                        Some("center") => Some(Alignment::Center),
                        Some("right" | "end") => Some(Alignment::Right),
                        Some("left" | "start") => Some(Alignment::Left),
                        _ => None,
                    };
                }
                "w:tblInd" if matches!(attribute(&attributes, "w:type"), None | Some("dxa")) => {
                    borders.indent = attribute(&attributes, "w:w").and_then(twips_to_points);
                }
                side if in_margins => {
                    let index = match side {
                        "w:top" => 0,
                        "w:bottom" => 1,
                        "w:left" | "w:start" => 2,
                        "w:right" | "w:end" => 3,
                        _ => continue,
                    };
                    // Only absolute widths (dxa, the default) are margins Pages can take.
                    if matches!(attribute(&attributes, "w:type"), None | Some("dxa")) {
                        borders.margins[index] =
                            attribute(&attributes, "w:w").and_then(twips_to_points);
                    }
                }
                side if in_borders => {
                    let index = match side {
                        "w:top" => 0,
                        "w:bottom" => 1,
                        "w:left" | "w:start" => 2,
                        "w:right" | "w:end" => 3,
                        "w:insideH" => 4,
                        "w:insideV" => 5,
                        _ => continue,
                    };
                    borders.sides[index] = Some(border_line(&attributes));
                }
                _ => {}
            },
            XmlEvent::End {
                name: "w:tblBorders",
            } => in_borders = false,
            XmlEvent::End {
                name: "w:tblCellMar",
            } => in_margins = false,
            XmlEvent::End { name: "w:tblPr" } => break,
            _ => {}
        }
    }
    (style, borders)
}

/// A paragraph's `w:tabs`: its tab stops (a cleared one is left out).
fn read_tabs(reader: &mut XmlReader<'_>) -> Vec<crate::document::TabStop> {
    use crate::document::{TabAlignment, TabStop};
    let mut tabs = Vec::new();
    while let Some(event) = reader.next() {
        match event {
            XmlEvent::Start {
                name: "w:tab",
                attributes,
                ..
            } => {
                let alignment = match attribute(&attributes, "w:val") {
                    Some("clear") => continue,
                    Some("center") => TabAlignment::Center,
                    Some("right" | "end") => TabAlignment::Right,
                    Some("decimal") => TabAlignment::Decimal,
                    _ => TabAlignment::Left,
                };
                let Some(position) = attribute(&attributes, "w:pos").and_then(twips_to_points)
                else {
                    continue;
                };
                let leader = match attribute(&attributes, "w:leader") {
                    Some("dot") => Some('.'),
                    Some("hyphen") => Some('-'),
                    Some("underscore" | "heavy") => Some('_'),
                    Some("middleDot") => Some('·'),
                    _ => None,
                };
                tabs.push(TabStop {
                    position,
                    alignment,
                    leader,
                });
            }
            XmlEvent::End { name: "w:tabs" } => break,
            _ => {}
        }
    }
    tabs
}

/// A paragraph's `w:pBdr`: the sides that draw a line, with the first
/// line's look (Pages draws every side with one line).
fn read_paragraph_border(reader: &mut XmlReader<'_>) -> Option<crate::document::ParagraphBorder> {
    let mut border = crate::document::ParagraphBorder {
        top: false,
        bottom: false,
        left: false,
        right: false,
        line: Border {
            width: 0.5,
            color: None,
        },
    };
    let mut line: Option<Border> = None;
    while let Some(event) = reader.next() {
        match event {
            XmlEvent::Start {
                name, attributes, ..
            } => {
                let side = match name {
                    "w:top" => &mut border.top,
                    "w:bottom" => &mut border.bottom,
                    "w:left" | "w:start" => &mut border.left,
                    "w:right" | "w:end" => &mut border.right,
                    _ => continue,
                };
                if let Some(drawn) = border_line(&attributes) {
                    *side = true;
                    line.get_or_insert(drawn);
                }
            }
            XmlEvent::End { name: "w:pBdr" } => break,
            _ => {}
        }
    }
    border.line = line?;
    Some(border)
}

/// A border element's line, or `None` for "no line" (nil/none).
fn border_line(attributes: &[(&str, Cow<'_, str>)]) -> Option<Border> {
    let kind = attribute(attributes, "w:val").unwrap_or("single");
    if matches!(kind, "nil" | "none") {
        return None;
    }
    let width = attribute(attributes, "w:sz")
        .and_then(|size| size.parse::<f32>().ok())
        .map_or(0.5, |eighths| (eighths / 8.0).max(0.25));
    let color = attribute(attributes, "w:color")
        .filter(|value| *value != "auto")
        .and_then(parse_color);
    Some(Border { width, color })
}
