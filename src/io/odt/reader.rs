//! Reading OpenDocument Text into the document model: `styles.xml` first
//! (fonts, named styles, list styles, page layouts, and master pages with
//! their headers and footers), then `content.xml` (its automatic styles,
//! then the body). Paragraphs and headings, spans, links, lists, tables
//! with merged cells, footnotes and endnotes (as footnotes), comments,
//! tracked changes, pictures and formulas in frames, page and column
//! sections, and fields' shown text. Text boxes and shapes' text follow
//! the paragraph they are anchored in.

use std::collections::HashMap;
use std::fmt;

use super::styles::{
    CellStyle, PageLayout, attribute, cell_properties, display_name, length, list_level,
    page_layout, paragraph_properties, skip, tab_stops, text_properties,
};
use crate::document::{
    Alignment, Anchor, AnchorBase, Block, Cell, CharacterStyle, Comment, Document, Id, Inline,
    InlineImage, ListItem, ListLabel, ListStyle, Media, Merge, Note, PageVariants, Paragraph,
    ParagraphProperties, ParagraphStyle, Placement, Revision, RevisionKind, Row, Run,
    RunProperties, Section, SectionStart, StyleId, TabStop, Table,
};
use crate::io::xml::{XmlEvent, XmlReader};
use crate::io::zip::{ZipArchive, ZipError};

#[derive(Debug)]
pub enum OdtError {
    Zip(ZipError),
    /// A part the package needs is missing or not UTF-8.
    Part(&'static str),
}

impl fmt::Display for OdtError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OdtError::Zip(error) => write!(formatter, "{error}"),
            OdtError::Part(name) => write!(formatter, "the package has no readable {name}"),
        }
    }
}

impl std::error::Error for OdtError {}

impl From<ZipError> for OdtError {
    fn from(error: ZipError) -> Self {
        OdtError::Zip(error)
    }
}

/// Reads an `.odt` package into a document.
pub fn read_odt(bytes: &[u8]) -> Result<Document, OdtError> {
    let zip = ZipArchive::parse(bytes)?;
    let content = part_text(&zip, "content.xml")?.ok_or(OdtError::Part("content.xml"))?;
    let styles = part_text(&zip, "styles.xml")?;
    let mut reader = Reader {
        zip: &zip,
        document: Document::default(),
        fonts: HashMap::new(),
        named_paragraph: HashMap::new(),
        named_text: HashMap::new(),
        named_lists: HashMap::new(),
        scopes: [Scope::default(), Scope::default()],
        scope: STYLES,
        layouts: HashMap::new(),
        masters: HashMap::new(),
        media: HashMap::new(),
        changes: HashMap::new(),
        comments: HashMap::new(),
        pending: Vec::new(),
        default_list: None,
        last_master: "Standard".to_string(),
        replies: Vec::new(),
        reply_names: std::collections::HashSet::new(),
        starts: HashMap::new(),
    };
    if let Some(styles) = &styles {
        reader.read_styles_part(styles);
    }
    reader.scope = CONTENT;
    reader.read_content(&content);
    for (reply, parent) in std::mem::take(&mut reader.replies) {
        let parent = reader.comments.get(&parent).copied();
        if let Some(comment) = reader.document.comments.get_mut(reply as usize) {
            comment.reply_to = parent;
        }
    }
    if reader.document.sections.is_empty() {
        reader.document.sections.push(Section {
            columns: 1,
            ..Section::default()
        });
    }
    Ok(reader.document)
}

fn part_text(zip: &ZipArchive<'_>, name: &'static str) -> Result<Option<String>, OdtError> {
    let Some(entry) = zip.find(name) else {
        return Ok(None);
    };
    let mut bytes = Vec::new();
    zip.read(entry, &mut bytes)?;
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| OdtError::Part(name))
}

/// Automatic styles are named per part: `content.xml`'s for the body,
/// `styles.xml`'s for the master pages' headers and footers.
const CONTENT: usize = 0;
const STYLES: usize = 1;

#[derive(Default)]
struct Scope {
    paragraph: HashMap<String, ParaStyle>,
    text: HashMap<String, TextStyle>,
    lists: HashMap<String, StyleId>,
    cells: HashMap<String, CellStyle>,
    columns: HashMap<String, f32>,
    rows: HashMap<String, f32>,
    tables: HashMap<String, TableStyle>,
    sections: HashMap<String, (u16, Option<f32>)>,
}

/// A paragraph style as read: its model style (itself when named, its
/// parent when automatic), direct properties, list style, and master page.
#[derive(Clone, Default)]
struct ParaStyle {
    named: Option<StyleId>,
    parent: Option<String>,
    paragraph: ParagraphProperties,
    run: RunProperties,
    master_page: Option<String>,
    column_break: bool,
}

#[derive(Clone, Default)]
struct TextStyle {
    named: Option<StyleId>,
    run: RunProperties,
}

#[derive(Clone, Default)]
struct TableStyle {
    alignment: Option<Alignment>,
    indent: Option<f32>,
}

/// A master page: its page layout and its headers and footers.
#[derive(Clone, Default)]
struct Master {
    layout: String,
    headers: PageVariants,
    footers: PageVariants,
}

/// Inline state while a paragraph's content is read.
struct Para {
    runs: Vec<Run>,
    /// Whether the text so far ends in collapsible white space (true at
    /// the start: a paragraph's leading spaces are not text).
    space: bool,
    link: Option<Id>,
    revision: Option<Id>,
}

impl Para {
    fn new() -> Para {
        Para {
            runs: Vec::new(),
            space: true,
            link: None,
            revision: None,
        }
    }
}

/// A run's style: the named character style and direct properties.
#[derive(Clone, Copy, Default)]
struct RunStyle {
    named: Option<StyleId>,
    run: RunProperties,
}

struct Reader<'z> {
    zip: &'z ZipArchive<'z>,
    document: Document,
    fonts: HashMap<String, String>,
    named_paragraph: HashMap<String, ParaStyle>,
    named_text: HashMap<String, TextStyle>,
    named_lists: HashMap<String, StyleId>,
    scopes: [Scope; 2],
    scope: usize,
    layouts: HashMap<String, PageLayout>,
    masters: HashMap<String, Master>,
    media: HashMap<String, usize>,
    /// Tracked changes by id: their revision, and a deletion's text.
    changes: HashMap<String, (Id, Vec<Run>)>,
    /// Comments by their range name.
    comments: HashMap<String, Id>,
    /// Blocks met inside a paragraph (text boxes, shapes' text) that
    /// follow it.
    pending: Vec<Block>,
    default_list: Option<StyleId>,
    /// The master page of the section being filled.
    last_master: String,
    /// Replies and their parents' names, linked once all are read; and
    /// the replies' own range names.
    replies: Vec<(Id, String)>,
    reply_names: std::collections::HashSet<String>,
    /// Each list style's levels' first numbers.
    starts: HashMap<StyleId, Vec<u32>>,
}

