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
    Alignment, Anchor, AnchorBase, Baseline, Block, Caps, Cell, CharacterStyle, Color, Document,
    FloatingContent, FloatingObject, Inline, InlineImage, LineSpacing, ListItem, ListLabel,
    ListLevel, ListStyle, Media, MediaId, Merge, NoteId, NumberFormat, NumberKind, PageSetup,
    PageVariants, Paragraph, ParagraphProperties, ParagraphStyle, Placement, Revision,
    RevisionKind, Row, Run, RunProperties, Section, SectionStart, Span, StyleId, Table,
};
use crate::io::xml::{XmlEvent, XmlReader};
use crate::io::zip::{ZipArchive, ZipError};

#[derive(Debug)]
pub enum DocxError {
    Zip(ZipError),
    /// A required part is missing or not text.
    Part(&'static str),
}

impl fmt::Display for DocxError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DocxError::Zip(error) => write!(formatter, "not a Word package: {error:?}"),
            DocxError::Part(name) => write!(formatter, "Word package has no readable {name}"),
        }
    }
}

impl std::error::Error for DocxError {}

const OFFICE_DOCUMENT_REL: &str = "officeDocument/2006/relationships/officeDocument";

/// Reads the document out of a `.docx` file's bytes.
pub fn read_docx(bytes: &[u8]) -> Result<Document, DocxError> {
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
        after_note_mark: false,
    };
    reader.read_styles();
    reader.read_numbering();
    reader.read_settings();
    reader.rels = reader.relationships(&document_part);
    reader.read_notes("footnotes.xml", "w:footnote");
    reader.read_notes("endnotes.xml", "w:endnote");
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
}

