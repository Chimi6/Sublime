//! Reads a Pages package into the document model: body text with its
//! paragraph and character styles resolved through the stylesheet, lists,
//! links, footnotes, breaks, tables, images, page-number fields, and the
//! sections with their page setup, columns, headers, and footers.

use std::collections::HashMap;

use super::package::{Entry, Package, Stream};
use crate::document::{
    Alignment, Anchor, AnchorBase, Baseline, Block, Caps, Cell, CharacterStyle, Color, Document,
    Inline, InlineImage, LineSpacing, ListItem, ListLabel, ListLevel, ListStyle, Media, MediaId,
    Merge, Note, NumberFormat, NumberKind, PageSetup, PageVariants, Paragraph, ParagraphProperties,
    ParagraphStyle, Placement, Row, Run, RunProperties, Section, SectionStart, StyleId, Table,
};
use crate::document::{FloatingContent, FloatingObject, Id, Revision, RevisionKind};
use crate::io::protobuf::reader::{FieldReader, Value};
use crate::io::protobuf::tree::{Node, Tree};

/// Pages marks a line break, a page break, and an attachment with these.
const LINE_SEPARATOR: char = '\u{2028}';
const PAGE_BREAK: char = '\u{5}';
const ATTACHMENT: char = '\u{FFFC}';
const FOOTNOTE_MARK: char = '\u{E}';
/// A section break and a layout (column) break each lead the first
/// paragraph of the new section, as a paragraph of their own without a
/// newline.
const SECTION_BREAK: char = '\u{4}';
const LAYOUT_BREAK: char = '\u{C}';

/// Where an object lives.
#[derive(Clone, Copy)]
struct Location {
    stream: usize,
    object: usize,
}

/// A view of the package with objects indexed by identifier.
struct Graph<'p> {
    streams: Vec<&'p Stream>,
    index: HashMap<u64, Location>,
}

/// A message of an object: its tree and the first entry of its chain.
#[derive(Clone, Copy)]
struct Message<'p> {
    tree: &'p Tree,
    first: u32,
}

impl<'p> Graph<'p> {
    fn new(package: &'p Package) -> Graph<'p> {
        let mut streams = Vec::new();
        let mut index = HashMap::new();
        for entry in &package.entries {
            if let Entry::Stream(stream) = entry {
                let stream_index = streams.len();
                streams.push(stream);
                for (object_index, object) in stream.objects.iter().enumerate() {
                    index.insert(
                        object.identifier,
                        Location {
                            stream: stream_index,
                            object: object_index,
                        },
                    );
                }
            }
        }
        Graph { streams, index }
    }

    fn object(&self, identifier: u64) -> Option<Message<'p>> {
        let location = self.index.get(&identifier)?;
        let stream = self.streams[location.stream];
        let object = &stream.objects[location.object];
        let message = object.messages.first()?;
        Some(Message {
            tree: &stream.tree,
            first: message.first,
        })
    }

    fn objects_of_type(&self, message_type: u32) -> Vec<(u64, Message<'p>)> {
        let mut found = Vec::new();
        for stream in &self.streams {
            for object in &stream.objects {
                if let Some(message) = object.messages.first()
                    && message.message_type == message_type
                {
                    found.push((
                        object.identifier,
                        Message {
                            tree: &stream.tree,
                            first: message.first,
                        },
                    ));
                }
            }
        }
        found
    }
}

/// Field access over a chain, by schema name.
#[derive(Clone, Copy)]
struct View<'p> {
    tree: &'p Tree,
    first: u32,
}

impl<'p> View<'p> {
    fn of(message: Message<'p>) -> View<'p> {
        View {
            tree: message.tree,
            first: message.first,
        }
    }

    fn entries<'a>(&'a self, name: &'a str) -> impl Iterator<Item = Node> + 'a {
        self.tree
            .chain(self.first)
            .filter(move |(_, entry)| {
                self.tree
                    .field(entry)
                    .is_some_and(|field| field.name == name)
            })
            .map(|(_, entry)| entry.value)
    }

    fn node(&self, name: &str) -> Option<Node> {
        self.entries(name).next()
    }

    fn message(&self, name: &str) -> Option<View<'p>> {
        match self.node(name)? {
            Node::Message(first) => Some(View {
                tree: self.tree,
                first,
            }),
            _ => None,
        }
    }

    fn messages(&self, name: &str) -> Vec<View<'p>> {
        self.entries(name)
            .filter_map(|node| match node {
                Node::Message(first) => Some(View {
                    tree: self.tree,
                    first,
                }),
                _ => None,
            })
            .collect()
    }

    fn reference(&self, name: &str) -> Option<u64> {
        match self.node(name)? {
            Node::Reference(identifier) => Some(identifier),
            _ => None,
        }
    }

    fn references(&self, name: &str) -> Vec<u64> {
        self.entries(name)
            .filter_map(|node| match node {
                Node::Reference(identifier) => Some(identifier),
                _ => None,
            })
            .collect()
    }

    fn bytes(&self, name: &str) -> Option<&'p [u8]> {
        match self.node(name)? {
            Node::Bytes(span) => Some(self.tree.bytes(span)),
            _ => None,
        }
    }

    /// A nested message the package kept encoded (`Tree::deferred`).
    fn deferred(&self, name: &str) -> Option<&'p [u8]> {
        match self.node(name)? {
            Node::Deferred(span) => Some(self.tree.bytes(span)),
            _ => None,
        }
    }

    fn string(&self, name: &str) -> Option<&'p str> {
        match self.node(name)? {
            Node::Str(span) => Some(self.tree.str(span)),
            _ => None,
        }
    }

    fn strings(&self, name: &str) -> Vec<&'p str> {
        self.entries(name)
            .filter_map(|node| match node {
                Node::Str(span) => Some(self.tree.str(span)),
                _ => None,
            })
            .collect()
    }

    fn float(&self, name: &str) -> Option<f32> {
        match self.node(name)? {
            Node::Float(value) => Some(value),
            Node::Double(value) => Some(value as f32),
            _ => None,
        }
    }

    fn floats(&self, name: &str) -> Vec<f32> {
        self.entries(name)
            .filter_map(|node| match node {
                Node::Float(value) => Some(value),
                _ => None,
            })
            .collect()
    }

    fn integer(&self, name: &str) -> Option<i64> {
        match self.node(name)? {
            Node::Int(value) => Some(value),
            Node::Uint(value) => Some(value as i64),
            _ => None,
        }
    }

    fn integers(&self, name: &str) -> Vec<i64> {
        self.entries(name)
            .filter_map(|node| match node {
                Node::Int(value) => Some(value),
                Node::Uint(value) => Some(value as i64),
                _ => None,
            })
            .collect()
    }

    fn boolean(&self, name: &str) -> Option<bool> {
        match self.node(name)? {
            Node::Bool(value) => Some(value),
            _ => None,
        }
    }
}

/// A run of an attribute table: from `start` to the next entry.
struct Span {
    start: usize,
    object: Option<u64>,
}

#[inline(never)]
fn attribute_table(view: &View<'_>, name: &str) -> Vec<Span> {
    if let Some(bytes) = view.deferred(name) {
        return parse_attribute_table(bytes);
    }
    let table = match view.message(name) {
        Some(table) => table,
        None => return Vec::new(),
    };
    table
        .messages("entries")
        .iter()
        .map(|entry| Span {
            start: entry.integer("character_index").unwrap_or(0) as usize,
            object: entry.reference("object"),
        })
        .collect()
}

/// `TSWP.ObjectAttributeTable` from its encoded bytes: entries (field 1)
/// of `character_index` (1) and `object` (2, a reference whose field 1
/// is the identifier). Twelve bytes an entry instead of three tree
/// entries.
#[inline(never)]
fn parse_attribute_table(bytes: &[u8]) -> Vec<Span> {
    let mut spans = Vec::with_capacity(bytes.len() / 8);
    for field in FieldReader::new(bytes).flatten() {
        let Value::Bytes(entry) = field.value else {
            continue;
        };
        if field.number != 1 {
            continue;
        }
        let mut start = 0usize;
        let mut object = None;
        for inner in FieldReader::new(entry).flatten() {
            match (inner.number, inner.value) {
                (1, Value::Varint(index)) => start = index as usize,
                (2, Value::Bytes(reference)) => {
                    object = FieldReader::new(reference).flatten().find_map(|field| {
                        match (field.number, field.value) {
                            (1, Value::Varint(identifier)) => Some(identifier),
                            _ => None,
                        }
                    });
                }
                _ => {}
            }
        }
        spans.push(Span { start, object });
    }
    spans
}

/// `TSWP.ParaDataAttributeTable` from its encoded bytes: entries of
/// `character_index` (1), `first` (2), `second` (3).
#[inline(never)]
fn parse_paragraph_data(bytes: &[u8]) -> Vec<(usize, u32, u32)> {
    let mut entries = Vec::with_capacity(bytes.len() / 6);
    for field in FieldReader::new(bytes).flatten() {
        let Value::Bytes(entry) = field.value else {
            continue;
        };
        if field.number != 1 {
            continue;
        }
        let (mut start, mut first, mut second) = (0usize, 0u32, 0u32);
        for inner in FieldReader::new(entry).flatten() {
            match (inner.number, inner.value) {
                (1, Value::Varint(value)) => start = value as usize,
                (2, Value::Varint(value)) => first = value as u32,
                (3, Value::Varint(value)) => second = value as u32,
                _ => {}
            }
        }
        entries.push((start, first, second));
    }
    entries
}