impl Reader<'_> {
    // ----- styles -----

    fn read_styles_part(&mut self, xml: &str) {
        let mut reader = XmlReader::new(xml);
        while let Some(event) = reader.next() {
            match event {
                XmlEvent::Start {
                    name: "office:font-face-decls",
                    self_closing: false,
                    ..
                } => self.read_fonts(&mut reader),
                XmlEvent::Start {
                    name: "office:styles",
                    self_closing: false,
                    ..
                } => self.read_style_list(&mut reader, "office:styles", true),
                XmlEvent::Start {
                    name: "office:automatic-styles",
                    self_closing: false,
                    ..
                } => self.read_style_list(&mut reader, "office:automatic-styles", false),
                XmlEvent::Start {
                    name: "office:master-styles",
                    self_closing: false,
                    ..
                } => self.read_masters(&mut reader),
                _ => {}
            }
        }
        self.link_named_styles();
    }

    fn read_fonts(&mut self, reader: &mut XmlReader<'_>) {
        while let Some(event) = reader.next() {
            match event {
                XmlEvent::Start {
                    name: "style:font-face",
                    attributes,
                    self_closing,
                } => {
                    let mut face_family = None;
                    if let (Some(name), Some(family)) = (
                        attribute(&attributes, "style:name"),
                        attribute(&attributes, "svg:font-family"),
                    ) {
                        let family = family.trim().trim_matches('\'').trim_matches('"');
                        self.fonts.insert(name.to_string(), family.to_string());
                        face_family = Some(family.to_string());
                    }
                    if !self_closing {
                        // Embedded faces: each file with its style and weight.
                        for event in reader.by_ref() {
                            match event {
                                XmlEvent::Start {
                                    name: "svg:font-face-uri",
                                    attributes,
                                    ..
                                } => {
                                    let (Some(family), Some(path)) =
                                        (&face_family, attribute(&attributes, "xlink:href"))
                                    else {
                                        continue;
                                    };
                                    let path = path.trim_start_matches("./");
                                    let Some(entry) = self.zip.find(path) else {
                                        continue;
                                    };
                                    let mut bytes = Vec::new();
                                    if self.zip.read(entry, &mut bytes).is_err() {
                                        continue;
                                    }
                                    let bold = attribute(&attributes, "loext:font-weight")
                                        .is_some_and(|weight| {
                                            weight == "bold"
                                                || weight
                                                    .parse::<u32>()
                                                    .is_ok_and(|weight| weight >= 600)
                                        });
                                    let italic = attribute(&attributes, "loext:font-style")
                                        .is_some_and(|style| {
                                            style == "italic" || style == "oblique"
                                        });
                                    let known = self.document.fonts.iter().any(|font| {
                                        font.family == *family
                                            && font.bold == bold
                                            && font.italic == italic
                                    });
                                    if !known {
                                        self.document.fonts.push(crate::document::EmbeddedFont {
                                            family: family.clone(),
                                            bold,
                                            italic,
                                            bytes,
                                        });
                                    }
                                }
                                XmlEvent::End {
                                    name: "style:font-face",
                                } => break,
                                _ => {}
                            }
                        }
                    }
                }
                XmlEvent::End {
                    name: "office:font-face-decls",
                } => break,
                _ => {}
            }
        }
    }

    /// The styles of `office:styles` (`named`) or `office:automatic-styles`.
    fn read_style_list(&mut self, reader: &mut XmlReader<'_>, end: &str, named: bool) {
        while let Some(event) = reader.next() {
            match event {
                XmlEvent::Start {
                    name: element @ ("style:style" | "style:default-style"),
                    attributes,
                    self_closing,
                } => {
                    let default = element == "style:default-style";
                    let family = attribute(&attributes, "style:family")
                        .unwrap_or("")
                        .to_string();
                    let name = if default {
                        "\u{0}default".to_string()
                    } else {
                        attribute(&attributes, "style:name")
                            .unwrap_or("")
                            .to_string()
                    };
                    let shown = display_name(&attributes, &name);
                    let parent =
                        attribute(&attributes, "style:parent-style-name").map(str::to_string);
                    let master_page = attribute(&attributes, "style:master-page-name")
                        .filter(|master| !master.is_empty())
                        .map(str::to_string);
                    let outline = attribute(&attributes, "style:default-outline-level")
                        .and_then(|level| level.parse::<u8>().ok())
                        .filter(|level| *level >= 1);
                    let mut style = ParaStyle {
                        parent,
                        master_page,
                        ..ParaStyle::default()
                    };
                    let mut tabs: Vec<TabStop> = Vec::new();
                    let mut cell = CellStyle::default();
                    let mut column = None;
                    let mut row = None;
                    let mut table = TableStyle::default();
                    let mut section = (1u16, None);
                    if !self_closing {
                        while let Some(event) = reader.next() {
                            match event {
                                XmlEvent::Start {
                                    name: "style:text-properties",
                                    attributes,
                                    ..
                                } => {
                                    style.run = text_properties(
                                        &attributes,
                                        &self.fonts,
                                        &mut self.document,
                                    );
                                }
                                XmlEvent::Start {
                                    name: "style:paragraph-properties",
                                    attributes,
                                    self_closing,
                                } => {
                                    style.paragraph = paragraph_properties(&attributes);
                                    if attribute(&attributes, "fo:break-before") == Some("column") {
                                        style.column_break = true;
                                    }
                                    if let Some(number) =
                                        attribute(&attributes, "style:page-number")
                                            .and_then(|number| number.parse::<u32>().ok())
                                    {
                                        // A restart of the page numbers,
                                        // kept with the master page change.
                                        let _ = number;
                                    }
                                    if !self_closing {
                                        let indent = style.paragraph.left_indent.unwrap_or(0.0);
                                        while let Some(event) = reader.next() {
                                            match event {
                                                XmlEvent::Start {
                                                    name: "style:tab-stops",
                                                    self_closing: false,
                                                    ..
                                                } => tabs = tab_stops(reader, indent),
                                                XmlEvent::End {
                                                    name: "style:paragraph-properties",
                                                } => break,
                                                _ => {}
                                            }
                                        }
                                    }
                                }
                                XmlEvent::Start {
                                    name: "style:table-cell-properties",
                                    attributes,
                                    ..
                                } => cell = cell_properties(&attributes),
                                XmlEvent::Start {
                                    name: "style:table-column-properties",
                                    attributes,
                                    ..
                                } => {
                                    column = attribute(&attributes, "style:column-width")
                                        .and_then(length);
                                }
                                XmlEvent::Start {
                                    name: "style:table-row-properties",
                                    attributes,
                                    ..
                                } => {
                                    row = attribute(&attributes, "style:min-row-height")
                                        .or_else(|| attribute(&attributes, "style:row-height"))
                                        .and_then(length);
                                }
                                XmlEvent::Start {
                                    name: "style:table-properties",
                                    attributes,
                                    ..
                                } => {
                                    table.alignment = match attribute(&attributes, "table:align") {
                                        Some("center") => Some(Alignment::Center),
                                        Some("right") => Some(Alignment::Right),
                                        Some("left") => Some(Alignment::Left),
                                        _ => None,
                                    };
                                    table.indent =
                                        attribute(&attributes, "fo:margin-left").and_then(length);
                                }
                                XmlEvent::Start {
                                    name: "style:columns",
                                    attributes,
                                    ..
                                } => {
                                    section = (
                                        attribute(&attributes, "fo:column-count")
                                            .and_then(|count| count.parse().ok())
                                            .unwrap_or(1u16)
                                            .max(1),
                                        attribute(&attributes, "fo:column-gap").and_then(length),
                                    );
                                }
                                XmlEvent::End { name } if name == element => break,
                                _ => {}
                            }
                        }
                    }
                    if !tabs.is_empty() {
                        style.paragraph.tabs = Some(self.document.intern_tabs(tabs));
                    }
                    if let Some(level) = outline {
                        style.paragraph.outline_level = Some(level - 1);
                    }
                    let scope = &mut self.scopes[self.scope];
                    match family.as_str() {
                        "paragraph" if named => {
                            let id = self.document.styles.paragraph.len();
                            self.document.styles.paragraph.push(ParagraphStyle {
                                name: if default {
                                    "Default Paragraph Style".to_string()
                                } else {
                                    shown
                                },
                                parent: None,
                                paragraph: style.paragraph,
                                run: style.run,
                            });
                            style.named = Some(id);
                            self.named_paragraph.insert(name, style);
                        }
                        "paragraph" => {
                            scope.paragraph.insert(name, style);
                        }
                        "text" if named && !default => {
                            let id = self.document.styles.character.len();
                            self.document.styles.character.push(CharacterStyle {
                                name: shown,
                                parent: None,
                                run: style.run,
                            });
                            self.named_text.insert(
                                name,
                                TextStyle {
                                    named: Some(id),
                                    run: RunProperties::default(),
                                },
                            );
                        }
                        "text" => {
                            let named = style
                                .parent
                                .as_ref()
                                .and_then(|parent| self.named_text.get(parent))
                                .and_then(|parent| parent.named);
                            scope.text.insert(
                                name,
                                TextStyle {
                                    named,
                                    run: style.run,
                                },
                            );
                        }
                        "table-cell" => {
                            scope.cells.insert(name, cell);
                        }
                        "table-column" => {
                            if let Some(width) = column {
                                scope.columns.insert(name, width);
                            }
                        }
                        "table-row" => {
                            if let Some(height) = row {
                                scope.rows.insert(name, height);
                            }
                        }
                        "table" => {
                            scope.tables.insert(name, table);
                        }
                        "section" => {
                            scope.sections.insert(name, section);
                        }
                        _ => {}
                    }
                }
                XmlEvent::Start {
                    name: "text:list-style",
                    attributes,
                    self_closing,
                } => {
                    let name = attribute(&attributes, "style:name")
                        .unwrap_or("")
                        .to_string();
                    let shown = display_name(&attributes, &name);
                    let (style, starts) = if self_closing {
                        (
                            ListStyle {
                                name: shown,
                                levels: Vec::new(),
                            },
                            Vec::new(),
                        )
                    } else {
                        read_list_style(reader, shown)
                    };
                    let id = self.document.styles.list.len();
                    self.document.styles.list.push(style);
                    self.starts.insert(id, starts);
                    if named {
                        self.named_lists.insert(name, id);
                    } else {
                        self.scopes[self.scope].lists.insert(name, id);
                    }
                }
                XmlEvent::Start {
                    name: "style:page-layout",
                    attributes,
                    self_closing,
                } => {
                    let name = attribute(&attributes, "style:name")
                        .unwrap_or("")
                        .to_string();
                    let layout = if self_closing {
                        PageLayout::default()
                    } else {
                        page_layout(reader)
                    };
                    self.layouts.insert(name, layout);
                }
                XmlEvent::Start {
                    name,
                    self_closing: false,
                    ..
                } if name != end => skip(reader, name),
                XmlEvent::End { name } if name == end => break,
                _ => {}
            }
        }
    }

    /// Named paragraph and character styles get their parents, and a
    /// paragraph style without one the default style's properties.
    fn link_named_styles(&mut self) {
        let default = self
            .named_paragraph
            .get("\u{0}default")
            .and_then(|style| style.named);
        let links: Vec<(StyleId, Option<StyleId>)> = self
            .named_paragraph
            .iter()
            .filter_map(|(name, style)| {
                let id = style.named?;
                if name == "\u{0}default" {
                    return None;
                }
                let parent = style
                    .parent
                    .as_ref()
                    .and_then(|parent| self.named_paragraph.get(parent))
                    .and_then(|parent| parent.named)
                    .or(default);
                Some((id, parent))
            })
            .collect();
        for (id, parent) in links {
            if parent != Some(id) {
                self.document.styles.paragraph[id].parent = parent;
            }
        }
        self.document.styles.default_paragraph = self
            .named_paragraph
            .get("Standard")
            .and_then(|style| style.named);
    }

    fn read_masters(&mut self, reader: &mut XmlReader<'_>) {
        while let Some(event) = reader.next() {
            match event {
                XmlEvent::Start {
                    name: "style:master-page",
                    attributes,
                    self_closing,
                } => {
                    let name = attribute(&attributes, "style:name")
                        .unwrap_or("")
                        .to_string();
                    let mut master = Master {
                        layout: attribute(&attributes, "style:page-layout-name")
                            .unwrap_or("")
                            .to_string(),
                        ..Master::default()
                    };
                    if !self_closing {
                        while let Some(event) = reader.next() {
                            match event {
                                XmlEvent::Start {
                                    name: part,
                                    attributes,
                                    self_closing,
                                } if part.starts_with("style:header")
                                    || part.starts_with("style:footer") =>
                                {
                                    let shown =
                                        attribute(&attributes, "style:display") != Some("false");
                                    if self_closing {
                                        continue;
                                    }
                                    let blocks = self.read_blocks(reader, part);
                                    if !shown {
                                        continue;
                                    }
                                    let variants = if part.starts_with("style:header") {
                                        &mut master.headers
                                    } else {
                                        &mut master.footers
                                    };
                                    if part.ends_with("-first") {
                                        variants.first = Some(blocks);
                                    } else if part.ends_with("-left") {
                                        variants.even = Some(blocks);
                                    } else {
                                        variants.default = Some(blocks);
                                    }
                                }
                                XmlEvent::Start {
                                    name,
                                    self_closing: false,
                                    ..
                                } => skip(reader, name),
                                XmlEvent::End {
                                    name: "style:master-page",
                                } => break,
                                _ => {}
                            }
                        }
                    }
                    self.masters.insert(name, master);
                }
                XmlEvent::End {
                    name: "office:master-styles",
                } => break,
                _ => {}
            }
        }
    }

    fn paragraph_style(&self, name: &str) -> Option<&ParaStyle> {
        self.scopes[self.scope]
            .paragraph
            .get(name)
            .or_else(|| self.named_paragraph.get(name))
    }

    fn text_style(&self, name: &str) -> Option<&TextStyle> {
        self.scopes[self.scope]
            .text
            .get(name)
            .or_else(|| self.named_text.get(name))
    }

    fn list_style(&mut self, name: Option<&str>) -> StyleId {
        if let Some(found) = name.and_then(|name| {
            self.scopes[self.scope]
                .lists
                .get(name)
                .or_else(|| self.named_lists.get(name))
        }) {
            return *found;
        }
        if let Some(id) = self.default_list {
            return id;
        }
        let id = self.document.styles.list.len();
        self.document.styles.list.push(ListStyle {
            name: "List".to_string(),
            levels: (0..10)
                .map(|level| crate::document::ListLevel {
                    label: ListLabel::Text("•".to_string()),
                    indent: 18.0 * (level as f32 + 1.0),
                    label_indent: 18.0 * level as f32,
                })
                .collect(),
        });
        self.default_list = Some(id);
        id
    }

    // ----- the body -----

    fn read_content(&mut self, xml: &str) {
        let mut reader = XmlReader::new(xml);
        while let Some(event) = reader.next() {
            match event {
                XmlEvent::Start {
                    name: "office:font-face-decls",
                    self_closing: false,
                    ..
                } => self.read_fonts(&mut reader),
                XmlEvent::Start {
                    name: "office:automatic-styles",
                    self_closing: false,
                    ..
                } => self.read_style_list(&mut reader, "office:automatic-styles", false),
                XmlEvent::Start {
                    name: "office:text",
                    self_closing: false,
                    ..
                } => self.read_body(&mut reader),
                _ => {}
            }
        }
    }

    /// The body's blocks into sections: a new one where a paragraph names
    /// another master page, or a text section changes the columns.
    fn read_body(&mut self, reader: &mut XmlReader<'_>) {
        self.start_section("Standard", SectionStart::NewPage, None);
        let mut first = true;
        while let Some(event) = reader.next() {
            match event {
                XmlEvent::Start {
                    name: "text:tracked-changes",
                    self_closing: false,
                    ..
                } => self.read_tracked_changes(reader),
                XmlEvent::Start {
                    name: "text:section",
                    attributes,
                    self_closing: false,
                } => {
                    let columns = attribute(&attributes, "text:style-name")
                        .and_then(|style| self.scopes[self.scope].sections.get(style).copied());
                    let blocks = self.read_blocks(reader, "text:section");
                    match columns {
                        Some((count, gap)) if count > 1 => {
                            let master = self.current_master();
                            self.start_section(
                                &master,
                                SectionStart::Continuous,
                                Some((count, gap)),
                            );
                            self.push_blocks(blocks);
                            self.start_section(&master, SectionStart::Continuous, None);
                        }
                        _ => self.push_blocks(blocks),
                    }
                    first = false;
                }
                XmlEvent::Start {
                    name,
                    attributes,
                    self_closing,
                } => {
                    if let Some(master) = self.master_change(name, &attributes) {
                        let empty = self
                            .document
                            .sections
                            .last()
                            .is_none_or(|section| section.blocks.is_empty());
                        if first && empty {
                            self.document.sections.pop();
                            self.start_section(&master, SectionStart::NewPage, None);
                        } else {
                            self.start_section(&master, SectionStart::NewPage, None);
                        }
                    }
                    let mut blocks = Vec::new();
                    self.read_block(reader, name, &attributes, self_closing, &mut blocks);
                    if !blocks.is_empty() {
                        first = false;
                    }
                    self.push_blocks(blocks);
                }
                XmlEvent::End {
                    name: "office:text",
                } => break,
                _ => {}
            }
        }
    }

    fn push_blocks(&mut self, blocks: Vec<Block>) {
        if let Some(section) = self.document.sections.last_mut() {
            section.blocks.extend(blocks);
        }
    }

    /// The master page a paragraph or heading starts, if its style names one.
    fn master_change(
        &self,
        name: &str,
        attributes: &[(&str, std::borrow::Cow<'_, str>)],
    ) -> Option<String> {
        if !matches!(name, "text:p" | "text:h") {
            return None;
        }
        let style = attribute(attributes, "text:style-name")?;
        self.paragraph_style(style)?.master_page.clone()
    }

    fn current_master(&self) -> String {
        self.last_master.clone()
    }

    fn start_section(
        &mut self,
        master: &str,
        start: SectionStart,
        columns: Option<(u16, Option<f32>)>,
    ) {
        let found = self
            .masters
            .get(master)
            .cloned()
            .or_else(|| self.masters.values().next().cloned());
        let master_page = found.unwrap_or_default();
        let layout = self
            .layouts
            .get(&master_page.layout)
            .cloned()
            .unwrap_or_default();
        let (count, gap) = columns.unwrap_or((layout.columns, layout.column_gap));
        let page = layout.setup(
            !master_page.headers.is_empty(),
            !master_page.footers.is_empty(),
        );
        if self.document.sections.is_empty() {
            self.document.page_color = layout.background;
        }
        self.last_master = master.to_string();
        self.document.sections.push(Section {
            page,
            columns: count,
            column_gap: gap,
            column_widths: Vec::new(),
            start,
            headers: master_page.headers,
            footers: master_page.footers,
            blocks: Vec::new(),
        });
    }

    /// Blocks until the end of `end`.
    fn read_blocks(&mut self, reader: &mut XmlReader<'_>, end: &str) -> Vec<Block> {
        let mut blocks = Vec::new();
        while let Some(event) = reader.next() {
            match event {
                XmlEvent::Start {
                    name,
                    attributes,
                    self_closing,
                } => self.read_block(reader, name, &attributes, self_closing, &mut blocks),
                XmlEvent::End { name } if name == end => break,
                _ => {}
            }
        }
        blocks
    }

    fn read_block(
        &mut self,
        reader: &mut XmlReader<'_>,
        name: &str,
        attributes: &[(&str, std::borrow::Cow<'_, str>)],
        self_closing: bool,
        blocks: &mut Vec<Block>,
    ) {
        match name {
            "text:p" | "text:h" => {
                let paragraph = self.read_paragraph(reader, name, attributes, self_closing, None);
                blocks.push(Block::Paragraph(paragraph));
                blocks.append(&mut self.pending);
            }
            "text:list" if !self_closing => {
                self.read_list(reader, attributes, 0, None, blocks);
            }
            "table:table" if !self_closing => {
                let table = self.read_table(reader, attributes);
                blocks.push(Block::Table(Box::new(table)));
            }
            "text:section" | "text:index-body" | "text:numbered-paragraph" if !self_closing => {
                let inner = self.read_blocks(reader, name);
                blocks.extend(inner);
            }
            _ if !self_closing && (name == "text:table-of-content" || name.ends_with("-index")) => {
                // An index's shown entries are in its body.
                while let Some(event) = reader.next() {
                    match event {
                        XmlEvent::Start {
                            name: "text:index-body",
                            self_closing: false,
                            ..
                        } => {
                            let inner = self.read_blocks(reader, "text:index-body");
                            blocks.extend(inner);
                        }
                        XmlEvent::Start {
                            name: inner,
                            self_closing: false,
                            ..
                        } => skip(reader, inner),
                        XmlEvent::End { name: inner } if inner == name => break,
                        _ => {}
                    }
                }
            }
            "text:tracked-changes" if !self_closing => self.read_tracked_changes(reader),
            _ if !self_closing && name.starts_with("draw:") => {
                // A frame anchored to the page: a picture becomes a
                // paragraph of its own; a text box's blocks follow.
                let mut para = Para::new();
                self.read_inline_element(
                    reader,
                    name,
                    attributes,
                    self_closing,
                    &mut para,
                    RunStyle::default(),
                );
                if !para.runs.is_empty() {
                    blocks.push(Block::Paragraph(Paragraph {
                        runs: para.runs,
                        ..Paragraph::default()
                    }));
                }
                blocks.append(&mut self.pending);
            }
            _ if !self_closing => skip(reader, name),
            _ => {}
        }
    }

    fn read_list(
        &mut self,
        reader: &mut XmlReader<'_>,
        attributes: &[(&str, std::borrow::Cow<'_, str>)],
        level: u8,
        inherited: Option<StyleId>,
        blocks: &mut Vec<Block>,
    ) {
        let named = attribute(attributes, "text:style-name").map(str::to_string);
        let style = match (&named, inherited) {
            (Some(name), _) => self.list_style(Some(name)),
            (None, Some(style)) => style,
            (None, None) => self.list_style(None),
        };
        let continues = attribute(attributes, "text:continue-numbering") == Some("true")
            || attribute(attributes, "text:continue-list").is_some();
        let mut first = level == 0 && !continues;
        while let Some(event) = reader.next() {
            match event {
                XmlEvent::Start {
                    name: item @ ("text:list-item" | "text:list-header"),
                    attributes,
                    self_closing: false,
                } => {
                    let restart = attribute(&attributes, "text:start-value")
                        .and_then(|value| value.parse::<u32>().ok());
                    let item_style = match attribute(&attributes, "text:style-override") {
                        Some(name) => self.list_style(Some(name)),
                        None => style,
                    };
                    let numbered = item == "text:list-item";
                    let mut marked = false;
                    while let Some(event) = reader.next() {
                        match event {
                            XmlEvent::Start {
                                name: "text:list",
                                attributes,
                                self_closing: false,
                            } => {
                                self.read_list(reader, &attributes, level + 1, Some(style), blocks)
                            }
                            XmlEvent::Start {
                                name: paragraph @ ("text:p" | "text:h"),
                                attributes,
                                self_closing,
                            } => {
                                // The item's first paragraph carries its
                                // label; any after it are continuations.
                                let list = (numbered && !marked).then(|| ListItem {
                                    style: item_style,
                                    level: level.min(9),
                                    starts_list: first || restart.is_some(),
                                    start: restart
                                        .unwrap_or_else(|| self.level_start(style, level)),
                                });
                                marked = true;
                                first = false;
                                let paragraph = self.read_paragraph(
                                    reader,
                                    paragraph,
                                    &attributes,
                                    self_closing,
                                    list,
                                );
                                blocks.push(Block::Paragraph(paragraph));
                                blocks.append(&mut self.pending);
                            }
                            XmlEvent::Start {
                                name,
                                attributes,
                                self_closing,
                            } => self.read_block(reader, name, &attributes, self_closing, blocks),
                            XmlEvent::End { name } if name == item => break,
                            _ => {}
                        }
                    }
                }
                XmlEvent::Start {
                    name,
                    self_closing: false,
                    ..
                } => skip(reader, name),
                XmlEvent::End { name: "text:list" } => break,
                _ => {}
            }
        }
    }

    /// The number a list style's level starts from.
    fn level_start(&self, style: StyleId, level: u8) -> u32 {
        self.starts
            .get(&style)
            .and_then(|starts| starts.get(level as usize))
            .copied()
            .unwrap_or(1)
    }

    fn read_paragraph(
        &mut self,
        reader: &mut XmlReader<'_>,
        name: &str,
        attributes: &[(&str, std::borrow::Cow<'_, str>)],
        self_closing: bool,
        list: Option<ListItem>,
    ) -> Paragraph {
        let mut paragraph = Paragraph {
            list,
            ..Paragraph::default()
        };
        let style = attribute(attributes, "text:style-name")
            .and_then(|style| self.paragraph_style(style))
            .cloned();
        let mut direct = ParagraphProperties::default();
        let mut column_break = false;
        if let Some(style) = &style {
            match style.named {
                // A named style is the paragraph's style; an automatic one
                // is direct formatting over its parent.
                Some(id) => paragraph.style = Some(id),
                None => {
                    paragraph.style = style
                        .parent
                        .as_ref()
                        .and_then(|parent| self.named_paragraph.get(parent))
                        .and_then(|parent| parent.named);
                    direct = style.paragraph;
                    paragraph.run_properties = self.document.intern_run_properties(style.run);
                }
            }
            column_break = style.column_break;
        }
        if name == "text:h" {
            let level = attribute(attributes, "text:outline-level")
                .and_then(|level| level.parse::<u8>().ok())
                .unwrap_or(1)
                .clamp(1, 10);
            direct.outline_level = Some(level - 1);
        }
        paragraph.properties = self.document.intern_paragraph_properties(direct);
        if self_closing {
            return paragraph;
        }
        let mut para = Para::new();
        if column_break {
            self.push_run(&mut para, RunStyle::default(), Inline::ColumnBreak);
        }
        self.read_inline(reader, name, &mut para, RunStyle::default());
        paragraph.runs = para.runs;
        paragraph
    }

    // ----- inline content -----

    fn read_inline(
        &mut self,
        reader: &mut XmlReader<'_>,
        end: &str,
        para: &mut Para,
        style: RunStyle,
    ) {
        while let Some(event) = reader.next() {
            match event {
                XmlEvent::Text(text) => self.push_text(para, style, &text),
                XmlEvent::End { name } if name == end => break,
                XmlEvent::Start {
                    name,
                    attributes,
                    self_closing,
                } => self.read_inline_element(reader, name, &attributes, self_closing, para, style),
                _ => {}
            }
        }
    }

    fn read_inline_element(
        &mut self,
        reader: &mut XmlReader<'_>,
        name: &str,
        attributes: &[(&str, std::borrow::Cow<'_, str>)],
        self_closing: bool,
        para: &mut Para,
        style: RunStyle,
    ) {
        match name {
            "text:span" => {
                let mut inner = style;
                if let Some(found) =
                    attribute(attributes, "text:style-name").and_then(|name| self.text_style(name))
                {
                    if found.named.is_some() {
                        inner.named = found.named;
                    }
                    inner.run.overlay(&found.run);
                }
                if !self_closing {
                    self.read_inline(reader, name, para, inner);
                }
            }
            "text:a" => {
                let previous = para.link;
                if let Some(target) = attribute(attributes, "xlink:href") {
                    para.link = Some(self.document.intern_link(target));
                }
                if !self_closing {
                    self.read_inline(reader, name, para, style);
                }
                para.link = previous;
            }
            "text:s" => {
                let count = attribute(attributes, "text:c")
                    .and_then(|count| count.parse::<usize>().ok())
                    .unwrap_or(1);
                self.push_literal(para, style, &" ".repeat(count));
                para.space = true;
            }
            "text:tab" => {
                self.push_run(para, style, Inline::Tab);
                para.space = true;
                if !self_closing {
                    skip(reader, name);
                }
            }
            "text:line-break" => {
                self.push_run(para, style, Inline::LineBreak);
                para.space = true;
            }
            "text:page-number" => {
                self.push_run(para, style, Inline::PageNumber);
                para.space = false;
                if !self_closing {
                    skip(reader, name);
                }
            }
            "text:page-count" => {
                self.push_run(para, style, Inline::PageCount);
                para.space = false;
                if !self_closing {
                    skip(reader, name);
                }
            }
            "text:note" if !self_closing => self.read_note(reader, para, style),
            "office:annotation" if !self_closing => {
                let range = attribute(attributes, "office:name").map(str::to_string);
                let parent = attribute(attributes, "loext:parent-name").map(str::to_string);
                let id = self.read_annotation(reader);
                if let Some(range) = &range {
                    self.comments.insert(range.clone(), id);
                }
                // A reply shares its parent's range and has no markers;
                // its parent may come later, so it is linked at the end.
                if let Some(parent) = parent {
                    self.replies.push((id, parent));
                    if let Some(range) = range {
                        self.reply_names.insert(range);
                    }
                    return;
                }
                self.push_run(para, style, Inline::CommentStart(id));
                if range.is_none() {
                    self.push_run(para, style, Inline::CommentEnd(id));
                }
            }
            "office:annotation-end" => {
                let name = attribute(attributes, "office:name").unwrap_or("");
                if !self.reply_names.contains(name) {
                    if let Some(id) = self.comments.get(name).copied() {
                        self.push_run(para, style, Inline::CommentEnd(id));
                    }
                }
            }
            "text:change-start" => {
                para.revision = attribute(attributes, "text:change-id")
                    .and_then(|id| self.changes.get(id))
                    .map(|(revision, _)| *revision);
            }
            "text:change-end" => para.revision = None,
            "text:change" => {
                // A deletion's place: its text, kept as deleted runs.
                if let Some((revision, runs)) =
                    attribute(attributes, "text:change-id").and_then(|id| self.changes.get(id))
                {
                    let revision = *revision;
                    for run in runs.clone() {
                        para.runs.push(Run {
                            revision: Some(revision),
                            ..run
                        });
                    }
                }
            }
            "field:fieldmark" | "field:fieldmark-start" => {
                // A form checkbox shows as its box; other form fields keep
                // the text they show.
                let checkbox = attribute(attributes, "field:type")
                    .is_some_and(|kind| kind.ends_with("FORMCHECKBOX"));
                let mut checked = false;
                if !self_closing {
                    for event in reader.by_ref() {
                        match event {
                            XmlEvent::Start {
                                name: "field:param",
                                attributes,
                                ..
                            } => {
                                if attribute(&attributes, "field:name") == Some("Checkbox_Checked")
                                {
                                    checked = attribute(&attributes, "field:value") == Some("true");
                                }
                            }
                            XmlEvent::End { name: inner } if inner == name => break,
                            _ => {}
                        }
                    }
                }
                if checkbox {
                    self.push_literal(para, style, if checked { "☒" } else { "☐" });
                    para.space = false;
                }
            }
            "draw:frame" if !self_closing => self.read_frame(reader, attributes, para, style),
            "draw:a" if !self_closing => self.read_inline(reader, name, para, style),
            "text:ruby" if !self_closing => {
                // The base text; the annotation above it is dropped.
                while let Some(event) = reader.next() {
                    match event {
                        XmlEvent::Start {
                            name: "text:ruby-base",
                            self_closing: false,
                            ..
                        } => self.read_inline(reader, "text:ruby-base", para, style),
                        XmlEvent::Start {
                            name: inner,
                            self_closing: false,
                            ..
                        } => skip(reader, inner),
                        XmlEvent::End { name: "text:ruby" } => break,
                        _ => {}
                    }
                }
            }
            "text:soft-page-break"
            | "text:bookmark"
            | "text:bookmark-start"
            | "text:bookmark-end"
            | "text:reference-mark"
            | "text:reference-mark-start"
            | "text:reference-mark-end"
            | "text:alphabetical-index-mark"
            | "text:toc-mark"
            | "text:user-index-mark" => {
                if !self_closing {
                    skip(reader, name);
                }
            }
            _ if name.starts_with("draw:") => {
                // A shape: its text follows the paragraph.
                if !self_closing {
                    let blocks = self.read_blocks(reader, name);
                    self.pending.extend(blocks);
                }
            }
            _ if name.starts_with("text:") || name.starts_with("office:") => {
                // Fields and marks with shown text keep their text.
                if !self_closing {
                    self.read_inline(reader, name, para, style);
                }
            }
            _ => {
                if !self_closing {
                    skip(reader, name);
                }
            }
        }
    }

    /// Text with ODF's white space rule: runs of spaces, tabs, and
    /// newlines are one space, and none at a paragraph's start.
    fn push_text(&mut self, para: &mut Para, style: RunStyle, text: &str) {
        let mut out = String::with_capacity(text.len());
        for character in text.chars() {
            if matches!(character, ' ' | '\t' | '\n' | '\r') {
                if !para.space {
                    out.push(' ');
                    para.space = true;
                }
            } else {
                out.push(character);
                para.space = false;
            }
        }
        if !out.is_empty() {
            self.push_literal_unchecked(para, style, &out);
        }
    }

    fn push_literal(&mut self, para: &mut Para, style: RunStyle, text: &str) {
        self.push_literal_unchecked(para, style, text);
    }

    fn push_literal_unchecked(&mut self, para: &mut Para, style: RunStyle, text: &str) {
        let span = self.document.push_text(text);
        self.push_run(para, style, Inline::Text(span));
    }

    fn push_run(&mut self, para: &mut Para, style: RunStyle, content: Inline) {
        let properties = self.document.intern_run_properties(style.run);
        para.runs.push(Run {
            style: style.named,
            properties,
            link: para.link,
            revision: para.revision,
            content,
        });
    }

    fn read_note(&mut self, reader: &mut XmlReader<'_>, para: &mut Para, style: RunStyle) {
        let mut blocks = Vec::new();
        while let Some(event) = reader.next() {
            match event {
                XmlEvent::Start {
                    name: "text:note-body",
                    self_closing: false,
                    ..
                } => {
                    let pending = std::mem::take(&mut self.pending);
                    blocks = self.read_blocks(reader, "text:note-body");
                    self.pending = pending;
                }
                XmlEvent::Start {
                    name,
                    self_closing: false,
                    ..
                } => skip(reader, name),
                XmlEvent::End { name: "text:note" } => break,
                _ => {}
            }
        }
        // The space Word puts after a note's mark, kept by LibreOffice as
        // a literal space, is not the note's text.
        if let Some(Block::Paragraph(first)) = blocks.first_mut() {
            if let Some(Run {
                content: Inline::Text(span),
                ..
            }) = first.runs.first_mut()
            {
                let text = self.document.text(*span);
                let trimmed = text.len() - text.trim_start_matches(' ').len();
                span.start += trimmed as u32;
            }
            first
                .runs
                .retain(|run| !matches!(run.content, Inline::Text(span) if span.start == span.end));
        }
        let id = self.document.footnotes.len();
        self.document.footnotes.push(Note { blocks });
        self.push_run(para, style, Inline::Footnote(id));
        para.space = false;
    }

    fn read_annotation(&mut self, reader: &mut XmlReader<'_>) -> Id {
        let mut comment = Comment::default();
        let mut lines: Vec<String> = Vec::new();
        while let Some(event) = reader.next() {
            match event {
                XmlEvent::Start {
                    name: field @ ("dc:creator" | "dc:date" | "meta:creator-initials"),
                    self_closing: false,
                    ..
                } => {
                    let text = element_text(reader, field);
                    match field {
                        "dc:creator" => comment.author = text,
                        "dc:date" => comment.date = Some(iso_date(&text)),
                        _ => comment.initials = Some(text),
                    }
                }
                XmlEvent::Start {
                    name: "text:p",
                    self_closing: false,
                    ..
                } => lines.push(element_text(reader, "text:p")),
                XmlEvent::Start {
                    name,
                    self_closing: false,
                    ..
                } => skip(reader, name),
                XmlEvent::End {
                    name: "office:annotation",
                } => break,
                _ => {}
            }
        }
        comment.text = lines.join("\n");
        self.document.comments.push(comment);
        (self.document.comments.len() - 1) as Id
    }

    fn read_tracked_changes(&mut self, reader: &mut XmlReader<'_>) {
        while let Some(event) = reader.next() {
            match event {
                XmlEvent::Start {
                    name: "text:changed-region",
                    attributes,
                    self_closing: false,
                } => {
                    let id = attribute(&attributes, "text:id")
                        .or_else(|| attribute(&attributes, "xml:id"))
                        .unwrap_or("")
                        .to_string();
                    let mut kind = None;
                    let mut author = None;
                    let mut date = None;
                    let mut deleted = Para::new();
                    while let Some(event) = reader.next() {
                        match event {
                            XmlEvent::Start {
                                name:
                                    change @ ("text:insertion" | "text:deletion" | "text:format-change"),
                                ..
                            } => {
                                kind = match change {
                                    "text:insertion" => Some(RevisionKind::Insertion),
                                    "text:deletion" => Some(RevisionKind::Deletion),
                                    _ => None,
                                };
                            }
                            XmlEvent::Start {
                                name: "dc:creator",
                                self_closing: false,
                                ..
                            } => author = Some(element_text(reader, "dc:creator")),
                            XmlEvent::Start {
                                name: "dc:date",
                                self_closing: false,
                                ..
                            } => date = Some(iso_date(&element_text(reader, "dc:date"))),
                            XmlEvent::Start {
                                name: paragraph @ ("text:p" | "text:h"),
                                self_closing: false,
                                ..
                            } => {
                                // Deleted paragraphs run together, a space
                                // between them.
                                if !deleted.runs.is_empty() {
                                    self.push_literal(&mut deleted, RunStyle::default(), " ");
                                }
                                deleted.space = true;
                                self.read_inline(
                                    reader,
                                    paragraph,
                                    &mut deleted,
                                    RunStyle::default(),
                                );
                            }
                            XmlEvent::End {
                                name: "text:changed-region",
                            } => break,
                            _ => {}
                        }
                    }
                    if let Some(kind) = kind {
                        let revision = self.document.push_revision(Revision { kind, author, date });
                        self.changes.insert(id, (revision, deleted.runs));
                    }
                }
                XmlEvent::End {
                    name: "text:tracked-changes",
                } => break,
                _ => {}
            }
        }
    }

    /// A frame: a picture, a formula, or a text box.
    fn read_frame(
        &mut self,
        reader: &mut XmlReader<'_>,
        attributes: &[(&str, std::borrow::Cow<'_, str>)],
        para: &mut Para,
        style: RunStyle,
    ) {
        let width = attribute(attributes, "svg:width")
            .and_then(length)
            .unwrap_or(0.0);
        let height = attribute(attributes, "svg:height")
            .and_then(length)
            .unwrap_or(0.0);
        let anchor = attribute(attributes, "text:anchor-type").unwrap_or("paragraph");
        let x = attribute(attributes, "svg:x")
            .and_then(length)
            .unwrap_or(0.0);
        let y = attribute(attributes, "svg:y")
            .and_then(length)
            .unwrap_or(0.0);
        let mut description: Option<String> = None;
        let mut picture: Option<usize> = None;
        let mut formula: Option<String> = None;
        while let Some(event) = reader.next() {
            match event {
                XmlEvent::Start {
                    name: "draw:image",
                    attributes,
                    self_closing,
                } => {
                    if picture.is_none() {
                        picture = attribute(&attributes, "xlink:href")
                            .and_then(|path| self.load_media(path));
                    }
                    if !self_closing {
                        skip(reader, "draw:image");
                    }
                }
                XmlEvent::Start {
                    name: "draw:object",
                    attributes,
                    self_closing,
                } => {
                    if let Some(path) = attribute(&attributes, "xlink:href") {
                        formula = self.load_formula(path);
                    }
                    if !self_closing {
                        skip(reader, "draw:object");
                    }
                }
                XmlEvent::Start {
                    name: "draw:text-box",
                    self_closing: false,
                    ..
                } => {
                    let pending = std::mem::take(&mut self.pending);
                    let blocks = self.read_blocks(reader, "draw:text-box");
                    self.pending = pending;
                    self.pending.extend(blocks);
                }
                XmlEvent::Start {
                    name: field @ ("svg:title" | "svg:desc"),
                    self_closing: false,
                    ..
                } => {
                    let text = element_text(reader, field);
                    if !text.is_empty() && (description.is_none() || field == "svg:title") {
                        description = Some(text);
                    }
                }
                XmlEvent::Start {
                    name,
                    self_closing: false,
                    ..
                } => skip(reader, name),
                XmlEvent::End { name: "draw:frame" } => break,
                _ => {}
            }
        }
        if let Some(mathml) = formula {
            let span = self.document.push_text(&mathml);
            self.push_run(para, style, Inline::Math(span));
            para.space = false;
            return;
        }
        let Some(media) = picture else {
            return;
        };
        let placement = match anchor {
            "as-char" => Placement::Inline,
            "page" => Placement::Floating {
                horizontal: Anchor {
                    from: AnchorBase::Page,
                    offset: x,
                },
                vertical: Anchor {
                    from: AnchorBase::Page,
                    offset: y,
                },
            },
            _ => Placement::Floating {
                horizontal: Anchor {
                    from: AnchorBase::Margin,
                    offset: x,
                },
                vertical: Anchor {
                    from: AnchorBase::Line,
                    offset: y,
                },
            },
        };
        let image = self.document.push_image(InlineImage {
            media,
            width,
            height,
            description,
            placement,
            crop: None,
        });
        self.push_run(para, style, Inline::Image(image));
        para.space = false;
    }

    fn load_media(&mut self, path: &str) -> Option<usize> {
        let path = path.trim_start_matches("./");
        if let Some(id) = self.media.get(path) {
            return Some(*id);
        }
        let entry = self.zip.find(path)?;
        let mut bytes = Vec::new();
        self.zip.read(entry, &mut bytes).ok()?;
        let base = path.rsplit('/').next().unwrap_or(path);
        let mut name = base.to_string();
        let mut counter = 1;
        while self.document.media.iter().any(|media| media.name == name) {
            counter += 1;
            name = format!("{counter}-{base}");
        }
        let id = self.document.media.len();
        self.document.media.push(Media { name, bytes });
        self.media.insert(path.to_string(), id);
        Some(id)
    }

    /// An embedded formula object's MathML, without its StarMath
    /// annotation.
    fn load_formula(&self, path: &str) -> Option<String> {
        let path = path.trim_start_matches("./").trim_end_matches('/');
        let entry = self.zip.find(&format!("{path}/content.xml"))?;
        let mut bytes = Vec::new();
        self.zip.read(entry, &mut bytes).ok()?;
        let text = String::from_utf8(bytes).ok()?;
        let start = text.find("<math")?;
        let end = text.rfind("</math>").map(|end| end + "</math>".len())?;
        let mut mathml = text[start..end].to_string();
        while let (Some(open), Some(close)) =
            (mathml.find("<annotation"), mathml.find("</annotation>"))
        {
            if close < open {
                break;
            }
            mathml.replace_range(open..close + "</annotation>".len(), "");
        }
        Some(mathml)
    }

    // ----- tables -----

    fn read_table(
        &mut self,
        reader: &mut XmlReader<'_>,
        attributes: &[(&str, std::borrow::Cow<'_, str>)],
    ) -> Table {
        let mut table = Table::default();
        if let Some(style) = attribute(attributes, "table:style-name")
            .and_then(|style| self.scopes[self.scope].tables.get(style))
        {
            table.alignment = style.alignment;
            table.indent = style.indent;
        }
        // Each row's cells as read, with their spans and whether covered.
        let mut grid: Vec<(Row, Spans)> = Vec::new();
        let mut in_header = false;
        let mut depth = 1usize;
        while let Some(event) = reader.next() {
            match event {
                XmlEvent::Start {
                    name: "table:table-column",
                    attributes,
                    ..
                } => {
                    let width = attribute(&attributes, "table:style-name")
                        .and_then(|style| self.scopes[self.scope].columns.get(style))
                        .copied()
                        .unwrap_or(0.0);
                    let repeated = attribute(&attributes, "table:number-columns-repeated")
                        .and_then(|count| count.parse::<usize>().ok())
                        .unwrap_or(1)
                        .min(1024);
                    table.columns.extend(std::iter::repeat_n(width, repeated));
                }
                XmlEvent::Start {
                    name: "table:table-header-rows",
                    self_closing: false,
                    ..
                } => in_header = true,
                XmlEvent::End {
                    name: "table:table-header-rows",
                } => in_header = false,
                XmlEvent::Start {
                    name: "table:table-row",
                    attributes,
                    self_closing: false,
                } => {
                    let height = attribute(&attributes, "table:style-name")
                        .and_then(|style| self.scopes[self.scope].rows.get(style))
                        .copied();
                    let repeated = attribute(&attributes, "table:number-rows-repeated")
                        .and_then(|count| count.parse::<usize>().ok())
                        .unwrap_or(1)
                        .min(1024);
                    let row = self.read_row(reader, height);
                    for _ in 0..repeated {
                        grid.push(row.clone());
                        if in_header {
                            table.header_rows += 1;
                        }
                    }
                }
                XmlEvent::Start {
                    name: "table:table",
                    self_closing: false,
                    ..
                } => depth += 1,
                XmlEvent::End {
                    name: "table:table",
                } => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
        table.rows = resolve_merges(grid);
        table
    }

    fn read_row(
        &mut self,
        reader: &mut XmlReader<'_>,
        height: Option<f32>,
    ) -> (Row, Vec<(u32, u32, bool)>) {
        let mut row = Row {
            cells: Vec::new(),
            height,
        };
        let mut spans = Vec::new();
        while let Some(event) = reader.next() {
            match event {
                XmlEvent::Start {
                    name: element @ ("table:table-cell" | "table:covered-table-cell"),
                    attributes,
                    self_closing,
                } => {
                    let covered = element == "table:covered-table-cell";
                    let columns = attribute(&attributes, "table:number-columns-spanned")
                        .and_then(|count| count.parse::<u32>().ok())
                        .unwrap_or(1)
                        .max(1);
                    let rows = attribute(&attributes, "table:number-rows-spanned")
                        .and_then(|count| count.parse::<u32>().ok())
                        .unwrap_or(1)
                        .max(1);
                    let repeated = attribute(&attributes, "table:number-columns-repeated")
                        .and_then(|count| count.parse::<usize>().ok())
                        .unwrap_or(1)
                        .min(1024);
                    let style = attribute(&attributes, "table:style-name")
                        .and_then(|style| self.scopes[self.scope].cells.get(style))
                        .cloned()
                        .unwrap_or_default();
                    let blocks = if self_closing {
                        Vec::new()
                    } else {
                        self.read_blocks(reader, element)
                    };
                    let cell = Cell {
                        blocks,
                        column_span: columns,
                        row_span: rows,
                        background: style.background,
                        merge: Merge::Origin,
                        borders: crate::document::CellBorders {
                            top: style.borders[0],
                            bottom: style.borders[1],
                            left: style.borders[2],
                            right: style.borders[3],
                        },
                        vertical_alignment: style.vertical_alignment,
                        margins: style.padding,
                    };
                    for _ in 0..repeated {
                        row.cells.push(cell.clone());
                        spans.push((columns, rows, covered));
                    }
                }
                XmlEvent::End {
                    name: "table:table-row",
                } => break,
                _ => {}
            }
        }
        (row, spans)
    }
}

/// A row's cells' column and row spans, and whether each is covered.
type Spans = Vec<(u32, u32, bool)>;

/// The grid's merges from the origins' spans: cells right of an origin
/// are covered from the left, those below it from above (the region's
/// first column) or the left.
fn resolve_merges(grid: Vec<(Row, Spans)>) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::with_capacity(grid.len());
    let mut spans: Vec<Spans> = Vec::with_capacity(grid.len());
    for (row, row_spans) in grid {
        rows.push(row);
        spans.push(row_spans);
    }
    for row in 0..rows.len() {
        for column in 0..rows[row].cells.len() {
            let (columns, down, covered) = spans[row][column];
            if covered {
                // Until an origin claims it, a stray covered cell is empty.
                if rows[row].cells[column].merge == Merge::Origin {
                    rows[row].cells[column].column_span = 1;
                    rows[row].cells[column].row_span = 1;
                }
                continue;
            }
            for below in row..(row + down as usize).min(rows.len()) {
                let end = (column + columns as usize).min(rows[below].cells.len());
                for (across, cell) in rows[below].cells[..end].iter_mut().enumerate().skip(column) {
                    if below == row && across == column {
                        continue;
                    }
                    cell.merge = if across == column {
                        Merge::Above
                    } else {
                        Merge::Left
                    };
                    cell.column_span = 0;
                    cell.row_span = 0;
                    cell.blocks.clear();
                    spans[below][across].2 = true;
                }
            }
        }
    }
    rows
}

fn read_list_style(reader: &mut XmlReader<'_>, name: String) -> (ListStyle, Vec<u32>) {
    let mut style = ListStyle {
        name,
        levels: Vec::new(),
    };
    let mut starts: Vec<u32> = Vec::new();
    while let Some(event) = reader.next() {
        match event {
            XmlEvent::Start {
                name: element,
                attributes,
                self_closing,
            } if element.starts_with("text:list-level-style") => {
                let (level, list_level, start) = if self_closing {
                    let (level, mut list_level, start) = (
                        attribute(&attributes, "text:level")
                            .and_then(|level| level.parse::<usize>().ok())
                            .unwrap_or(1)
                            .saturating_sub(1),
                        crate::document::ListLevel::default(),
                        1,
                    );
                    list_level.label = ListLabel::None;
                    (level, list_level, start)
                } else {
                    list_level(reader, element, &attributes)
                };
                if level < 10 {
                    while style.levels.len() <= level {
                        style.levels.push(crate::document::ListLevel::default());
                        starts.push(1);
                    }
                    style.levels[level] = list_level;
                    starts[level] = start;
                }
            }
            XmlEvent::End {
                name: "text:list-style",
            } => break,
            _ => {}
        }
    }
    (style, starts)
}

fn element_text(reader: &mut XmlReader<'_>, name: &str) -> String {
    let mut text = String::new();
    let mut depth = 1usize;
    for event in reader.by_ref() {
        match event {
            XmlEvent::Text(piece) => text.push_str(&piece),
            XmlEvent::Start { name: "text:s", .. } => text.push(' '),
            XmlEvent::Start {
                name: inner,
                self_closing: false,
                ..
            } if inner == name => depth += 1,
            XmlEvent::End { name: inner } if inner == name => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
    }
    text.trim().to_string()
}

/// An ODF date (`2026-10-13T09:30:00` or with fractions) as the model's
/// `YYYY-MM-DDTHH:MM:SSZ`.
fn iso_date(value: &str) -> String {
    let value = value.trim();
    let base = value
        .split('.')
        .next()
        .unwrap_or(value)
        .trim_end_matches('Z');
    if base.len() >= 19 {
        format!("{}Z", &base[..19])
    } else {
        value.to_string()
    }
}
