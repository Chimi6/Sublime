//! Reads a Pages package into the document model: body text with its
//! paragraph and character styles resolved through the stylesheet, lists,
//! links, footnotes, and breaks. Tables, images, and sections come next.

use std::collections::HashMap;

use super::package::{Entry, Package, Stream};
use crate::document::{
    Alignment, Baseline, Block, Caps, CharacterStyle, Color, Document, Inline, LineSpacing,
    ListItem, ListLabel, ListLevel, ListStyle, Note, NumberFormat, NumberKind, Paragraph,
    ParagraphProperties, ParagraphStyle, Run, RunProperties, Section, StyleId,
};
use crate::io::protobuf::tree::{Node, Tree};

/// Pages marks a line break, a page break, and an attachment with these.
const LINE_SEPARATOR: char = '\u{2028}';
const PAGE_BREAK: char = '\u{5}';
const ATTACHMENT: char = '\u{FFFC}';
const FOOTNOTE_MARK: char = '\u{E}';

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

    fn entries(&self, name: &str) -> impl Iterator<Item = Node> + '_ {
        let name = name.to_string();
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

fn attribute_table(view: &View<'_>, name: &str) -> Vec<Span> {
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

/// The paragraph data table: `(start, level, starts_list)` per entry.
fn paragraph_data(view: &View<'_>) -> Vec<(usize, u8, bool)> {
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
                entry.integer("first").unwrap_or(0) as u8,
                entry.integer("second").unwrap_or(0) != 0,
            )
        })
        .collect()
}

/// The entry of a table that covers `position`: the last one at or before it.
fn covering(spans: &[Span], position: usize) -> Option<u64> {
    spans
        .iter()
        .take_while(|span| span.start <= position)
        .last()
        .and_then(|span| span.object)
}

pub struct Reader<'p> {
    graph: Graph<'p>,
    document: Document,
    /// Pages style object -> model style, per table.
    paragraph_styles: HashMap<u64, StyleId>,
    character_styles: HashMap<u64, StyleId>,
    list_styles: HashMap<u64, Option<StyleId>>,
    /// The last paragraph style object seen, for entries without one.
    last_paragraph_style: Option<u64>,
}

/// Reads the document the package holds.
pub fn read_document(package: &Package) -> Document {
    let mut reader = Reader {
        graph: Graph::new(package),
        document: Document::default(),
        paragraph_styles: HashMap::new(),
        character_styles: HashMap::new(),
        list_styles: HashMap::new(),
        last_paragraph_style: None,
    };
    reader.read();
    reader.document
}

/// Type ids of the messages the reader looks for.
const DOCUMENT_ARCHIVE: u32 = 10000;
const STORAGE_ARCHIVE: u32 = 2001;