/// The paragraph data table: `(start, (level, starts_list))` per entry.
fn paragraph_data(view: &View<'_>) -> Vec<(usize, (u8, bool))> {
    if let Some(bytes) = view.deferred("table_para_data") {
        return parse_paragraph_data(bytes)
            .into_iter()
            .map(|(start, first, second)| (start, (first as u8, second != 0)))
            .collect();
    }
    let table = match view.message("table_para_data") {
        Some(table) => table,
        None => return Vec::new(),
    };
    table
        .messages("entries")
        .iter()
        .map(|entry| {
            (
                entry.integer("character_index").unwrap_or(0) as usize,
                (
                    entry.integer("first").unwrap_or(0) as u8,
                    entry.integer("second").unwrap_or(0) != 0,
                ),
            )
        })
        .collect()
}

/// The paragraph starts table: `(start, first number)` per entry, the
/// number a list starts at for the paragraph that starts it.
fn paragraph_starts(view: &View<'_>) -> Vec<(usize, u32)> {
    if let Some(bytes) = view.deferred("table_para_starts") {
        return parse_paragraph_data(bytes)
            .into_iter()
            .map(|(start, first, _)| (start, first))
            .collect();
    }
    let table = match view.message("table_para_starts") {
        Some(table) => table,
        None => return Vec::new(),
    };
    table
        .messages("entries")
        .iter()
        .map(|entry| {
            (
                entry.integer("character_index").unwrap_or(0) as usize,
                entry
                    .integer("first")
                    .unwrap_or(0)
                    .clamp(0, i64::from(u32::MAX)) as u32,
            )
        })
        .collect()
}

/// The entry of a table that covers `position`: the last one at or before
/// it. Tables are sorted by start, so this is a binary search.
fn covering(spans: &[Span], position: usize) -> Option<u64> {
    let count = spans.partition_point(|span| span.start <= position);
    spans
        .get(count.wrapping_sub(1))
        .and_then(|span| span.object)
}

/// The entries of a table that start inside `[from, to)`.
fn within(spans: &[Span], from: usize, to: usize) -> &[Span] {
    let first = spans.partition_point(|span| span.start < from);
    let end = spans.partition_point(|span| span.start < to);
    &spans[first..end.max(first)]
}

/// The last entry of a sorted `(start, ..)` table at or before `position`.
fn last_at<T>(entries: &[(usize, T)], position: usize) -> Option<&(usize, T)> {
    let count = entries.partition_point(|(start, _)| *start <= position);
    entries.get(count.wrapping_sub(1))
}

pub struct Reader<'p> {
    package: &'p Package,
    graph: Graph<'p>,
    document: Document,
    /// Pages style object -> model style, per table.
    paragraph_styles: HashMap<u64, StyleId>,
    character_styles: HashMap<u64, StyleId>,
    list_styles: HashMap<u64, Option<StyleId>>,
    /// The last paragraph style object seen, for entries without one.
    last_paragraph_style: Option<u64>,
    /// Style objects resolved before: the named style and the interned
    /// direct formatting of the variation chain.
    /// The maps share one key and value type so the binary carries one
    /// hash map instantiation for them all.
    resolved_paragraph: HashMap<u64, usize>,
    resolved_paragraphs: Vec<ResolvedParagraph>,
    resolved_character: HashMap<u64, usize>,
    resolved_characters: Vec<(Option<StyleId>, Option<Id>)>,
    /// Change objects -> interned revisions.
    revision_ids: HashMap<u64, usize>,
    /// Run boundaries of the paragraph being split, reused.
    boundary_scratch: Vec<usize>,
    /// Tables met inside the paragraph being read; they go before it.
    pending_blocks: Vec<Block>,
    /// Table of contents entries met inside the paragraph; they follow it.
    following_blocks: Vec<Block>,
    /// Data identifier -> media, for images used more than once.
    media: HashMap<u64, MediaId>,
    /// Data identifier -> file name under `Data/`, from the package
    /// metadata, built on first use.
    data_files: Option<HashMap<u64, String>>,
    /// Merge owner UUID -> merged regions, from the calculation engine,
    /// built on first use.
    merges: Option<HashMap<[u64; 4], Vec<Region>>>,
}

/// A paragraph style object resolved: the named style, the interned
/// paragraph properties, and the interned run properties of its chain.
type ResolvedParagraph = (Option<StyleId>, Option<Id>, Option<Id>);

/// A merged cell region: origin and size.
#[derive(Clone, Copy)]
struct Region {
    row: usize,
    column: usize,
    rows: usize,
    columns: usize,
}

/// Reads the document the package holds.
pub fn read_document(package: &Package) -> Document {
    let mut reader = Reader {
        package,
        graph: Graph::new(package),
        document: Document::default(),
        paragraph_styles: HashMap::new(),
        character_styles: HashMap::new(),
        list_styles: HashMap::new(),
        last_paragraph_style: None,
        resolved_paragraph: HashMap::new(),
        resolved_paragraphs: Vec::new(),
        resolved_character: HashMap::new(),
        resolved_characters: Vec::new(),
        revision_ids: HashMap::new(),
        boundary_scratch: Vec::new(),
        pending_blocks: Vec::new(),
        following_blocks: Vec::new(),
        media: HashMap::new(),
        data_files: None,
        merges: None,
    };
    reader.read();
    reader.document
}

/// Type ids of the messages the reader looks for.
const DOCUMENT_ARCHIVE: u32 = 10000;
const STORAGE_ARCHIVE: u32 = 2001;
const CALCULATION_ENGINE: u32 = 4000;
const PACKAGE_METADATA: u32 = 11006;

/// Cell storage: the flag bits of the fields a cell record carries, in
/// the order they appear after the 12-byte header.
const CELL_FIELDS: [(u32, usize); 21] = [
    (0x1, 16),     // decimal128
    (0x2, 8),      // double
    (0x4, 8),      // seconds since 2001
    (0x8, 4),      // string id
    (0x10, 4),     // rich text id
    (0x20, 4),     // cell style id
    (0x40, 4),     // text style id
    (0x80, 4),     // conditional style id
    (0x100, 4),    // conditional rule style id
    (0x200, 4),    // formula id
    (0x400, 4),    // control id
    (0x800, 4),    // formula error id
    (0x1000, 4),   // suggestion id
    (0x2000, 4),   // number format id
    (0x4000, 4),   // currency format id
    (0x8000, 4),   // date format id
    (0x10000, 4),  // duration format id
    (0x20000, 4),  // text format id
    (0x40000, 4),  // boolean format id
    (0x80000, 4),  // comment id
    (0x100000, 4), // import warning id
];

/// What a cell record says.
#[derive(Default)]
struct CellRecord {
    kind: u8,
    double: Option<f64>,
    seconds: Option<f64>,
    string: Option<u32>,
    rich_text: Option<u32>,
    cell_style: Option<u32>,
    text_style: Option<u32>,
}

/// Decodes one cell record (storage version 5).
fn cell_record(bytes: &[u8]) -> Option<CellRecord> {
    if bytes.len() < 12 || bytes[0] != 5 {
        return None;
    }
    let flags = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    let mut record = CellRecord {
        kind: bytes[1],
        ..CellRecord::default()
    };
    let mut offset = 12usize;
    for (flag, size) in CELL_FIELDS {
        if flags & flag == 0 {
            continue;
        }
        let field = bytes.get(offset..offset + size)?;
        match flag {
            0x2 => record.double = Some(f64::from_le_bytes(field.try_into().ok()?)),
            0x4 => record.seconds = Some(f64::from_le_bytes(field.try_into().ok()?)),
            0x8 => record.string = Some(u32::from_le_bytes(field.try_into().ok()?)),
            0x10 => record.rich_text = Some(u32::from_le_bytes(field.try_into().ok()?)),
            0x20 => record.cell_style = Some(u32::from_le_bytes(field.try_into().ok()?)),
            0x40 => record.text_style = Some(u32::from_le_bytes(field.try_into().ok()?)),
            _ => {}
        }
        offset += size;
    }
    Some(record)
}

/// The lookup tables of one table's data store.
struct TableLists {
    strings: HashMap<u32, String>,
    /// Rich text id -> its storage object.
    rich_text: HashMap<u32, u64>,
    /// Style id -> the style object (a cell style or a paragraph style).
    styles: HashMap<u32, u64>,
}