/// A section's properties as read; the header and footer parts are
/// loaded when the section closes.
#[derive(Default)]
struct SectionHeader {
    page: PageSetup,
    columns: u16,
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
    footnotes: Vec<(i64, NoteId)>,
    /// Media parts already loaded, by package path.
    media: Vec<(String, MediaId)>,
    /// The relationships of the part being read.
    rels: Vec<Relationship>,
    /// Pages begun so far, by explicit breaks, for floating objects.
    page: u32,
    even_headers: bool,
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
                            self.document.styles.paragraph.push(ParagraphStyle {
                                name: style.name.unwrap_or_else(|| id.clone()),
                                parent: None,
                                paragraph: style.paragraph,
                                run: style.run,
                            });
                            paragraph_parents.push(style.based_on);
                            self.paragraph_styles.insert(id, index);
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
            let blocks = self.read_blocks(&mut reader, element);
            if is_separator || id < 0 {
                continue;
            }
            let note = self.document.footnotes.len();
            self.document
                .footnotes
                .push(crate::document::Note { blocks });
            self.footnotes.push((id, note));
        }
        self.rels = saved_rels;
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
            let part_blocks = self.read_blocks(&mut reader, root);
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
        self.document.sections.push(Section {
            page: header.page,
            columns: header.columns.max(1),
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
        let mut link: Option<u32> = None;
        let mut field_link: Option<u32> = None;
        let mut revision: Option<u32> = None;
        let mut field = Field::None;
        let mut instruction = String::new();
        let mut page_field = false;
        while let Some(event) = reader.next() {
            match event {
                XmlEvent::Start {
                    name,
                    attributes,
                    self_closing,
                } => match name {
                    "w:pPr" if !self_closing => {
                        let header = self.read_paragraph_properties(reader);
                        paragraph.style = header.style;
                        paragraph.list = header
                            .numbering
                            .filter(|(id, _)| *id > 0)
                            .and_then(|(id, level)| self.list_item(id, level));
                        paragraph.properties =
                            self.document.intern_paragraph_properties(header.properties);
                        paragraph.run_properties = self.document.intern_run_properties(header.mark);
                        section = header.section;
                    }
                    "w:r" if !self_closing => {
                        let run_link = field_link.or(link);
                        let control = self.read_run(
                            reader,
                            &mut paragraph,
                            run_link,
                            revision,
                            field,
                            &mut instruction,
                            page_field,
                        );
                        match control {
                            RunControl::None => {}
                            RunControl::FieldBegin => {
                                field = Field::Instruction;
                                instruction.clear();
                                page_field = false;
                            }
                            RunControl::FieldSeparate => {
                                field = Field::Result;
                                let (target, page) =
                                    self.field_meaning(&instruction, &mut paragraph);
                                field_link = target;
                                page_field = page;
                            }
                            RunControl::FieldEnd => {
                                if field == Field::Instruction {
                                    // No result: a bare page field still counts.
                                    let (_, page) =
                                        self.field_meaning(&instruction, &mut paragraph);
                                    let _ = page;
                                }
                                field = Field::None;
                                field_link = None;
                                page_field = false;
                            }
                        }
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
                            field_link = target;
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
                    "w:fldSimple" => field_link = None,
                    _ => {}
                },
                XmlEvent::Text(_) => {}
            }
        }
        blocks.push(Block::Paragraph(paragraph));
        section
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
    /// the paragraph. Field characters are reported back to the paragraph.
    #[allow(clippy::too_many_arguments)]
    #[inline(never)]
    fn read_run(
        &mut self,
        reader: &mut XmlReader<'_>,
        paragraph: &mut Paragraph,
        link: Option<u32>,
        revision: Option<u32>,
        field: Field,
        instruction: &mut String,
        page_field: bool,
    ) -> RunControl {
        let mut style: Option<StyleId> = None;
        let mut properties: Option<u32> = None;
        let mut control = RunControl::None;
        let hidden = field == Field::Instruction || (field == Field::Result && page_field);
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
                        instruction.push_str(self.document.text(span));
                    }
                    "w:fldChar" => {
                        control = match attribute(&attributes, "w:fldCharType") {
                            Some("begin") => RunControl::FieldBegin,
                            Some("separate") => RunControl::FieldSeparate,
                            Some("end") => RunControl::FieldEnd,
                            _ => control,
                        };
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
                        let content = if attribute(&attributes, "w:type") == Some("page") {
                            self.page += 1;
                            Inline::PageBreak
                        } else {
                            Inline::LineBreak
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
                    "w:footnoteReference" | "w:endnoteReference" if !hidden => {
                        let id: i64 = attribute(&attributes, "w:id")
                            .and_then(|value| value.parse().ok())
                            .unwrap_or(-1);
                        if let Some((_, note)) =
                            self.footnotes.iter().find(|(note_id, _)| *note_id == id)
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
                        if let Some(content) = self.read_drawing(reader) {
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
                                Drawn::Floating(object) => self.document.floating.push(object),
                            }
                        }
                    }
                    "w:pict" | "w:object" | "mc:Fallback" | "w:rPrChange" if !self_closing => {
                        skip_element(reader, name);
                    }
                    _ => {}
                },
                XmlEvent::End { name: "w:r" } => break,
                _ => {}
            }
        }
        control
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
                            if let Some(points) =
                                attribute(&attributes, "w:firstLine").and_then(twips_to_points)
                            {
                                header.properties.first_line_indent = Some(points);
                            }
                            if let Some(points) =
                                attribute(&attributes, "w:hanging").and_then(twips_to_points)
                            {
                                header.properties.first_line_indent = Some(-points);
                            }
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
                XmlEvent::End { name: "w:pPr" } => break,
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
                            let font = attribute(&attributes, "w:ascii")
                                .or_else(|| attribute(&attributes, "w:hAnsi"))
                                .or_else(|| attribute(&attributes, "w:cs"));
                            if let Some(font) = font {
                                read.properties.font = Some(self.document.intern_string(font));
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
        // Per row, per cell as read: (cell, grid span, vertical merge)
        let mut rows: Vec<(Row, Vec<(u32, VerticalMerge)>)> = Vec::new();
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
                    "w:tblPr" if !self_closing => skip_element(reader, name),
                    "w:tr" if !self_closing => {
                        let (row, spans, is_header) = self.read_row(reader);
                        if is_header && leading {
                            header_rows += 1;
                        } else {
                            leading = false;
                        }
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
        table
    }

    /// A row's cells as read, with each cell's grid span and vertical
    /// merge, and whether the row repeats as a header.
    fn read_row(&mut self, reader: &mut XmlReader<'_>) -> (Row, Vec<(u32, VerticalMerge)>, bool) {
        let mut row = Row::default();
        let mut spans = Vec::new();
        let mut is_header = false;
        while let Some(event) = reader.next() {
            match event {
                XmlEvent::Start {
                    name,
                    attributes,
                    self_closing,
                } => match name {
                    "w:tblHeader" => is_header = toggle(attribute(&attributes, "w:val")),
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
        (row, spans, is_header)
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
                        span = read.0;
                        merge = read.1;
                        cell.background = read.2;
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

    /// A drawing: a picture, inline or anchored, or a text box (a floating
    /// object). Groups yield their first picture or text box.
    #[inline(never)]
    fn read_drawing(&mut self, reader: &mut XmlReader<'_>) -> Option<Drawn> {
        let mut width = 0.0f32;
        let mut height = 0.0f32;
        let mut description: Option<String> = None;
        let mut anchored = false;
        let mut horizontal = Anchor {
            from: AnchorBase::Margin,
            offset: 0.0,
        };
        let mut vertical = Anchor {
            from: AnchorBase::Line,
            offset: 0.0,
        };
        let mut position_axis: Option<bool> = None;
        let mut media: Option<MediaId> = None;
        let mut text_box: Option<(Vec<Block>, Option<Color>)> = None;
        let mut fill: Option<Color> = None;
        let mut in_shape_properties = false;
        while let Some(event) = reader.next() {
            match event {
                XmlEvent::Start {
                    name,
                    attributes,
                    self_closing,
                } => match name {
                    "wp:anchor" => anchored = true,
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
                    "a:blip" => {
                        if media.is_none() {
                            media = attribute(&attributes, "r:embed")
                                .and_then(|id| self.load_media(id));
                        }
                    }
                    "wps:spPr" if !self_closing => in_shape_properties = true,
                    "a:srgbClr" if in_shape_properties && fill.is_none() => {
                        fill = attribute(&attributes, "val").and_then(parse_color);
                    }
                    "w:txbxContent" if !self_closing => {
                        let blocks = self.read_blocks(reader, "w:txbxContent");
                        if text_box.is_none() {
                            text_box = Some((blocks, None));
                        }
                    }
                    "mc:Fallback" if !self_closing => skip_element(reader, name),
                    _ => {}
                },
                XmlEvent::End { name } => match name {
                    "w:drawing" => break,
                    "wps:spPr" => in_shape_properties = false,
                    _ => {}
                },
                XmlEvent::Text(_) => {}
            }
        }
        if let Some((blocks, _)) = text_box {
            return Some(Drawn::Floating(FloatingObject {
                page: self.page,
                x: horizontal.offset,
                y: vertical.offset,
                width,
                height,
                content: FloatingContent::TextBox { blocks, fill },
            }));
        }
        let media = media?;
        // A picture positioned on the page (both axes) is page furniture,
        // not part of the text flow; the writer puts floating objects
        // there.
        if anchored && horizontal.from == AnchorBase::Page && vertical.from == AnchorBase::Page {
            return Some(Drawn::Floating(FloatingObject {
                page: self.page,
                x: horizontal.offset,
                y: vertical.offset,
                width,
                height,
                content: FloatingContent::Image(media),
            }));
        }
        let placement = if anchored {
            Placement::Floating {
                horizontal,
                vertical,
            }
        } else {
            Placement::Inline
        };
        Some(Drawn::Image(InlineImage {
            media,
            width,
            height,
            description,
            placement,
        }))
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

enum RunControl {
    None,
    FieldBegin,
    FieldSeparate,
    FieldEnd,
}

enum Drawn {
    Image(InlineImage),
    Floating(FloatingObject),
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
                    let (level, start) = read_level(reader);
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

fn read_level(reader: &mut XmlReader<'_>) -> (ListLevel, u32) {
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
        "bullet" => ListLabel::Text(text),
        _ => {
            let kind = match format {
                "lowerLetter" => NumberKind::LowerLetter,
                "upperLetter" => NumberKind::UpperLetter,
                "lowerRoman" => NumberKind::LowerRoman,
                "upperRoman" => NumberKind::UpperRoman,
                _ => NumberKind::Decimal,
            };
            // The pattern names this level's number as `%1` in the model.
            let mut pattern = text;
            for level in (1..=9).rev() {
                pattern = pattern.replace(&format!("%{level}"), "%1");
            }
            if pattern.is_empty() {
                pattern.push_str("%1.");
            }
            ListLabel::Number(NumberFormat { kind, pattern })
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

/// (grid span, vertical merge, background)
fn read_cell_properties(reader: &mut XmlReader<'_>) -> (u32, VerticalMerge, Option<Color>) {
    let mut span = 1u32;
    let mut merge = VerticalMerge::None;
    let mut background = None;
    while let Some(event) = reader.next() {
        match event {
            XmlEvent::Start {
                name, attributes, ..
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
                _ => {}
            },
            XmlEvent::End { name: "w:tcPr" } => break,
            _ => {}
        }
    }
    (span.max(1), merge, background)
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
            expanded.cells.push(origin);
            row_merges.push(merge);
            for _ in 1..span {
                expanded.cells.push(Cell {
                    column_span: 0,
                    row_span: 0,
                    merge: Merge::Left,
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

fn plain_run(content: Inline) -> Run {
    Run {
        style: None,
        properties: None,
        link: None,
        revision: None,
        content,
    }
}