impl Reader<'_> {
    fn read(&mut self) {
        let root = self
            .graph
            .objects_of_type(DOCUMENT_ARCHIVE)
            .into_iter()
            .next()
            .map(|(_, message)| message);
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
        let mut section = Section {
            columns: 1,
            ..Section::default()
        };
        if let Some(body) = body {
            section.blocks = self.storage_blocks(View::of(body));
        }
        self.document.sections.push(section);
    }

    /// The paragraphs of a text storage, in order.
    fn storage_blocks(&mut self, storage: View<'_>) -> Vec<Block> {
        let text: String = storage.strings("text").concat();
        let paragraph_styles = attribute_table(&storage, "table_para_style");
        let character_styles = attribute_table(&storage, "table_char_style");
        let list_styles = attribute_table(&storage, "table_list_style");
        let smart_fields = attribute_table(&storage, "table_smartfield");
        let attachments = attribute_table(&storage, "table_attachment");
        let footnotes = attribute_table(&storage, "table_footnote");
        let data = paragraph_data(&storage);
        let mut blocks = Vec::new();
        let mut start = 0usize;
        let bytes = text.as_bytes();
        // Character indices in Pages count UTF-16 units; the tables are
        // converted to byte offsets as the text is walked.
        let mut offsets = Utf16Offsets::new(&text);
        loop {
            let end = bytes[start..]
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(bytes.len(), |relative| start + relative);
            let start_units = offsets.units_at(start);
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
                &data,
            );
            blocks.push(Block::Paragraph(paragraph));
            if end >= bytes.len() {
                break;
            }
            start = end + 1;
        }
        blocks
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
        data: &[(usize, u8, bool)],
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
            let (level, starts_list) = data
                .iter()
                .take_while(|(start, _, _)| *start <= unit_start)
                .last()
                .map_or((0, false), |(_, level, starts)| (*level, *starts));
            paragraph.list = Some(ListItem {
                style: list,
                level,
                starts_list,
            });
        }
        let mut text = text;
        if text.starts_with(PAGE_BREAK) {
            paragraph.page_break_before = true;
            text = &text[PAGE_BREAK.len_utf8()..];
        }
        // Split the text at every boundary of the character style, link,
        // and attachment tables, and at the special characters.
        let paragraph_unit_end =
            unit_start + text.encode_utf16().count() + usize::from(paragraph.page_break_before);
        let mut boundaries: Vec<usize> = Vec::new();
        for span in character_styles
            .iter()
            .chain(smart_fields)
            .chain(attachments)
            .chain(footnotes)
        {
            if span.start > unit_start && span.start < paragraph_unit_end {
                boundaries.push(span.start);
            }
        }
        boundaries.sort_unstable();
        boundaries.dedup();
        let mut position_bytes =
            byte_start + usize::from(paragraph.page_break_before) * PAGE_BREAK.len_utf8();
        let mut position_units = unit_start + usize::from(paragraph.page_break_before);
        let mut piece_start = 0usize;
        let mut boundary_index = 0usize;
        for (index, ch) in text.char_indices() {
            let unit = position_units;
            let at_boundary =
                boundary_index < boundaries.len() && boundaries[boundary_index] == unit;
            let special = matches!(ch, LINE_SEPARATOR | '\t' | ATTACHMENT | FOOTNOTE_MARK);
            if (at_boundary || special) && piece_start < index {
                let piece_units = position_units - text[piece_start..index].encode_utf16().count();
                self.push_text_run(
                    &mut paragraph,
                    &text[piece_start..index],
                    piece_units,
                    character_styles,
                    smart_fields,
                );
                piece_start = index;
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
                }
                piece_start = index + ch.len_utf8();
            }
            position_units += ch.len_utf16();
            position_bytes += ch.len_utf8();
            let _ = offsets;
        }
        if piece_start < text.len() {
            let piece_units = position_units - text[piece_start..].encode_utf16().count();
            self.push_text_run(
                &mut paragraph,
                &text[piece_start..],
                piece_units,
                character_styles,
                smart_fields,
            );
        }
        let _ = position_bytes;
        paragraph
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
        paragraph.runs.push(Run {
            style,
            properties,
            link: self.link_at(unit, smart_fields),
            content: Inline::Text(text.to_string()),
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
                let note = self.footnote(object)?;
                Inline::Footnote(note)
            }
            _ => return None,
        };
        Some(Run {
            style,
            properties,
            link: self.link_at(unit, smart_fields),
            content,
        })
    }

    fn link_at(&self, unit: usize, smart_fields: &[Span]) -> Option<String> {
        let object = covering(smart_fields, unit)?;
        let message = self.graph.object(object)?;
        let view = View::of(message);
        view.string("url_ref").map(str::to_string)
    }

    /// A footnote reference attachment: its storage becomes a note.
    fn footnote(&mut self, attachment: u64) -> Option<usize> {
        let message = self.graph.object(attachment)?;
        let storage = View::of(message).reference("contained_storage")?;
        let storage = self.graph.object(storage)?;
        // A note is its own storage; the body's running style must not
        // leak in or out.
        let outer_style = self.last_paragraph_style.take();
        let mut blocks = self.storage_blocks(View::of(storage));
        self.last_paragraph_style = outer_style;
        // The note's text starts with the mark placeholder; drop it.
        if let Some(Block::Paragraph(first)) = blocks.first_mut()
            && let Some(run) = first.runs.first_mut()
            && let Inline::Text(text) = &mut run.content
        {
            let trimmed = text.trim_start_matches(ATTACHMENT).trim_start().to_string();
            *text = trimmed;
        }
        self.document.footnotes.push(Note { blocks });
        Some(self.document.footnotes.len() - 1)
    }

    fn run_formatting(
        &mut self,
        unit: usize,
        character_styles: &[Span],
    ) -> (Option<StyleId>, RunProperties) {
        match covering(character_styles, unit) {
            Some(object) => self.resolve_character_style(object),
            None => (None, RunProperties::default()),
        }
    }

    // ----- styles -----

    /// Walks the variation chain of a paragraph style object: unnamed
    /// styles become direct properties, the first named one is the style.
    fn resolve_paragraph_style(
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
            let view = View::of(message);
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
                .map(run_properties)
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

    fn resolve_character_style(&mut self, object: u64) -> (Option<StyleId>, RunProperties) {
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
                    .map(run_properties)
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
            let view = View::of(message);
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
                .map(run_properties)
                .unwrap_or_default();
            if let Some(parent) = meta.and_then(|m| m.reference("parent")) {
                style.parent = Some(self.paragraph_style_id(parent));
            }
        }
        self.document.styles.paragraph[id] = style;
        id
    }

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
                .map(run_properties)
                .unwrap_or_default();
            if let Some(parent) = meta.and_then(|m| m.reference("parent")) {
                style.parent = Some(self.character_style_id(parent));
            }
        }
        self.document.styles.character[id] = style;
        id
    }

    /// The list style, unless it is the "no list" style.
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
        outline_level: view.integer("outline_level").map(|level| level as u8),
        background: view.message("fill").and_then(color),
    }
}

/// Converts `TSWP.CharacterStylePropertiesArchive`.
fn run_properties(view: View<'_>) -> RunProperties {
    RunProperties {
        font: view.string("font_name").map(str::to_string),
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
        language: view.string("language").map(str::to_string),
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
/// attribute tables count characters.
struct Utf16Offsets {
    /// (byte offset, unit offset) at every character start.
    table: Vec<(usize, usize)>,
}

impl Utf16Offsets {
    fn new(text: &str) -> Utf16Offsets {
        let mut table = Vec::with_capacity(text.len() + 1);
        let mut units = 0usize;
        for (index, ch) in text.char_indices() {
            table.push((index, units));
            units += ch.len_utf16();
        }
        table.push((text.len(), units));
        Utf16Offsets { table }
    }

    fn units_at(&mut self, byte: usize) -> usize {
        let index = self.table.partition_point(|(offset, _)| *offset < byte);
        self.table.get(index).map_or(0, |(_, units)| *units)
    }
}