impl Reader<'_> {
    fn read(&mut self) {
        let root = self
            .graph
            .objects_of_type(DOCUMENT_ARCHIVE)
            .into_iter()
            .next()
            .map(|(_, message)| message);
        let page = root
            .map(|root| page_setup(View::of(root)))
            .unwrap_or_default();
        let body = root
            .and_then(|root| View::of(root).reference("body_storage"))
            .and_then(|identifier| self.graph.object(identifier))
            .or_else(|| {
                // Without a root, the first in-document body storage.
                self.graph
                    .objects_of_type(STORAGE_ARCHIVE)
                    .into_iter()
                    .map(|(_, message)| message)
                    .find(|message| {
                        let view = View::of(*message);
                        view.integer("kind").unwrap_or(0) == 0
                            && view.boolean("in_document").unwrap_or(false)
                    })
            });
        let Some(body) = body else {
            self.document.sections.push(Section {
                page,
                columns: 1,
                ..Section::default()
            });
            return;
        };
        let storage = View::of(body);
        let section_starts = attribute_table(&storage, "table_section");
        let layout_starts = attribute_table(&storage, "table_layout_style");
        let paragraphs = self.storage_paragraphs(storage);
        // Sections start at the entries of the section table (a new
        // page) and at the entries of the layout table (continuous, for
        // a column change). Both tables cover the whole text from 0.
        let mut sections: Vec<Section> = Vec::new();
        let mut current = Section {
            page: page.clone(),
            columns: 1,
            ..Section::default()
        };
        let mut boundary_done = usize::MAX;
        for (start, block) in paragraphs {
            let section_here = within(&section_starts, start, start + 1)
                .last()
                .and_then(|span| span.object);
            let layout_here = within(&layout_starts, start, start + 1)
                .last()
                .and_then(|span| span.object);
            let is_boundary =
                (section_here.is_some() || layout_here.is_some()) && boundary_done != start;
            if is_boundary {
                boundary_done = start;
                let first = sections.is_empty() && current.blocks.is_empty();
                if !first {
                    let mut next = Section {
                        page: page.clone(),
                        columns: current.columns,
                        start: SectionStart::Continuous,
                        headers: current.headers.clone(),
                        footers: current.footers.clone(),
                        blocks: Vec::new(),
                    };
                    if section_here.is_some() {
                        next.start = SectionStart::NewPage;
                    }
                    sections.push(std::mem::replace(&mut current, next));
                }
                if let Some(section_object) = section_here {
                    let previous = sections.last();
                    let (headers, footers) = self.section_page_text(section_object, previous);
                    current.headers = headers;
                    current.footers = footers;
                }
                if let Some(layout_object) = layout_here {
                    current.columns = self.column_count(layout_object);
                }
            }
            current.blocks.push(block);
        }
        sections.push(current);
        self.document.sections = sections;
        if let Some(floating) = root.and_then(|root| View::of(root).reference("floating_drawables"))
        {
            self.floating_drawables(floating);
        }
    }

    /// `TP.FloatingDrawablesArchive`: the objects placed on pages.
    #[inline(never)]
    fn floating_drawables(&mut self, archive: u64) {
        let Some(message) = self.graph.object(archive) else {
            return;
        };
        for group in View::of(message).messages("page_groups") {
            let page = group.integer("page_index").unwrap_or(0).max(0) as u32;
            for entry in group.messages("drawables") {
                if let Some(drawable) = entry.reference("drawable") {
                    self.floating_object(drawable, page, 0.0, 0.0);
                }
            }
        }
    }

    /// One placed drawable, told apart by its fields: a group's children
    /// are placed relative to it, an image by its data, a shape by the
    /// text it owns. Lines, charts, and empty shapes are left out.
    #[inline(never)]
    fn floating_object(&mut self, drawable: u64, page: u32, offset_x: f32, offset_y: f32) {
        let Some(message) = self.graph.object(drawable) else {
            return;
        };
        let view = View::of(message);
        // The drawable base sits one `super` down for images and groups,
        // two for shapes.
        let base = view
            .message("super")
            .filter(|base| base.message("geometry").is_some())
            .or_else(|| {
                view.message("super")
                    .and_then(|shape| shape.message("super"))
            });
        let geometry = base.and_then(|base| base.message("geometry"));
        let position = geometry.and_then(|geometry| geometry.message("position"));
        let size = geometry.and_then(|geometry| geometry.message("size"));
        let x = offset_x + position.and_then(|p| p.float("x")).unwrap_or(0.0);
        let y = offset_y + position.and_then(|p| p.float("y")).unwrap_or(0.0);
        let width = size.and_then(|s| s.float("width")).unwrap_or(0.0);
        let height = size.and_then(|s| s.float("height")).unwrap_or(0.0);
        let children = view.references("children");
        if !children.is_empty() {
            for child in children {
                self.floating_object(child, page, x, y);
            }
            return;
        }
        let content = if let Some(data) = view.message("data") {
            let identifier = data.integer("identifier").unwrap_or(0) as u64;
            match self.media_for(identifier) {
                Some(media) => FloatingContent::Image(media),
                None => return,
            }
        } else if let Some(storage) = view.reference("owned_storage") {
            let Some(storage) = self.graph.object(storage) else {
                return;
            };
            let blocks = self.nested_blocks(View::of(storage));
            let has_text = blocks.iter().any(|block| match block {
                Block::Paragraph(paragraph) => !paragraph.runs.is_empty(),
                Block::Table(_) => true,
            });
            if !has_text {
                return;
            }
            let fill = view
                .message("super")
                .and_then(|shape| shape.reference("style"))
                .and_then(|style| self.shape_fill(style));
            FloatingContent::TextBox { blocks, fill }
        } else {
            return;
        };
        if width <= 0.0 || height <= 0.0 {
            return;
        }
        self.document.floating.push(FloatingObject {
            page,
            x,
            y,
            width,
            height,
            content,
        });
    }

    /// The solid fill of a shape style, through its parents.
    fn shape_fill(&self, style: u64) -> Option<Color> {
        let mut current = Some(style);
        let mut depth = 0;
        while let Some(identifier) = current {
            let message = self.graph.object(identifier)?;
            let view = View::of(message);
            let base = view.message("super")?;
            if let Some(fill) = base
                .message("shape_properties")
                .and_then(|properties| properties.message("fill"))
            {
                return fill.message("color").and_then(color);
            }
            current = base.message("super").and_then(|s| s.reference("parent"));
            depth += 1;
            if depth > 64 {
                break;
            }
        }
        None
    }

    /// The headers and footers of a section, from its page templates.
    #[inline(never)]
    fn section_page_text(
        &mut self,
        section: u64,
        previous: Option<&Section>,
    ) -> (PageVariants, PageVariants) {
        let Some(message) = self.graph.object(section) else {
            return (PageVariants::default(), PageVariants::default());
        };
        let view = View::of(message);
        if view
            .boolean("inherit_previous_header_footer")
            .unwrap_or(false)
            && let Some(previous) = previous
        {
            return (previous.headers.clone(), previous.footers.clone());
        }
        let first_differs = view
            .boolean("section_template_first_page_different")
            .unwrap_or(false);
        let even_differs = view
            .boolean("section_template_even_odd_pages_different")
            .unwrap_or(false);
        let odd = view.reference("odd_section_template_page");
        let even = view.reference("even_section_template_page");
        let first = view.reference("first_section_template_page");
        let mut headers = PageVariants::default();
        let mut footers = PageVariants::default();
        if let Some(template) = odd {
            let (header, footer) = self.template_page_text(template);
            headers.default = header;
            footers.default = footer;
        }
        if even_differs && let Some(template) = even {
            let (header, footer) = self.template_page_text(template);
            headers.even = header;
            footers.even = footer;
        }
        if first_differs && let Some(template) = first {
            let (header, footer) = self.template_page_text(template);
            headers.first = header;
            footers.first = footer;
        }
        (headers, footers)
    }

    /// A page template's header and footer: its left, center, and right
    /// storages joined, the empty ones dropped.
    #[inline(never)]
    fn template_page_text(&mut self, template: u64) -> (Option<Vec<Block>>, Option<Vec<Block>>) {
        let Some(message) = self.graph.object(template) else {
            return (None, None);
        };
        let view = View::of(message);
        let headers = view.references("headers");
        let footers = view.references("footers");
        (self.page_area(&headers), self.page_area(&footers))
    }

    #[inline(never)]
    fn page_area(&mut self, storages: &[u64]) -> Option<Vec<Block>> {
        let mut blocks = Vec::new();
        for (position, storage) in storages.iter().enumerate() {
            let Some(message) = self.graph.object(*storage) else {
                continue;
            };
            let mut area = self.nested_blocks(View::of(message));
            let has_text = area.iter().any(|block| match block {
                Block::Paragraph(paragraph) => !paragraph.runs.is_empty(),
                Block::Table(_) => true,
            });
            if !has_text {
                continue;
            }
            // The center and right areas align that way unless told otherwise.
            let alignment = match position {
                1 => Some(Alignment::Center),
                2 => Some(Alignment::Right),
                _ => None,
            };
            if let Some(alignment) = alignment {
                for block in &mut area {
                    if let Block::Paragraph(paragraph) = block {
                        let mut properties = self.document.paragraph_properties(paragraph);
                        if properties.alignment.is_none() {
                            properties.alignment = Some(alignment);
                            paragraph.properties =
                                self.document.intern_paragraph_properties(properties);
                        }
                    }
                }
            }
            blocks.append(&mut area);
        }
        if blocks.is_empty() {
            None
        } else {
            Some(blocks)
        }
    }

    /// The column count of a column style, through its variation chain.
    #[inline(never)]
    fn column_count(&self, layout: u64) -> u16 {
        let mut current = Some(layout);
        let mut depth = 0;
        while let Some(identifier) = current {
            let Some(message) = self.graph.object(identifier) else {
                break;
            };
            let view = View::of(message);
            let columns = view
                .message("column_properties")
                .and_then(|properties| properties.message("columns"));
            if let Some(columns) = columns {
                if let Some(count) = columns
                    .message("equal_columns")
                    .and_then(|equal| equal.integer("count"))
                {
                    return count.clamp(1, 64) as u16;
                }
                if let Some(unequal) = columns.message("non_equal_columns") {
                    return (unequal.messages("columns").len() + 1).clamp(1, 64) as u16;
                }
            }
            current = view.message("super").and_then(|s| s.reference("parent"));
            depth += 1;
            if depth > 64 {
                break;
            }
        }
        1
    }

    /// The blocks of a storage read on its own: the running paragraph
    /// style and any pending tables of the enclosing text are kept apart.
    fn nested_blocks(&mut self, storage: View<'_>) -> Vec<Block> {
        let outer_style = self.last_paragraph_style.take();
        let outer_pending = std::mem::take(&mut self.pending_blocks);
        let outer_following = std::mem::take(&mut self.following_blocks);
        let blocks = self.storage_blocks(storage);
        self.last_paragraph_style = outer_style;
        self.pending_blocks = outer_pending;
        self.following_blocks = outer_following;
        blocks
    }

    /// The paragraphs of a text storage, in order.
    fn storage_blocks(&mut self, storage: View<'_>) -> Vec<Block> {
        self.storage_paragraphs(storage)
            .into_iter()
            .map(|(_, block)| block)
            .collect()
    }

    /// The blocks of a text storage with the UTF-16 offset of the
    /// paragraph each came from. A table sits before the paragraph that
    /// held its attachment, as Pages exports it.
    fn storage_paragraphs(&mut self, storage: View<'_>) -> Vec<(usize, Block)> {
        let text: String = storage.strings("text").concat();
        let paragraph_styles = attribute_table(&storage, "table_para_style");
        let character_styles = attribute_table(&storage, "table_char_style");
        let list_styles = attribute_table(&storage, "table_list_style");
        let smart_fields = attribute_table(&storage, "table_smartfield");
        let attachments = attribute_table(&storage, "table_attachment");
        let footnotes = attribute_table(&storage, "table_footnote");
        let insertions = attribute_table(&storage, "table_insertion");
        let deletions = attribute_table(&storage, "table_deletion");
        let data = paragraph_data(&storage);
        let starts = paragraph_starts(&storage);
        let mut blocks = Vec::new();
        let mut start = 0usize;
        let bytes = text.as_bytes();
        // Character indices in Pages count UTF-16 units; a cursor walks
        // the text once, converting byte offsets as it goes.
        let mut offsets = Utf16Offsets::new(&text);
        loop {
            // Paragraphs end at a newline, or at a carriage return in
            // documents written by scripts.
            let end = bytes[start..]
                .iter()
                .position(|byte| matches!(*byte, b'\n' | b'\r'))
                .map_or(bytes.len(), |relative| start + relative);
            let mut start_units = offsets.units_at(start);
            // A break character is an empty paragraph of its own before
            // the paragraph it leads: a page break run, or the end of a
            // section.
            let leading = text[start..end].chars().next();
            if let Some(leading) = leading
                && matches!(leading, PAGE_BREAK | SECTION_BREAK | LAYOUT_BREAK)
            {
                let mut marker = self.break_paragraph(start_units, &paragraph_styles);
                if leading == PAGE_BREAK {
                    marker.runs.push(Run {
                        style: None,
                        properties: None,
                        link: None,
                        revision: None,
                        content: Inline::PageBreak,
                    });
                }
                blocks.push((start_units, Block::Paragraph(marker)));
                start += leading.len_utf8();
                start_units += 1;
            }
            let paragraph = self.paragraph(
                &text[start..end],
                start,
                start_units,
                &mut offsets,
                &paragraph_styles,
                &character_styles,
                &list_styles,
                &smart_fields,
                &attachments,
                &footnotes,
                &insertions,
                &deletions,
                &data,
                &starts,
            );
            for table in self.pending_blocks.drain(..) {
                blocks.push((start_units, table));
            }
            blocks.push((start_units, Block::Paragraph(paragraph)));
            for entry in self.following_blocks.drain(..) {
                blocks.push((start_units, entry));
            }
            if end >= bytes.len() {
                break;
            }
            start = end + 1;
        }
        // A newline at the very end leaves an empty paragraph Pages does
        // not show; it is dropped, as Pages' own export drops it.
        if text.ends_with(['\n', '\r'])
            && let Some((_, Block::Paragraph(last))) = blocks.last()
            && last.runs.is_empty()
        {
            blocks.pop();
        }
        blocks
    }

    /// The empty paragraph a break character stands for, in the style
    /// that covers it.
    fn break_paragraph(&mut self, unit: usize, paragraph_styles: &[Span]) -> Paragraph {
        let mut paragraph = Paragraph::default();
        let style_object = covering(paragraph_styles, unit).or(self.last_paragraph_style);
        if let Some(style_object) = style_object {
            self.last_paragraph_style = Some(style_object);
            let (style, properties, run) = self.resolve_paragraph_style(style_object);
            paragraph.style = style;
            paragraph.properties = properties;
            paragraph.run_properties = run;
        }
        paragraph
    }

    #[allow(clippy::too_many_arguments)]
    fn paragraph(
        &mut self,
        text: &str,
        byte_start: usize,
        unit_start: usize,
        offsets: &mut Utf16Offsets,
        paragraph_styles: &[Span],
        character_styles: &[Span],
        list_styles: &[Span],
        smart_fields: &[Span],
        attachments: &[Span],
        footnotes: &[Span],
        insertions: &[Span],
        deletions: &[Span],
        data: &[(usize, (u8, bool))],
        starts: &[(usize, u32)],
    ) -> Paragraph {
        let mut paragraph = Paragraph::default();
        // Paragraph style: the variation chain gives direct properties, the
        // named ancestor gives the style.
        // An entry without a style means the previous paragraph's style
        // continues, which is also how Pages exports it.
        let style_object = covering(paragraph_styles, unit_start).or(self.last_paragraph_style);
        if let Some(style_object) = style_object {
            self.last_paragraph_style = Some(style_object);
            let (style, properties, run) = self.resolve_paragraph_style(style_object);
            paragraph.style = style;
            paragraph.properties = properties;
            paragraph.run_properties = run;
        }
        // List membership: a list style other than "None" plus the level.
        if let Some(list_object) = covering(list_styles, unit_start)
            && let Some(list) = self.resolve_list_style(list_object)
        {
            let (level, starts_list) = last_at(data, unit_start)
                .map_or((0, false), |(_, (level, starts))| (*level, *starts));
            let start = last_at(starts, unit_start)
                .map_or(1, |(_, number)| *number)
                .max(1);
            paragraph.list = Some(ListItem {
                style: list,
                level,
                starts_list,
                start,
            });
        }
        // Split the text at every boundary of the character style, link,
        // attachment, and change tables, and at the special characters.
        let paragraph_unit_end = unit_start + text.encode_utf16().count();
        let mut boundaries = std::mem::take(&mut self.boundary_scratch);
        boundaries.clear();
        for table in [
            character_styles,
            smart_fields,
            attachments,
            footnotes,
            insertions,
            deletions,
        ] {
            for span in within(table, unit_start + 1, paragraph_unit_end) {
                boundaries.push(span.start);
            }
        }
        boundaries.sort_unstable();
        boundaries.dedup();
        paragraph.runs.reserve(boundaries.len() + 1);
        let tracked = !insertions.is_empty() || !deletions.is_empty();
        let mut position_bytes = byte_start;
        let mut position_units = unit_start;
        let mut piece_start = 0usize;
        let mut piece_start_units = unit_start;
        let mut boundary_index = 0usize;
        for (index, ch) in text.char_indices() {
            let unit = position_units;
            let at_boundary =
                boundary_index < boundaries.len() && boundaries[boundary_index] == unit;
            let special = matches!(ch, LINE_SEPARATOR | '\t' | ATTACHMENT | FOOTNOTE_MARK);
            if (at_boundary || special) && piece_start < index {
                self.push_text_run(
                    &mut paragraph,
                    &text[piece_start..index],
                    piece_start_units,
                    character_styles,
                    smart_fields,
                );
                if tracked {
                    self.mark_revision(&mut paragraph, piece_start_units, insertions, deletions);
                }
                piece_start = index;
                piece_start_units = unit;
            }
            while boundary_index < boundaries.len() && boundaries[boundary_index] <= unit {
                boundary_index += 1;
            }
            if special {
                let run = self.special_run(
                    ch,
                    unit,
                    character_styles,
                    smart_fields,
                    attachments,
                    footnotes,
                );
                if let Some(run) = run {
                    paragraph.runs.push(run);
                    if tracked {
                        self.mark_revision(&mut paragraph, unit, insertions, deletions);
                    }
                }
                piece_start = index + ch.len_utf8();
                piece_start_units = unit + ch.len_utf16();
            }
            position_units += ch.len_utf16();
            position_bytes += ch.len_utf8();
            let _ = offsets;
        }
        if piece_start < text.len() {
            self.push_text_run(
                &mut paragraph,
                &text[piece_start..],
                piece_start_units,
                character_styles,
                smart_fields,
            );
            if tracked {
                self.mark_revision(&mut paragraph, piece_start_units, insertions, deletions);
            }
        }
        let _ = position_bytes;
        self.boundary_scratch = boundaries;
        paragraph
    }

    /// Marks the paragraph's last run as a tracked change when the
    /// insertion or deletion table covers it.
    #[inline(never)]
    fn mark_revision(
        &mut self,
        paragraph: &mut Paragraph,
        unit: usize,
        insertions: &[Span],
        deletions: &[Span],
    ) {
        let change = covering(deletions, unit)
            .map(|object| (RevisionKind::Deletion, object))
            .or_else(|| covering(insertions, unit).map(|object| (RevisionKind::Insertion, object)));
        let Some((kind, object)) = change else {
            return;
        };
        if paragraph.runs.is_empty() {
            return;
        }
        if let Some(id) = self.revision_ids.get(&object) {
            if let Some(run) = paragraph.runs.last_mut() {
                run.revision = Some(*id as Id);
            }
            return;
        }
        let change = self.graph.object(object).map(View::of);
        let author = change
            .and_then(|change| change.reference("session"))
            .and_then(|session| self.graph.object(session))
            .and_then(|session| View::of(session).reference("author"))
            .and_then(|author| self.graph.object(author))
            .and_then(|author| View::of(author).string("name"))
            .map(str::to_string);
        let date = change
            .and_then(|change| change.message("date"))
            .and_then(|date| date.float("seconds"))
            .map(|seconds| format_timestamp(f64::from(seconds)));
        let id = self.document.push_revision(Revision { kind, author, date });
        self.revision_ids.insert(object, id as usize);
        if let Some(run) = paragraph.runs.last_mut() {
            run.revision = Some(id);
        }
    }

    fn push_text_run(
        &mut self,
        paragraph: &mut Paragraph,
        text: &str,
        unit: usize,
        character_styles: &[Span],
        smart_fields: &[Span],
    ) {
        let (style, properties) = self.run_formatting(unit, character_styles);
        let link = self.link_id(unit, smart_fields);
        let span = self.document.push_text(text);
        paragraph.runs.push(Run {
            style,
            properties,
            link,
            revision: None,
            content: Inline::Text(span),
        });
    }

    fn special_run(
        &mut self,
        ch: char,
        unit: usize,
        character_styles: &[Span],
        smart_fields: &[Span],
        attachments: &[Span],
        footnotes: &[Span],
    ) -> Option<Run> {
        let (style, properties) = self.run_formatting(unit, character_styles);
        let content = match ch {
            LINE_SEPARATOR => Inline::LineBreak,
            '\t' => Inline::Tab,
            FOOTNOTE_MARK | ATTACHMENT => {
                let object = footnotes
                    .iter()
                    .chain(attachments)
                    .find(|span| span.start == unit)
                    .and_then(|span| span.object)?;
                self.attachment(object)?
            }
            _ => return None,
        };
        let link = self.link_id(unit, smart_fields);
        Some(Run {
            style,
            properties,
            link,
            revision: None,
            content,
        })
    }

    fn link_id(&mut self, unit: usize, smart_fields: &[Span]) -> Option<Id> {
        let target = self.link_at(unit, smart_fields)?;
        Some(self.document.intern_link(&target))
    }

    fn link_at(&self, unit: usize, smart_fields: &[Span]) -> Option<String> {
        let object = covering(smart_fields, unit)?;
        let message = self.graph.object(object)?;
        let view = View::of(message);
        view.string("url_ref").map(str::to_string)
    }

    /// What an attachment object stands for, told apart by its fields: a
    /// footnote holds a storage, a drawable attachment a drawable, and a
    /// number attachment a page number or count.
    #[inline(never)]
    fn attachment(&mut self, object: u64) -> Option<Inline> {
        let message = self.graph.object(object)?;
        let view = View::of(message);
        if view.reference("contained_storage").is_some() {
            return self.footnote(object).map(Inline::Footnote);
        }
        // A table of contents attachment extends the drawable attachment,
        // so its drawable sits one level down.
        let drawable = view.reference("drawable").or_else(|| {
            view.message("super")
                .and_then(|base| base.reference("drawable"))
        });
        if let Some(drawable) = drawable {
            return self.drawable(drawable, view);
        }
        // A table of contents entry's page number is stored as text.
        if let Some(number) = view.string("page_number") {
            return Some(Inline::Text(self.document.push_text(number)));
        }
        if view.string("number_format_name").is_some() {
            let kind = view
                .message("super")
                .and_then(|textual| textual.integer("kind"))
                .unwrap_or(0);
            return Some(match kind {
                1 => Inline::PageCount,
                _ => Inline::PageNumber,
            });
        }
        None
    }

    /// A drawable in the text: a table becomes a pending block, an image
    /// an inline.
    #[inline(never)]
    fn drawable(&mut self, drawable: u64, attachment: View<'_>) -> Option<Inline> {
        let message = self.graph.object(drawable)?;
        let view = View::of(message);
        if let Some(model) = view.reference("tableModel") {
            let table = self.table(model)?;
            self.pending_blocks.push(Block::Table(table));
            return None;
        }
        // An equation is an image object carrying its MathML source.
        if let Some(mathml) = view.string("equation_source_text") {
            return Some(Inline::Math(self.document.push_text(mathml)));
        }
        if view.message("data").is_some() {
            let image = self.image(view, attachment)?;
            return Some(Inline::Image(self.document.push_image(image)));
        }
        // A table of contents keeps its rendered entries in the storage
        // its shape owns; they follow the paragraph as text, as Pages
        // exports them.
        if view.reference("toc_settings").is_some() {
            let storage = view
                .message("super")
                .and_then(|shape| shape.reference("owned_storage"))
                .and_then(|storage| self.graph.object(storage))?;
            let entries = self.nested_blocks(View::of(storage));
            self.following_blocks.extend(entries);
            return None;
        }
        None
    }

    /// `TSD.ImageArchive` with its attachment's placement.
    #[inline(never)]
    fn image(&mut self, image: View<'_>, attachment: View<'_>) -> Option<InlineImage> {
        let data = image.message("data")?.integer("identifier")? as u64;
        let media = self.media_for(data)?;
        let drawable = image.message("super");
        let size = drawable
            .and_then(|drawable| drawable.message("geometry"))
            .and_then(|geometry| geometry.message("size"))
            .or_else(|| image.message("originalSize"));
        let width = size.and_then(|size| size.float("width")).unwrap_or(0.0);
        let height = size.and_then(|size| size.float("height")).unwrap_or(0.0);
        let description = drawable
            .and_then(|drawable| drawable.string("accessibility_description"))
            .filter(|text| !text.is_empty())
            .map(str::to_string);
        // Inline attachments carry no offsets (NaN); anchored ones do.
        let horizontal = attachment
            .float("h_offset")
            .filter(|value| value.is_finite());
        let placement = match horizontal {
            Some(offset) => Placement::Floating {
                horizontal: Anchor {
                    from: match attachment.integer("h_offset_type").unwrap_or(0) {
                        2 => AnchorBase::Page,
                        _ => AnchorBase::Margin,
                    },
                    offset,
                },
                vertical: Anchor {
                    from: match attachment.integer("v_offset_type").unwrap_or(0) {
                        2 => AnchorBase::Page,
                        1 => AnchorBase::Margin,
                        _ => AnchorBase::Line,
                    },
                    offset: attachment
                        .float("v_offset")
                        .filter(|value| value.is_finite())
                        .unwrap_or(0.0),
                },
            },
            None => Placement::Inline,
        };
        Some(InlineImage {
            media,
            width,
            height,
            description,
            placement,
        })
    }

    /// The media entry for a data identifier, read from `Data/` on first
    /// use.
    #[inline(never)]
    fn media_for(&mut self, data: u64) -> Option<MediaId> {
        if let Some(id) = self.media.get(&data) {
            return Some(*id);
        }
        if self.data_files.is_none() {
            self.data_files = Some(self.read_data_files());
        }
        let file_name = self.data_files.as_ref()?.get(&data)?.clone();
        let path = format!("Data/{file_name}");
        let bytes = self.package.entries.iter().find_map(|entry| match entry {
            Entry::File { name, bytes } if *name == path => Some(bytes.clone()),
            _ => None,
        })?;
        let id = self.document.media.len();
        self.document.media.push(Media {
            name: file_name,
            bytes,
        });
        self.media.insert(data, id);
        Some(id)
    }

    /// Data identifier -> file name, from `TSP.PackageMetadata.datas`.
    #[inline(never)]
    fn read_data_files(&self) -> HashMap<u64, String> {
        let mut files = HashMap::new();
        for (_, message) in self.graph.objects_of_type(PACKAGE_METADATA) {
            for data in View::of(message).messages("datas") {
                let identifier = data.integer("identifier").unwrap_or(0) as u64;
                let name = data.string("file_name").unwrap_or("");
                if !name.is_empty() {
                    files.insert(identifier, name.to_string());
                }
            }
        }
        files
    }

    // ----- tables -----

    /// `TST.TableModelArchive` to a table: the grid from the tiles' cell
    /// storage, column widths and row heights from the header buckets,
    /// merged regions from the calculation engine.
    #[inline(never)]
    fn table(&mut self, model: u64) -> Option<Table> {
        let message = self.graph.object(model)?;
        let view = View::of(message);
        let row_count = view.integer("number_of_rows")?.clamp(0, 1 << 20) as usize;
        let column_count = view.integer("number_of_columns")?.clamp(0, 1 << 16) as usize;
        let header_rows = view.integer("number_of_header_rows").unwrap_or(0) as u32;
        let store = view.message("base_data_store")?;
        let mut columns = vec![view.float("default_column_width").unwrap_or(100.0); column_count];
        if let Some(bucket) = store.reference("columnHeaders") {
            self.header_sizes(bucket, &mut columns);
        }
        let mut heights: Vec<f32> =
            vec![view.float("default_row_height").unwrap_or(0.0); row_count];
        if let Some(row_headers) = store.message("rowHeaders") {
            for bucket in row_headers.references("buckets") {
                self.header_sizes(bucket, &mut heights);
            }
        }
        let lists = TableLists {
            strings: self.data_list_strings(store.reference("stringTable")),
            rich_text: self
                .data_list_references(store.reference("rich_text_table"), "rich_text_payload"),
            styles: self.data_list_references(store.reference("styleTable"), "reference"),
        };
        let header_text_style = view.reference("header_row_text_style");
        let body_text_style = view.reference("body_text_style");
        let header_fill = view
            .reference("header_row_style")
            .and_then(|style| self.cell_fill(style));
        let body_fill = view
            .reference("body_cell_style")
            .and_then(|style| self.cell_fill(style));
        let mut rows: Vec<Row> = (0..row_count)
            .map(|row| Row {
                cells: (0..column_count)
                    .map(|_| Cell {
                        column_span: 1,
                        row_span: 1,
                        background: if (row as u32) < header_rows {
                            header_fill
                        } else {
                            body_fill
                        },
                        ..Cell::default()
                    })
                    .collect(),
                height: Some(heights[row]).filter(|height| *height > 0.0),
            })
            .collect();
        if let Some(tiles) = store.message("tiles") {
            let tile_size = tiles.integer("tile_size").unwrap_or(256).max(1) as usize;
            for tile in tiles.messages("tiles") {
                let base = tile.integer("tileid").unwrap_or(0).max(0) as usize * tile_size;
                let Some(tile) = tile.reference("tile").and_then(|id| self.graph.object(id)) else {
                    continue;
                };
                for info in View::of(tile).messages("rowInfos") {
                    let row = base + info.integer("tile_row_index").unwrap_or(0).max(0) as usize;
                    if row >= row_count {
                        continue;
                    }
                    self.tile_row(
                        &info,
                        &mut rows[row].cells,
                        &lists,
                        if (row as u32) < header_rows {
                            header_text_style
                        } else {
                            body_text_style
                        },
                    );
                }
            }
        }
        if let Some(owner) = view
            .message("merge_owner")
            .and_then(|owner| owner.message("owner_id"))
            .and_then(uuid)
        {
            for region in self.merge_regions(owner) {
                apply_merge(&mut rows, region);
            }
        }
        Some(Table {
            rows,
            header_rows,
            columns,
        })
    }

    /// One tile row: the cells at the offsets that are not empty.
    #[inline(never)]
    fn tile_row(
        &mut self,
        info: &View<'_>,
        cells: &mut [Cell],
        lists: &TableLists,
        default_text_style: Option<u64>,
    ) {
        let Some(offsets) = info.bytes("cell_offsets") else {
            return;
        };
        let Some(buffer) = info.bytes("cell_storage_buffer") else {
            return;
        };
        let wide = info.boolean("has_wide_offsets").unwrap_or(false);
        for (column, cell) in cells.iter_mut().enumerate() {
            let Some(pair) = offsets.get(column * 2..column * 2 + 2) else {
                break;
            };
            let offset = u16::from_le_bytes([pair[0], pair[1]]);
            if offset == u16::MAX {
                continue;
            }
            let mut offset = usize::from(offset);
            if wide {
                offset *= 4;
            }
            let Some(record) = buffer.get(offset..).and_then(cell_record) else {
                continue;
            };
            self.fill_cell(cell, &record, lists, default_text_style);
        }
    }

    #[inline(never)]
    fn fill_cell(
        &mut self,
        cell: &mut Cell,
        record: &CellRecord,
        lists: &TableLists,
        default_text_style: Option<u64>,
    ) {
        if let Some(style) = record
            .cell_style
            .and_then(|id| lists.styles.get(&id))
            .and_then(|object| self.cell_fill(*object))
        {
            cell.background = Some(style);
        }
        let text = match record.kind {
            // Rich text: a storage of its own.
            9 => {
                let storage = record
                    .rich_text
                    .and_then(|id| lists.rich_text.get(&id))
                    .and_then(|payload| self.graph.object(*payload))
                    .and_then(|payload| View::of(payload).reference("storage"))
                    .and_then(|storage| self.graph.object(storage));
                if let Some(storage) = storage {
                    cell.blocks = self.nested_blocks(View::of(storage));
                }
                return;
            }
            3 => record.string.and_then(|id| lists.strings.get(&id)).cloned(),
            2 | 10 => record.double.map(format_number),
            6 => record.double.map(|value| {
                if value != 0.0 {
                    "TRUE".to_string()
                } else {
                    "FALSE".to_string()
                }
            }),
            5 => record.seconds.map(format_date),
            7 => record.double.map(format_duration),
            _ => None,
        };
        let Some(text) = text else {
            return;
        };
        let style_object = record
            .text_style
            .and_then(|id| lists.styles.get(&id))
            .copied()
            .or(default_text_style);
        let mut paragraph = Paragraph::default();
        if let Some(style_object) = style_object {
            let (style, properties, run) = self.resolve_paragraph_style(style_object);
            paragraph.style = style;
            paragraph.properties = properties;
            paragraph.run_properties = run;
        }
        let span = self.document.push_text(&text);
        paragraph.runs.push(Run {
            style: None,
            properties: None,
            link: None,
            revision: None,
            content: Inline::Text(span),
        });
        cell.blocks = vec![Block::Paragraph(paragraph)];
    }

    /// The fill color of a `TST.CellStyleArchive`, through its parents.
    fn cell_fill(&self, style: u64) -> Option<Color> {
        let mut current = Some(style);
        let mut depth = 0;
        while let Some(identifier) = current {
            let message = self.graph.object(identifier)?;
            let view = View::of(message);
            let fill = view
                .message("cell_properties")
                .and_then(|properties| properties.message("cell_fill"));
            if let Some(fill) = fill {
                return fill.message("color").and_then(color);
            }
            current = view.message("super").and_then(|s| s.reference("parent"));
            depth += 1;
            if depth > 64 {
                break;
            }
        }
        None
    }

    /// Sizes from a `TST.HeaderStorageBucket`, by index.
    fn header_sizes(&self, bucket: u64, sizes: &mut [f32]) {
        let Some(message) = self.graph.object(bucket) else {
            return;
        };
        for header in View::of(message).messages("headers") {
            let index = header.integer("index").unwrap_or(-1);
            if index < 0 || index as usize >= sizes.len() {
                continue;
            }
            if let Some(size) = header.float("size") {
                sizes[index as usize] = size;
            }
        }
    }

    /// Key -> string of a `TST.TableDataList`.
    fn data_list_strings(&self, list: Option<u64>) -> HashMap<u32, String> {
        let mut strings = HashMap::new();
        let Some(message) = list.and_then(|id| self.graph.object(id)) else {
            return strings;
        };
        for entry in View::of(message).messages("entries") {
            if let Some(key) = entry.integer("key")
                && let Some(text) = entry.string("string")
            {
                strings.insert(key as u32, text.to_string());
            }
        }
        strings
    }

    /// Key -> referenced object of a `TST.TableDataList`, for the field
    /// that holds the reference.
    fn data_list_references(&self, list: Option<u64>, field: &str) -> HashMap<u32, u64> {
        let mut references = HashMap::new();
        let Some(message) = list.and_then(|id| self.graph.object(id)) else {
            return references;
        };
        for entry in View::of(message).messages("entries") {
            if let Some(key) = entry.integer("key")
                && let Some(reference) = entry.reference(field)
            {
                references.insert(key as u32, reference);
            }
        }
        references
    }

    /// The merged regions a merge owner records, from the calculation
    /// engine's dependency tracker, where the owner's range dependencies
    /// are the regions.
    fn merge_regions(&mut self, owner: [u64; 4]) -> Vec<Region> {
        if self.merges.is_none() {
            self.merges = Some(self.read_merges());
        }
        self.merges
            .as_ref()
            .and_then(|merges| merges.get(&owner))
            .cloned()
            .unwrap_or_default()
    }

    #[inline(never)]
    fn read_merges(&self) -> HashMap<[u64; 4], Vec<Region>> {
        let mut merges: HashMap<[u64; 4], Vec<Region>> = HashMap::new();
        let mut collect = |owner: View<'_>, id_field: &str| {
            let Some(id) = owner.message(id_field).and_then(uuid) else {
                return;
            };
            let Some(dependencies) = owner.message("range_dependencies") else {
                return;
            };
            for dependency in dependencies.messages("back_dependency") {
                let range = dependency
                    .message("internal_range_reference")
                    .and_then(|reference| reference.message("range"));
                let Some(range) = range else {
                    continue;
                };
                let left = range.integer("top_left_column").unwrap_or(0).max(0) as usize;
                let top = range.integer("top_left_row").unwrap_or(0).max(0) as usize;
                let right = range.integer("bottom_right_column").unwrap_or(0).max(0) as usize;
                let bottom = range.integer("bottom_right_row").unwrap_or(0).max(0) as usize;
                if right < left || bottom < top {
                    continue;
                }
                merges.entry(id).or_default().push(Region {
                    row: top,
                    column: left,
                    rows: bottom - top + 1,
                    columns: right - left + 1,
                });
            }
        };
        for (_, message) in self.graph.objects_of_type(CALCULATION_ENGINE) {
            let Some(tracker) = View::of(message).message("dependency_tracker") else {
                continue;
            };
            for owner in tracker.messages("formula_owner_info") {
                collect(owner, "formula_owner_id");
            }
            for owner in tracker.references("formula_owner_dependencies") {
                if let Some(owner) = self.graph.object(owner) {
                    collect(View::of(owner), "formula_owner_uid");
                }
            }
        }
        merges
    }

    /// A footnote reference attachment: its storage becomes a note.
    #[inline(never)]
    fn footnote(&mut self, attachment: u64) -> Option<usize> {
        let message = self.graph.object(attachment)?;
        let storage = View::of(message).reference("contained_storage")?;
        let storage = self.graph.object(storage)?;
        // A note is its own storage; the body's running style must not
        // leak in or out.
        let mut blocks = self.nested_blocks(View::of(storage));
        // The note's text starts with the mark placeholder; drop it.
        if let Some(Block::Paragraph(first)) = blocks.first_mut()
            && let Some(run) = first.runs.first_mut()
            && let Inline::Text(span) = &mut run.content
        {
            let text = self.document.text(*span);
            let kept = text.trim_start_matches(ATTACHMENT).trim_start().len();
            span.start = span.end - kept as u32;
        }
        self.document.footnotes.push(Note { blocks });
        Some(self.document.footnotes.len() - 1)
    }

    fn run_formatting(
        &mut self,
        unit: usize,
        character_styles: &[Span],
    ) -> (Option<StyleId>, Option<Id>) {
        match covering(character_styles, unit) {
            Some(object) => self.resolve_character_style(object),
            None => (None, None),
        }
    }

    // ----- styles -----

    /// Walks the variation chain of a paragraph style object: unnamed
    /// styles become direct properties, the first named one is the style.
    #[inline(never)]
    fn resolve_paragraph_style(
        &mut self,
        object: u64,
    ) -> (Option<StyleId>, Option<Id>, Option<Id>) {
        if let Some(index) = self.resolved_paragraph.get(&object) {
            return self.resolved_paragraphs[*index];
        }
        let (style, properties, run) = self.resolve_paragraph_chain(object);
        let resolved = (
            style,
            self.document.intern_paragraph_properties(properties),
            self.document.intern_run_properties(run),
        );
        self.resolved_paragraph
            .insert(object, self.resolved_paragraphs.len());
        self.resolved_paragraphs.push(resolved);
        resolved
    }

    #[inline(never)]
    fn resolve_paragraph_chain(
        &mut self,
        object: u64,
    ) -> (Option<StyleId>, ParagraphProperties, RunProperties) {
        let mut properties = ParagraphProperties::default();
        let mut run = RunProperties::default();
        let mut current = Some(object);
        let mut overrides: Vec<(ParagraphProperties, RunProperties)> = Vec::new();
        let mut named = None;
        while let Some(identifier) = current {
            let message = match self.graph.object(identifier) {
                Some(message) => message,
                None => break,
            };
            let view = paragraph_style_view(View::of(message));
            let name = view
                .message("super")
                .and_then(|s| s.string("name"))
                .unwrap_or("");
            if !name.is_empty() {
                named = Some(self.paragraph_style_id(identifier));
                break;
            }
            let paragraph = view
                .message("para_properties")
                .map(paragraph_properties)
                .unwrap_or_default();
            let characters = view
                .message("char_properties")
                .map(|properties| self.run_properties(properties))
                .unwrap_or_default();
            overrides.push((paragraph, characters));
            current = view.message("super").and_then(|s| s.reference("parent"));
        }
        // Outermost first, so inner overrides win.
        for (paragraph, characters) in overrides.into_iter().rev() {
            properties.overlay(&paragraph);
            run.overlay(&characters);
        }
        (named, properties, run)
    }

    #[inline(never)]
    fn resolve_character_style(&mut self, object: u64) -> (Option<StyleId>, Option<Id>) {
        if let Some(index) = self.resolved_character.get(&object) {
            return self.resolved_characters[*index];
        }
        let (style, run) = self.resolve_character_chain(object);
        let resolved = (style, self.document.intern_run_properties(run));
        self.resolved_character
            .insert(object, self.resolved_characters.len());
        self.resolved_characters.push(resolved);
        resolved
    }

    #[inline(never)]
    fn resolve_character_chain(&mut self, object: u64) -> (Option<StyleId>, RunProperties) {
        let mut run = RunProperties::default();
        let mut current = Some(object);
        let mut overrides: Vec<RunProperties> = Vec::new();
        let mut named = None;
        while let Some(identifier) = current {
            let message = match self.graph.object(identifier) {
                Some(message) => message,
                None => break,
            };
            let view = View::of(message);
            let name = view
                .message("super")
                .and_then(|s| s.string("name"))
                .unwrap_or("");
            // Pages' default character style is named "None": no style.
            if !name.is_empty() && name != "None" {
                named = Some(self.character_style_id(identifier));
                break;
            }
            overrides.push(
                view.message("char_properties")
                    .map(|properties| self.run_properties(properties))
                    .unwrap_or_default(),
            );
            current = view.message("super").and_then(|s| s.reference("parent"));
        }
        for characters in overrides.into_iter().rev() {
            run.overlay(&characters);
        }
        (named, run)
    }

    /// The model style for a named Pages paragraph style, created on
    /// first use with its parent chain.
    #[inline(never)]
    fn paragraph_style_id(&mut self, object: u64) -> StyleId {
        if let Some(id) = self.paragraph_styles.get(&object) {
            return *id;
        }
        let id = self.document.styles.paragraph.len();
        self.document
            .styles
            .paragraph
            .push(ParagraphStyle::default());
        self.paragraph_styles.insert(object, id);
        let mut style = ParagraphStyle::default();
        if let Some(message) = self.graph.object(object) {
            let view = paragraph_style_view(View::of(message));
            let meta = view.message("super");
            style.name = meta
                .and_then(|m| m.string("name"))
                .unwrap_or("")
                .to_string();
            style.paragraph = view
                .message("para_properties")
                .map(paragraph_properties)
                .unwrap_or_default();
            style.run = view
                .message("char_properties")
                .map(|properties| self.run_properties(properties))
                .unwrap_or_default();
            if let Some(parent) = meta.and_then(|m| m.reference("parent")) {
                style.parent = Some(self.paragraph_style_id(parent));
            }
        }
        self.document.styles.paragraph[id] = style;
        id
    }

    #[inline(never)]
    fn character_style_id(&mut self, object: u64) -> StyleId {
        if let Some(id) = self.character_styles.get(&object) {
            return *id;
        }
        let id = self.document.styles.character.len();
        self.document
            .styles
            .character
            .push(CharacterStyle::default());
        self.character_styles.insert(object, id);
        let mut style = CharacterStyle::default();
        if let Some(message) = self.graph.object(object) {
            let view = View::of(message);
            let meta = view.message("super");
            style.name = meta
                .and_then(|m| m.string("name"))
                .unwrap_or("")
                .to_string();
            style.run = view
                .message("char_properties")
                .map(|properties| self.run_properties(properties))
                .unwrap_or_default();
            if let Some(parent) = meta.and_then(|m| m.reference("parent")) {
                style.parent = Some(self.character_style_id(parent));
            }
        }
        self.document.styles.character[id] = style;
        id
    }

    /// The list style, unless it is the "no list" style.
    #[inline(never)]
    fn resolve_list_style(&mut self, object: u64) -> Option<StyleId> {
        if let Some(id) = self.list_styles.get(&object) {
            return *id;
        }
        let message = self.graph.object(object)?;
        let view = View::of(message);
        let label_types = view.integers("label_types");
        let is_list = label_types.iter().any(|kind| *kind != 0);
        if !is_list {
            self.list_styles.insert(object, None);
            return None;
        }
        let name = view
            .message("super")
            .and_then(|m| m.string("name"))
            .unwrap_or("")
            .to_string();
        let number_types = view.integers("number_types");
        let strings = view.strings("strings");
        let indents = view.floats("indents");
        let text_indents = view.floats("text_indents");
        let mut levels = Vec::new();
        for (level, kind) in label_types.iter().enumerate() {
            let label = match kind {
                2 => ListLabel::Text(strings.get(level).map_or("\u{2022}", |s| s).to_string()),
                3 => {
                    ListLabel::Number(number_format(number_types.get(level).copied().unwrap_or(0)))
                }
                _ => ListLabel::None,
            };
            let indent = indents.get(level).copied().unwrap_or(0.0);
            levels.push(ListLevel {
                label,
                indent,
                label_indent: indent + text_indents.get(level).copied().unwrap_or(0.0),
            });
        }
        let id = self.document.styles.list.len();
        self.document.styles.list.push(ListStyle { name, levels });
        self.list_styles.insert(object, Some(id));
        Some(id)
    }
}

/// Converts `TSWP.ParagraphStylePropertiesArchive`.
fn paragraph_properties(view: View<'_>) -> ParagraphProperties {
    ParagraphProperties {
        alignment: view.integer("alignment").and_then(|value| match value {
            0 => Some(Alignment::Left),
            1 => Some(Alignment::Right),
            2 => Some(Alignment::Center),
            3 => Some(Alignment::Justify),
            _ => None, // 4 is "natural": the language's default
        }),
        first_line_indent: view.float("first_line_indent"),
        left_indent: view.float("left_indent"),
        right_indent: view.float("right_indent"),
        space_before: view.float("space_before"),
        space_after: view.float("space_after"),
        line_spacing: view.message("line_spacing").and_then(|spacing| {
            let amount = spacing.float("amount")?;
            Some(match spacing.integer("mode").unwrap_or(0) {
                1 => LineSpacing::Minimum(amount),
                2 => LineSpacing::Exact(amount),
                _ => LineSpacing::Relative(amount),
            })
        }),
        keep_with_next: view.boolean("keep_with_next"),
        keep_lines_together: view.boolean("keep_lines_together"),
        widow_control: view.boolean("widow_control"),
        // Pages stores "no outline level" as -1 in a uint32 field.
        outline_level: view
            .integer("outline_level")
            .filter(|level| (0..=8).contains(level))
            .map(|level| level as u8),
        background: view.message("fill").and_then(color),
    }
}

impl Reader<'_> {
    /// Converts `TSWP.CharacterStylePropertiesArchive`, interning the font
    /// and language names.
    #[inline(never)]
    fn run_properties(&mut self, view: View<'_>) -> RunProperties {
        RunProperties {
            // A font the document asked for but the Mac lacked is kept
            // beside the substitute; the request is what the document means.
            font: view
                .string("compatibility_font_name")
                .or_else(|| view.string("font_name"))
                .map(|name| self.document.intern_string(name)),
            size: view.float("font_size"),
            bold: view.boolean("bold"),
            italic: view.boolean("italic"),
            underline: view.integer("underline").map(|value| value != 0),
            strike: view.integer("strikethru").map(|value| value != 0),
            color: view.message("font_color").and_then(color),
            highlight: view.message("background_color").and_then(color),
            baseline: view.integer("superscript").and_then(|value| match value {
                1 => Some(Baseline::Superscript),
                2 => Some(Baseline::Subscript),
                _ => None,
            }),
            caps: view
                .integer("capitalization")
                .and_then(|value| match value {
                    1 => Some(Caps::All),
                    2 => Some(Caps::Small),
                    _ => None,
                }),
            language: view
                .string("language")
                .map(|language| self.document.intern_string(language)),
        }
    }
}

/// The paragraph style inside a style object: a table of contents entry
/// style wraps one, so its `super` is the paragraph style rather than
/// the style's own name and parent.
fn paragraph_style_view(view: View<'_>) -> View<'_> {
    match view.message("super") {
        Some(base) if base.message("super").is_some() => base,
        _ => view,
    }
}

/// Marks the cells a merged region covers and sizes its origin.
fn apply_merge(rows: &mut [Row], region: Region) {
    let last_row = region.row + region.rows - 1;
    let last_column = region.column + region.columns - 1;
    if region.rows == 0 || region.columns == 0 || last_row >= rows.len() {
        return;
    }
    if rows[region.row].cells.len() <= last_column {
        return;
    }
    for (row, grid_row) in rows.iter_mut().enumerate() {
        if row < region.row || row > last_row {
            continue;
        }
        for (column, cell) in grid_row.cells.iter_mut().enumerate() {
            if column < region.column || column > last_column {
                continue;
            }
            if row == region.row && column == region.column {
                cell.column_span = region.columns as u32;
                cell.row_span = region.rows as u32;
            } else if column == region.column {
                cell.merge = Merge::Above;
                cell.column_span = region.columns as u32;
                cell.blocks.clear();
            } else {
                cell.merge = Merge::Left;
                cell.blocks.clear();
            }
        }
    }
}

/// `TSP.CFUUIDArchive` as four words.
fn uuid(view: View<'_>) -> Option<[u64; 4]> {
    Some([
        view.integer("uuid_w0")? as u64,
        view.integer("uuid_w1")? as u64,
        view.integer("uuid_w2")? as u64,
        view.integer("uuid_w3")? as u64,
    ])
}

/// The page of `TP.DocumentArchive`.
fn page_setup(root: View<'_>) -> PageSetup {
    let default = PageSetup::default();
    PageSetup {
        width: root.float("page_width").unwrap_or(default.width),
        height: root.float("page_height").unwrap_or(default.height),
        margin_top: root.float("top_margin").unwrap_or(default.margin_top),
        margin_bottom: root.float("bottom_margin").unwrap_or(default.margin_bottom),
        margin_left: root.float("left_margin").unwrap_or(default.margin_left),
        margin_right: root.float("right_margin").unwrap_or(default.margin_right),
        header_distance: root
            .float("header_margin")
            .unwrap_or(default.header_distance),
        footer_distance: root
            .float("footer_margin")
            .unwrap_or(default.footer_distance),
    }
}

/// A cell's number the short way: integers without a fraction, others
/// with the digits that round-trip.
fn format_number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

/// A cell's date from seconds since 2001-01-01, as `YYYY-MM-DD`, with the
/// time when it has one.
fn format_date(seconds: f64) -> String {
    let total = seconds.floor() as i64;
    let days = total.div_euclid(86_400) + 730_791; // days from 0000-03-01 to 2001-01-01
    let seconds_of_day = total.rem_euclid(86_400);
    // Civil date from a day count (Howard Hinnant's algorithm).
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    let mut text = format!("{year:04}-{month:02}-{day:02}");
    if seconds_of_day != 0 {
        let _ = std::fmt::Write::write_fmt(
            &mut text,
            format_args!(
                " {:02}:{:02}",
                seconds_of_day / 3600,
                seconds_of_day % 3600 / 60
            ),
        );
    }
    text
}

/// Seconds since 2001-01-01 as an ISO 8601 UTC timestamp.
fn format_timestamp(seconds: f64) -> String {
    let total = seconds.floor() as i64;
    let date = format_date((total.div_euclid(86_400) * 86_400) as f64);
    let seconds_of_day = total.rem_euclid(86_400);
    format!(
        "{date}T{:02}:{:02}:{:02}Z",
        seconds_of_day / 3600,
        seconds_of_day % 3600 / 60,
        seconds_of_day % 60
    )
}

/// A cell's duration in seconds as weeks, days, hours, minutes, seconds,
/// the zero parts left out.
fn format_duration(seconds: f64) -> String {
    let mut remaining = seconds.round() as i64;
    let mut parts = Vec::new();
    for (unit, size) in [
        ("w", 604_800),
        ("d", 86_400),
        ("h", 3600),
        ("m", 60),
        ("s", 1),
    ] {
        let count = remaining / size;
        if count != 0 {
            parts.push(format!("{count}{unit}"));
            remaining -= count * size;
        }
    }
    if parts.is_empty() {
        "0s".to_string()
    } else {
        parts.join(" ")
    }
}

/// `TSP.Color` with an RGB model to sRGB bytes.
fn color(view: View<'_>) -> Option<Color> {
    let channel =
        |name: &str| (view.float(name).unwrap_or(0.0).clamp(0.0, 1.0) * 255.0).round() as u8;
    Some(Color {
        red: channel("r"),
        green: channel("g"),
        blue: channel("b"),
    })
}

/// `TSWP.ListStyleArchive.NumberType` to a format.
fn number_format(number_type: i64) -> NumberFormat {
    let kind = match number_type {
        3..=5 => NumberKind::UpperRoman,
        6..=8 => NumberKind::LowerRoman,
        9..=11 => NumberKind::UpperLetter,
        12..=14 => NumberKind::LowerLetter,
        _ => NumberKind::Decimal,
    };
    let pattern = match number_type % 3 {
        1 => "(%1)",
        2 => "%1)",
        _ => "%1.",
    };
    NumberFormat {
        kind,
        pattern: pattern.to_string(),
    }
}

/// Maps byte offsets of a string to UTF-16 unit offsets, which is how the
/// attribute tables count characters. Offsets are asked for in increasing
/// order, so a cursor over the text does it in one pass without a table.
struct Utf16Offsets<'t> {
    text: &'t str,
    byte: usize,
    units: usize,
}

impl<'t> Utf16Offsets<'t> {
    fn new(text: &'t str) -> Utf16Offsets<'t> {
        Utf16Offsets {
            text,
            byte: 0,
            units: 0,
        }
    }

    fn units_at(&mut self, byte: usize) -> usize {
        if byte < self.byte {
            self.byte = 0;
            self.units = 0;
        }
        let end = byte.min(self.text.len());
        for ch in self.text[self.byte..end].chars() {
            self.units += ch.len_utf16();
        }
        self.byte = end;
        self.units
    }
}
