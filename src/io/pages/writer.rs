//! Writes a Pages package from the document model. Apple's object graph is
//! intricate and interdependent, so a real blank document is the scaffolding
//! (see `DOCS/formats/pages.md`): the template carries the stylesheet, theme,
//! section, and settings, and the writer replaces only the body text storage,
//! setting the text and one paragraph-style run per paragraph. Everything the
//! template already holds (the named styles, page setup) is reused.

use std::collections::HashMap;

use super::package::{Entry, Object, ObjectMessage, Package, PackageError, Stream};
use super::schema::SCHEMA;
use crate::document::{
    Block, Color, Document, Id, Inline, ListItem, ListLabel, MediaId, NumberKind, Paragraph,
};
use crate::io::protobuf::schema::MessageRef;
use crate::io::protobuf::tree::{Chain, Entry as TreeEntry, NONE, Node, Tree, TreeError};

/// A blank Pages 12 document with the preview thumbnails stripped: the
/// scaffolding every written document is built on. Made once on a Mac and
/// committed; how, and why a real document rather than a generated graph,
/// is in `DOCS/formats/pages.md`.
const TEMPLATE: &[u8] = include_bytes!("style_template.pages");

const STORAGE_ARCHIVE: u32 = 2001;
const DOCUMENT_ARCHIVE: u32 = 10000;
const PARAGRAPH_STYLE: u32 = 2022;
const CHARACTER_STYLE: u32 = 2021;
const LIST_STYLE: u32 = 2023;
const DRAWABLE_ATTACHMENT: u32 = 2003;
const HYPERLINK_FIELD: u32 = 2032;
const IMAGE_ARCHIVE: u32 = 3005;
const PACKAGE_METADATA: u32 = 11006;
const STYLESHEET: u32 = 401;
const RICH_TEXT_PAYLOAD: u32 = 6218;
/// Style-table keys a rich cell record names: its paragraph (text) style and
/// its cell style, and the number-format-table key every cell shares.
const TEXT_STYLE_KEY: u32 = 1;
const CELL_STYLE_KEY: u32 = 2;
const CELL_STYLE: u32 = 6004;
const TABLE_STYLE: u32 = 6003;
const FORMAT_KEY: u32 = 1;
/// The template's body cell padding (points, every side).
const TEMPLATE_CELL_PADDING: f32 = 4.0;
/// The fixed number of per-column offset slots a tile row allocates, as Pages
/// writes them (a 510-byte `cell_offsets` array of u16).
const TILE_COLUMN_SLOTS: usize = 255;
/// The object-replacement character that stands for an anchored drawable
/// (a table here) in the body text.
const ATTACHMENT: char = '\u{FFFC}';

/// Renders the document model to a `.pages` package.
pub fn write(document: &Document) -> Result<Vec<u8>, PackageError> {
    let mut package = Package::read(TEMPLATE)?;
    let styles = collect_style_names(&package);
    let formats = collect_char_formats(&package);
    let lists = collect_list_styles(&package);
    rebuild_body(&mut package, document, &styles, &formats, &lists)?;
    clear_template_rules(&mut package);
    if let Some(section) = document.sections.first() {
        set_page_setup(&mut package, &section.page)?;
    }
    // Declare every cross-component reference and UUID-map every text storage
    // the rewrite introduced, as Pages requires to load the package.
    reconcile_components(&mut package)?;
    // Pages allocates its own objects (view-state, undo) from the metadata's
    // high-water mark, so it must sit above every object written here.
    let highest = max_identifier(&package);
    set_last_object_identifier(&mut package, highest)?;
    // Written in the current format, so stamped with the writer version the
    // current Pages records; an older stamp makes Pages run its upgrade passes,
    // one of which strips style overrides (a table's cell fills among them).
    set_write_version(&mut package, [4, 1, 0])?;
    // Record the document's origin as a Word import, as Pages' own importer
    // does, rather than the template it was assembled from.
    set_origin(&mut package, "docx");
    // A fresh document identity per output: Pages keeps a local view-state
    // sidecar keyed by the document UUID, and reusing the template's would pair
    // every converted file with another file's stale objects.
    renew_document_identity(&mut package);
    package.write(Vec::new())
}

/// Maps each named paragraph and character style in the template to its
/// object identifier, so the writer can point a paragraph at "Heading" or
/// "Body" by name. The first of a repeated name wins.
fn collect_style_names(package: &Package) -> HashMap<String, u64> {
    let mut names = HashMap::new();
    for entry in &package.entries {
        let Entry::Stream(stream) = entry else {
            continue;
        };
        for object in &stream.objects {
            let Some(message) = object.messages.first() else {
                continue;
            };
            if message.message_type != PARAGRAPH_STYLE && message.message_type != CHARACTER_STYLE {
                continue;
            }
            if let Some(name) = style_name(&stream.tree, message.first) {
                names.entry(name).or_insert(object.identifier);
            }
        }
    }
    names
}

/// Maps each named list style in the template to its object identifier, so a
/// bullet or numbered paragraph can point at "Bullet" or "Numbered" by name,
/// and a plain paragraph at "None". Kept apart from the paragraph and
/// character styles, whose names ("None") would otherwise collide.
fn collect_list_styles(package: &Package) -> HashMap<String, u64> {
    let mut names = HashMap::new();
    for entry in &package.entries {
        let Entry::Stream(stream) = entry else {
            continue;
        };
        for object in &stream.objects {
            let Some(message) = object.messages.first() else {
                continue;
            };
            if message.message_type != LIST_STYLE {
                continue;
            }
            if let Some(name) = style_name(&stream.tree, message.first) {
                names.entry(name).or_insert(object.identifier);
            }
        }
    }
    names
}

/// The template list style a paragraph's list membership maps to. The names
/// tried are the theme's live list styles, which render, in preference to the
/// preset definitions of the same shape, which do not: "Bullets" (then
/// "Bullet") for a marker, "Numbered" or "Lettered" for a number, "None"
/// otherwise. The first name the template has wins.
fn list_style_for(
    document: &Document,
    lists: &HashMap<String, u64>,
    item: Option<&ListItem>,
) -> Option<u64> {
    let candidates: &[&str] = match item {
        None => &["None"],
        Some(item) => {
            let Some(style) = document.styles.list.get(item.style) else {
                return lists.get("None").copied();
            };
            let label = style
                .levels
                .get(item.level as usize)
                .or_else(|| style.levels.first())
                .map(|level| &level.label);
            match label {
                Some(ListLabel::Text(_)) => &["Bullets", "Bullet"],
                Some(ListLabel::Number(format)) => match format.kind {
                    NumberKind::LowerLetter | NumberKind::UpperLetter => &["Lettered"],
                    _ => &["Numbered"],
                },
                _ => &["None"],
            }
        }
    };
    candidates
        .iter()
        .find_map(|name| lists.get(*name))
        .or_else(|| lists.get("None"))
        .copied()
}

/// A style's name, at `super.name`.
fn style_name(tree: &Tree, first: u32) -> Option<String> {
    let base = message_field(tree, first, "super")?;
    Some(str_field(tree, base, "name")?.to_string())
}

/// Replaces the body storage's text, paragraph styles, character styles, and
/// list membership from the model, and rewrites the template's own tables to
/// carry the model's tables (reusing them keeps their calculation-engine
/// registration, which Pages requires and cannot be synthesized).
fn rebuild_body(
    package: &mut Package,
    document: &Document,
    styles: &HashMap<String, u64>,
    formats: &HashMap<Format, u64>,
    lists: &HashMap<String, u64>,
) -> Result<(), PackageError> {
    let body = flatten(document);
    let areas = document
        .sections
        .first()
        .map(|section| page_areas(document, section))
        .unwrap_or_default();
    let boxes = text_boxes(document);
    let mut next_id = max_identifier(package) + 1;
    let mut next_data_id = max_data_id(package) + 1;

    // Synthesise a character style for every direct formatting in the body and
    // in table cells, so both the body storage and rich cells can point at them.
    let stylesheet_id = stylesheet_identifier(package);
    let base_style = base_char_style(package);
    let (all_formats, style_refs) = match (base_style, stylesheet_id) {
        (Some(parent), Some(sheet)) => {
            let needed = body
                .char_marks
                .iter()
                .map(|mark| mark.format)
                .chain(body.tables.iter().flat_map(|table| {
                    table.cells.iter().flat_map(|cell| {
                        cell.marks
                            .iter()
                            .map(|(_, format)| *format)
                            .collect::<Vec<_>>()
                    })
                }))
                .chain(areas.iter().flat_map(|area| {
                    area.content
                        .marks
                        .iter()
                        .map(|(_, format)| *format)
                        .collect::<Vec<_>>()
                }))
                .chain(boxes.iter().flat_map(|text_box| {
                    text_box
                        .content
                        .marks
                        .iter()
                        .map(|(_, format)| *format)
                        .collect::<Vec<_>>()
                }))
                .filter(Format::has_direct);
            synthesize_char_styles(
                package,
                document,
                formats,
                needed,
                parent,
                sheet,
                &mut next_id,
            )?
        }
        _ => (formats.clone(), Vec::new()),
    };

    // The styles a rich cell references for its plain paragraphs.
    let cell_styles = (|| {
        Some(CellStyles {
            stylesheet: stylesheet_id?,
            paragraph: *styles.get("Body")?,
            list: *lists.get("None")?,
            base_char: base_style?,
        })
    })();

    // Rewrite one template table per model table (with rich cells for the
    // formatted ones); each returns the attachment the body anchors it with.
    // A paragraph-style variation for every distinct paragraph formatting, of
    // the style each paragraph sits in (named style in the body, Body in
    // cells, the area's own style in headers and footers).
    let (para_styles, para_refs) = match (stylesheet_id, cell_styles) {
        (Some(sheet), Some(cell)) => {
            let mut needed: Vec<(u64, ParaFormat)> = Vec::new();
            for mark in &body.paragraphs {
                let parent = style_identifier(document, styles, mark.style_name.as_deref());
                needed.push((parent, mark.format));
            }
            for table in &body.tables {
                for content in &table.cells {
                    needed.extend(
                        content
                            .paragraphs
                            .iter()
                            .map(|(_, format)| (cell.paragraph, *format)),
                    );
                }
            }
            for area in &areas {
                let parent = area_paragraph_style(package, area).unwrap_or(cell.paragraph);
                needed.extend(
                    area.content
                        .paragraphs
                        .iter()
                        .map(|(_, format)| (parent, *format)),
                );
            }
            for text_box in &boxes {
                needed.extend(
                    text_box
                        .content
                        .paragraphs
                        .iter()
                        .map(|(_, format)| (cell.paragraph, *format)),
                );
            }
            synthesize_para_styles(package, needed, sheet, &mut next_id)?
        }
        _ => (HashMap::new(), Vec::new()),
    };

    // A list-style variation per Word list the body uses, with its own labels
    // and indents, of the template list style of the same shape.
    let word_lists = match stylesheet_id {
        Some(sheet) => {
            synthesize_list_styles(package, document, &body, lists, sheet, &mut next_id)?
        }
        None => HashMap::new(),
    };

    if let Some(styles) = cell_styles {
        write_page_areas(
            package,
            &areas,
            &all_formats,
            styles,
            &para_styles,
            &mut next_id,
        )?;
        write_text_boxes(
            package,
            &boxes,
            &all_formats,
            styles,
            &para_styles,
            &mut next_id,
        )?;
    }

    // The template carries one table: a document with more gets a clone of
    // the pristine prototype per extra table (cloned before any rewrite).
    let mut template_tables = collect_template_tables(package);
    if let Some(prototype) = template_tables.first().copied() {
        while template_tables.len() < body.tables.len() {
            let attach = clone_template_table(package, &prototype, &mut next_id)?;
            let table = traverse_template_table(package, attach)
                .ok_or_else(|| malformed("cloned table does not resolve"))?;
            template_tables.push(table);
        }
    }
    let mut anchors: Vec<(u32, u64)> = Vec::new();
    let mut fill_styles: CellFillStyles = HashMap::new();
    let mut unbanded: HashMap<u64, u64> = HashMap::new();
    for (mark, table) in body.tables.iter().zip(&template_tables) {
        reuse_table(
            package,
            table,
            mark,
            &all_formats,
            cell_styles,
            &para_styles,
            &mut fill_styles,
            &mut unbanded,
            &mut next_id,
        )?;
        anchors.push((mark.offset, table.attach_id));
    }
    // The template carries one image as a prototype: the first model image
    // reuses it; any others clone it so every image reaches Pages, each with its
    // own objects and data files.
    if let Some(prototype) = collect_template_images(package).first().copied() {
        // Data already written, by digest: Pages keys its data store by digest
        // and aborts on two files with the same one, so a picture used twice
        // shares one data file between its images.
        let mut written: HashMap<Vec<u8>, (u64, Option<u64>)> = HashMap::new();
        for (index, mark) in body.images.iter().enumerate() {
            let bytes = document.media[mark.media].bytes.clone();
            let digest = sha1(&bytes).to_vec();
            let attach_id = if index == 0 {
                reuse_image(package, &prototype, mark, &bytes)?;
                written.insert(digest, (prototype.data_id, prototype.thumb_id));
                prototype.attach_id
            } else {
                let shared = written.get(&digest).copied();
                let (attach, full, thumb) = clone_image(
                    package,
                    &prototype,
                    mark,
                    &bytes,
                    shared,
                    &mut next_id,
                    &mut next_data_id,
                )?;
                written.entry(digest).or_insert((full, thumb));
                attach
            };
            anchors.push((mark.offset, attach_id));
        }
    }
    // Attachments anchor by ascending character offset in one table.
    anchors.sort_by_key(|(offset, _)| *offset);

    let stream = document_stream(package)?;
    let body_id = body_storage_identifier(stream)?;
    let Some(index) = stream
        .objects
        .iter()
        .position(|object| object.identifier == body_id)
    else {
        return Err(malformed("body storage object is missing"));
    };
    let old_first = match stream.objects[index].messages.first() {
        Some(message) if message.message_type == STORAGE_ARCHIVE => message.first,
        _ => return Err(malformed("body storage is not a StorageArchive")),
    };

    // Each link range becomes a hyperlink field object the body anchors by a
    // smart-field attribute table (like the character styles, offset-keyed).
    let (link_objects, smart_entries) =
        build_link_objects(&mut stream.tree, document, &body.link_marks, &mut next_id)?;

    let (new_first, list_refs) = build_storage(
        &mut stream.tree,
        old_first,
        document,
        &body,
        styles,
        &all_formats,
        lists,
        &word_lists,
        &anchors,
        &smart_entries,
        &para_styles,
    )?;
    stream.objects[index].messages[0].first = new_first;
    let info = stream.objects[index].info;
    let mut references: Vec<u64> = list_refs;
    references.extend(anchors.iter().map(|(_, id)| *id));
    references.extend(link_objects.iter().map(|object| object.identifier));
    references.extend(style_refs);
    references.extend(para_refs);
    stream.objects.extend(link_objects);
    add_object_references(&mut stream.tree, info, &references)?;
    Ok(())
}

/// The highest object identifier any stream uses; a new object takes the
/// next one, since identifiers are unique across the whole package.
fn max_identifier(package: &Package) -> u64 {
    let mut max = 0;
    for entry in &package.entries {
        if let Entry::Stream(stream) = entry {
            for object in &stream.objects {
                max = max.max(object.identifier);
            }
        }
    }
    max
}

/// An offset-keyed object-attribute table: at each offset, the object that
/// applies there, or `None` for a gap.
type AttrEntries = Vec<(u32, Option<u64>)>;

/// Builds a `TSWP.HyperlinkFieldArchive` object per contiguous link range,
/// returning the new objects (for the document stream) and the smart-field
/// entries that anchor them: `(offset, Some(object))` where a link starts,
/// `(offset, None)` where the text is unlinked again.
fn build_link_objects(
    tree: &mut Tree,
    document: &Document,
    marks: &[LinkMark],
    next_id: &mut u64,
) -> Result<(Vec<Object>, AttrEntries), PackageError> {
    let mut objects = Vec::new();
    let mut entries = Vec::new();
    for mark in marks {
        match mark.link.and_then(|id| document.links.get(id as usize)) {
            Some(url) => {
                let identifier = *next_id;
                *next_id += 1;
                objects.push(build_hyperlink_object(tree, identifier, url)?);
                entries.push((mark.offset, Some(identifier)));
            }
            None => entries.push((mark.offset, None)),
        }
    }
    Ok((objects, entries))
}

/// One hyperlink field object: the target URL, with a per-field UUID the way
/// Pages tags each link occurrence. Its `ArchiveInfo` header carries the
/// registry type so the reader (and Pages) decode it as a hyperlink.
fn build_hyperlink_object(
    tree: &mut Tree,
    identifier: u64,
    url: &str,
) -> Result<Object, PackageError> {
    let url = &encode_url(url);
    let hyperlink = message_ref("TSWP.HyperlinkFieldArchive")?;
    let smart = message_ref("TSWP.SmartFieldArchive")?;
    let mut base = Chain::new();
    let uuid = link_uuid(identifier);
    let uuid_span = tree.push_bytes(uuid.as_bytes()).map_err(tree_error)?;
    push_field(
        tree,
        &mut base,
        smart,
        "text_attribute_uuid_string",
        Node::Str(uuid_span),
    )?;
    let mut message = Chain::new();
    push_field(
        tree,
        &mut message,
        hyperlink,
        "super",
        Node::Message(base.first),
    )?;
    let url_span = tree.push_bytes(url.as_bytes()).map_err(tree_error)?;
    push_field(
        tree,
        &mut message,
        hyperlink,
        "url_ref",
        Node::Str(url_span),
    )?;
    let info = build_archive_info(tree, identifier, HYPERLINK_FIELD)?;
    Ok(Object {
        identifier,
        info,
        messages: vec![ObjectMessage {
            message_type: HYPERLINK_FIELD,
            first: message.first,
        }],
    })
}

/// A UUID-shaped string, unique per object identifier. Pages tags each link
/// occurrence with one; the exact value is not referenced elsewhere.
fn link_uuid(identifier: u64) -> String {
    format!("00000000-0000-4000-8000-{identifier:012X}")
}

/// Builds a `TSP.ArchiveInfo` header for a new single-message object: its
/// identifier and one `MessageInfo` giving the registry type and the version
/// triple Pages writes for text objects. `MessageInfo.length` is filled in by
/// the encoder from the message's size, but the field must be present for it
/// to patch.
fn build_archive_info(
    tree: &mut Tree,
    identifier: u64,
    message_type: u32,
) -> Result<u32, PackageError> {
    let archive = message_ref("TSP.ArchiveInfo")?;
    let info = message_ref("TSP.MessageInfo")?;
    let mut header = Chain::new();
    push_field(
        tree,
        &mut header,
        info,
        "type",
        Node::Uint(u64::from(message_type)),
    )?;
    for version in [1u64, 0, 5] {
        push_field(tree, &mut header, info, "version", Node::Uint(version))?;
    }
    push_field(tree, &mut header, info, "length", Node::Uint(0))?;
    let mut chain = Chain::new();
    push_field(
        tree,
        &mut chain,
        archive,
        "identifier",
        Node::Uint(identifier),
    )?;
    push_field(
        tree,
        &mut chain,
        archive,
        "message_infos",
        Node::Message(header.first),
    )?;
    Ok(chain.first)
}

/// Builds a `TSP.Color` message for an RGB colour (channels are 0..1 floats).
fn build_color(tree: &mut Tree, color: Color) -> Result<u32, PackageError> {
    let message = message_ref("TSP.Color")?;
    let mut chain = Chain::new();
    push_field(tree, &mut chain, message, "model", Node::Uint(1))?;
    push_field(
        tree,
        &mut chain,
        message,
        "r",
        Node::Float(f32::from(color.red) / 255.0),
    )?;
    push_field(
        tree,
        &mut chain,
        message,
        "g",
        Node::Float(f32::from(color.green) / 255.0),
    )?;
    push_field(
        tree,
        &mut chain,
        message,
        "b",
        Node::Float(f32::from(color.blue) / 255.0),
    )?;
    push_field(tree, &mut chain, message, "a", Node::Float(1.0))?;
    push_field(tree, &mut chain, message, "rgbspace", Node::Uint(1))?;
    Ok(chain.first)
}

/// Synthesises a character style carrying a run's direct formatting (bold,
/// italic, size, font, underline, colour) as a variation of the theme's base
/// style, returning its message and header chains. The object is registered in
/// the document stream by the caller under `identifier`.
fn build_char_style(
    tree: &mut Tree,
    identifier: u64,
    format: Format,
    document: &Document,
    parent: u64,
    stylesheet: u64,
) -> Result<(u32, u32), PackageError> {
    let style = message_ref("TSWP.CharacterStyleArchive")?;
    let base = message_ref("TSS.StyleArchive")?;
    let properties = message_ref("TSWP.CharacterStylePropertiesArchive")?;

    let mut super_chain = Chain::new();
    push_field(
        tree,
        &mut super_chain,
        base,
        "parent",
        Node::Reference(parent),
    )?;
    push_field(
        tree,
        &mut super_chain,
        base,
        "is_variation",
        Node::Bool(true),
    )?;
    push_field(
        tree,
        &mut super_chain,
        base,
        "stylesheet",
        Node::Reference(stylesheet),
    )?;

    let mut props = Chain::new();
    let mut count = 0u64;
    if format.bold {
        push_field(tree, &mut props, properties, "bold", Node::Bool(true))?;
        count += 1;
    }
    if format.italic {
        push_field(tree, &mut props, properties, "italic", Node::Bool(true))?;
        count += 1;
    }
    if let Some(half_points) = format.size {
        push_field(
            tree,
            &mut props,
            properties,
            "font_size",
            Node::Float(f32::from(half_points) / 2.0),
        )?;
        count += 1;
    }
    if let Some(font) = format.font {
        let span = tree
            .push_bytes(document.string(font).as_bytes())
            .map_err(tree_error)?;
        push_field(tree, &mut props, properties, "font_name", Node::Str(span))?;
        count += 1;
    }
    if format.underline {
        push_field(tree, &mut props, properties, "underline", Node::Uint(1))?;
        count += 1;
    }
    if let Some(color) = format.color {
        let color_first = build_color(tree, color)?;
        push_field(
            tree,
            &mut props,
            properties,
            "font_color",
            Node::Message(color_first),
        )?;
        // Pages renders the text colour from the character fill, not
        // `font_color` alone, so the same colour goes in `tsd_fill.color`.
        let fill_kind = properties
            .field_named("tsd_fill")
            .ok_or_else(|| malformed("char properties have no tsd_fill"))?
            .kind;
        let fill = message_of(fill_kind)?;
        let fill_color = build_color(tree, color)?;
        let mut fill_chain = Chain::new();
        push_field(
            tree,
            &mut fill_chain,
            fill,
            "color",
            Node::Message(fill_color),
        )?;
        push_field(
            tree,
            &mut props,
            properties,
            "tsd_fill",
            Node::Message(fill_chain.first),
        )?;
        count += 1;
    }

    let mut chain = Chain::new();
    push_field(
        tree,
        &mut chain,
        style,
        "super",
        Node::Message(super_chain.first),
    )?;
    push_field(tree, &mut chain, style, "override_count", Node::Uint(count))?;
    push_field(
        tree,
        &mut chain,
        style,
        "char_properties",
        Node::Message(props.first),
    )?;

    let info = build_archive_info(tree, identifier, CHARACTER_STYLE)?;
    add_object_references(tree, info, &[parent, stylesheet])?;
    Ok((chain.first, info))
}

/// The document's stylesheet object identifier (`TSS.StylesheetArchive`).
fn stylesheet_identifier(package: &Package) -> Option<u64> {
    for entry in &package.entries {
        let Entry::Stream(stream) = entry else {
            continue;
        };
        for object in &stream.objects {
            if first_type(object) == Some(STYLESHEET) {
                return Some(object.identifier);
            }
        }
    }
    None
}

/// The theme's base ("None") character style, used as the parent of a
/// synthesised direct-formatting variation.
fn base_char_style(package: &Package) -> Option<u64> {
    for entry in &package.entries {
        let Entry::Stream(stream) = entry else {
            continue;
        };
        for object in &stream.objects {
            if first_type(object) == Some(CHARACTER_STYLE)
                && let Some(first) = object.messages.first().map(|message| message.first)
                && style_name(&stream.tree, first).as_deref() == Some("None")
            {
                return Some(object.identifier);
            }
        }
    }
    None
}

/// A table the template carries, by the identifiers of the objects a rewrite
/// touches: the drawable attachment the body anchors, the model whose sizes
/// change, and the tile, string table, and header buckets holding the cells.
#[derive(Clone, Copy)]
struct TemplateTable {
    attach_id: u64,
    info_id: u64,
    model_id: u64,
    tile_id: u64,
    string_id: u64,
    rich_text_id: u64,
    style_id: u64,
    cell_style_id: u64,
    format_id: u64,
    col_bucket_id: u64,
    row_bucket_id: u64,
}

/// Every table the template carries, found from its drawable attachments.
fn collect_template_tables(package: &Package) -> Vec<TemplateTable> {
    let mut tables = Vec::new();
    for entry in &package.entries {
        let Entry::Stream(stream) = entry else {
            continue;
        };
        for object in &stream.objects {
            if first_type(object) == Some(DRAWABLE_ATTACHMENT)
                && let Some(table) = traverse_template_table(package, object.identifier)
            {
                tables.push(table);
            }
        }
    }
    tables
}

/// Follows a drawable attachment to the table objects a rewrite needs.
fn traverse_template_table(package: &Package, attach_id: u64) -> Option<TemplateTable> {
    let info_id = object_reference(package, attach_id, "drawable")?;
    let model_id = object_reference(package, info_id, "tableModel")?;
    let (tree, model_first) = object_message(package, model_id)?;
    let store = message_field(tree, model_first, "base_data_store")?;
    let string_id = reference_of(field_value(tree, store, "stringTable"))?;
    let rich_text_id = reference_of(field_value(tree, store, "rich_text_table"))?;
    let style_id = reference_of(field_value(tree, store, "styleTable"))?;
    let format_id = reference_of(field_value(tree, store, "format_table"))?;
    let cell_style_id = reference_of(field_value(tree, model_first, "body_cell_style"))?;
    let col_bucket_id = reference_of(field_value(tree, store, "columnHeaders"))?;
    let tiles = message_field(tree, store, "tiles")?;
    let one_tile = message_field(tree, tiles, "tiles")?;
    let tile_id = reference_of(field_value(tree, one_tile, "tile"))?;
    let row_headers = message_field(tree, store, "rowHeaders")?;
    let row_bucket_id = reference_of(field_value(tree, row_headers, "buckets"))?;
    Some(TemplateTable {
        attach_id,
        info_id,
        model_id,
        tile_id,
        string_id,
        rich_text_id,
        style_id,
        cell_style_id,
        format_id,
        col_bucket_id,
        row_bucket_id,
    })
}

fn reference_of(value: Option<Node>) -> Option<u64> {
    match value? {
        Node::Reference(identifier) => Some(identifier),
        _ => None,
    }
}

/// The tree and message-chain start of the object with `id`, in whatever
/// stream holds it.
fn object_message(package: &Package, id: u64) -> Option<(&Tree, u32)> {
    for entry in &package.entries {
        if let Entry::Stream(stream) = entry
            && let Some(object) = stream.objects.iter().find(|object| object.identifier == id)
        {
            return Some((&stream.tree, object.messages.first()?.first));
        }
    }
    None
}

/// The object with `id` in whatever stream holds it.
fn package_object(package: &Package, id: u64) -> Option<&Object> {
    for entry in &package.entries {
        if let Entry::Stream(stream) = entry
            && let Some(object) = stream.objects.iter().find(|object| object.identifier == id)
        {
            return Some(object);
        }
    }
    None
}

/// A reference field of an object anywhere in the package.
fn object_reference(package: &Package, id: u64, field: &str) -> Option<u64> {
    let (tree, first) = object_message(package, id)?;
    reference_of(field_value(tree, first, field))
}

/// Rewrites the template table's tile, string table, header buckets, and
/// model sizes to carry the model table's cells.
#[allow(clippy::too_many_arguments)]
fn reuse_table(
    package: &mut Package,
    table: &TemplateTable,
    mark: &TableMark,
    formats: &HashMap<Format, u64>,
    cell_styles: Option<CellStyles>,
    paras: &ParaStyles,
    fill_styles: &mut CellFillStyles,
    unbanded: &mut HashMap<u64, u64>,
    next_id: &mut u64,
) -> Result<(), PackageError> {
    // Word tables do not band their rows as the template's table style does:
    // each table takes a variation of its style with banding off.
    if let (Some(styles), Some(parent)) = (
        cell_styles,
        object_reference(package, table.model_id, "table_style"),
    ) {
        let variation = match unbanded.get(&parent) {
            Some(id) => *id,
            None => {
                let id = create_unbanded_table_style(package, parent, styles.stylesheet, next_id)?;
                unbanded.insert(parent, id);
                id
            }
        };
        let stream = stream_containing(package, table.model_id)?;
        if let Some(object) = stream
            .objects
            .iter()
            .find(|object| object.identifier == table.model_id)
        {
            let (first, info) = (object.messages[0].first, object.info);
            if let Some(index) = field_entry(&stream.tree, first, "table_style") {
                stream.tree.entries[index as usize].value = Node::Reference(variation);
            }
            add_object_references(&mut stream.tree, info, &[variation])?;
        }
    }
    // Every cell becomes a rich-text cell (as Pages itself writes them): its own
    // storage and payload objects, an entry in the table's rich-text list, and a
    // kind-9 record in the tile that names the cell's styles. This matches the
    // shape Pages lays out and keeps the string table empty.
    let mut rich: HashMap<usize, u32> = HashMap::new();
    let mut cell_style_keys: HashMap<usize, u32> = HashMap::new();
    if let Some(styles) = cell_styles {
        let mut entries: Vec<(u32, u64)> = Vec::new();
        let mut new_objects: Vec<Object> = Vec::new();
        {
            // Build the payloads and cell storages into the rich-text list's own
            // stream: Pages loads that stream as one component and aborts if the
            // payloads it references live elsewhere (e.g. the document stream).
            let stream = stream_containing(package, table.rich_text_id)?;
            let covered = mark.covered();
            for (index, cell) in mark.cells.iter().enumerate() {
                // An empty cell has no text storage: Pages writes it as a bare
                // record naming its styles, and an empty storage is "repaired".
                if covered.contains(&index) || cell.is_empty() {
                    continue;
                }
                let storage_id = *next_id;
                let payload_id = *next_id + 1;
                *next_id += 2;
                let attachments = create_number_attachments(stream, &cell.fields, next_id)?;
                let (message, refs) = build_text_storage(
                    &mut stream.tree,
                    cell,
                    formats,
                    styles,
                    Some(5),
                    &attachments,
                    paras,
                )?;
                let info = build_archive_info(&mut stream.tree, storage_id, STORAGE_ARCHIVE)?;
                add_object_references(&mut stream.tree, info, &refs)?;
                new_objects.push(Object {
                    identifier: storage_id,
                    info,
                    messages: vec![ObjectMessage {
                        message_type: STORAGE_ARCHIVE,
                        first: message,
                    }],
                });
                let payload = build_rich_payload(&mut stream.tree, storage_id)?;
                let payload_info =
                    build_archive_info(&mut stream.tree, payload_id, RICH_TEXT_PAYLOAD)?;
                add_object_references(&mut stream.tree, payload_info, &[storage_id])?;
                new_objects.push(Object {
                    identifier: payload_id,
                    info: payload_info,
                    messages: vec![ObjectMessage {
                        message_type: RICH_TEXT_PAYLOAD,
                        first: payload,
                    }],
                });
                let key = entries.len() as u32 + 1;
                rich.insert(index, key);
                entries.push((key, payload_id));
            }
            stream.objects.extend(new_objects);
        }
        if !entries.is_empty() {
            let payloads: Vec<u64> = entries.iter().map(|(_, id)| *id).collect();
            rewrite_object(package, table.rich_text_id, |tree| {
                build_rich_text_list(tree, &entries)
            })?;
            add_object_refs(package, table.rich_text_id, &payloads)?;
        }
        // The style table each rich cell record points at: key 1 the cell's
        // paragraph (text) style, key 2 its cell style. Pages needs a cell to
        // resolve both to lay it out.
        // Each cell's own cell style, as Pages writes a Word table: a variation
        // of the body cell style that sets its fill (empty for none, which also
        // switches off the table style's banded rows).
        // Each cell's look: its fill and its vertical alignment.
        let looks: Vec<CellLook> = mark
            .backgrounds
            .iter()
            .zip(&mark.alignments)
            .map(|(fill, alignment)| (*fill, *alignment))
            .collect();
        let mut distinct = looks.clone();
        distinct.sort_by_key(|(fill, alignment)| {
            (
                fill.map(|color| (color.red, color.green, color.blue)),
                *alignment,
            )
        });
        distinct.dedup();
        // The cells' padding (left, top, right, bottom): the source's margins,
        // else the template's own.
        let padding = mark
            .cell_margins
            .map_or([TEMPLATE_CELL_PADDING; 4], |margins| {
                [margins.left, margins.top, margins.right, margins.bottom]
            });
        let padding_key = padding.map(f32::to_bits);
        // A plain cell with the template's padding keeps the table's own body
        // cell style.
        if padding == [TEMPLATE_CELL_PADDING; 4] {
            for alignment in [ALIGN_UNSTATED, 0] {
                fill_styles.insert(
                    (table.cell_style_id, (None, alignment), padding_key),
                    table.cell_style_id,
                );
            }
        }
        // Shared across tables: Pages merges identical variations on load, which
        // would leave a second table's copy dangling.
        let missing: Vec<CellLook> = distinct
            .iter()
            .filter(|look| !fill_styles.contains_key(&(table.cell_style_id, **look, padding_key)))
            .copied()
            .collect();
        if !missing.is_empty() {
            let created = create_cell_styles(
                package,
                table.cell_style_id,
                &missing,
                padding,
                styles.stylesheet,
                next_id,
            )?;
            for (look, id) in created {
                fill_styles.insert((table.cell_style_id, look, padding_key), id);
            }
        }
        // Refcounts are the number of cell records naming each key: Pages
        // decrements them as it restyles cells, and one that runs out leaves
        // the table's other cells with dangling keys.
        let covered = mark.covered();
        let cells = mark.rows * mark.columns;
        let mut style_entries: Vec<(u32, u64, u64)> =
            vec![(TEXT_STYLE_KEY, styles.paragraph, cells as u64)];
        let mut look_keys: HashMap<CellLook, u32> = HashMap::new();
        for look in &distinct {
            let key = style_entries.len() as u32 + 1;
            let users = looks
                .iter()
                .enumerate()
                .filter(|(index, other)| *other == look && !covered.contains(index))
                .count() as u64;
            let variation = fill_styles[&(table.cell_style_id, *look, padding_key)];
            style_entries.push((key, variation, users.max(1)));
            look_keys.insert(*look, key);
        }
        for (index, look) in looks.iter().enumerate() {
            cell_style_keys.insert(index, look_keys[look]);
        }
        let references: Vec<u64> = style_entries.iter().map(|(_, id, _)| *id).collect();
        rewrite_object(package, table.style_id, |tree| {
            build_style_list(tree, &style_entries)
        })?;
        add_object_refs(package, table.style_id, &references)?;
        let rich_cells = rich.len() as u64;
        set_list_refcount(
            package,
            table.format_id,
            u64::from(FORMAT_KEY),
            rich_cells.max(1),
        )?;
    }

    let styled = cell_styles.is_some();
    rewrite_object(package, table.tile_id, |tree| {
        build_tile(tree, mark, &rich, &cell_style_keys, styled)
    })?;
    // When cells are rich their text lives in their own storages, so the string
    // table stays empty (as Pages writes it); otherwise plain cells use it.
    let string_cells: &[CellContent] = if styled { &[] } else { &mark.cells };
    rewrite_object(package, table.string_id, |tree| {
        build_string_list(tree, string_cells)
    })?;
    let rows = mark.rows as u64;
    rewrite_object(package, table.col_bucket_id, |tree| {
        build_header_bucket(tree, &mark.widths, rows)
    })?;
    let heights: Vec<f32> = mark
        .heights
        .iter()
        .map(|height| {
            if *height > 0.0 {
                *height
            } else {
                DEFAULT_ROW_HEIGHT
            }
        })
        .collect();
    let columns = mark.columns as u64;
    rewrite_object(package, table.row_bucket_id, |tree| {
        build_header_bucket(tree, &heights, columns)
    })?;
    // Resize the column/row UID map, which Pages sizes the grid from, so the
    // table has exactly the model's rows and columns and no empty extras.
    if let Some(uid_map_id) = object_reference(package, table.model_id, "base_column_row_uids") {
        let (cols, rows) = (mark.columns, mark.rows);
        rewrite_object(package, uid_map_id, |tree| {
            build_column_row_uids(tree, cols, rows, table.model_id)
        })?;
    }
    // Resize the stroke sidecar's grid: it carries the cell-border counts, and
    // Pages lays strokes out against them before the cells, so a stale count
    // (the template's) reads past the grid and aborts.
    if let Some(sidecar_id) = object_reference(package, table.model_id, "stroke_sidecar") {
        let (cols, rows) = (mark.columns as u64, mark.rows as u64);
        // The table's own lines, as the stroke layers Pages keeps for a Word
        // table: per column its left and right edge, per row its top and bottom.
        let layers = match mark.borders {
            Some(borders) => build_stroke_layers(package, sidecar_id, mark, borders, next_id)?,
            None => StrokeLayers::default(),
        };
        rewrite_object_with(package, sidecar_id, |tree, old_first| {
            rebuild_stroke_sidecar(tree, old_first, cols, rows, &layers)
        })?;
        let references: Vec<u64> = layers.all().collect();
        add_object_refs(package, sidecar_id, &references)?;
    }
    // The drawable frame Pages lays the table into: sized to the table itself,
    // or it squeezes the columns and clips the rows to the template's frame.
    let width: f32 = mark.widths.iter().sum();
    let height: f32 = heights.iter().sum();
    set_table_frame(package, table.info_id, width, height)?;
    update_model_dims(package, table.model_id, mark)?;
    set_owner_table_ranges(package, table, mark)?;
    if !mark.merges.is_empty() {
        write_merges(package, table, &mark.merges, next_id)?;
    }
    Ok(())
}

/// Sets a table drawable's frame size (`super.geometry.size`).
fn set_table_frame(
    package: &mut Package,
    info_id: u64,
    width: f32,
    height: f32,
) -> Result<(), PackageError> {
    let stream = stream_containing(package, info_id)?;
    let first = stream
        .objects
        .iter()
        .find(|object| object.identifier == info_id)
        .and_then(|object| object.messages.first())
        .map(|message| message.first)
        .ok_or_else(|| malformed("table info is missing"))?;
    let size = message_field(&stream.tree, first, "super")
        .and_then(|drawable| message_field(&stream.tree, drawable, "geometry"))
        .and_then(|geometry| message_field(&stream.tree, geometry, "size"))
        .ok_or_else(|| malformed("table info has no geometry size"))?;
    set_field_float(&mut stream.tree, size, "width", width);
    set_field_float(&mut stream.tree, size, "height", height);
    Ok(())
}

/// Resizes a `TST.StrokeSidecarArchive` to `columns` by `rows`. The grid counts
/// and any per-column/row stroke-layer arrays are sized to the grid, so the
/// stale ones are dropped (leaving default cell-style borders) and the counts
/// reset; every other field is kept.
fn rebuild_stroke_sidecar(
    tree: &mut Tree,
    old_first: u32,
    columns: u64,
    rows: u64,
    layers: &StrokeLayers,
) -> Result<u32, PackageError> {
    let sidecar = message_ref("TST.StrokeSidecarArchive")?;
    let dropped = [
        "column_count",
        "row_count",
        "left_column_stroke_layers",
        "right_column_stroke_layers",
        "top_row_stroke_layers",
        "bottom_row_stroke_layers",
    ];
    let mut kept: Vec<(u32, Node)> = Vec::new();
    for (_, entry) in tree.chain(old_first) {
        let name = tree.field(entry).map(|field| field.name);
        if name.is_some_and(|name| dropped.contains(&name)) {
            continue;
        }
        kept.push((entry.number, entry.value));
    }
    let mut chain = Chain::new();
    for (number, value) in kept {
        let slot = sidecar
            .slot(number)
            .ok_or_else(|| malformed("sidecar field without a schema slot"))?;
        let field = sidecar
            .field_at(slot)
            .ok_or_else(|| malformed("sidecar field slot out of range"))?;
        tree.push_known(&mut chain, sidecar, slot, field, number, value)
            .map_err(tree_error)?;
    }
    push_field(
        tree,
        &mut chain,
        sidecar,
        "column_count",
        Node::Uint(columns),
    )?;
    push_field(tree, &mut chain, sidecar, "row_count", Node::Uint(rows))?;
    for (name, ids) in [
        ("left_column_stroke_layers", &layers.left),
        ("right_column_stroke_layers", &layers.right),
        ("top_row_stroke_layers", &layers.top),
        ("bottom_row_stroke_layers", &layers.bottom),
    ] {
        for id in ids {
            push_field(tree, &mut chain, sidecar, name, Node::Reference(*id))?;
        }
    }
    Ok(chain.first)
}

/// A fresh column/row UID map: a distinct UUID per column and row, unique to
/// this table (seeded by it — the calculation engine resolves ranges through
/// these, so two tables must never share one), kept sorted by value with the
/// index<->UID permutations Pages reads, so Pages sizes the grid to exactly
/// `columns` by `rows`.
fn build_column_row_uids(
    tree: &mut Tree,
    columns: usize,
    rows: usize,
    seed: u64,
) -> Result<u32, PackageError> {
    let map = message_ref("TST.ColumnRowUIDMapArchive")?;
    let uuid = message_ref("TSP.UUID")?;
    let mut chain = Chain::new();
    for (count, salt, names) in [
        (
            columns,
            0x436F_6C75_6D6E_0000u64,
            [
                "sorted_column_uids",
                "column_index_for_uid",
                "column_uid_for_index",
            ],
        ),
        (
            rows,
            0x526F_7773_0000_0000u64,
            ["sorted_row_uids", "row_index_for_uid", "row_uid_for_index"],
        ),
    ] {
        // (upper, lower, index) per column or row, sorted by UUID value.
        let mut uids: Vec<(u64, u64, usize)> = (0..count)
            .map(|index| {
                let (lower, upper) =
                    object_uuid(seed.wrapping_mul(0x1_0000_0001) ^ salt ^ index as u64);
                (upper, lower, index)
            })
            .collect();
        uids.sort();
        let mut position = vec![0usize; count];
        for (sorted, (_, _, index)) in uids.iter().enumerate() {
            position[*index] = sorted;
        }
        for (upper, lower, _) in &uids {
            let mut value = Chain::new();
            push_field(tree, &mut value, uuid, "lower", Node::Uint(*lower))?;
            push_field(tree, &mut value, uuid, "upper", Node::Uint(*upper))?;
            push_field(tree, &mut chain, map, names[0], Node::Message(value.first))?;
        }
        for (_, _, index) in &uids {
            push_field(tree, &mut chain, map, names[1], Node::Uint(*index as u64))?;
        }
        for sorted in &position {
            push_field(tree, &mut chain, map, names[2], Node::Uint(*sorted as u64))?;
        }
    }
    Ok(chain.first)
}

/// Replaces the message chain of the object with `id`, wherever it lives, with
/// one the builder produces in that stream's tree.
fn rewrite_object(
    package: &mut Package,
    id: u64,
    builder: impl FnOnce(&mut Tree) -> Result<u32, PackageError>,
) -> Result<(), PackageError> {
    for entry in &mut package.entries {
        if let Entry::Stream(stream) = entry
            && let Some(position) = stream
                .objects
                .iter()
                .position(|object| object.identifier == id)
        {
            let new_first = builder(&mut stream.tree)?;
            stream.objects[position].messages[0].first = new_first;
            return Ok(());
        }
    }
    Err(malformed("table object to rewrite is missing"))
}

/// Updates a table model's row, column, and header counts and default sizes,
/// keeping every other field (styles, data store, calc-engine references).
fn update_model_dims(
    package: &mut Package,
    model_id: u64,
    mark: &TableMark,
) -> Result<(), PackageError> {
    for entry in &mut package.entries {
        if let Entry::Stream(stream) = entry
            && let Some(position) = stream
                .objects
                .iter()
                .position(|object| object.identifier == model_id)
        {
            let old_first = stream.objects[position].messages[0].first;
            let new_first = rebuild_model_dims(&mut stream.tree, mark, old_first)?;
            stream.objects[position].messages[0].first = new_first;
            return Ok(());
        }
    }
    Err(malformed("table model to rewrite is missing"))
}

/// An image the template carries, by the identifiers a rewrite touches: the
/// drawable attachment the body anchors, the image archive whose size and
/// bytes change, and the data references naming its full and thumbnail files.
#[derive(Clone, Copy)]
struct TemplateImage {
    attach_id: u64,
    image_id: u64,
    data_id: u64,
    thumb_id: Option<u64>,
}

/// Every image the template carries, found from its drawable attachments (the
/// ones whose drawable is an image, not a table).
fn collect_template_images(package: &Package) -> Vec<TemplateImage> {
    let mut images = Vec::new();
    for entry in &package.entries {
        let Entry::Stream(stream) = entry else {
            continue;
        };
        for object in &stream.objects {
            if first_type(object) == Some(DRAWABLE_ATTACHMENT)
                && let Some(image) = traverse_template_image(package, object.identifier)
            {
                images.push(image);
            }
        }
    }
    images
}

/// Follows a drawable attachment to an image and its data references, or
/// `None` if the attachment's drawable is not an image.
fn traverse_template_image(package: &Package, attach_id: u64) -> Option<TemplateImage> {
    let image_id = object_reference(package, attach_id, "drawable")?;
    let image_object = package_object(package, image_id)?;
    if first_type(image_object) != Some(IMAGE_ARCHIVE) {
        return None;
    }
    let (tree, image_first) = object_message(package, image_id)?;
    let data = message_field(tree, image_first, "data")?;
    let data_id = match field_value(tree, data, "identifier")? {
        Node::Uint(id) => id,
        _ => return None,
    };
    let thumb_id = message_field(tree, image_first, "thumbnailData")
        .and_then(|thumb| field_value(tree, thumb, "identifier"))
        .and_then(|value| match value {
            Node::Uint(id) => Some(id),
            _ => None,
        });
    Some(TemplateImage {
        attach_id,
        image_id,
        data_id,
        thumb_id,
    })
}

/// Rewrites the template image to carry the model image: its bytes replace the
/// template's data files, its size replaces the image archive's, and the
/// attachment is made inline so it anchors at the body's `U+FFFC`.
fn reuse_image(
    package: &mut Package,
    image: &TemplateImage,
    mark: &ImageMark,
    bytes: &[u8],
) -> Result<(), PackageError> {
    // The display size: the model's if it gave one, else the image's own
    // pixels fitted to the body width; the natural size is the pixel size.
    let pixels = image_dimensions(bytes);
    let (width, height) = display_size(mark, pixels);
    let (natural_w, natural_h) = pixels.map_or((width, height), |(w, h)| (w as f32, h as f32));

    rewrite_object_with(package, image.image_id, |tree, old_first| {
        rebuild_image(tree, old_first, width, height, natural_w, natural_h, mark)
    })?;
    // Make the attachment inline: an inline drawable carries no offsets (Pages
    // and the reader both read the missing offset as "in the text line").
    rewrite_object_with(package, image.attach_id, |tree, old_first| {
        rebuild_inline_attachment(tree, old_first)
    })?;

    // Replace the full picture's bytes and recompute its digest and size (Pages
    // keys its data store by the digest, and aborts loading if a data file's
    // recorded size or digest does not match its bytes).
    if let Some(name) = data_file_name(package, image.data_id) {
        replace_data_file(package, &name, bytes);
    }
    update_data_metadata(package, image.data_id, &sha1(bytes), natural_w, natural_h)?;
    // The thumbnail gets the same picture but with distinct bytes, so its digest
    // differs from the full picture's: the data store keys on the digest and
    // aborts when two data files collide on one.
    if let Some(thumb_id) = image.thumb_id {
        let thumbnail = distinct_thumbnail(bytes);
        if let Some(name) = data_file_name(package, thumb_id) {
            replace_data_file(package, &name, &thumbnail);
        }
        update_data_metadata(package, thumb_id, &sha1(&thumbnail), natural_w, natural_h)?;
    }
    Ok(())
}

/// The same image with distinct bytes (so its digest differs), by adding a
/// metadata chunk that decoders ignore: a `tEXt` chunk for PNG, a comment
/// segment for JPEG, else a trailing byte. Used for the reused thumbnail.
fn distinct_thumbnail(bytes: &[u8]) -> Vec<u8> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G'])
        && let Some(iend) = bytes.windows(4).rposition(|window| window == b"IEND")
        && iend >= 4
    {
        // A tEXt chunk: length (of type+data minus the 4-byte type), the type
        // and data, then the CRC over type and data, inserted before IEND.
        let type_and_data: &[u8] = b"tEXtsublime\0thumbnail";
        let length = (type_and_data.len() - 4) as u32;
        let mut out = Vec::with_capacity(bytes.len() + type_and_data.len() + 8);
        out.extend_from_slice(&bytes[..iend - 4]);
        out.extend_from_slice(&length.to_be_bytes());
        out.extend_from_slice(type_and_data);
        out.extend_from_slice(&png_crc32(type_and_data).to_be_bytes());
        out.extend_from_slice(&bytes[iend - 4..]);
        return out;
    }
    if bytes.starts_with(&[0xFF, 0xD8]) {
        // A JPEG comment (COM) segment right after the start-of-image marker.
        let comment: &[u8] = b"sublime-thumbnail";
        let segment_length = (comment.len() + 2) as u16;
        let mut out = Vec::with_capacity(bytes.len() + comment.len() + 4);
        out.extend_from_slice(&bytes[..2]);
        out.extend_from_slice(&[0xFF, 0xFE]);
        out.extend_from_slice(&segment_length.to_be_bytes());
        out.extend_from_slice(comment);
        out.extend_from_slice(&bytes[2..]);
        return out;
    }
    let mut out = bytes.to_vec();
    out.push(0);
    out
}

/// The PNG CRC-32 of a chunk's type and data.
fn png_crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// The highest data-reference identifier the package metadata records; a new
/// image's data files take the next ones (a small namespace, separate from
/// object identifiers).
fn max_data_id(package: &Package) -> u64 {
    let mut max = 0;
    for entry in &package.entries {
        let Entry::Stream(stream) = entry else {
            continue;
        };
        for object in &stream.objects {
            if first_type(object) != Some(PACKAGE_METADATA) {
                continue;
            }
            let Some(first) = object.messages.first().map(|message| message.first) else {
                continue;
            };
            for (_, field) in stream.tree.chain(first) {
                if stream.tree.field(field).map(|field| field.name) == Some("datas")
                    && let Node::Message(datas_first) = field.value
                    && let Some(Node::Uint(id)) =
                        field_value(&stream.tree, datas_first, "identifier")
                {
                    max = max.max(id);
                }
            }
        }
    }
    max
}

/// The file extension for image bytes (Pages sniffs the content, but the data
/// file and its metadata name must agree).
fn image_extension(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0xFF, 0xD8]) {
        "jpg"
    } else {
        "png"
    }
}

/// Deep-copies a message chain within the same tree, applying `remap` to each
/// copied entry (to repoint references). Byte and string spans are shared —
/// the tree's text buffer is append-only, so the originals stay valid.
fn clone_chain(
    tree: &mut Tree,
    first: u32,
    remap: &mut dyn FnMut(&mut TreeEntry),
) -> Result<u32, PackageError> {
    let originals: Vec<TreeEntry> = tree.chain(first).map(|(_, entry)| *entry).collect();
    let mut chain = Chain::new();
    for mut entry in originals {
        entry.next = NONE;
        if let Node::Message(nested) = entry.value {
            entry.value = Node::Message(clone_chain(tree, nested, remap)?);
        }
        remap(&mut entry);
        tree.push(&mut chain, entry).map_err(tree_error)?;
    }
    Ok(chain.first)
}

/// Clones an object (its message and header) within the document stream under a
/// new identifier, returning the cloned message and header chain starts so the
/// caller can adjust them. The clone is appended to the stream's objects.
fn clone_object(
    stream: &mut Stream,
    source_id: u64,
    new_id: u64,
) -> Result<(u32, u32), PackageError> {
    let (message_type, message_first, info_first) = {
        let source = stream
            .objects
            .iter()
            .find(|object| object.identifier == source_id)
            .ok_or_else(|| malformed("clone source object is missing"))?;
        let message = source
            .messages
            .first()
            .ok_or_else(|| malformed("clone source has no message"))?;
        (message.message_type, message.first, source.info)
    };
    let new_message = clone_chain(&mut stream.tree, message_first, &mut |_| {})?;
    let new_info = clone_chain(&mut stream.tree, info_first, &mut |_| {})?;
    if let Some(index) = field_entry(&stream.tree, new_info, "identifier") {
        stream.tree.entries[index as usize].value = Node::Uint(new_id);
    }
    stream.objects.push(Object {
        identifier: new_id,
        info: new_info,
        messages: vec![ObjectMessage {
            message_type,
            first: new_message,
        }],
    });
    Ok((new_message, new_info))
}

/// Repoints one reference in an object header chain (`message_infos`) — a
/// `data_references` or `object_references` value — from `old` to `new`.
fn remap_info_reference(tree: &mut Tree, info_first: u32, field: &str, old: u64, new: u64) {
    let Some(infos_first) = message_field(tree, info_first, "message_infos") else {
        return;
    };
    let mut cursor = infos_first;
    while cursor != NONE {
        let entry = &tree.entries[cursor as usize];
        let next = entry.next;
        if tree.field(entry).map(|name| name.name) == Some(field) && entry.value == Node::Uint(old)
        {
            tree.entries[cursor as usize].value = Node::Uint(new);
            return;
        }
        cursor = next;
    }
}

/// Sets the `identifier` inside a data-reference field (`data` or
/// `thumbnailData`) of an image message to a new data id, in place.
fn set_message_data_id(tree: &mut Tree, message_first: u32, field: &str, new: u64) {
    if let Some(data_first) = message_field(tree, message_first, field)
        && let Some(index) = field_entry(tree, data_first, "identifier")
    {
        tree.entries[index as usize].value = Node::Uint(new);
    }
}

/// Appends a data file to the package.
fn add_data_file(package: &mut Package, name: &str, bytes: &[u8]) {
    package.entries.push(Entry::File {
        name: format!("Data/{name}"),
        bytes: bytes.to_vec(),
    });
}

/// The stream holding the package metadata, and the metadata message's chain.
fn metadata_message(package: &mut Package) -> Option<(&mut Stream, u32)> {
    for entry in &mut package.entries {
        if let Entry::Stream(stream) = entry
            && let Some(first) = stream
                .objects
                .iter()
                .find(|object| first_type(object) == Some(PACKAGE_METADATA))
                .and_then(|object| object.messages.first())
                .map(|message| message.first)
        {
            return Some((stream, first));
        }
    }
    None
}

/// Appends a `Node::Message` to a repeated field at the end of a chain.
fn append_message_field(
    tree: &mut Tree,
    parent: MessageRef,
    parent_first: u32,
    field_name: &str,
    message_first: u32,
) -> Result<(), PackageError> {
    let (slot, field) = parent
        .slot_named(field_name)
        .ok_or_else(|| malformed("field is not in the schema"))?;
    let mut last = parent_first;
    for (index, _) in tree.chain(parent_first) {
        last = index;
    }
    let mut chain = Chain {
        first: parent_first,
        last,
    };
    tree.push_known(
        &mut chain,
        parent,
        slot,
        field,
        field.number,
        Node::Message(message_first),
    )
    .map_err(tree_error)?;
    Ok(())
}

/// The chain start of the `datas` entry (a `TSP.DataInfo`) for a data id.
fn datas_entry(tree: &Tree, metadata_first: u32, data_id: u64) -> Option<u32> {
    data_info_chain(tree, metadata_first, data_id)
}

/// The component info (in the package metadata) whose data references include
/// `data_id`, and the chain start of that data-reference entry.
fn component_data_reference(tree: &Tree, metadata_first: u32, data_id: u64) -> Option<(u32, u32)> {
    for (_, field) in tree.chain(metadata_first) {
        let Node::Message(component_first) = field.value else {
            continue;
        };
        if tree.field(field).map(|name| name.name) != Some("components")
            && tree.field(field).map(|name| name.name) != Some("versioned_components")
        {
            continue;
        }
        for (_, inner) in tree.chain(component_first) {
            if tree.field(inner).map(|name| name.name) == Some("data_references")
                && let Node::Message(reference_first) = inner.value
                && field_value(tree, reference_first, "data_identifier")
                    == Some(Node::Uint(data_id))
            {
                return Some((component_first, reference_first));
            }
        }
    }
    None
}

/// Clones the template's one image into a fresh image for `mark`: new image and
/// attachment objects, new data files, and cloned metadata entries, all with
/// distinct identifiers so a document with several images keeps them all.
/// Returns the new attachment id for the body to anchor.
#[allow(clippy::too_many_arguments)]
fn clone_image(
    package: &mut Package,
    proto: &TemplateImage,
    mark: &ImageMark,
    bytes: &[u8],
    shared: Option<(u64, Option<u64>)>,
    next_object_id: &mut u64,
    next_data_id: &mut u64,
) -> Result<(u64, u64, Option<u64>), PackageError> {
    let new_image_id = *next_object_id;
    let new_attach_id = *next_object_id + 1;
    *next_object_id += 2;
    // Reuse the data of an identical picture already written, or take new ids.
    let (new_full, new_thumb) = match shared {
        Some((full, thumb)) => (full, thumb.or(proto.thumb_id.map(|_| full))),
        None => {
            let full = *next_data_id;
            *next_data_id += 1;
            let thumb = proto.thumb_id.map(|_| {
                let id = *next_data_id;
                *next_data_id += 1;
                id
            });
            (full, thumb)
        }
    };

    let extension = image_extension(bytes);
    let full_name = format!("image-{new_full}.{extension}");
    let thumbnail = distinct_thumbnail(bytes);
    let thumb_name = new_thumb.map(|id| format!("image-{id}.{extension}"));

    let pixels = image_dimensions(bytes);
    let (width, height) = display_size(mark, pixels);
    let (natural_w, natural_h) = pixels.map_or((width, height), |(w, h)| (w as f32, h as f32));

    // Clone the image and attachment objects into the document stream, with the
    // new data ids and the attachment pointing at the new image.
    {
        let stream = document_stream(package)?;
        let (image_message, image_info) = clone_object(stream, proto.image_id, new_image_id)?;
        // Its own (empty) title and caption: Pages repairs a caption shared by
        // two drawables on load.
        if let Some(super_first) = message_field(&stream.tree, image_message, "super") {
            for name in ["title", "caption"] {
                let Some(index) = field_entry(&stream.tree, super_first, name) else {
                    continue;
                };
                let Node::Reference(old) = stream.tree.entries[index as usize].value else {
                    continue;
                };
                if !stream.objects.iter().any(|object| object.identifier == old) {
                    continue;
                }
                let new = *next_object_id;
                *next_object_id += 1;
                clone_object(stream, old, new)?;
                stream.tree.entries[index as usize].value = Node::Reference(new);
                remap_info_reference(&mut stream.tree, image_info, "object_references", old, new);
            }
        }
        set_message_data_id(&mut stream.tree, image_message, "data", new_full);
        remap_info_reference(
            &mut stream.tree,
            image_info,
            "data_references",
            proto.data_id,
            new_full,
        );
        if let (Some(old_thumb), Some(new_thumb)) = (proto.thumb_id, new_thumb) {
            set_message_data_id(&mut stream.tree, image_message, "thumbnailData", new_thumb);
            remap_info_reference(
                &mut stream.tree,
                image_info,
                "data_references",
                old_thumb,
                new_thumb,
            );
        }
        let (attach_message, attach_info) = clone_object(stream, proto.attach_id, new_attach_id)?;
        if let Some(index) = field_entry(&stream.tree, attach_message, "drawable") {
            stream.tree.entries[index as usize].value = Node::Reference(new_image_id);
        }
        remap_info_reference(
            &mut stream.tree,
            attach_info,
            "object_references",
            proto.image_id,
            new_image_id,
        );
    }

    // Rewrite the clone's sizes and traced path, and make its attachment inline.
    rewrite_object_with(package, new_image_id, |tree, old_first| {
        rebuild_image(tree, old_first, width, height, natural_w, natural_h, mark)
    })?;
    rewrite_object_with(package, new_attach_id, |tree, old_first| {
        rebuild_inline_attachment(tree, old_first)
    })?;

    if shared.is_some() {
        // The data exists: record this image as one more user of it.
        add_data_user(package, new_full, new_image_id)?;
        if let Some(thumb) = new_thumb
            && thumb != new_full
        {
            add_data_user(package, thumb, new_image_id)?;
        }
        return Ok((new_attach_id, new_full, new_thumb));
    }

    // Add the data files and clone their metadata entries under the new ids.
    add_data_file(package, &full_name, bytes);
    clone_data_metadata(
        package,
        proto.data_id,
        new_full,
        &full_name,
        &sha1(bytes),
        natural_w,
        natural_h,
        new_image_id,
    )?;
    if let (Some(old_thumb), Some(new_thumb), Some(name)) = (proto.thumb_id, new_thumb, thumb_name)
    {
        add_data_file(package, &name, &thumbnail);
        clone_data_metadata(
            package,
            old_thumb,
            new_thumb,
            &name,
            &sha1(&thumbnail),
            natural_w,
            natural_h,
            new_image_id,
        )?;
    }
    Ok((new_attach_id, new_full, new_thumb))
}

/// Clones the package-metadata entries for a data reference under a new id: its
/// `datas` entry (digest, file name, pixel size) and its component's
/// data-reference (which object uses it), so the new data file is registered
/// exactly as the prototype's was.
#[allow(clippy::too_many_arguments)]
fn clone_data_metadata(
    package: &mut Package,
    proto_data_id: u64,
    new_data_id: u64,
    file_name: &str,
    digest: &[u8],
    width: f32,
    height: f32,
    object_id: u64,
) -> Result<(), PackageError> {
    let metadata = message_ref("TSP.PackageMetadata")?;
    let components = message_of(
        metadata
            .field_named("components")
            .ok_or_else(|| malformed("metadata has no components"))?
            .kind,
    )?;
    let (stream, meta_first) =
        metadata_message(package).ok_or_else(|| malformed("package metadata is missing"))?;
    let tree = &mut stream.tree;

    // Clone and retarget the datas entry.
    let source = datas_entry(tree, meta_first, proto_data_id)
        .ok_or_else(|| malformed("source datas entry is missing"))?;
    let new_datas = clone_chain(tree, source, &mut |_| {})?;
    set_field_uint(tree, new_datas, "identifier", new_data_id);
    set_field_bytes(tree, new_datas, "digest", digest)?;
    set_field_str(tree, new_datas, "file_name", file_name)?;
    set_field_str(tree, new_datas, "preferred_file_name", "image.png")?;
    if let Some(size) = message_field(tree, new_datas, "attributes")
        .and_then(|attributes| message_field(tree, attributes, "image_data_attributes"))
        .and_then(|image| message_field(tree, image, "pixel_size"))
    {
        set_field_float(tree, size, "width", width);
        set_field_float(tree, size, "height", height);
    }
    append_message_field(tree, metadata, meta_first, "datas", new_datas)?;

    // Clone and retarget the component's data-reference (data id -> object).
    if let Some((component_first, source_reference)) =
        component_data_reference(tree, meta_first, proto_data_id)
    {
        let new_reference = clone_chain(tree, source_reference, &mut |_| {})?;
        // Only the new image uses the new data: the prototype's entry may list
        // other images that share its data, and a clone naming them makes
        // Pages count references those images never make.
        keep_first_field(tree, new_reference, "object_reference_list");
        set_field_uint(tree, new_reference, "data_identifier", new_data_id);
        if let Some(list) = message_field(tree, new_reference, "object_reference_list") {
            set_field_uint(tree, list, "object_identifier", object_id);
        }
        append_message_field(
            tree,
            components,
            component_first,
            "data_references",
            new_reference,
        )?;
    }
    Ok(())
}

/// Unlinks every occurrence of a repeated field after its first from a chain.
fn keep_first_field(tree: &mut Tree, first: u32, name: &str) {
    let mut seen = false;
    let mut previous = NONE;
    let mut cursor = first;
    while cursor != NONE {
        let entry = tree.entries[cursor as usize];
        let matches = tree.field(&entry).is_some_and(|field| field.name == name);
        if matches && seen && previous != NONE {
            tree.entries[previous as usize].next = entry.next;
        } else {
            seen |= matches;
            previous = cursor;
        }
        cursor = entry.next;
    }
}

/// Sets a named uint field in a chain, in place, if present.
fn set_field_uint(tree: &mut Tree, first: u32, name: &str, value: u64) {
    if let Some(index) = field_entry(tree, first, name) {
        tree.entries[index as usize].value = Node::Uint(value);
    }
}

/// Sets a named float field in a chain, in place, if present.
fn set_field_float(tree: &mut Tree, first: u32, name: &str, value: f32) {
    if let Some(index) = field_entry(tree, first, name) {
        tree.entries[index as usize].value = Node::Float(value);
    }
}

/// Sets a named string field in a chain, in place, if present.
fn set_field_str(tree: &mut Tree, first: u32, name: &str, value: &str) -> Result<(), PackageError> {
    if let Some(index) = field_entry(tree, first, name) {
        let span = tree.push_bytes(value.as_bytes()).map_err(tree_error)?;
        tree.entries[index as usize].value = Node::Str(span);
    }
    Ok(())
}

/// Sets a named bytes field in a chain, in place, if present.
fn set_field_bytes(
    tree: &mut Tree,
    first: u32,
    name: &str,
    value: &[u8],
) -> Result<(), PackageError> {
    if let Some(index) = field_entry(tree, first, name) {
        let span = tree.push_bytes(value).map_err(tree_error)?;
        tree.entries[index as usize].value = Node::Bytes(span);
    }
    Ok(())
}

/// Updates a data reference's metadata for the replaced bytes: its digest (so
/// Pages treats the bytes as new instead of serving a cached original) and its
/// recorded pixel size (which the template left at its original picture's size;
/// left mismatched against the new bytes, Pages aborts while loading). Both are
/// patched in place — the package metadata is a fragile root object, and
/// rebuilding its chain reorders fields and makes Pages abort loading it.
fn update_data_metadata(
    package: &mut Package,
    data_id: u64,
    digest: &[u8],
    width: f32,
    height: f32,
) -> Result<(), PackageError> {
    for entry in &mut package.entries {
        let Entry::Stream(stream) = entry else {
            continue;
        };
        let Some(metadata_first) = stream
            .objects
            .iter()
            .find(|object| first_type(object) == Some(PACKAGE_METADATA))
            .and_then(|object| object.messages.first())
            .map(|message| message.first)
        else {
            continue;
        };
        let Some(datas_first) = data_info_chain(&stream.tree, metadata_first, data_id) else {
            return Ok(());
        };
        if let Some(index) = field_entry(&stream.tree, datas_first, "digest") {
            let span = stream.tree.push_bytes(digest).map_err(tree_error)?;
            stream.tree.entries[index as usize].value = Node::Bytes(span);
        }
        // datas -> attributes -> image_data_attributes -> pixel_size -> w/h.
        if let Some(size) = message_field(&stream.tree, datas_first, "attributes")
            .and_then(|attributes| message_field(&stream.tree, attributes, "image_data_attributes"))
            .and_then(|image| message_field(&stream.tree, image, "pixel_size"))
        {
            if let Some(index) = field_entry(&stream.tree, size, "width") {
                stream.tree.entries[index as usize].value = Node::Float(width);
            }
            if let Some(index) = field_entry(&stream.tree, size, "height") {
                stream.tree.entries[index as usize].value = Node::Float(height);
            }
        }
        return Ok(());
    }
    Ok(())
}

/// The message chain start of the `datas` entry whose identifier matches.
fn data_info_chain(tree: &Tree, metadata_first: u32, data_id: u64) -> Option<u32> {
    for (_, entry) in tree.chain(metadata_first) {
        if tree.field(entry).map(|field| field.name) == Some("datas")
            && let Node::Message(datas_first) = entry.value
            && field_value(tree, datas_first, "identifier") == Some(Node::Uint(data_id))
        {
            return Some(datas_first);
        }
    }
    None
}

/// The tree index of a named field within a message chain.
fn field_entry(tree: &Tree, first: u32, name: &str) -> Option<u32> {
    for (index, entry) in tree.chain(first) {
        if tree.field(entry).map(|field| field.name) == Some(name) {
            return Some(index);
        }
    }
    None
}

/// The SHA-1 digest of `data`, as Pages stores it for a data reference.
fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());
    let mut words = [0u32; 80];
    for block in message.chunks_exact(64) {
        for (index, word) in words.iter_mut().take(16).enumerate() {
            let start = index * 4;
            *word = u32::from_be_bytes([
                block[start],
                block[start + 1],
                block[start + 2],
                block[start + 3],
            ]);
        }
        for index in 16..80 {
            words[index] =
                (words[index - 3] ^ words[index - 8] ^ words[index - 14] ^ words[index - 16])
                    .rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (index, word) in words.iter().enumerate() {
            let (f, k) = match index {
                0..=19 => ((b & c) | ((!b) & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    let mut digest = [0u8; 20];
    for (index, word) in h.iter().enumerate() {
        digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    digest
}

/// The display size for a reused image: the model's, when it carried one;
/// otherwise the pixel size scaled to fit the body text width.
fn display_size(mark: &ImageMark, pixels: Option<(u32, u32)>) -> (f32, f32) {
    if mark.width > 0.0 && mark.height > 0.0 {
        return (mark.width, mark.height);
    }
    match pixels {
        Some((w, h)) if w > 0 && h > 0 => {
            let (w, h) = (w as f32, h as f32);
            const MAX_WIDTH: f32 = 460.0;
            if w > MAX_WIDTH {
                (MAX_WIDTH, h * MAX_WIDTH / w)
            } else {
                (w, h)
            }
        }
        _ => (mark.width.max(1.0), mark.height.max(1.0)),
    }
}

/// Rebuilds an image archive, replacing its geometry size, original size, and
/// natural size (and its accessibility description), keeping everything else —
/// the style, captions, parent, data references, and traced path.
#[allow(clippy::too_many_arguments)]
fn rebuild_image(
    tree: &mut Tree,
    old_first: u32,
    width: f32,
    height: f32,
    natural_w: f32,
    natural_h: f32,
    mark: &ImageMark,
) -> Result<u32, PackageError> {
    let image = message_ref("TSD.ImageArchive")?;
    let super_first =
        message_field(tree, old_first, "super").ok_or_else(|| malformed("image has no super"))?;
    let new_super = rebuild_image_super(tree, super_first, width, height, mark)?;
    let original = build_size(tree, width, height)?;
    let natural = build_size(tree, natural_w, natural_h)?;
    let mut overrides = vec![
        ("super", Node::Message(new_super)),
        ("originalSize", Node::Message(original)),
        ("naturalSize", Node::Message(natural)),
    ];
    // The template's traced path is a rectangle at its original picture's
    // pixel size; rewritten to the new size so Pages' outline matches the
    // image. The instant-alpha cut-out path (if any) is dropped.
    let traced = build_traced_path(tree, natural_w, natural_h)?;
    overrides.push(("traced_path", Node::Message(traced)));
    rebuild_message(tree, image, old_first, overrides, &["instantAlphaPath"])
}

/// A rectangular traced path covering the whole image, the outline Pages draws
/// for a plain picture: move to a corner, line around, close, move back.
fn build_traced_path(tree: &mut Tree, width: f32, height: f32) -> Result<u32, PackageError> {
    let path = message_ref("TSP.Path")?;
    let element = message_ref("TSP.Path.Element")?;
    let point = message_ref("TSP.Point")?;
    let corners = [
        (1u64, 0.0, 0.0),
        (2, width, 0.0),
        (2, width, height),
        (2, 0.0, height),
    ];
    let mut chain = Chain::new();
    for (kind, x, y) in corners {
        let mut element_chain = Chain::new();
        push_field(tree, &mut element_chain, element, "type", Node::Uint(kind))?;
        let mut point_chain = Chain::new();
        push_field(tree, &mut point_chain, point, "x", Node::Float(x))?;
        push_field(tree, &mut point_chain, point, "y", Node::Float(y))?;
        push_field(
            tree,
            &mut element_chain,
            element,
            "points",
            Node::Message(point_chain.first),
        )?;
        push_field(
            tree,
            &mut chain,
            path,
            "elements",
            Node::Message(element_chain.first),
        )?;
    }
    // A close element (type 5) then a move back to the origin (type 1).
    let mut close = Chain::new();
    push_field(tree, &mut close, element, "type", Node::Uint(5))?;
    push_field(
        tree,
        &mut chain,
        path,
        "elements",
        Node::Message(close.first),
    )?;
    let mut back = Chain::new();
    push_field(tree, &mut back, element, "type", Node::Uint(1))?;
    let mut origin = Chain::new();
    push_field(tree, &mut origin, point, "x", Node::Float(0.0))?;
    push_field(tree, &mut origin, point, "y", Node::Float(0.0))?;
    push_field(
        tree,
        &mut back,
        element,
        "points",
        Node::Message(origin.first),
    )?;
    push_field(
        tree,
        &mut chain,
        path,
        "elements",
        Node::Message(back.first),
    )?;
    Ok(chain.first)
}

/// Rebuilds the image's drawable base, replacing the geometry's size (so the
/// frame matches the image) and the accessibility description.
fn rebuild_image_super(
    tree: &mut Tree,
    super_first: u32,
    width: f32,
    height: f32,
    mark: &ImageMark,
) -> Result<u32, PackageError> {
    let drawable = message_ref("TSD.DrawableArchive")?;
    let geometry_first = message_field(tree, super_first, "geometry")
        .ok_or_else(|| malformed("image drawable has no geometry"))?;
    let new_geometry = rebuild_geometry(tree, geometry_first, width, height)?;
    let mut overrides = vec![("geometry", Node::Message(new_geometry))];
    // An inline image wraps as Pages' own inline pictures do (type 0, no fit,
    // no margin); the template picture's floating wrap would pin every clone
    // to its fixed spot on the page.
    if let Some(wrap_first) = message_field(tree, super_first, "exterior_text_wrap") {
        let wrap = message_ref("TSD.ExteriorTextWrapArchive")?;
        let inline_wrap = rebuild_message(
            tree,
            wrap,
            wrap_first,
            vec![
                ("type", Node::Uint(0)),
                ("fit_type", Node::Uint(0)),
                ("margin", Node::Float(0.0)),
            ],
            &[],
        )?;
        overrides.push(("exterior_text_wrap", Node::Message(inline_wrap)));
    }
    if let Some(description) = &mark.description {
        let span = tree
            .push_bytes(description.as_bytes())
            .map_err(tree_error)?;
        overrides.push(("accessibility_description", Node::Str(span)));
    }
    rebuild_message(tree, drawable, super_first, overrides, &[])
}

/// Rebuilds a geometry, replacing only its size.
fn rebuild_geometry(
    tree: &mut Tree,
    geometry_first: u32,
    width: f32,
    height: f32,
) -> Result<u32, PackageError> {
    let geometry = message_ref("TSD.GeometryArchive")?;
    let size = build_size(tree, width, height)?;
    // Inline, the text line places the picture; its own position is the origin.
    let point = message_ref("TSP.Point")?;
    let mut position = Chain::new();
    push_field(tree, &mut position, point, "x", Node::Float(0.0))?;
    push_field(tree, &mut position, point, "y", Node::Float(0.0))?;
    rebuild_message(
        tree,
        geometry,
        geometry_first,
        vec![
            ("size", Node::Message(size)),
            ("position", Node::Message(position.first)),
        ],
        &[],
    )
}

/// A `TSP.Size` message.
fn build_size(tree: &mut Tree, width: f32, height: f32) -> Result<u32, PackageError> {
    let size = message_ref("TSP.Size")?;
    let mut chain = Chain::new();
    push_field(tree, &mut chain, size, "width", Node::Float(width))?;
    push_field(tree, &mut chain, size, "height", Node::Float(height))?;
    Ok(chain.first)
}

/// Rebuilds a drawable attachment as inline: only the drawable reference, no
/// wrap offsets, so the image sits in the text line at its `U+FFFC`.
fn rebuild_inline_attachment(tree: &mut Tree, old_first: u32) -> Result<u32, PackageError> {
    let attachment = message_ref("TSWP.DrawableAttachmentArchive")?;
    let drawable = field_value(tree, old_first, "drawable")
        .ok_or_else(|| malformed("attachment has no drawable"))?;
    let mut chain = Chain::new();
    push_field(tree, &mut chain, attachment, "drawable", drawable)?;
    // An inline attachment (its drawable's wrap type is inline) carries zero
    // offsets: older Pages wrote NaN, which the current one rewrites to zero
    // on load and reports as a file modified while being read.
    push_field(tree, &mut chain, attachment, "h_offset_type", Node::Uint(0))?;
    push_field(tree, &mut chain, attachment, "h_offset", Node::Float(0.0))?;
    push_field(tree, &mut chain, attachment, "v_offset_type", Node::Uint(0))?;
    push_field(tree, &mut chain, attachment, "v_offset", Node::Float(0.0))?;
    Ok(chain.first)
}

/// Rebuilds a message chain, replacing the fields named in `overrides` with
/// the given nodes and dropping those in `remove`, keeping every other field
/// and — importantly — its original order: iWork's persistence expects a
/// message's `super` base to come first, so an override is substituted in
/// place rather than appended (a field not already present is appended). Nodes
/// referencing nested chains must be built first.
fn rebuild_message(
    tree: &mut Tree,
    message: MessageRef,
    old_first: u32,
    overrides: Vec<(&str, Node)>,
    remove: &[&str],
) -> Result<u32, PackageError> {
    // Resolve the override and remove names to field numbers up front.
    let overrides: Vec<(u32, Node)> = overrides
        .iter()
        .filter_map(|(name, node)| message.field_named(name).map(|field| (field.number, *node)))
        .collect();
    let remove: Vec<u32> = remove
        .iter()
        .filter_map(|name| message.field_named(name).map(|field| field.number))
        .collect();
    let originals: Vec<(u32, Node)> = tree
        .chain(old_first)
        .map(|(_, entry)| (entry.number, entry.value))
        .collect();

    let push = |tree: &mut Tree, chain: &mut Chain, number: u32, value: Node| {
        let slot = message
            .slot(number)
            .ok_or_else(|| malformed("field without a schema slot"))?;
        let field = message
            .field_at(slot)
            .ok_or_else(|| malformed("field slot out of range"))?;
        tree.push_known(chain, message, slot, field, number, value)
            .map_err(tree_error)
    };

    // Build the field list: originals (with overrides substituted, removals
    // dropped), then any overridden field the original lacked. A stable sort by
    // field number then matches how iWork writes a message — fields in number
    // order, `super` (field 1) first — which its persistence layer expects.
    let mut fields: Vec<(u32, Node)> = Vec::new();
    let mut emitted: Vec<u32> = Vec::new();
    for (number, value) in &originals {
        if remove.contains(number) {
            continue;
        }
        let value = overrides
            .iter()
            .find(|(over, _)| over == number)
            .map_or(*value, |(_, node)| *node);
        fields.push((*number, value));
        emitted.push(*number);
    }
    for (number, node) in &overrides {
        if !emitted.contains(number) {
            fields.push((*number, *node));
        }
    }
    fields.sort_by_key(|(number, _)| *number);

    let mut chain = Chain::new();
    for (number, value) in fields {
        push(tree, &mut chain, number, value)?;
    }
    Ok(chain.first)
}

/// Runs `builder` over the message chain of the object with `id`, passing the
/// old chain start and storing the new one.
fn rewrite_object_with(
    package: &mut Package,
    id: u64,
    builder: impl FnOnce(&mut Tree, u32) -> Result<u32, PackageError>,
) -> Result<(), PackageError> {
    for entry in &mut package.entries {
        if let Entry::Stream(stream) = entry
            && let Some(position) = stream
                .objects
                .iter()
                .position(|object| object.identifier == id)
        {
            let old_first = stream.objects[position].messages[0].first;
            let new_first = builder(&mut stream.tree, old_first)?;
            stream.objects[position].messages[0].first = new_first;
            return Ok(());
        }
    }
    Err(malformed("object to rewrite is missing"))
}

/// The `Data/` file name a data reference names, from the package metadata.
fn data_file_name(package: &Package, data_id: u64) -> Option<String> {
    for entry in &package.entries {
        let Entry::Stream(stream) = entry else {
            continue;
        };
        for object in &stream.objects {
            if first_type(object) != Some(PACKAGE_METADATA) {
                continue;
            }
            let first = object.messages.first()?.first;
            for (_, field_entry) in stream.tree.chain(first) {
                if stream.tree.field(field_entry).map(|field| field.name) == Some("datas")
                    && let Node::Message(data_first) = field_entry.value
                    && field_value(&stream.tree, data_first, "identifier")
                        == Some(Node::Uint(data_id))
                    && let Some(name) = str_field(&stream.tree, data_first, "file_name")
                    && !name.is_empty()
                {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

/// Replaces the bytes of the `Data/<name>` file with the model image's.
fn replace_data_file(package: &mut Package, name: &str, bytes: &[u8]) {
    let path = format!("Data/{name}");
    for entry in &mut package.entries {
        if let Entry::File {
            name: file_name,
            bytes: file_bytes,
        } = entry
            && *file_name == path
        {
            *file_bytes = bytes.to_vec();
            return;
        }
    }
}

/// The pixel width and height of a PNG or JPEG, read from its header.
fn image_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) && bytes.len() >= 24 {
        let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
        let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
        return Some((width, height));
    }
    if bytes.starts_with(&[0xFF, 0xD8]) {
        return jpeg_dimensions(bytes);
    }
    None
}

/// The pixel size of a JPEG, from its first start-of-frame marker.
fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut index = 2;
    while index + 9 < bytes.len() {
        if bytes[index] != 0xFF {
            index += 1;
            continue;
        }
        let marker = bytes[index + 1];
        // Start-of-frame markers carry the size; skip the rest by their length.
        let is_sof = matches!(marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF);
        if is_sof {
            let height = u16::from_be_bytes([bytes[index + 5], bytes[index + 6]]);
            let width = u16::from_be_bytes([bytes[index + 7], bytes[index + 8]]);
            return Some((u32::from(width), u32::from(height)));
        }
        // Standalone markers (no length) versus segments with a length word.
        if matches!(marker, 0xD0..=0xD9 | 0x01) {
            index += 2;
        } else {
            let length = u16::from_be_bytes([bytes[index + 2], bytes[index + 3]]) as usize;
            index += 2 + length;
        }
    }
    None
}

fn rebuild_model_dims(
    tree: &mut Tree,
    mark: &TableMark,
    old_first: u32,
) -> Result<u32, PackageError> {
    let model = message_ref("TST.TableModelArchive")?;
    let owned = [
        "number_of_rows",
        "number_of_columns",
        "number_of_header_rows",
        "number_of_header_columns",
        "number_of_footer_rows",
        "default_row_height",
        "default_column_width",
    ];
    let mut kept: Vec<(u32, Node)> = Vec::new();
    for (_, entry) in tree.chain(old_first) {
        let name = tree.field(entry).map(|field| field.name);
        if name.is_some_and(|name| owned.contains(&name)) {
            continue;
        }
        kept.push((entry.number, entry.value));
    }
    let mut chain = Chain::new();
    for (number, value) in kept {
        let slot = model
            .slot(number)
            .ok_or_else(|| malformed("model field without a schema slot"))?;
        let field = model
            .field_at(slot)
            .ok_or_else(|| malformed("model field slot out of range"))?;
        tree.push_known(&mut chain, model, slot, field, number, value)
            .map_err(tree_error)?;
    }
    push_field(
        tree,
        &mut chain,
        model,
        "number_of_rows",
        Node::Uint(mark.rows as u64),
    )?;
    push_field(
        tree,
        &mut chain,
        model,
        "number_of_columns",
        Node::Uint(mark.columns as u64),
    )?;
    push_field(
        tree,
        &mut chain,
        model,
        "number_of_header_rows",
        Node::Uint(u64::from(mark.header_rows)),
    )?;
    // A model table carries no header column or footer row; drop the
    // template's if it had them.
    push_field(
        tree,
        &mut chain,
        model,
        "number_of_header_columns",
        Node::Uint(0),
    )?;
    push_field(
        tree,
        &mut chain,
        model,
        "number_of_footer_rows",
        Node::Uint(0),
    )?;
    push_field(
        tree,
        &mut chain,
        model,
        "default_row_height",
        Node::Double(f64::from(DEFAULT_ROW_HEIGHT)),
    )?;
    let default_width = if mark.widths.is_empty() {
        100.0
    } else {
        mark.widths.iter().sum::<f32>() / mark.widths.len() as f32
    };
    push_field(
        tree,
        &mut chain,
        model,
        "default_column_width",
        Node::Double(f64::from(default_width)),
    )?;
    Ok(chain.first)
}

/// Adds object identifiers to a storage object's `ArchiveInfo`, so the objects
/// the new tables reference survive the document-scope reachability prune (and
/// so the file is self-consistent). Identifiers already listed are left alone.
fn add_object_references(
    tree: &mut Tree,
    info: u32,
    references: &[u64],
) -> Result<(), PackageError> {
    if references.is_empty() {
        return Ok(());
    }
    let message_info = message_ref("TSP.MessageInfo")?;
    let (slot, field) = message_info
        .slot_named("object_references")
        .ok_or_else(|| malformed("MessageInfo has no object_references"))?;
    let info_first = message_field(tree, info, "message_infos")
        .ok_or_else(|| malformed("ArchiveInfo has no message_infos"))?;

    let mut existing: Vec<u64> = Vec::new();
    let mut last = info_first;
    for (index, entry) in tree.chain(info_first) {
        last = index;
        if tree.field(entry).map(|field| field.name) == Some("object_references")
            && let Node::Uint(id) = entry.value
        {
            existing.push(id);
        }
    }
    let mut chain = Chain {
        first: info_first,
        last,
    };
    for id in references {
        if existing.contains(id) {
            continue;
        }
        tree.push_known(
            &mut chain,
            message_info,
            slot,
            field,
            field.number,
            Node::Uint(*id),
        )
        .map_err(tree_error)?;
    }
    Ok(())
}

/// The `Index/Document.iwa` stream.
fn document_stream(package: &mut Package) -> Result<&mut Stream, PackageError> {
    for entry in &mut package.entries {
        if let Entry::Stream(stream) = entry
            && stream.name == "Index/Document.iwa"
        {
            return Ok(stream);
        }
    }
    Err(malformed("Index/Document.iwa is missing"))
}

/// The stream that holds the object with `id`. Pages loads a table's data-list
/// stream as one component and expects the list's payloads and cell storages to
/// live in it, so rich cells must be built into the list's own stream, not the
/// document stream.
fn stream_containing(package: &mut Package, id: u64) -> Result<&mut Stream, PackageError> {
    for entry in &mut package.entries {
        if let Entry::Stream(stream) = entry
            && stream.objects.iter().any(|object| object.identifier == id)
        {
            return Ok(stream);
        }
    }
    Err(malformed("the object's stream is missing"))
}

/// The identifier of the object `DocumentArchive.body_storage` points at.
fn body_storage_identifier(stream: &Stream) -> Result<u64, PackageError> {
    let document = stream
        .objects
        .iter()
        .find(|object| first_type(object) == Some(DOCUMENT_ARCHIVE))
        .ok_or_else(|| malformed("DocumentArchive is missing"))?;
    match field_value(&stream.tree, document.messages[0].first, "body_storage") {
        Some(Node::Reference(identifier)) => Ok(identifier),
        _ => Err(malformed("DocumentArchive has no body_storage")),
    }
}

/// Builds a new StorageArchive message chain: the template's fields kept as
/// they were, with the text and the paragraph-, character-, list-, and
/// table-attribute tables replaced. Tables anchor the template's own (reused)
/// tables by the identifiers in `anchors`.
#[allow(clippy::too_many_arguments)]
fn build_storage(
    tree: &mut Tree,
    old_first: u32,
    document: &Document,
    body: &Body,
    styles: &HashMap<String, u64>,
    formats: &HashMap<Format, u64>,
    lists: &HashMap<String, u64>,
    word_lists: &HashMap<usize, u64>,
    anchors: &[(u32, u64)],
    smart_entries: &[(u32, Option<u64>)],
    paras: &ParaStyles,
) -> Result<(u32, Vec<u64>), PackageError> {
    let storage = message_ref("TSWP.StorageArchive")?;
    let has_tables = !anchors.is_empty();
    let has_links = smart_entries.iter().any(|(_, object)| object.is_some());
    // Fields the writer owns; everything else is copied from the template. Every
    // character-indexed table must be owned: the template's entries index its
    // own (longer) body text, so copying them onto shorter text leaves indices
    // past the end that crash layout. The list tables are always rebuilt; the
    // metadata tables the writer has no data for are dropped (Pages defaults
    // them) so they carry no stale indices.
    let mut owned = vec![
        "text",
        "table_para_style",
        "table_char_style",
        "table_list_style",
        "table_para_data",
        "table_para_starts",
        "table_para_bidi",
        "table_language",
        "table_dictation",
        // Owned always: the template's own entries anchor its table and
        // picture at offsets that are ordinary text in a new body.
        "table_attachment",
    ];
    if has_links {
        owned.push("table_smartfield");
    }
    // Collect the kept fields before touching the tree (an immutable borrow).
    let mut kept: Vec<(u32, Node)> = Vec::new();
    for (_, entry) in tree.chain(old_first) {
        let name = tree.field(entry).map(|field| field.name);
        if name.is_some_and(|name| owned.contains(&name)) {
            continue;
        }
        kept.push((entry.number, entry.value));
    }

    let mut chain = Chain::new();
    for (number, value) in kept {
        let slot = storage
            .slot(number)
            .ok_or_else(|| malformed("storage field without a schema slot"))?;
        let field = storage
            .field_at(slot)
            .ok_or_else(|| malformed("storage field slot out of range"))?;
        tree.push_known(&mut chain, storage, slot, field, number, value)
            .map_err(tree_error)?;
    }

    let span = tree.push_bytes(body.text.as_bytes()).map_err(tree_error)?;
    push_field(tree, &mut chain, storage, "text", Node::Str(span))?;

    let paragraph_entries: Vec<(u32, Option<u64>)> = body
        .paragraphs
        .iter()
        .map(|paragraph| {
            let identifier = style_identifier(document, styles, paragraph.style_name.as_deref());
            let identifier = paras
                .get(&(identifier, paragraph.format))
                .copied()
                .unwrap_or(identifier);
            (paragraph.offset, Some(identifier))
        })
        .collect();
    emit_reference_table(
        tree,
        &mut chain,
        storage,
        "table_para_style",
        &paragraph_entries,
    )?;

    // The character-style table must start at offset 0 and give every run a
    // style, mirroring how Pages itself writes it: the template's base
    // character style covers plain text, and a styled run overrides it. A
    // table that starts partway in, or names a style the storage never
    // declares, is dropped whole by Pages and nothing renders.
    let base = template_base_char(tree, old_first);
    let mut char_entries: Vec<(u32, Option<u64>)> = body
        .char_marks
        .iter()
        .map(|mark| (mark.offset, char_style_for(mark.format, formats).or(base)))
        .collect();
    if char_entries.first().is_none_or(|(offset, _)| *offset != 0) {
        char_entries.insert(0, (0, base));
    }
    if char_entries.iter().any(|(_, object)| object.is_some()) {
        emit_reference_table(tree, &mut chain, storage, "table_char_style", &char_entries)?;
    }

    // Objects the new tables reference that the template's storage did not
    // already declare (the list styles); returned so the caller can add them
    // to the object's references, or the document scope prunes them away.
    let mut references: Vec<u64> = Vec::new();
    {
        // A list paragraph points at the template's Bullet or Numbered list
        // style and carries its nesting level; a plain one reverts to the
        // "None" style at level 0. Like the other tables, all three start at
        // offset 0. table_para_starts carries the number an ordered list
        // restarts at. These are rebuilt for every body (not only lists) so
        // their character indices always match the current text.
        let list_entries: Vec<(u32, Option<u64>)> = body
            .paragraphs
            .iter()
            .map(|mark| {
                let word_list = mark.list.and_then(|item| word_lists.get(&item.style));
                (
                    mark.offset,
                    word_list
                        .copied()
                        .or_else(|| list_style_for(document, lists, mark.list.as_ref())),
                )
            })
            .collect();
        for (_, object) in &list_entries {
            if let Some(id) = object
                && !references.contains(id)
            {
                references.push(*id);
            }
        }
        emit_reference_table(
            tree,
            &mut chain,
            storage,
            "table_list_style",
            &dedup(list_entries),
        )?;

        let data_entries: Vec<(u32, u64, u64)> = body
            .paragraphs
            .iter()
            .map(|mark| {
                let (level, starts) = mark.list.map_or((0, 0), |item| {
                    (u64::from(item.level), u64::from(item.starts_list))
                });
                (mark.offset, level, starts)
            })
            .collect();
        emit_data_table(
            tree,
            &mut chain,
            storage,
            "table_para_data",
            &dedup_data(data_entries),
        )?;

        let start_entries: Vec<(u32, u64, u64)> = body
            .paragraphs
            .iter()
            .map(|mark| {
                let start = mark
                    .list
                    .filter(|item| item.starts_list)
                    .map_or(0, |item| u64::from(item.start));
                (mark.offset, start, 0)
            })
            .collect();
        emit_data_table(
            tree,
            &mut chain,
            storage,
            "table_para_starts",
            &dedup_data(start_entries),
        )?;
    }

    // One bidirectional-text entry for the whole text, as Pages writes it.
    emit_data_table(tree, &mut chain, storage, "table_para_bidi", &[(0, 0, 0)])?;

    // Each model table anchors a reused template table by its drawable
    // attachment at the offset of its U+FFFC character.
    if has_tables {
        let attach_entries: Vec<(u32, Option<u64>)> = anchors
            .iter()
            .map(|(offset, attach_id)| (*offset, Some(*attach_id)))
            .collect();
        emit_reference_table(
            tree,
            &mut chain,
            storage,
            "table_attachment",
            &attach_entries,
        )?;
    }

    // The smart-field table anchors each hyperlink object over its run. Like
    // the character table it must begin at offset 0, so an unlinked lead-in
    // gets an explicit gap entry there.
    if has_links {
        let mut link_entries = smart_entries.to_vec();
        if link_entries.first().is_none_or(|(offset, _)| *offset != 0) {
            link_entries.insert(0, (0, None));
        }
        emit_reference_table(tree, &mut chain, storage, "table_smartfield", &link_entries)?;
    }

    Ok((sorted_chain(tree, storage, chain.first)?, references))
}

/// Rebuilds a message chain in field-number order (stable within a field),
/// the canonical order Pages writes.
fn sorted_chain(tree: &mut Tree, message: MessageRef, first: u32) -> Result<u32, PackageError> {
    let mut entries: Vec<TreeEntry> = tree.chain(first).map(|(_, entry)| *entry).collect();
    entries.sort_by_key(|entry| entry.number);
    let mut chain = Chain::new();
    for mut entry in entries {
        entry.next = NONE;
        tree.push(&mut chain, entry).map_err(tree_error)?;
    }
    let _ = message;
    Ok(chain.first)
}

/// Collapses consecutive entries with the same reference, keeping the first
/// (so the table still begins at offset 0 and each run is one entry).
fn dedup(entries: Vec<(u32, Option<u64>)>) -> Vec<(u32, Option<u64>)> {
    let mut out: Vec<(u32, Option<u64>)> = Vec::new();
    for entry in entries {
        if out.last().map(|last| last.1) != Some(entry.1) {
            out.push(entry);
        }
    }
    out
}

/// Collapses consecutive `(offset, first, second)` entries with equal values.
fn dedup_data(entries: Vec<(u32, u64, u64)>) -> Vec<(u32, u64, u64)> {
    let mut out: Vec<(u32, u64, u64)> = Vec::new();
    for entry in entries {
        if out.last().map(|last| (last.1, last.2)) != Some((entry.1, entry.2)) {
            out.push(entry);
        }
    }
    out
}

/// The template style for a run formatting: an exact match, else the bold or
/// italic style alone, else none.
fn char_style_for(format: Format, formats: &HashMap<Format, u64>) -> Option<u64> {
    if format.is_plain() {
        return None;
    }
    if let Some(id) = formats.get(&format) {
        return Some(*id);
    }
    if format.bold
        && let Some(id) = formats.get(&Format {
            bold: true,
            ..Format::default()
        })
    {
        return Some(*id);
    }
    if format.italic
        && let Some(id) = formats.get(&Format {
            italic: true,
            ..Format::default()
        })
    {
        return Some(*id);
    }
    None
}

/// Character formatting the writer carries. Bold and italic reuse the theme's
/// own styles (they need its weighted faces to render); a run that also sets a
/// colour, size, font, or underline gets a character style synthesised with all
/// of them, so a document's direct formatting reaches Pages.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
struct Format {
    bold: bool,
    italic: bool,
    underline: bool,
    /// Font size in half-points (so it stays hashable), e.g. 24 for 12 pt.
    size: Option<u16>,
    /// The font name, interned in `Document::strings`.
    font: Option<Id>,
    color: Option<Color>,
}

impl Format {
    fn is_plain(&self) -> bool {
        *self == Format::default()
    }

    /// Whether the run carries formatting beyond bold and italic, which the
    /// theme's own styles cannot express and so must be synthesised.
    fn has_direct(&self) -> bool {
        self.underline || self.size.is_some() || self.font.is_some() || self.color.is_some()
    }
}

/// A paragraph start: its offset (UTF-16 units), style name, and list
/// membership (the model's item, resolved to a template list style later).
struct ParagraphMark {
    offset: u32,
    style_name: Option<String>,
    list: Option<ListItem>,
    format: ParaFormat,
    /// Word's contextual spacing: no space against a same-style neighbour.
    contextual: bool,
}

/// A point in the text where the character formatting changes.
struct CharMark {
    offset: u32,
    format: Format,
}

/// A point in the text where the link target changes: the interned link id
/// (an index into `Document::links`), or `None` for unlinked text.
struct LinkMark {
    offset: u32,
    link: Option<Id>,
}

/// One table cell's content: its text and the formatting runs within it (each
/// a `(offset in the cell's text, format)`). A cell with only default
/// formatting stays a plain string cell; one with any formatting becomes a
/// rich-text cell carrying its own character styles.
#[derive(Default)]
struct CellContent {
    text: String,
    marks: Vec<(u32, Format)>,
    /// Page-number (kind 0) and page-count (kind 1) fields, by offset of the
    /// U+FFFC standing for each.
    fields: Vec<(u32, u64)>,
    /// Where each Pages paragraph starts, with its paragraph formatting.
    paragraphs: Vec<(u32, ParaFormat)>,
}

impl CellContent {
    /// No text at all (a field stands in the text as U+FFFC, so it counts).
    fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

/// A table anchored at a `U+FFFC` character: its offset and its grid of cell
/// content, with the column widths, row heights, and header-row count.
struct TableMark {
    offset: u32,
    rows: usize,
    columns: usize,
    header_rows: u32,
    /// Row-major cell content, `rows * columns` entries.
    cells: Vec<CellContent>,
    widths: Vec<f32>,
    heights: Vec<f32>,
    /// Merged regions as (row, column, rows, columns), origin first.
    merges: Vec<(usize, usize, usize, usize)>,
    /// Row-major cell background colours (shading), `None` for none.
    backgrounds: Vec<Option<crate::document::Color>>,
    /// Row-major cells' vertical alignment, as Pages codes it.
    alignments: Vec<u64>,
    /// The table's grid lines, when the source states them.
    borders: Option<crate::document::TableBorders>,
    /// The cells' padding, when the source states it.
    cell_margins: Option<crate::document::CellMargins>,
    /// Row-major cells' own edges, over the table's lines.
    cell_borders: Vec<crate::document::CellBorders>,
}

impl TableMark {
    /// Row-major indices of the cells merged regions cover (every cell of a
    /// region but its top-left origin).
    fn covered(&self) -> std::collections::HashSet<usize> {
        let mut covered = std::collections::HashSet::new();
        for &(row, column, rows, columns) in &self.merges {
            for r in row..row + rows {
                for c in column..column + columns {
                    if (r, c) != (row, column) {
                        covered.insert(r * self.columns + c);
                    }
                }
            }
        }
        covered
    }
}

/// An inline image anchored at a `U+FFFC` character: its offset, the media
/// index whose bytes it carries, its display size in points, and its
/// accessibility description.
struct ImageMark {
    offset: u32,
    media: MediaId,
    width: f32,
    height: f32,
    description: Option<String>,
}

/// The document flattened for the storage: the text, the paragraph and
/// character marks, and the anchored tables and images.
struct Body {
    text: String,
    paragraphs: Vec<ParagraphMark>,
    char_marks: Vec<CharMark>,
    link_marks: Vec<LinkMark>,
    tables: Vec<TableMark>,
    images: Vec<ImageMark>,
}

/// Flattens the document into one text string (paragraphs joined by `\n`)
/// with paragraph and character marks. Each table becomes one anchor
/// paragraph holding a `U+FFFC`, with its grid recorded for later synthesis.
fn flatten(document: &Document) -> Body {
    let mut walk = Walk {
        text: String::new(),
        paragraphs: Vec::new(),
        char_marks: Vec::new(),
        link_marks: Vec::new(),
        tables: Vec::new(),
        images: Vec::new(),
        offset: 0,
        current: Format::default(),
        current_link: None,
        pending_break: false,
        list_counters: ListCounters::default(),
    };
    for (index, section) in document.sections.iter().enumerate() {
        // A section that starts a new page breaks the page before it.
        if index > 0 && section.start != crate::document::SectionStart::Continuous {
            walk.pending_break = true;
        }
        walk.blocks(document, &section.blocks);
    }
    apply_contextual_spacing(&mut walk.paragraphs);
    Body {
        text: walk.text,
        paragraphs: walk.paragraphs,
        char_marks: walk.char_marks,
        link_marks: walk.link_marks,
        tables: walk.tables,
        images: walk.images,
    }
}

/// Word's contextual spacing, which Pages lacks: between two paragraphs of
/// the same style, one that asks for it drops its space on that side.
fn apply_contextual_spacing(paragraphs: &mut [ParagraphMark]) {
    for index in 1..paragraphs.len() {
        let (before, after) = paragraphs.split_at_mut(index);
        let (previous, next) = (&mut before[index - 1], &mut after[0]);
        if previous.style_name != next.style_name {
            continue;
        }
        if previous.contextual {
            previous.format.space_after = Some(0);
        }
        if next.contextual {
            next.format.space_before = Some(0);
        }
    }
}

struct Walk {
    text: String,
    paragraphs: Vec<ParagraphMark>,
    char_marks: Vec<CharMark>,
    link_marks: Vec<LinkMark>,
    tables: Vec<TableMark>,
    images: Vec<ImageMark>,
    offset: u32,
    current: Format,
    current_link: Option<Id>,
    /// A page break waiting to open the next paragraph (Pages writes one as
    /// U+0005 at the start of the paragraph that follows it).
    pending_break: bool,
    /// Numbering for list items inside table cells, continuing across cells.
    list_counters: ListCounters,
}

/// Pages' page-break character, which opens the paragraph after the break.
const PAGE_BREAK: char = '\u{5}';

impl Walk {
    fn blocks(&mut self, document: &Document, blocks: &[Block]) {
        for block in blocks {
            match block {
                Block::Paragraph(paragraph) => self.paragraph(document, paragraph),
                Block::Table(table) => self.table(document, table),
            }
        }
    }

    /// A table anchors as one paragraph holding a single `U+FFFC` run; its
    /// cells are collected as plain text for the synthesized table objects.
    fn table(&mut self, document: &Document, table: &crate::document::Table) {
        if table.rows.is_empty() || table.columns.is_empty() {
            return;
        }
        // Pages cannot break a table row across pages: a row taller than the
        // page is clipped. Such a table (often a whole section wrapped in one
        // layout cell) is unwrapped into body content, row by row, where Pages
        // paginates freely; tables nested in it become tables in their own right.
        let page = document
            .sections
            .first()
            .map(|section| section.page.clone())
            .unwrap_or_default();
        let body_height = (page.height - page.margin_top - page.margin_bottom).max(144.0);
        // A table wider than the text area is shrunk to fit it, as Pages would
        // draw it anyway; its rows are estimated at those widths.
        let text_width = (page.width - page.margin_left - page.margin_right).max(72.0);
        let total: f32 = table.columns.iter().sum();
        let scale = if total > text_width {
            text_width / total
        } else {
            1.0
        };
        let widths: Vec<f32> = table.columns.iter().map(|width| width * scale).collect();
        if max_row_height(document, table, &widths) > body_height * 1.1 {
            for row in &table.rows {
                for cell in &row.cells {
                    if cell.merge == crate::document::Merge::Origin {
                        self.blocks(document, &cell.blocks);
                    }
                }
            }
            return;
        }
        if !self.paragraphs.is_empty() {
            self.mark(Format::default());
            self.link_mark(None);
            self.text.push('\n');
            self.offset += 1;
        }
        self.paragraphs.push(ParagraphMark {
            offset: self.offset,
            style_name: None,
            list: None,
            format: ParaFormat::default(),
            contextual: false,
        });
        self.take_break();
        let columns = table.columns.len();
        let rows = table.rows.len();
        let mut cells: Vec<CellContent> = (0..rows * columns)
            .map(|_| CellContent::default())
            .collect();
        for (r, row) in table.rows.iter().enumerate() {
            for (c, cell) in row.cells.iter().enumerate().take(columns) {
                cells[r * columns + c] = flatten_cell(document, cell, &mut self.list_counters);
            }
        }
        let mut backgrounds = vec![None; rows * columns];
        let mut cell_borders = vec![crate::document::CellBorders::default(); rows * columns];
        let mut alignments = vec![ALIGN_UNSTATED; rows * columns];
        for (r, row) in table.rows.iter().enumerate() {
            for (c, cell) in row.cells.iter().enumerate().take(columns) {
                backgrounds[r * columns + c] = cell.background;
                cell_borders[r * columns + c] = cell.borders;
                alignments[r * columns + c] = match cell.vertical_alignment {
                    Some(crate::document::VerticalAlignment::Top) => 0,
                    Some(crate::document::VerticalAlignment::Center) => 1,
                    Some(crate::document::VerticalAlignment::Bottom) => 2,
                    None => ALIGN_UNSTATED,
                };
            }
        }
        let mut merges = Vec::new();
        for (r, row) in table.rows.iter().enumerate() {
            for (c, cell) in row.cells.iter().enumerate().take(columns) {
                let span_rows = (cell.row_span.max(1) as usize).min(rows - r);
                let span_columns = (cell.column_span.max(1) as usize).min(columns - c);
                if cell.merge == crate::document::Merge::Origin
                    && (span_rows > 1 || span_columns > 1)
                {
                    merges.push((r, c, span_rows, span_columns));
                }
            }
        }
        // A row Word leaves to its content starts one body line tall (plus the
        // cell margins); Pages grows it to fit taller content, as Word does.
        let (pad_top, pad_bottom) = table
            .cell_margins
            .map_or((TEMPLATE_CELL_PADDING, TEMPLATE_CELL_PADDING), |margins| {
                (margins.top, margins.bottom)
            });
        let line = default_font_size(document) * 1.2 + pad_top + pad_bottom;
        let heights = table
            .rows
            .iter()
            .map(|row| {
                // Word counts a cell's space before its first paragraph and
                // after its last in the row; Pages lays neither out in a cell.
                let spacing = row
                    .cells
                    .iter()
                    .filter(|cell| cell.merge == crate::document::Merge::Origin)
                    .map(|cell| {
                        let mut paragraphs = Vec::new();
                        collect_paragraphs(&cell.blocks, &mut paragraphs);
                        let before = paragraphs.first().map_or(0.0, |paragraph| {
                            document
                                .effective_paragraph(paragraph)
                                .space_before
                                .unwrap_or(0.0)
                        });
                        let after = paragraphs.last().map_or(0.0, |paragraph| {
                            document
                                .effective_paragraph(paragraph)
                                .space_after
                                .unwrap_or(0.0)
                        });
                        before.max(0.0) + after.max(0.0)
                    })
                    .fold(0.0f32, f32::max);
                let content = line + spacing;
                row.height
                    .filter(|height| *height > 0.0)
                    .map_or(content, |height| height.max(content))
            })
            .collect();
        self.tables.push(TableMark {
            offset: self.offset,
            rows,
            columns,
            header_rows: table.header_rows,
            cells,
            widths,
            heights,
            merges,
            backgrounds,
            alignments,
            borders: table.borders,
            cell_margins: table.cell_margins,
            cell_borders,
        });
        self.mark(Format::default());
        self.link_mark(None);
        self.text.push(ATTACHMENT);
        self.offset += 1;
    }

    fn paragraph(&mut self, document: &Document, paragraph: &Paragraph) {
        // A paragraph holding only a page break becomes the break character
        // opening the next paragraph, as Pages writes it.
        let live: Vec<&crate::document::Run> = paragraph
            .runs
            .iter()
            .filter(|run| !document.is_deleted(run))
            .collect();
        if !live.is_empty()
            && live
                .iter()
                .all(|run| matches!(run.content, Inline::PageBreak))
        {
            self.pending_break = true;
            return;
        }
        if !self.paragraphs.is_empty() {
            self.mark(Format::default());
            self.link_mark(None);
            self.text.push('\n');
            self.offset += 1;
        }
        let style_name = paragraph
            .style
            .map(|style| document.styles.paragraph[style].name.clone());
        self.paragraphs.push(ParagraphMark {
            offset: self.offset,
            style_name,
            list: paragraph.list,
            format: para_format(document, paragraph),
            contextual: document
                .effective_paragraph(paragraph)
                .contextual_spacing
                .unwrap_or(false),
        });
        self.take_break();
        for run in &paragraph.runs {
            if document.is_deleted(run) {
                continue;
            }
            if let Inline::Image(id) = run.content {
                if let Some(image) = document.image(id) {
                    self.image(image);
                }
                continue;
            }
            let raw = match run.content {
                Inline::Text(span) => document.text(span),
                Inline::Tab => "\t",
                Inline::LineBreak => "\u{2028}",
                Inline::PageBreak => {
                    self.mark(Format::default());
                    self.link_mark(None);
                    self.text.push(PAGE_BREAK);
                    self.offset += 1;
                    self.continue_paragraph();
                    continue;
                }
                _ => continue,
            };
            let piece = without_attachments(raw);
            if piece.is_empty() {
                continue;
            }
            self.mark(run_format(document, paragraph, run));
            self.link_mark(run.link);
            self.text.push_str(&piece);
            self.offset += utf16_len(&piece);
        }
    }

    /// Writes a pending page break at the current (paragraph-start) offset.
    fn take_break(&mut self) {
        if self.pending_break {
            self.pending_break = false;
            self.mark(Format::default());
            self.link_mark(None);
            self.text.push(PAGE_BREAK);
            self.offset += 1;
            self.continue_paragraph();
        }
    }

    /// The page-break character ends a paragraph in Pages' text model, so
    /// what follows it is a new paragraph and needs its own style entry (the
    /// same style as the one it continues); without it Pages discards every
    /// paragraph style of the storage on load.
    fn continue_paragraph(&mut self) {
        if let Some(last) = self.paragraphs.last() {
            let mark = ParagraphMark {
                offset: self.offset,
                style_name: last.style_name.clone(),
                list: last.list,
                format: last.format,
                contextual: last.contextual,
            };
            self.paragraphs.push(mark);
        }
    }

    /// An inline image anchors as one `U+FFFC` run in the text flow (unlike a
    /// table, it needs no paragraph of its own). Its bytes and size are
    /// recorded for the rewrite that carries them into a reused template image.
    fn image(&mut self, image: &crate::document::InlineImage) {
        self.mark(Format::default());
        self.link_mark(None);
        self.images.push(ImageMark {
            offset: self.offset,
            media: image.media,
            width: image.width,
            height: image.height,
            description: image.description.clone(),
        });
        self.text.push(ATTACHMENT);
        self.offset += 1;
    }

    fn mark(&mut self, format: Format) {
        if format != self.current {
            self.char_marks.push(CharMark {
                offset: self.offset,
                format,
            });
            self.current = format;
        }
    }

    fn link_mark(&mut self, link: Option<Id>) {
        if link != self.current_link {
            self.link_marks.push(LinkMark {
                offset: self.offset,
                link,
            });
            self.current_link = link;
        }
    }
}

/// A table cell's text: its paragraphs' runs joined, paragraphs separated by
/// a newline. Only text runs contribute; nested tables and other inlines are
/// skipped (an MVP that carries the words).
fn flatten_cell(
    document: &Document,
    cell: &crate::document::Cell,
    counters: &mut ListCounters,
) -> CellContent {
    flatten_lines(document, &block_lines(&cell.blocks), counters)
}

/// One line of flattened text: segments joined by tabs, each segment's
/// paragraphs joined by soft line breaks.
type Line<'a> = Vec<Vec<&'a Paragraph>>;

/// The lines a run of blocks flattens to. A paragraph is a line of its own; a
/// table (Pages has no tables inside table cells or headers) becomes one line
/// per row with its cells separated by tabs, so its text and reading order
/// survive. A cell's content, nested tables included, collapses into one
/// segment.
fn block_lines(blocks: &[Block]) -> Vec<Line<'_>> {
    let mut lines = Vec::new();
    for block in blocks {
        match block {
            Block::Paragraph(paragraph) => lines.push(vec![vec![paragraph]]),
            Block::Table(table) => {
                for row in &table.rows {
                    let segments: Line<'_> = row
                        .cells
                        .iter()
                        .filter(|cell| cell.merge == crate::document::Merge::Origin)
                        .map(|cell| {
                            let mut paragraphs = Vec::new();
                            collect_paragraphs(&cell.blocks, &mut paragraphs);
                            paragraphs
                        })
                        .collect();
                    if segments.iter().any(|segment| !segment.is_empty()) {
                        lines.push(segments);
                    }
                }
            }
        }
    }
    lines
}

/// Every paragraph in `blocks`, depth first through nested tables.
fn collect_paragraphs<'a>(blocks: &'a [Block], out: &mut Vec<&'a Paragraph>) {
    for block in blocks {
        match block {
            Block::Paragraph(paragraph) => out.push(paragraph),
            Block::Table(table) => {
                for row in &table.rows {
                    for cell in &row.cells {
                        collect_paragraphs(&cell.blocks, out);
                    }
                }
            }
        }
    }
}

/// Flattens lines (see `block_lines`) into one text with its formatting runs:
/// lines end with a paragraph break, segments with a tab, and a segment's
/// paragraphs with a soft line break.
/// List numbering for text Pages holds without list styles (table cells,
/// headers, text boxes): each (list style, level)'s current number.
#[derive(Default)]
struct ListCounters {
    counts: HashMap<(usize, u8), u32>,
}

impl ListCounters {
    /// The label a list paragraph shows, advancing its list's count.
    fn label(&mut self, document: &Document, item: &crate::document::ListItem) -> Option<String> {
        if item.starts_list {
            self.counts.retain(|(style, _), _| *style != item.style);
        }
        // An item ends the numbering of the levels below it.
        self.counts
            .retain(|(style, level), _| *style != item.style || *level <= item.level);
        let count = self
            .counts
            .entry((item.style, item.level))
            .and_modify(|count| *count += 1)
            .or_insert(if item.starts_list {
                item.start.max(1)
            } else {
                1
            });
        let level = document
            .styles
            .list
            .get(item.style)?
            .levels
            .get(item.level as usize)?;
        let count = *count;
        match &level.label {
            crate::document::ListLabel::None => None,
            crate::document::ListLabel::Text(text) => Some(text.clone()),
            crate::document::ListLabel::Number(format) if format.tiered => {
                // Its parents' numbers lead: 1.2.3.
                let style = document.styles.list.get(item.style)?;
                let mut label = String::new();
                for parent in 0..item.level {
                    let number = self.counts.get(&(item.style, parent)).copied().unwrap_or(1);
                    let kind = match style.levels.get(parent as usize).map(|level| &level.label) {
                        Some(crate::document::ListLabel::Number(parent_format)) => {
                            parent_format.kind
                        }
                        _ => crate::document::NumberKind::Decimal,
                    };
                    label.push_str(&kind.format(number));
                    label.push('.');
                }
                Some(label + &format.label(count))
            }
            crate::document::ListLabel::Number(format) => Some(format.label(count)),
        }
        .filter(|label| !label.is_empty())
    }
}

fn flatten_lines(
    document: &Document,
    lines: &[Line<'_>],
    counters: &mut ListCounters,
) -> CellContent {
    let mut text = String::new();
    let mut marks: Vec<(u32, Format)> = Vec::new();
    let mut fields: Vec<(u32, u64)> = Vec::new();
    let mut starts: Vec<(u32, ParaFormat)> = Vec::new();
    let mut current = Format::default();
    let mut offset = 0u32;
    let separator = |text: &mut String,
                     offset: &mut u32,
                     marks: &mut Vec<(u32, Format)>,
                     current: &mut Format,
                     character: char| {
        if *current != Format::default() {
            marks.push((*offset, Format::default()));
            *current = Format::default();
        }
        text.push(character);
        *offset += 1;
    };
    let paragraphs = lines.iter().enumerate().flat_map(|(line_index, line)| {
        line.iter()
            .enumerate()
            .flat_map(move |(segment_index, segment)| {
                segment
                    .iter()
                    .enumerate()
                    .map(move |(paragraph_index, paragraph)| {
                        let lead = if paragraph_index > 0 {
                            Some('\u{2028}')
                        } else if segment_index > 0 {
                            Some('\t')
                        } else if line_index > 0 {
                            Some('\n')
                        } else {
                            None
                        };
                        (lead, *paragraph)
                    })
            })
    });
    for (lead, paragraph) in paragraphs {
        if let Some(character) = lead {
            separator(&mut text, &mut offset, &mut marks, &mut current, character);
        }
        // A line (not a tab or soft break) begins a Pages paragraph.
        if matches!(lead, None | Some('\n')) {
            starts.push((offset, para_format(document, paragraph)));
        }
        // A list item's label, as text: Pages keeps no list styles here.
        if let Some(label) = paragraph
            .list
            .as_ref()
            .and_then(|item| counters.label(document, item))
        {
            let format = paragraph
                .runs
                .iter()
                .find(|run| !document.is_deleted(run))
                .map_or(current, |run| run_format(document, paragraph, run));
            if format != current {
                marks.push((offset, format));
                current = format;
            }
            let piece = format!("{} ", without_attachments(&label));
            text.push_str(&piece);
            offset += utf16_len(&piece);
        }
        for run in &paragraph.runs {
            if document.is_deleted(run) {
                continue;
            }
            let format = run_format(document, paragraph, run);
            let field = match run.content {
                Inline::PageNumber => Some(0),
                Inline::PageCount => Some(1),
                _ => None,
            };
            if let Some(kind) = field {
                if format != current {
                    marks.push((offset, format));
                    current = format;
                }
                fields.push((offset, kind));
                text.push(ATTACHMENT);
                offset += 1;
                continue;
            }
            let raw = match run.content {
                Inline::Text(span) => document.text(span),
                Inline::Tab => "\t",
                Inline::LineBreak => "\u{2028}",
                _ => continue,
            };
            let piece = without_attachments(raw);
            if piece.is_empty() {
                continue;
            }
            if format != current {
                marks.push((offset, format));
                current = format;
            }
            text.push_str(&piece);
            offset += utf16_len(&piece);
        }
    }
    CellContent {
        text,
        marks,
        fields,
        paragraphs: starts,
    }
}

/// A run's own bold and italic, independent of the paragraph.
fn run_format(document: &Document, paragraph: &Paragraph, run: &crate::document::Run) -> Format {
    // The run's effective formatting: the paragraph style's, overlaid with the
    // character style's, overlaid with the run's own — so a font, size, or
    // colour inherited from a named style reaches the run and is carried.
    let properties = document.effective_run(paragraph, run);
    Format {
        bold: properties.bold.unwrap_or(false),
        italic: properties.italic.unwrap_or(false),
        underline: properties.underline.unwrap_or(false),
        size: properties
            .size
            .map(|points| (points * 2.0).round().clamp(1.0, 65535.0) as u16),
        font: properties.font,
        color: properties.color,
    }
}

/// Maps bold and italic formatting to the character styles the template's own
/// body already uses, so a run can point at one by reference. These are the
/// theme's live emphasis variations — the styles Pages renders and the body
/// references — not the stylesheet's like-named definitions, which open but do
/// not render (the same live-versus-preset split as the list styles).
fn collect_char_formats(package: &Package) -> HashMap<Format, u64> {
    let mut formats = HashMap::new();
    let Some((tree, body_first)) = body_storage_message(package) else {
        return formats;
    };
    let Some(table) = message_field(tree, body_first, "table_char_style") else {
        return formats;
    };
    for object_id in table_object_refs(tree, table) {
        if let Some(format) = char_format_of(package, object_id) {
            formats.entry(format).or_insert(object_id);
        }
    }
    formats
}

/// The bold/italic formatting a character style carries, or `None` if it is
/// plain or carries any other visual override (a colored preset like "Red
/// Bold" or a heading style is not the plain emphasis the writer reuses).
fn char_format_of(package: &Package, id: u64) -> Option<Format> {
    let (tree, first) = object_message(package, id)?;
    let properties = message_field(tree, first, "char_properties")?;
    let mut format = Format::default();
    let mut has_other = false;
    for (_, field_entry) in tree.chain(properties) {
        match tree.field(field_entry).map(|field| field.name) {
            Some("bold") => format.bold = matches!(field_entry.value, Node::Bool(true)),
            Some("italic") => format.italic = matches!(field_entry.value, Node::Bool(true)),
            // The bold or italic font name is the weighted face and is welcome;
            // every other visual property disqualifies it.
            Some("font_name")
            | Some("compatibility_font_name_null")
            | Some("tsd_fill_should_fill_text_container") => {}
            Some(_) => has_other = true,
            None => {}
        }
    }
    (!format.is_plain() && !has_other).then_some(format)
}

/// The object identifiers an attribute table's entries point at, in order.
fn table_object_refs(tree: &Tree, table_first: u32) -> Vec<u64> {
    let mut refs = Vec::new();
    for (_, entry) in tree.chain(table_first) {
        if tree.field(entry).map(|field| field.name) == Some("entries")
            && let Node::Message(entry_first) = entry.value
            && let Some(Node::Reference(id)) = field_value(tree, entry_first, "object")
        {
            refs.push(id);
        }
    }
    refs
}

/// The tree and first message chain of the document's body storage (what
/// `DocumentArchive.body_storage` points at), read-only.
fn body_storage_message(package: &Package) -> Option<(&Tree, u32)> {
    for entry in &package.entries {
        let Entry::Stream(stream) = entry else {
            continue;
        };
        let Some(object) = stream
            .objects
            .iter()
            .find(|object| first_type(object) == Some(DOCUMENT_ARCHIVE))
        else {
            continue;
        };
        let first = object.messages.first()?.first;
        if let Some(Node::Reference(body_id)) = field_value(&stream.tree, first, "body_storage") {
            return object_message(package, body_id);
        }
    }
    None
}

fn utf16_len(text: &str) -> u32 {
    text.chars()
        .map(|character| character.len_utf16() as u32)
        .sum()
}

/// Drops object-replacement characters (`U+FFFC`) that a text run carries on
/// its own — an image or drawing the reader left in the text but that has no
/// drawable of its own here. Pages crashes laying out an attachment character
/// with no attachment, so such orphans must not reach the text flow. Real
/// inline images anchor through their own path and never come through here.
fn without_attachments(piece: &str) -> std::borrow::Cow<'_, str> {
    if piece.contains(ATTACHMENT) {
        std::borrow::Cow::Owned(
            piece
                .chars()
                .filter(|character| *character != ATTACHMENT)
                .collect(),
        )
    } else {
        std::borrow::Cow::Borrowed(piece)
    }
}

/// The template style identifier for a model style name, mapped to Pages'
/// own names, falling back to Body.
fn style_identifier(
    _document: &Document,
    styles: &HashMap<String, u64>,
    name: Option<&str>,
) -> u64 {
    let pages_name = name.map_or("Body", pages_style_name);
    styles
        .get(pages_name)
        .or_else(|| styles.get("Body"))
        .copied()
        .unwrap_or(0)
}

/// The Pages built-in style name for a model paragraph-style name.
fn pages_style_name(name: &str) -> &'static str {
    match name {
        "Title" => "Title",
        "Subtitle" => "Subtitle",
        "Heading 1" | "Heading" => "Heading",
        "Heading 2" => "Heading 2",
        "Heading 3" | "Heading 4" | "Heading 5" | "Heading 6" => "Heading 3",
        "Caption" => "Caption",
        "Footnote" => "Footnote",
        _ => "Body",
    }
}

// ----- tree building -----

/// Builds an object-attribute table (`character_index` to an object) and
/// adds it as `field_name` of the parent message.
fn emit_reference_table(
    tree: &mut Tree,
    chain: &mut Chain,
    parent: MessageRef,
    field_name: &str,
    entries: &[(u32, Option<u64>)],
) -> Result<(), PackageError> {
    let (slot, field) = parent
        .slot_named(field_name)
        .ok_or_else(|| malformed("table field is not in the schema"))?;
    let table = message_of(field.kind)?;
    let (entries_slot, entries_field) = table
        .slot_named("entries")
        .ok_or_else(|| malformed("attribute table has no entries field"))?;
    let entry = message_of(entries_field.kind)?;

    let mut table_chain = Chain::new();
    for (character_index, identifier) in entries {
        let mut entry_chain = Chain::new();
        push_field(
            tree,
            &mut entry_chain,
            entry,
            "character_index",
            Node::Uint(*character_index as u64),
        )?;
        if let Some(identifier) = identifier {
            push_field(
                tree,
                &mut entry_chain,
                entry,
                "object",
                Node::Reference(*identifier),
            )?;
        }
        tree.push_known(
            &mut table_chain,
            table,
            entries_slot,
            entries_field,
            entries_field.number,
            Node::Message(entry_chain.first),
        )
        .map_err(tree_error)?;
    }
    tree.push_known(
        chain,
        parent,
        slot,
        field,
        field.number,
        Node::Message(table_chain.first),
    )
    .map_err(tree_error)?;
    Ok(())
}

/// Emits an attribute table whose entries carry two integers (`first` and
/// `second`) rather than an object reference — the shape of `table_para_data`
/// (level, list-start flag) and `table_para_starts` (list-start number).
fn emit_data_table(
    tree: &mut Tree,
    chain: &mut Chain,
    parent: MessageRef,
    field_name: &str,
    entries: &[(u32, u64, u64)],
) -> Result<(), PackageError> {
    let (slot, field) = parent
        .slot_named(field_name)
        .ok_or_else(|| malformed("table field is not in the schema"))?;
    let table = message_of(field.kind)?;
    let (entries_slot, entries_field) = table
        .slot_named("entries")
        .ok_or_else(|| malformed("attribute table has no entries field"))?;
    let entry = message_of(entries_field.kind)?;

    let mut table_chain = Chain::new();
    for (character_index, first, second) in entries {
        let mut entry_chain = Chain::new();
        push_field(
            tree,
            &mut entry_chain,
            entry,
            "character_index",
            Node::Uint(*character_index as u64),
        )?;
        push_field(tree, &mut entry_chain, entry, "first", Node::Uint(*first))?;
        push_field(tree, &mut entry_chain, entry, "second", Node::Uint(*second))?;
        tree.push_known(
            &mut table_chain,
            table,
            entries_slot,
            entries_field,
            entries_field.number,
            Node::Message(entry_chain.first),
        )
        .map_err(tree_error)?;
    }
    tree.push_known(
        chain,
        parent,
        slot,
        field,
        field.number,
        Node::Message(table_chain.first),
    )
    .map_err(tree_error)?;
    Ok(())
}

/// The default point size for a row with no explicit height.
const DEFAULT_ROW_HEIGHT: f32 = 20.0;

/// A `stringTable` data list (listType 1): one entry per cell, keyed by cell
/// index + 1, holding the cell's text.
fn build_string_list(tree: &mut Tree, cells: &[CellContent]) -> Result<u32, PackageError> {
    let data_list = message_ref("TST.TableDataList")?;
    let entry_ref = message_ref("TST.TableDataList.ListEntry")?;
    let mut chain = Chain::new();
    push_field(tree, &mut chain, data_list, "listType", Node::Uint(1))?;
    push_field(
        tree,
        &mut chain,
        data_list,
        "nextListID",
        Node::Uint(cells.len() as u64 + 1),
    )?;
    for (index, cell) in cells.iter().enumerate() {
        let mut entry = Chain::new();
        push_field(
            tree,
            &mut entry,
            entry_ref,
            "key",
            Node::Uint(index as u64 + 1),
        )?;
        push_field(tree, &mut entry, entry_ref, "refcount", Node::Uint(1))?;
        let span = tree.push_bytes(cell.text.as_bytes()).map_err(tree_error)?;
        push_field(tree, &mut entry, entry_ref, "string", Node::Str(span))?;
        push_field(
            tree,
            &mut chain,
            data_list,
            "entries",
            Node::Message(entry.first),
        )?;
    }
    Ok(chain.first)
}

/// The style objects a rich cell storage needs to reference for plain content:
/// a base paragraph style, a "None" list style, and a base character style.
#[derive(Clone, Copy)]
struct CellStyles {
    stylesheet: u64,
    paragraph: u64,
    list: u64,
    base_char: u64,
}

/// Builds a `TSWP.StorageArchive` of `kind` (5 a table cell, 1 a header or
/// footer) holding `content`, its paragraphs in `styles.paragraph`, its runs'
/// character styles, and each field's attachment object at its offset.
fn build_text_storage(
    tree: &mut Tree,
    cell: &CellContent,
    formats: &HashMap<Format, u64>,
    styles: CellStyles,
    kind: Option<u64>,
    attachments: &[(u32, u64)],
    paras: &ParaStyles,
) -> Result<(u32, Vec<u64>), PackageError> {
    let storage = message_ref("TSWP.StorageArchive")?;
    let mut chain = Chain::new();
    // A shape's text storage carries no kind, as Pages writes it.
    if let Some(kind) = kind {
        push_field(tree, &mut chain, storage, "kind", Node::Uint(kind))?;
    }
    push_field(
        tree,
        &mut chain,
        storage,
        "style_sheet",
        Node::Reference(styles.stylesheet),
    )?;
    let text_span = tree.push_bytes(cell.text.as_bytes()).map_err(tree_error)?;
    push_field(tree, &mut chain, storage, "text", Node::Str(text_span))?;
    push_field(tree, &mut chain, storage, "in_document", Node::Bool(true))?;

    let mut refs = vec![styles.paragraph, styles.list, styles.base_char];
    // Each paragraph points at its formatting's variation of the base style,
    // one entry per paragraph even where neighbours share a style: a storage
    // whose paragraph table skips a paragraph start is "repaired" by Pages,
    // which resets every paragraph to the default style.
    // (A trailing break starts an empty last paragraph at the very end.)
    let mut starts = vec![0u32];
    let mut offset = 0u32;
    for character in cell.text.chars() {
        offset += character.len_utf16() as u32;
        if character == '\n' {
            starts.push(offset);
        }
    }
    let paragraph_entries: Vec<(u32, Option<u64>)> = starts
        .into_iter()
        .map(|start| {
            let style = cell
                .paragraphs
                .iter()
                .rev()
                .find(|(offset, _)| *offset <= start)
                .and_then(|(_, format)| paras.get(&(styles.paragraph, *format)).copied())
                .unwrap_or(styles.paragraph);
            (start, Some(style))
        })
        .collect();
    for (_, id) in &paragraph_entries {
        if let Some(id) = id
            && !refs.contains(id)
        {
            refs.push(*id);
        }
    }
    emit_reference_table(
        tree,
        &mut chain,
        storage,
        "table_para_style",
        &paragraph_entries,
    )?;
    emit_reference_table(
        tree,
        &mut chain,
        storage,
        "table_list_style",
        &[(0, Some(styles.list))],
    )?;

    // The character table: each formatting run points at its style (a plain run
    // at the base), starting at offset 0.
    let mut char_entries: Vec<(u32, Option<u64>)> = cell
        .marks
        .iter()
        .map(|(offset, format)| {
            (
                *offset,
                char_style_for(*format, formats).or(Some(styles.base_char)),
            )
        })
        .collect();
    if char_entries.first().is_none_or(|(offset, _)| *offset != 0) {
        char_entries.insert(0, (0, Some(styles.base_char)));
    }
    // Empty text has no characters to style; an entry there is out of range.
    if cell.text.is_empty() {
        char_entries.clear();
    }
    for (_, id) in &char_entries {
        if let Some(id) = id
            && !refs.contains(id)
        {
            refs.push(*id);
        }
    }
    emit_reference_table(tree, &mut chain, storage, "table_char_style", &char_entries)?;
    if !attachments.is_empty() {
        let entries: Vec<(u32, Option<u64>)> = attachments
            .iter()
            .map(|(offset, id)| (*offset, Some(*id)))
            .collect();
        emit_reference_table(tree, &mut chain, storage, "table_attachment", &entries)?;
        refs.extend(attachments.iter().map(|(_, id)| *id));
    }
    // The per-paragraph tables Pages expects on a cell storage.
    emit_data_table(tree, &mut chain, storage, "table_para_data", &[(0, 0, 0)])?;
    emit_data_table(tree, &mut chain, storage, "table_para_starts", &[(0, 0, 0)])?;
    emit_data_table(tree, &mut chain, storage, "table_para_bidi", &[(0, 0, 0)])?;
    // A shape's text takes drop caps: Pages expects its (empty) drop-cap table
    // and adds one on load, as a modification, when it is missing.
    if kind.is_none() {
        emit_reference_table(
            tree,
            &mut chain,
            storage,
            "table_drop_cap_style",
            &[(0, None)],
        )?;
    }
    Ok((chain.first, refs))
}

/// The message type of `parent`'s field called `name`.
fn child_message(parent: MessageRef, name: &str) -> Result<MessageRef, PackageError> {
    let (_, field) = parent
        .slot_named(name)
        .ok_or_else(|| malformed("field is not in the schema"))?;
    message_of(field.kind)
}

/// Builds a `TST.RichTextPayloadArchive` pointing at a cell's storage, with the
/// unassigned cell-id sentinel Pages writes.
fn build_rich_payload(tree: &mut Tree, storage_id: u64) -> Result<u32, PackageError> {
    let payload = message_ref("TST.RichTextPayloadArchive")?;
    let cell_id = child_message(payload, "cellid")?;
    let coord = child_message(cell_id, "expanded_coord")?;
    let mut chain = Chain::new();
    push_field(
        tree,
        &mut chain,
        payload,
        "storage",
        Node::Reference(storage_id),
    )?;
    let mut id_chain = Chain::new();
    push_field(
        tree,
        &mut id_chain,
        cell_id,
        "packedData",
        Node::Fixed32(16_777_215),
    )?;
    let mut coord_chain = Chain::new();
    push_field(tree, &mut coord_chain, coord, "column", Node::Uint(32_767))?;
    push_field(
        tree,
        &mut coord_chain,
        coord,
        "row",
        Node::Uint(2_147_483_647),
    )?;
    push_field(
        tree,
        &mut id_chain,
        cell_id,
        "expanded_coord",
        Node::Message(coord_chain.first),
    )?;
    push_field(
        tree,
        &mut chain,
        payload,
        "cellid",
        Node::Message(id_chain.first),
    )?;
    Ok(chain.first)
}

/// Rebuilds the table's rich-text data list (listType 8): one entry per rich
/// cell, keyed and pointing at its payload.
fn build_rich_text_list(tree: &mut Tree, entries: &[(u32, u64)]) -> Result<u32, PackageError> {
    let data_list = message_ref("TST.TableDataList")?;
    let entry_ref = message_ref("TST.TableDataList.ListEntry")?;
    let mut chain = Chain::new();
    push_field(tree, &mut chain, data_list, "listType", Node::Uint(8))?;
    push_field(
        tree,
        &mut chain,
        data_list,
        "nextListID",
        Node::Uint(entries.len() as u64 + 1),
    )?;
    for (key, payload) in entries {
        let mut entry = Chain::new();
        push_field(
            tree,
            &mut entry,
            entry_ref,
            "key",
            Node::Uint(u64::from(*key)),
        )?;
        push_field(tree, &mut entry, entry_ref, "refcount", Node::Uint(1))?;
        push_field(
            tree,
            &mut entry,
            entry_ref,
            "rich_text_payload",
            Node::Reference(*payload),
        )?;
        push_field(
            tree,
            &mut chain,
            data_list,
            "entries",
            Node::Message(entry.first),
        )?;
    }
    Ok(chain.first)
}

/// Cell-style variations by (parent cell style, fill), shared by all tables.
/// A cell's look in Pages: its fill and its vertical alignment code.
type CellLook = (Option<crate::document::Color>, u64);

/// Cell style variations by (parent, look, padding as bits).
type CellFillStyles = HashMap<(u64, CellLook, [u32; 4]), u64>;

/// Pages' vertical alignment for a cell whose source states none (as Pages
/// writes a Word cell without one; it lays out at the top).
const ALIGN_UNSTATED: u64 = 3;

/// Rebuilds the table's style data list (listType 4) from (key, style)
/// entries: key 1 the paragraph (text) style every cell uses, then the cell
/// styles cell records name in their `cell_style` field.
fn build_style_list(tree: &mut Tree, entries: &[(u32, u64, u64)]) -> Result<u32, PackageError> {
    let data_list = message_ref("TST.TableDataList")?;
    let entry_ref = message_ref("TST.TableDataList.ListEntry")?;
    let mut chain = Chain::new();
    push_field(tree, &mut chain, data_list, "listType", Node::Uint(4))?;
    push_field(
        tree,
        &mut chain,
        data_list,
        "nextListID",
        Node::Uint(entries.len() as u64 + 1),
    )?;
    for (key, style, refcount) in entries {
        let mut entry = Chain::new();
        push_field(
            tree,
            &mut entry,
            entry_ref,
            "key",
            Node::Uint(u64::from(*key)),
        )?;
        push_field(
            tree,
            &mut entry,
            entry_ref,
            "refcount",
            Node::Uint(*refcount),
        )?;
        push_field(
            tree,
            &mut entry,
            entry_ref,
            "reference",
            Node::Reference(*style),
        )?;
        push_field(
            tree,
            &mut chain,
            data_list,
            "entries",
            Node::Message(entry.first),
        )?;
    }
    Ok(chain.first)
}

/// Creates, in the document stream, a `TST.CellStyleArchive` variation of
/// `parent` per fill (the colour, or an empty fill for none), with the
/// vertical alignment and padding Pages gives imported Word cells, and
/// registers them with the stylesheet.
fn create_cell_styles(
    package: &mut Package,
    parent: u64,
    looks: &[CellLook],
    padding_sides: [f32; 4],
    stylesheet: u64,
    next_id: &mut u64,
) -> Result<HashMap<CellLook, u64>, PackageError> {
    let archive = message_ref("TST.CellStyleArchive")?;
    let base = child_message(archive, "super")?;
    let properties = child_message(archive, "cell_properties")?;
    let fill = child_message(properties, "cell_fill")?;
    let padding = child_message(properties, "padding")?;
    let mut out = HashMap::new();
    // In the stylesheet's own stream, where Pages keeps style variations; one
    // elsewhere is swapped out on load (and a table's cell references with it).
    let stream = stream_containing(package, stylesheet)?;
    for &(color, alignment) in looks {
        let id = *next_id;
        *next_id += 1;
        let tree = &mut stream.tree;
        let mut style = Chain::new();
        push_field(tree, &mut style, base, "parent", Node::Reference(parent))?;
        push_field(tree, &mut style, base, "is_variation", Node::Bool(true))?;
        push_field(
            tree,
            &mut style,
            base,
            "stylesheet",
            Node::Reference(stylesheet),
        )?;
        let mut fill_chain = Chain::new();
        if let Some(color) = color {
            let color_first = build_color(tree, color)?;
            push_field(
                tree,
                &mut fill_chain,
                fill,
                "color",
                Node::Message(color_first),
            )?;
        }
        let mut pad = Chain::new();
        for (side, amount) in ["left", "top", "right", "bottom"]
            .into_iter()
            .zip(padding_sides)
        {
            push_field(tree, &mut pad, padding, side, Node::Float(amount))?;
        }
        let mut props = Chain::new();
        // No fill is no fill override: an empty one is redundant, and Pages'
        // clean-up of redundant overrides drops the style's references.
        if color.is_some() {
            push_field(
                tree,
                &mut props,
                properties,
                "cell_fill",
                Node::Message(fill_chain.first),
            )?;
        }
        // Top is the body cell style's own alignment: stating it again is a
        // redundant override, which Pages cleans up on load (as it does an
        // empty fill), so it is left out, as Pages writes a top-aligned cell.
        if alignment != 0 {
            push_field(
                tree,
                &mut props,
                properties,
                "vertical_alignment",
                Node::Uint(alignment),
            )?;
        }
        push_field(
            tree,
            &mut props,
            properties,
            "padding",
            Node::Message(pad.first),
        )?;
        let mut chain = Chain::new();
        push_field(
            tree,
            &mut chain,
            archive,
            "super",
            Node::Message(style.first),
        )?;
        let overrides = 1 + u64::from(color.is_some()) + u64::from(alignment != 0);
        push_field(
            tree,
            &mut chain,
            archive,
            "override_count",
            Node::Uint(overrides),
        )?;
        push_field(
            tree,
            &mut chain,
            archive,
            "cell_properties",
            Node::Message(props.first),
        )?;
        let info = build_archive_info(tree, id, CELL_STYLE)?;
        add_object_references(tree, info, &[parent, stylesheet])?;
        stream.objects.push(Object {
            identifier: id,
            info,
            messages: vec![ObjectMessage {
                message_type: CELL_STYLE,
                first: chain.first,
            }],
        });
        out.insert((color, alignment), id);
    }
    let pairs: Vec<(u64, u64)> = out.values().map(|id| (parent, *id)).collect();
    register_in_stylesheet(package, stylesheet, &pairs)?;
    Ok(out)
}

/// Adds object references to an object's `ArchiveInfo`, wherever it lives.
fn add_object_refs(package: &mut Package, id: u64, refs: &[u64]) -> Result<(), PackageError> {
    for entry in &mut package.entries {
        if let Entry::Stream(stream) = entry
            && let Some(info) = stream
                .objects
                .iter()
                .find(|object| object.identifier == id)
                .map(|object| object.info)
        {
            return add_object_references(&mut stream.tree, info, refs);
        }
    }
    Ok(())
}

/// Synthesises a character style in the document stream for each distinct
/// direct formatting in `needed`, returning the style map (the base map plus
/// the new styles) and the new style ids (to keep in the scope).
fn synthesize_char_styles(
    package: &mut Package,
    document: &Document,
    base: &HashMap<Format, u64>,
    needed: impl Iterator<Item = Format>,
    parent: u64,
    stylesheet: u64,
    next_id: &mut u64,
) -> Result<(HashMap<Format, u64>, Vec<u64>), PackageError> {
    let mut all = base.clone();
    let mut refs: Vec<u64> = Vec::new();
    let mut objects: Vec<Object> = Vec::new();
    let stream = document_stream(package)?;
    for format in needed {
        if all.contains_key(&format) {
            continue;
        }
        let id = *next_id;
        *next_id += 1;
        let (message, info) =
            build_char_style(&mut stream.tree, id, format, document, parent, stylesheet)?;
        objects.push(Object {
            identifier: id,
            info,
            messages: vec![ObjectMessage {
                message_type: CHARACTER_STYLE,
                first: message,
            }],
        });
        all.insert(format, id);
        refs.push(id);
    }
    stream.objects.extend(objects);
    Ok((all, refs))
}

/// A header bucket: one `Header{index, size, numberOfCells}` per column width
/// or row height.
fn build_header_bucket(
    tree: &mut Tree,
    sizes: &[f32],
    cross_count: u64,
) -> Result<u32, PackageError> {
    let bucket = message_ref("TST.HeaderStorageBucket")?;
    let header = message_ref("TST.HeaderStorageBucket.Header")?;
    let mut chain = Chain::new();
    push_field(
        tree,
        &mut chain,
        bucket,
        "bucketHashFunction",
        Node::Uint(1),
    )?;
    for (index, size) in sizes.iter().enumerate() {
        let mut entry = Chain::new();
        push_field(tree, &mut entry, header, "index", Node::Uint(index as u64))?;
        push_field(tree, &mut entry, header, "size", Node::Float(*size))?;
        push_field(tree, &mut entry, header, "hidingState", Node::Uint(0))?;
        push_field(
            tree,
            &mut entry,
            header,
            "numberOfCells",
            Node::Uint(cross_count),
        )?;
        push_field(
            tree,
            &mut chain,
            bucket,
            "headers",
            Node::Message(entry.first),
        )?;
    }
    Ok(chain.first)
}

/// A tile: one `TileRowInfo` per row, each with a packed cell buffer where
/// every cell is a plain-text record pointing at its stringTable key.
fn build_tile(
    tree: &mut Tree,
    mark: &TableMark,
    rich: &HashMap<usize, u32>,
    cell_styles: &HashMap<usize, u32>,
    styled: bool,
) -> Result<u32, PackageError> {
    let tile = message_ref("TST.Tile")?;
    let row_info = message_ref("TST.TileRowInfo")?;
    let mut chain = Chain::new();
    // Pages leaves these pre-BNC geometry fields at 0 in a BNC-saved tile; the
    // real geometry comes from the model dimensions and the row infos below.
    push_field(tree, &mut chain, tile, "maxColumn", Node::Uint(0))?;
    push_field(tree, &mut chain, tile, "maxRow", Node::Uint(0))?;
    push_field(tree, &mut chain, tile, "numCells", Node::Uint(0))?;
    push_field(
        tree,
        &mut chain,
        tile,
        "numrows",
        Node::Uint(mark.rows as u64),
    )?;
    push_field(tree, &mut chain, tile, "storage_version", Node::Uint(5))?;
    push_field(
        tree,
        &mut chain,
        tile,
        "last_saved_in_BNC",
        Node::Bool(true),
    )?;
    let covered = mark.covered();
    for row in 0..mark.rows {
        // Pages allocates a fixed 255-slot column offset array per row (510
        // bytes): the byte offset of each present column's record in the
        // buffer, then 0xFFFF for every empty column.
        let mut offsets = vec![0xFFu8; TILE_COLUMN_SLOTS * 2];
        let mut buffer: Vec<u8> = Vec::new();
        for column in 0..mark.columns {
            let cell = row * mark.columns + column;
            let key = cell as u32 + 1;
            let slot = column * 2;
            offsets[slot..slot + 2].copy_from_slice(&(buffer.len() as u16).to_le_bytes());
            let style = cell_styles.get(&cell).copied().unwrap_or(CELL_STYLE_KEY);
            if covered.contains(&cell) {
                buffer.extend_from_slice(&covered_record_bytes());
            } else if styled && !rich.contains_key(&cell) {
                buffer.extend_from_slice(&empty_record_bytes(style));
            } else {
                buffer.extend_from_slice(&cell_record_bytes(key, rich.get(&cell).copied(), style));
            }
        }
        let mut entry = Chain::new();
        push_field(
            tree,
            &mut entry,
            row_info,
            "tile_row_index",
            Node::Uint(row as u64),
        )?;
        push_field(
            tree,
            &mut entry,
            row_info,
            "cell_count",
            Node::Uint(mark.columns as u64),
        )?;
        // Pages needs both the legacy (pre-BNC) and current buffers; the same
        // storage-version-5 bytes serve for each.
        let buffer_span = tree.push_bytes(&buffer).map_err(tree_error)?;
        push_field(
            tree,
            &mut entry,
            row_info,
            "cell_storage_buffer_pre_bnc",
            Node::Bytes(buffer_span),
        )?;
        let offsets_span = tree.push_bytes(&offsets).map_err(tree_error)?;
        push_field(
            tree,
            &mut entry,
            row_info,
            "cell_offsets_pre_bnc",
            Node::Bytes(offsets_span),
        )?;
        push_field(tree, &mut entry, row_info, "storage_version", Node::Uint(5))?;
        push_field(
            tree,
            &mut entry,
            row_info,
            "cell_storage_buffer",
            Node::Bytes(buffer_span),
        )?;
        push_field(
            tree,
            &mut entry,
            row_info,
            "cell_offsets",
            Node::Bytes(offsets_span),
        )?;
        push_field(
            tree,
            &mut chain,
            tile,
            "rowInfos",
            Node::Message(entry.first),
        )?;
    }
    Ok(chain.first)
}

/// The record of a cell a merged region covers: an empty cell (kind 0) that
/// names only its text style, as Pages writes it; the region's text lives in
/// its origin cell.
fn covered_record_bytes() -> Vec<u8> {
    let mut bytes = vec![0u8; 16];
    bytes[0] = 5;
    bytes[8] = 0x40;
    bytes[12..16].copy_from_slice(&TEXT_STYLE_KEY.to_le_bytes());
    bytes
}

/// A storage-version-5 cell record. A rich cell (kind 9) is the 32-byte record
/// Pages writes: marker 5, kind 9, then the fields named by its flag word — the
/// rich_text key, the cell-style key, the text-style key, a constant, and the
/// number-format key. A plain cell (kind 3) is a 16-byte record naming a
/// stringTable key.
fn cell_record_bytes(string_key: u32, rich_key: Option<u32>, cell_style_key: u32) -> Vec<u8> {
    match rich_key {
        Some(rich) => {
            let mut bytes = vec![0u8; 32];
            bytes[0] = 5;
            bytes[1] = 9;
            // rich_text | cell_style | text_style | 0x1000 | text_format
            let flags: u32 = 0x10 | 0x20 | 0x40 | 0x1000 | 0x20000;
            bytes[8..12].copy_from_slice(&flags.to_le_bytes());
            bytes[12..16].copy_from_slice(&rich.to_le_bytes());
            bytes[16..20].copy_from_slice(&cell_style_key.to_le_bytes());
            bytes[20..24].copy_from_slice(&TEXT_STYLE_KEY.to_le_bytes());
            bytes[24..28].copy_from_slice(&5u32.to_le_bytes());
            bytes[28..32].copy_from_slice(&FORMAT_KEY.to_le_bytes());
            bytes
        }
        None => {
            let mut bytes = vec![0u8; 16];
            bytes[0] = 5;
            bytes[1] = 3;
            bytes[8] = 0x08;
            bytes[12..16].copy_from_slice(&string_key.to_le_bytes());
            bytes
        }
    }
}

/// An empty cell's record, as Pages writes one: no value, only its cell
/// and text styles.
fn empty_record_bytes(cell_style_key: u32) -> Vec<u8> {
    let mut bytes = vec![0u8; 20];
    bytes[0] = 5;
    // cell_style | text_style
    bytes[8..12].copy_from_slice(&(0x20u32 | 0x40).to_le_bytes());
    bytes[12..16].copy_from_slice(&cell_style_key.to_le_bytes());
    bytes[16..20].copy_from_slice(&TEXT_STYLE_KEY.to_le_bytes());
    bytes
}

/// Pushes a scalar or reference field by name onto a chain.
fn push_field(
    tree: &mut Tree,
    chain: &mut Chain,
    message: MessageRef,
    name: &str,
    value: Node,
) -> Result<(), PackageError> {
    let (slot, field) = message
        .slot_named(name)
        .ok_or_else(|| malformed("field is not in the schema"))?;
    // A signed field (int32/sint32) carries a Node::Int, which the encoder
    // writes as two's complement or ZigZag; an unsigned value passed for one
    // would otherwise be written raw and read back as a different number.
    let value = match (field.kind, value) {
        (
            crate::io::protobuf::schema::Kind::Int | crate::io::protobuf::schema::Kind::Sint,
            Node::Uint(unsigned),
        ) => Node::Int(unsigned as i64),
        (_, value) => value,
    };
    tree.push_known(chain, message, slot, field, field.number, value)
        .map_err(tree_error)?;
    Ok(())
}

// ----- small readers over a chain -----

fn field_value(tree: &Tree, first: u32, name: &str) -> Option<Node> {
    let mut cursor = first;
    while cursor != NONE {
        let entry = &tree.entries[cursor as usize];
        if tree.field(entry).is_some_and(|field| field.name == name) {
            return Some(entry.value);
        }
        cursor = entry.next;
    }
    None
}

/// The character style the template applies at offset 0 of its own body: the
/// base that plain text should keep. Read from the template's existing
/// `table_char_style` so it is a style the storage already declares.
fn template_base_char(tree: &Tree, storage_first: u32) -> Option<u64> {
    let table = message_field(tree, storage_first, "table_char_style")?;
    let entry = message_field(tree, table, "entries")?;
    match field_value(tree, entry, "object")? {
        Node::Reference(identifier) => Some(identifier),
        _ => None,
    }
}

fn message_field(tree: &Tree, first: u32, name: &str) -> Option<u32> {
    match field_value(tree, first, name)? {
        Node::Message(nested) => Some(nested),
        _ => None,
    }
}

fn str_field<'t>(tree: &'t Tree, first: u32, name: &str) -> Option<&'t str> {
    match field_value(tree, first, name)? {
        Node::Str(span) => Some(tree.str(span)),
        _ => None,
    }
}

fn first_type(object: &Object) -> Option<u32> {
    object.messages.first().map(|message| message.message_type)
}

fn message_ref(name: &str) -> Result<MessageRef, PackageError> {
    SCHEMA
        .message(name)
        .ok_or_else(|| malformed("schema message is missing"))
}

fn message_of(kind: crate::io::protobuf::schema::Kind) -> Result<MessageRef, PackageError> {
    match kind {
        crate::io::protobuf::schema::Kind::Message(index) => SCHEMA
            .message_at(index)
            .ok_or_else(|| malformed("nested message index out of range")),
        _ => Err(malformed("field is not a message")),
    }
}

fn tree_error(error: TreeError) -> PackageError {
    PackageError::Tree {
        stream: "writer".to_string(),
        error,
    }
}

fn malformed(what: &'static str) -> PackageError {
    PackageError::Malformed {
        stream: "writer".to_string(),
        what,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::pages::{Package, Scope, read_document};

    /// A document written to a package reads back with its paragraphs and
    /// their text intact, through the document reader the same way Pages
    /// would parse it.
    #[test]
    fn writes_a_document_the_reader_reopens() {
        let markdown = "# Title\n\nBody one.\n\n## Sub\n\nBody two.\n";
        let mut builder = crate::document::from_events::DocumentBuilder::new();
        crate::io::markdown::parse_into(markdown, Default::default(), &mut builder);
        let document = builder.finish();

        let bytes = write(&document).expect("write the package");
        let package = Package::read_scope(&bytes, Scope::Document).expect("read it back");
        let round = read_document(&package);
        let texts = round.paragraph_texts();

        assert!(texts.iter().any(|text| text == "Title"), "{texts:?}");
        assert!(texts.iter().any(|text| text == "Body one."), "{texts:?}");
        assert!(texts.iter().any(|text| text == "Body two."), "{texts:?}");
    }

    /// Bold and italic runs come back bold and italic: the writer points them
    /// at the template's character styles, and the reader resolves those to
    /// effective formatting the same way Pages does.
    #[test]
    fn writes_bold_and_italic_runs() {
        use crate::document::{Block, Inline};

        let markdown = "Plain **bold words** and *italic words* here.\n";
        let mut builder = crate::document::from_events::DocumentBuilder::new();
        crate::io::markdown::parse_into(markdown, Default::default(), &mut builder);
        let document = builder.finish();

        let bytes = write(&document).expect("write the package");
        let package = Package::read_scope(&bytes, Scope::Document).expect("read it back");
        let round = read_document(&package);

        let paragraph = round.sections[0]
            .blocks
            .iter()
            .find_map(|block| match block {
                Block::Paragraph(paragraph) => Some(paragraph),
                Block::Table(_) => None,
            })
            .expect("a paragraph");
        let effective = |needle: &str| {
            let run = paragraph
                .runs
                .iter()
                .find(|run| {
                    matches!(run.content, Inline::Text(span) if round.text(span).contains(needle))
                })
                .unwrap_or_else(|| panic!("no run containing {needle:?}"));
            round.effective_run(paragraph, run)
        };
        assert_eq!(effective("bold words").bold, Some(true));
        assert_eq!(effective("italic words").italic, Some(true));
        assert_ne!(effective("Plain ").bold, Some(true));
        assert_ne!(effective("Plain ").italic, Some(true));
    }

    /// A run's direct formatting — colour, size, and font — comes back intact:
    /// the writer synthesises a character style carrying it, which the reader
    /// resolves to effective run properties the way Pages does.
    #[test]
    fn writes_direct_formatting() {
        use crate::document::{Block, Color, Inline, Run, RunProperties};

        let markdown = "Plain and formatted text.\n";
        let mut builder = crate::document::from_events::DocumentBuilder::new();
        crate::io::markdown::parse_into(markdown, Default::default(), &mut builder);
        let mut document = builder.finish();

        let font = document.intern_string("Georgia");
        let red = Color {
            red: 200,
            green: 20,
            blue: 20,
        };
        let properties = document.intern_run_properties(RunProperties {
            size: Some(18.0),
            font: Some(font),
            color: Some(red),
            underline: Some(true),
            ..RunProperties::default()
        });
        let span = document.push_text("styled");
        let Some(Block::Paragraph(paragraph)) = document.sections[0].blocks.first_mut() else {
            panic!("a paragraph");
        };
        paragraph.runs.push(Run {
            style: None,
            properties,
            link: None,
            revision: None,
            content: Inline::Text(span),
        });

        let bytes = write(&document).expect("write the package");
        let package = Package::read_scope(&bytes, Scope::Document).expect("read it back");
        let round = read_document(&package);

        let paragraph = round.sections[0]
            .blocks
            .iter()
            .find_map(|block| match block {
                Block::Paragraph(paragraph) => Some(paragraph),
                Block::Table(_) => None,
            })
            .expect("a paragraph");
        let run = paragraph
            .runs
            .iter()
            .find(|run| matches!(run.content, Inline::Text(s) if round.text(s).contains("styled")))
            .expect("the styled run");
        let effective = round.effective_run(paragraph, run);
        assert_eq!(effective.color, Some(red), "colour round-trips");
        assert_eq!(effective.size, Some(18.0), "size round-trips");
        assert_eq!(effective.underline, Some(true), "underline round-trips");
        let font_name = effective.font.map(|id| round.string(id).to_string());
        assert_eq!(font_name.as_deref(), Some("Georgia"), "font round-trips");
    }

    /// A link comes back as a run carrying its target URL: the writer emits a
    /// hyperlink field object and a smart-field table, and the reader resolves
    /// the run's link the same way Pages does. Plain text keeps no link.
    #[test]
    fn writes_links() {
        use crate::document::{Block, Inline};

        let markdown = "Visit [the site](https://example.com/path) today.\n";
        let mut builder = crate::document::from_events::DocumentBuilder::new();
        crate::io::markdown::parse_into(markdown, Default::default(), &mut builder);
        let document = builder.finish();

        let bytes = write(&document).expect("write the package");
        let package = Package::read_scope(&bytes, Scope::Document).expect("read it back");
        let round = read_document(&package);

        let paragraph = round.sections[0]
            .blocks
            .iter()
            .find_map(|block| match block {
                Block::Paragraph(paragraph) => Some(paragraph),
                Block::Table(_) => None,
            })
            .expect("a paragraph");
        let link_of = |needle: &str| {
            let run = paragraph
                .runs
                .iter()
                .find(|run| {
                    matches!(run.content, Inline::Text(span) if round.text(span).contains(needle))
                })
                .unwrap_or_else(|| panic!("no run containing {needle:?}"));
            round.link(run).map(str::to_string)
        };
        assert_eq!(
            link_of("the site").as_deref(),
            Some("https://example.com/path")
        );
        assert_eq!(link_of("Visit "), None);
    }

    #[test]
    fn sha1_matches_known_vectors() {
        let hex = |bytes: [u8; 20]| {
            bytes
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        };
        assert_eq!(hex(sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(
            hex(sha1(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(
            hex(sha1(b"The quick brown fox jumps over the lazy dog")),
            "2fd4e1c67a2d28fced849ee1bb76e7391b93eb12"
        );
    }

    /// A minimal PNG: a valid signature and `IHDR` (so `image_dimensions`
    /// reads its size) with a stub `IEND`. The bytes are stored and read back
    /// verbatim; they are never decoded by the writer or our reader.
    fn fake_png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        bytes.extend_from_slice(&13u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        bytes.extend_from_slice(&0u32.to_be_bytes());
        bytes.extend_from_slice(b"IEND");
        bytes.extend_from_slice(&0u32.to_be_bytes());
        bytes
    }

    /// An inline image comes back as an image run carrying the same bytes and,
    /// since the model gave no size, the picture's own pixel dimensions: the
    /// writer reuses the template's image, swapping its bytes and size.
    #[test]
    fn writes_an_image() {
        use crate::document::{Block, Inline, InlineImage, Media, Placement, Run};

        let markdown = "Here is an image below.\n";
        let mut builder = crate::document::from_events::DocumentBuilder::new();
        crate::io::markdown::parse_into(markdown, Default::default(), &mut builder);
        let mut document = builder.finish();

        let png = fake_png(120, 60);
        let media = document.media.len();
        document.media.push(Media {
            name: "test.png".into(),
            bytes: png.clone(),
        });
        let image = document.push_image(InlineImage {
            media,
            width: 0.0,
            height: 0.0,
            description: Some("a test image".into()),
            placement: Placement::Inline,
        });
        let Some(Block::Paragraph(paragraph)) = document.sections[0].blocks.first_mut() else {
            panic!("a paragraph");
        };
        paragraph.runs.push(Run {
            style: None,
            properties: None,
            link: None,
            revision: None,
            content: Inline::Image(image),
        });

        let bytes = write(&document).expect("write the package");
        let package = Package::read_scope(&bytes, Scope::Document).expect("read it back");
        let round = read_document(&package);

        let image = round
            .sections
            .iter()
            .flat_map(|section| &section.blocks)
            .find_map(|block| match block {
                Block::Paragraph(paragraph) => {
                    paragraph.runs.iter().find_map(|run| match run.content {
                        Inline::Image(id) => round.image(id),
                        _ => None,
                    })
                }
                Block::Table(_) => None,
            })
            .expect("an image run");
        assert_eq!(
            round.media[image.media].bytes, png,
            "image bytes round-trip"
        );
        assert_eq!(image.width, 120.0, "width is the pixel width");
        assert_eq!(image.height, 60.0, "height is the pixel height");
    }

    /// A bullet list and a numbered list come back as list paragraphs with the
    /// right marker kind and nesting level.
    #[test]
    fn writes_bullet_and_numbered_lists() {
        use crate::document::{Block, ListLabel};

        let markdown = "- first\n- second\n  - nested\n\n1. one\n2. two\n";
        let mut builder = crate::document::from_events::DocumentBuilder::new();
        crate::io::markdown::parse_into(markdown, Default::default(), &mut builder);
        let document = builder.finish();

        let bytes = write(&document).expect("write the package");
        let package = Package::read_scope(&bytes, Scope::Document).expect("read it back");
        let round = read_document(&package);

        let paragraphs: Vec<_> = round.sections[0]
            .blocks
            .iter()
            .filter_map(|block| match block {
                Block::Paragraph(paragraph) => Some(paragraph),
                Block::Table(_) => None,
            })
            .collect();
        let label = |paragraph: &crate::document::Paragraph| {
            let item = paragraph.list.expect("a list item");
            let style = &round.styles.list[item.style];
            (item.level, style.levels[item.level as usize].label.clone())
        };
        // first, second (level 0 bullets), nested (level 1 bullet).
        assert!(matches!(label(paragraphs[0]), (0, ListLabel::Text(_))));
        assert!(matches!(label(paragraphs[2]), (1, ListLabel::Text(_))));
        // one, two (level 0 numbers).
        assert!(matches!(label(paragraphs[3]), (0, ListLabel::Number(_))));
        assert!(matches!(label(paragraphs[4]), (0, ListLabel::Number(_))));
    }

    /// A markdown table comes back as a table block with the right grid and
    /// cell text: the writer rewrites one of the template's own tables.
    #[test]
    fn writes_a_table() {
        use crate::document::{Block, Inline};

        let markdown = "| Name | Value |\n| --- | --- |\n| alpha | 1 |\n| beta | 2 |\n";
        let mut builder = crate::document::from_events::DocumentBuilder::new();
        crate::io::markdown::parse_into(markdown, Default::default(), &mut builder);
        let document = builder.finish();

        let bytes = write(&document).expect("write the package");
        let package = Package::read_scope(&bytes, Scope::Document).expect("read it back");
        let round = read_document(&package);

        let table = round.sections[0]
            .blocks
            .iter()
            .find_map(|block| match block {
                Block::Table(table) => Some(table),
                Block::Paragraph(_) => None,
            })
            .expect("a table block");
        assert_eq!(table.columns.len(), 2);
        assert_eq!(table.rows.len(), 3);
        let cell = |row: usize, column: usize| {
            let paragraph = match &table.rows[row].cells[column].blocks[0] {
                Block::Paragraph(paragraph) => paragraph,
                Block::Table(_) => panic!("nested table"),
            };
            match paragraph.runs[0].content {
                Inline::Text(span) => round.text(span).to_string(),
                _ => panic!("no text"),
            }
        };
        assert_eq!(cell(0, 0), "Name");
        assert_eq!(cell(0, 1), "Value");
        assert_eq!(cell(1, 0), "alpha");
        assert_eq!(cell(2, 1), "2");
    }
}

// ----- component metadata -----

/// Brings `PackageMetadata`'s per-component bookkeeping in line with the
/// objects the writer produced. Pages loads each `.iwa` stream as a component
/// and aborts if an object in it references an object in another component that
/// the component does not declare in `external_references`, or if a text storage
/// has no `object_uuid_map_entries` entry. The template's own entries are kept;
/// only missing ones are added, so an untouched component is left as it was.
/// Returns how many entries were added.
fn reconcile_components(package: &mut Package) -> Result<usize, PackageError> {
    // Where each object lives, what each stream references, and its storages.
    let mut owner: HashMap<u64, String> = HashMap::new();
    let mut references: HashMap<String, Vec<u64>> = HashMap::new();
    let mut storages: HashMap<String, Vec<u64>> = HashMap::new();
    for entry in &package.entries {
        let Entry::Stream(stream) = entry else {
            continue;
        };
        for object in &stream.objects {
            owner.insert(object.identifier, stream.name.clone());
            let refs = references.entry(stream.name.clone()).or_default();
            for id in archive_references(&stream.tree, object.info) {
                if !refs.contains(&id) {
                    refs.push(id);
                }
            }
            // Text storages and styles carry persistent UUIDs in Pages; a style
            // without one is re-created on load and its references swapped,
            // which drops a table's cell styles.
            if matches!(
                first_type(object),
                Some(
                    STORAGE_ARCHIVE
                        | PARAGRAPH_STYLE
                        | CHARACTER_STYLE
                        | CELL_STYLE
                        | SHAPE_STYLE
                        | LIST_STYLE
                )
            ) {
                storages
                    .entry(stream.name.clone())
                    .or_default()
                    .push(object.identifier);
            }
        }
    }

    let Some((stream, metadata_first)) = metadata_message(package) else {
        return Ok(0);
    };
    let tree = &mut stream.tree;
    let metadata = message_ref("TSP.PackageMetadata")?;
    let component = child_message(metadata, "components")?;
    let external = child_message(component, "external_references")?;
    let uuid_entry = child_message(component, "object_uuid_map_entries")?;
    let uuid = child_message(uuid_entry, "uuid")?;

    // Each component: the metadata entry holding it, its id, and its stream.
    struct Component {
        entry: u32,
        first: u32,
        identifier: u64,
        stream: String,
    }
    let mut components: Vec<Component> = Vec::new();
    for (index, field) in tree.chain(metadata_first) {
        if tree.field(field).map(|f| f.name) != Some("components") {
            continue;
        }
        let Node::Message(first) = field.value else {
            continue;
        };
        let Some(Node::Uint(identifier)) = field_value(tree, first, "identifier") else {
            continue;
        };
        let locator = str_field(tree, first, "locator")
            .filter(|name| !name.is_empty())
            .or_else(|| str_field(tree, first, "preferred_locator"));
        let Some(locator) = locator else {
            continue;
        };
        components.push(Component {
            entry: index,
            first,
            identifier,
            stream: format!("Index/{locator}.iwa"),
        });
    }
    let component_of: HashMap<&str, u64> = components
        .iter()
        .map(|c| (c.stream.as_str(), c.identifier))
        .collect();

    let mut added = 0;
    let mut rebuilt: Vec<(u32, u32)> = Vec::new();
    for component_entry in &components {
        // What the component already declares.
        let mut declared: Vec<(u64, Option<u64>)> = Vec::new();
        let mut mapped: Vec<u64> = Vec::new();
        for (_, field) in tree.chain(component_entry.first) {
            let name = tree.field(field).map(|f| f.name);
            let Node::Message(first) = field.value else {
                continue;
            };
            match name {
                Some("external_references") => {
                    let component_id = match field_value(tree, first, "component_identifier") {
                        Some(Node::Uint(id)) => id,
                        _ => continue,
                    };
                    let object_id = match field_value(tree, first, "object_identifier") {
                        Some(Node::Uint(id)) => Some(id),
                        _ => None,
                    };
                    declared.push((component_id, object_id));
                }
                Some("object_uuid_map_entries") => {
                    if let Some(Node::Uint(id)) = field_value(tree, first, "identifier") {
                        mapped.push(id);
                    }
                }
                _ => {}
            }
        }

        // What it needs: every reference out of its stream, by owning component.
        let mut missing_refs: Vec<(u64, Option<u64>)> = Vec::new();
        for id in references
            .get(&component_entry.stream)
            .into_iter()
            .flatten()
        {
            let Some(target_stream) = owner.get(id) else {
                continue;
            };
            if *target_stream == component_entry.stream {
                continue;
            }
            let Some(&target) = component_of.get(target_stream.as_str()) else {
                continue;
            };
            // A reference to another component's root object is recorded as a
            // weak component reference with no object identifier.
            let wanted = if *id == target {
                (target, None)
            } else {
                (target, Some(*id))
            };
            let known = declared.iter().any(|(c, o)| {
                *c == wanted.0 && (*o == wanted.1 || (wanted.1.is_none() && o.is_none()))
            });
            if !known && !missing_refs.contains(&wanted) {
                missing_refs.push(wanted);
            }
        }
        let missing_uuids: Vec<u64> = storages
            .get(&component_entry.stream)
            .into_iter()
            .flatten()
            .copied()
            .filter(|id| !mapped.contains(id))
            .collect();
        if missing_refs.is_empty() && missing_uuids.is_empty() {
            continue;
        }
        added += missing_refs.len() + missing_uuids.len();

        // Rebuild the component message in field-number order with the new
        // entries alongside the kept ones.
        let mut fields: Vec<(u32, Node)> = tree
            .chain(component_entry.first)
            .map(|(_, field)| (field.number, field.value))
            .collect();
        for (component_id, object_id) in missing_refs {
            let mut chain = Chain::new();
            push_field(
                tree,
                &mut chain,
                external,
                "component_identifier",
                Node::Uint(component_id),
            )?;
            match object_id {
                Some(object_id) => push_field(
                    tree,
                    &mut chain,
                    external,
                    "object_identifier",
                    Node::Uint(object_id),
                )?,
                None => push_field(tree, &mut chain, external, "is_weak", Node::Bool(true))?,
            }
            let number = component
                .slot_named("external_references")
                .map(|(_, f)| f.number)
                .ok_or_else(|| malformed("ComponentInfo has no external_references"))?;
            fields.push((number, Node::Message(chain.first)));
        }
        for id in missing_uuids {
            let mut uuid_chain = Chain::new();
            let (lower, upper) = object_uuid(id);
            push_field(tree, &mut uuid_chain, uuid, "lower", Node::Uint(lower))?;
            push_field(tree, &mut uuid_chain, uuid, "upper", Node::Uint(upper))?;
            let mut chain = Chain::new();
            push_field(tree, &mut chain, uuid_entry, "identifier", Node::Uint(id))?;
            push_field(
                tree,
                &mut chain,
                uuid_entry,
                "uuid",
                Node::Message(uuid_chain.first),
            )?;
            let number = component
                .slot_named("object_uuid_map_entries")
                .map(|(_, f)| f.number)
                .ok_or_else(|| malformed("ComponentInfo has no object_uuid_map_entries"))?;
            fields.push((number, Node::Message(chain.first)));
        }
        fields.sort_by_key(|(number, _)| *number);
        let mut chain = Chain::new();
        for (number, value) in fields {
            let slot = component
                .slot(number)
                .ok_or_else(|| malformed("component field without a schema slot"))?;
            let field = component
                .field_at(slot)
                .ok_or_else(|| malformed("component field slot out of range"))?;
            tree.push_known(&mut chain, component, slot, field, number, value)
                .map_err(tree_error)?;
        }
        rebuilt.push((component_entry.entry, chain.first));
    }
    for (entry, first) in rebuilt {
        tree.entries[entry as usize].value = Node::Message(first);
    }
    Ok(added)
}

/// Every object identifier an object's `ArchiveInfo` lists as referenced.
fn archive_references(tree: &Tree, info: u32) -> Vec<u64> {
    let mut out = Vec::new();
    for (_, field) in tree.chain(info) {
        if tree.field(field).map(|f| f.name) != Some("message_infos") {
            continue;
        }
        let Node::Message(first) = field.value else {
            continue;
        };
        for (_, inner) in tree.chain(first) {
            if tree.field(inner).map(|f| f.name) != Some("object_references") {
                continue;
            }
            match inner.value {
                Node::Uint(id) => out.push(id),
                Node::Bytes(span) | Node::RawBytes(span) => {
                    let bytes = tree.bytes(span);
                    let mut value = 0u64;
                    let mut shift = 0;
                    for byte in bytes {
                        value |= u64::from(byte & 0x7f) << shift;
                        if byte & 0x80 == 0 {
                            out.push(value);
                            value = 0;
                            shift = 0;
                        } else {
                            shift += 7;
                        }
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// A stable, distinct UUID for a written object, derived from its identifier.
fn object_uuid(id: u64) -> (u64, u64) {
    fn mix(mut z: u64) -> u64 {
        z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    (mix(id), mix(id ^ 0x5355_424C_494D_4521))
}

/// Sets `PackageMetadata.last_object_identifier`, the identifier Pages
/// allocates new objects above.
fn set_last_object_identifier(package: &mut Package, identifier: u64) -> Result<(), PackageError> {
    let (stream, first) =
        metadata_message(package).ok_or_else(|| malformed("PackageMetadata is missing"))?;
    for (index, field) in stream.tree.chain(first) {
        if stream.tree.field(field).map(|f| f.name) == Some("last_object_identifier") {
            stream.tree.entries[index as usize].value = Node::Uint(identifier);
            return Ok(());
        }
    }
    Err(malformed("PackageMetadata has no last_object_identifier"))
}

/// Replaces every UUID in the document's identity files (`DocumentIdentifier`
/// and the document, version, private, and share UUIDs in `Properties.plist`)
/// with fresh ones, the same old UUID mapping to the same new one throughout.
/// UUIDs are fixed-length ASCII, so the binary plist keeps its layout.
fn renew_document_identity(package: &mut Package) {
    let mut renamed: Vec<([u8; 36], [u8; 36])> = Vec::new();
    for entry in &mut package.entries {
        let Entry::File { name, bytes } = entry else {
            continue;
        };
        if name != "Metadata/DocumentIdentifier" && name != "Metadata/Properties.plist" {
            continue;
        }
        let mut at = 0;
        while at + 36 <= bytes.len() {
            let window: [u8; 36] = bytes[at..at + 36].try_into().expect("36 bytes");
            if !is_uuid(&window) {
                at += 1;
                continue;
            }
            let fresh = match renamed.iter().find(|(old, _)| *old == window) {
                Some((_, new)) => *new,
                None => {
                    let new = fresh_uuid(renamed.len() as u64);
                    renamed.push((window, new));
                    new
                }
            };
            bytes[at..at + 36].copy_from_slice(&fresh);
            at += 36;
        }
    }
}

fn is_uuid(text: &[u8; 36]) -> bool {
    text.iter().enumerate().all(|(index, byte)| match index {
        8 | 13 | 18 | 23 => *byte == b'-',
        _ => byte.is_ascii_hexdigit(),
    })
}

/// A random (version 4) UUID in the uppercase text form Pages writes, drawn
/// from the clock, the process, and `salt` so each call and run differs.
fn fresh_uuid(salt: u64) -> [u8; 36] {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos() as u64)
        .unwrap_or(0);
    let seed = nanos ^ (u64::from(std::process::id()) << 32) ^ salt.wrapping_mul(0x9E37_79B9);
    let (high, low) = object_uuid(seed);
    let mut raw = [0u8; 16];
    raw[..8].copy_from_slice(&high.to_be_bytes());
    raw[8..].copy_from_slice(&low.to_be_bytes());
    raw[6] = (raw[6] & 0x0f) | 0x40;
    raw[8] = (raw[8] & 0x3f) | 0x80;
    let hex: String = raw.iter().map(|byte| format!("{byte:02X}")).collect();
    let text = format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    );
    text.as_bytes().try_into().expect("36 bytes")
}

/// Carries the document's page size and margins onto `DocumentArchive`, so a
/// document laid out for, say, narrow margins keeps its text width and page
/// breaks instead of inheriting the template's US Letter with one-inch margins.
fn set_page_setup(
    package: &mut Package,
    page: &crate::document::PageSetup,
) -> Result<(), PackageError> {
    let stream = document_stream(package)?;
    let first = stream
        .objects
        .iter()
        .find(|object| first_type(object) == Some(DOCUMENT_ARCHIVE))
        .and_then(|object| object.messages.first())
        .map(|message| message.first)
        .ok_or_else(|| malformed("DocumentArchive is missing"))?;
    let tree = &mut stream.tree;
    for (name, value) in [
        ("page_width", page.width),
        ("page_height", page.height),
        ("left_margin", page.margin_left),
        ("right_margin", page.margin_right),
        ("top_margin", page.margin_top),
        ("bottom_margin", page.margin_bottom),
        ("header_margin", page.header_distance),
        ("footer_margin", page.footer_distance),
    ] {
        if value.is_finite() && value >= 0.0 {
            set_field_float(tree, first, name, value);
        }
    }
    let landscape = u64::from(page.width > page.height);
    set_field_uint(tree, first, "orientation", landscape);
    Ok(())
}

// ----- table cloning -----

const CALCULATION_ENGINE: u32 = 4000;
const FORMULA_OWNER_DEPENDENCIES: u32 = 4008;

/// What turns a copy of the prototype table's objects into a distinct table:
/// new object identifiers, a fresh UUID family (a table's UUIDs share their
/// last twelve bytes and differ in the first word), and new internal formula
/// owner numbers in the calculation engine.
struct CloneMap {
    ids: HashMap<u64, u64>,
    families: HashMap<[u8; 12], [u8; 12]>,
    owners: HashMap<u64, u64>,
}

impl CloneMap {
    fn id(&self, id: u64) -> u64 {
        self.ids.get(&id).copied().unwrap_or(id)
    }

    fn uuid(&self, bytes: [u8; 16]) -> Option<[u8; 16]> {
        let family: [u8; 12] = bytes[4..].try_into().expect("12 bytes");
        let fresh = self.families.get(&family)?;
        let mut out = bytes;
        out[4..].copy_from_slice(fresh);
        Some(out)
    }
}

/// The UUIDs a message chain holds directly, by encoding: `uuid_w0..w3` words,
/// a `lower`/`upper` pair, and each UUID string, with the entries holding them.
enum UuidAt {
    Words([u32; 4], [u8; 16]),
    Pair([u32; 2], [u8; 16]),
    Text([u8; 16]),
}

fn chain_uuids(tree: &Tree, first: u32) -> Vec<UuidAt> {
    let mut words = [None; 4];
    let mut pair = [None; 2];
    let mut count = 0;
    let mut out = Vec::new();
    for (index, entry) in tree.chain(first) {
        count += 1;
        let name = tree.field(entry).map(|field| field.name);
        match (name, entry.value) {
            (Some("uuid_w0"), Node::Uint(v)) => words[0] = Some((index, v as u32)),
            (Some("uuid_w1"), Node::Uint(v)) => words[1] = Some((index, v as u32)),
            (Some("uuid_w2"), Node::Uint(v)) => words[2] = Some((index, v as u32)),
            (Some("uuid_w3"), Node::Uint(v)) => words[3] = Some((index, v as u32)),
            (Some("lower"), Node::Uint(v)) => pair[0] = Some((index, v)),
            (Some("upper"), Node::Uint(v)) => pair[1] = Some((index, v)),
            (_, Node::Str(span)) => {
                if let Some(bytes) = parse_uuid_text(tree.bytes(span)) {
                    let _ = index;
                    out.push(UuidAt::Text(bytes));
                }
            }
            _ => {}
        }
    }
    if let [Some(a), Some(b), Some(c), Some(d)] = words {
        let mut bytes = [0u8; 16];
        for (slot, (_, word)) in [a, b, c, d].iter().enumerate() {
            bytes[slot * 4..slot * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        out.push(UuidAt::Words([a.0, b.0, c.0, d.0], bytes));
    }
    if let [Some(lower), Some(upper)] = pair
        && count == 2
    {
        let mut bytes = [0u8; 16];
        bytes[..8].copy_from_slice(&lower.1.to_le_bytes());
        bytes[8..].copy_from_slice(&upper.1.to_le_bytes());
        out.push(UuidAt::Pair([lower.0, upper.0], bytes));
    }
    out
}

fn parse_uuid_text(text: &[u8]) -> Option<[u8; 16]> {
    let window: &[u8; 36] = text.try_into().ok()?;
    if !is_uuid(window) {
        return None;
    }
    let hex: Vec<u8> = text.iter().copied().filter(|byte| *byte != b'-').collect();
    let mut out = [0u8; 16];
    for (index, pair) in hex.chunks(2).enumerate() {
        out[index] = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(out)
}

fn format_uuid_text(bytes: [u8; 16]) -> String {
    let hex: String = bytes.iter().map(|byte| format!("{byte:02X}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// Calls `visit` on a chain and every nested chain below it.
fn walk_chains(tree: &Tree, first: u32, visit: &mut dyn FnMut(&Tree, u32)) {
    if first == NONE {
        return;
    }
    visit(tree, first);
    let nested: Vec<u32> = tree
        .chain(first)
        .filter_map(|(_, entry)| match entry.value {
            Node::Message(child) => Some(child),
            _ => None,
        })
        .collect();
    for child in nested {
        walk_chains(tree, child, visit);
    }
}

/// Whether a chain (or anything below it) names a UUID of a remapped family
/// or references a cloned object.
fn mentions_clone(tree: &Tree, first: u32, map: &CloneMap) -> bool {
    let mut found = false;
    walk_chains(tree, first, &mut |tree, chain| {
        if found {
            return;
        }
        for uuid in chain_uuids(tree, chain) {
            let bytes = match uuid {
                UuidAt::Words(_, b) | UuidAt::Pair(_, b) | UuidAt::Text(b) => b,
            };
            if map.uuid(bytes).is_some() {
                found = true;
                return;
            }
        }
        for (_, entry) in tree.chain(chain) {
            if let Node::Reference(id) = entry.value
                && map.ids.contains_key(&id)
            {
                found = true;
                return;
            }
        }
    });
    found
}

/// Copies a chain from `source` into `target` (they may not be the same tree),
/// applying the clone map: references and `object_references` to cloned
/// objects, internal formula owner numbers, and UUIDs of remapped families.
fn copy_chain(
    source: &Tree,
    first: u32,
    target: &mut Tree,
    map: &CloneMap,
) -> Result<u32, PackageError> {
    let entries: Vec<TreeEntry> = source.chain(first).map(|(_, entry)| *entry).collect();
    let mut chain = Chain::new();
    for original in entries {
        let mut entry = original;
        entry.next = NONE;
        let name = source.field(&original).map(|field| field.name);
        entry.value = match original.value {
            Node::Message(child) => Node::Message(copy_chain(source, child, target, map)?),
            Node::Str(span) => {
                let text = source.bytes(span);
                let remapped = parse_uuid_text(text)
                    .and_then(|bytes| map.uuid(bytes))
                    .map(|bytes| format_uuid_text(bytes).into_bytes());
                let bytes = remapped.as_deref().unwrap_or(text);
                Node::Str(target.push_bytes(bytes).map_err(tree_error)?)
            }
            Node::Bytes(span) => {
                Node::Bytes(target.push_bytes(source.bytes(span)).map_err(tree_error)?)
            }
            Node::RawBytes(span) => {
                Node::RawBytes(target.push_bytes(source.bytes(span)).map_err(tree_error)?)
            }
            Node::Deferred(span) => {
                Node::Deferred(target.push_bytes(source.bytes(span)).map_err(tree_error)?)
            }
            Node::Reference(id) => Node::Reference(map.id(id)),
            Node::Uint(value) if name == Some("object_references") => Node::Uint(map.id(value)),
            // Internal formula-owner numbers appear under several names
            // (internal_owner_id, internal_owner_id_for_edge, to_owner_id, a
            // range reference's owner_id); every one naming a cloned owner
            // must follow it, or the clone stays wired to the prototype.
            Node::Uint(value) if name.is_some_and(|name| name.contains("owner_id")) => {
                Node::Uint(map.owners.get(&value).copied().unwrap_or(value))
            }
            other => other,
        };
        target.push(&mut chain, entry).map_err(tree_error)?;
    }
    // UUIDs held as words or a lower/upper pair, rewritten in place.
    for uuid in chain_uuids(target, chain.first) {
        match uuid {
            UuidAt::Words(indices, bytes) => {
                if let Some(fresh) = map.uuid(bytes) {
                    for (slot, index) in indices.iter().enumerate() {
                        let word = u32::from_le_bytes(
                            fresh[slot * 4..slot * 4 + 4].try_into().expect("4"),
                        );
                        target.entries[*index as usize].value = Node::Uint(u64::from(word));
                    }
                }
            }
            UuidAt::Pair(indices, bytes) => {
                if let Some(fresh) = map.uuid(bytes) {
                    let lower = u64::from_le_bytes(fresh[..8].try_into().expect("8"));
                    let upper = u64::from_le_bytes(fresh[8..].try_into().expect("8"));
                    target.entries[indices[0] as usize].value = Node::Uint(lower);
                    target.entries[indices[1] as usize].value = Node::Uint(upper);
                }
            }
            UuidAt::Text(_) => {}
        }
    }
    Ok(chain.first)
}

/// Copies a chain within one tree, through a scratch tree.
fn copy_within(tree: &mut Tree, first: u32, map: &CloneMap) -> Result<u32, PackageError> {
    let mut scratch = Tree::new(&SCHEMA);
    let copied = copy_chain(tree, first, &mut scratch, map)?;
    let identity = CloneMap {
        ids: HashMap::new(),
        families: HashMap::new(),
        owners: HashMap::new(),
    };
    copy_chain(&scratch, copied, tree, &identity)
}

/// Inserts a copy of every repeated entry of a shared (not cloned) chain that
/// concerns the prototype table — one naming its UUID family or referencing a
/// cloned object — right after the original, remapped to the clone. This is
/// how the calculation engine's owner map, dependency lists, and per-table
/// registries learn about the new table. Returns whether anything was added.
fn register_clone(tree: &mut Tree, first: u32, map: &CloneMap) -> Result<bool, PackageError> {
    let entries: Vec<(u32, TreeEntry)> = tree
        .chain(first)
        .map(|(index, entry)| (index, *entry))
        .collect();
    let mut added = false;
    for (index, entry) in entries {
        let Some(field) = tree.field(&entry) else {
            continue;
        };
        let duplicate = match entry.value {
            Node::Message(child) if field.repeated && mentions_clone(tree, child, map) => {
                Some(Node::Message(copy_within(tree, child, map)?))
            }
            Node::Message(child) => {
                added |= register_clone(tree, child, map)?;
                None
            }
            Node::Reference(id) if field.repeated && map.ids.contains_key(&id) => {
                Some(Node::Reference(map.id(id)))
            }
            _ => None,
        };
        if let Some(value) = duplicate {
            let mut copy = entry;
            copy.value = value;
            copy.next = tree.entries[index as usize].next;
            let mut chain = Chain::new();
            let new_index = tree.push(&mut chain, copy).map_err(tree_error)?;
            tree.entries[new_index as usize].next = copy.next;
            tree.entries[index as usize].next = new_index;
            added = true;
        }
    }
    Ok(added)
}

/// Every Reference a chain holds, at any depth.
fn chain_references(tree: &Tree, first: u32) -> Vec<u64> {
    let mut out = Vec::new();
    walk_chains(tree, first, &mut |tree, chain| {
        for (_, entry) in tree.chain(chain) {
            if let Node::Reference(id) = entry.value
                && !out.contains(&id)
            {
                out.push(id);
            }
        }
    });
    out
}

/// Clones the prototype table — its drawable attachment, table info and model,
/// tiles, data lists, header buckets, and calculation-engine owners — into a
/// distinct table, registered with the calculation engine and the package
/// metadata. Returns the new drawable attachment for the body to anchor.
fn clone_template_table(
    package: &mut Package,
    proto: &TemplateTable,
    next_id: &mut u64,
) -> Result<u64, PackageError> {
    // Where every object lives and what it references.
    let mut location: HashMap<u64, (usize, usize)> = HashMap::new();
    for (stream_index, entry) in package.entries.iter().enumerate() {
        if let Entry::Stream(stream) = entry {
            for (object_index, object) in stream.objects.iter().enumerate() {
                location.insert(object.identifier, (stream_index, object_index));
            }
        }
    }
    let (engine_stream, _) = *location
        .get(&proto.info_id)
        .ok_or_else(|| malformed("table info is missing"))?;

    // The cluster: objects reachable from the table info, short of the shared
    // stylesheet, the document, and the calculation engine archive itself.
    let mut cluster: Vec<u64> = Vec::new();
    extend_cluster(package, &location, &mut cluster, vec![proto.info_id]);

    // The UUID families the cluster uses, each given fresh bytes.
    let mut families: HashMap<[u8; 12], [u8; 12]> = HashMap::new();
    for id in &cluster {
        let (stream_index, object_index) = location[id];
        let stream = stream_at(package, stream_index).expect("stream");
        for message in &stream.objects[object_index].messages {
            walk_chains(&stream.tree, message.first, &mut |tree, chain| {
                for uuid in chain_uuids(tree, chain) {
                    let bytes = match uuid {
                        UuidAt::Words(_, b) | UuidAt::Pair(_, b) | UuidAt::Text(b) => b,
                    };
                    let family: [u8; 12] = bytes[4..].try_into().expect("12");
                    if family.iter().any(|byte| *byte != 0) && !families.contains_key(&family) {
                        let fresh = fresh_uuid(families.len() as u64 + *next_id);
                        let fresh = parse_uuid_text(&fresh).expect("uuid");
                        families.insert(family, fresh[4..].try_into().expect("12"));
                    }
                }
            });
        }
    }
    let mut map = CloneMap {
        ids: HashMap::new(),
        families,
        owners: HashMap::new(),
    };

    // The engine's per-owner dependency objects for the table's owners are
    // cloned too; the owner map gives each owner a new internal number.
    let engine = stream_at(package, engine_stream).expect("stream");
    let engine_archive = engine
        .objects
        .iter()
        .find(|object| first_type(object) == Some(CALCULATION_ENGINE))
        .ok_or_else(|| malformed("calculation engine is missing"))?;
    let engine_id = engine_archive.identifier;
    let mut max_owner = 0u64;
    let mut table_owners: Vec<u64> = Vec::new();
    walk_chains(
        &engine.tree,
        engine_archive.messages[0].first,
        &mut |tree, chain| {
            let mut internal = None;
            let mut mentions = false;
            for (_, entry) in tree.chain(chain) {
                if tree.field(entry).map(|f| f.name) == Some("internal_owner_id")
                    && let Node::Uint(value) = entry.value
                {
                    internal = Some(value);
                    max_owner = max_owner.max(value);
                }
                if let Node::Message(child) = entry.value {
                    for uuid in chain_uuids(tree, child) {
                        if let UuidAt::Words(_, bytes) | UuidAt::Pair(_, bytes) = uuid
                            && map.uuid(bytes).is_some()
                        {
                            mentions = true;
                        }
                    }
                }
            }
            if let (Some(value), true) = (internal, mentions) {
                table_owners.push(value);
            }
        },
    );
    for (offset, owner) in table_owners.iter().enumerate() {
        map.owners.insert(*owner, max_owner + 1 + offset as u64);
    }
    // Each such object, with what it references (its cell-record tiles),
    // belongs to the clone: sharing them would give two owners one record.
    let owner_objects: Vec<u64> = engine
        .objects
        .iter()
        .filter(|object| {
            first_type(object) == Some(FORMULA_OWNER_DEPENDENCIES)
                && mentions_clone(&engine.tree, object.messages[0].first, &map)
        })
        .map(|object| object.identifier)
        .collect();
    extend_cluster(package, &location, &mut cluster, owner_objects);
    cluster.push(proto.attach_id);
    for id in &cluster {
        map.ids.insert(*id, *next_id);
        *next_id += 1;
    }

    // Copy each object: into its own stream when it shares the engine or the
    // document stream, otherwise into a new single-object component stream.
    let mut new_streams: Vec<(Stream, u64, u64)> = Vec::new();
    for id in &cluster {
        let (stream_index, object_index) = location[id];
        let new_id = map.id(*id);
        let (name, source_object) = {
            let stream = stream_at(package, stream_index).expect("stream");
            (stream.name.clone(), stream.objects[object_index].identifier)
        };
        let shared = stream_index == engine_stream || name == "Index/Document.iwa";
        if shared {
            let Entry::Stream(stream) = &mut package.entries[stream_index] else {
                unreachable!()
            };
            let source = stream.objects[object_index].clone_shape();
            let mut messages = Vec::new();
            for (message_type, first) in &source.1 {
                messages.push(ObjectMessage {
                    message_type: *message_type,
                    first: copy_within(&mut stream.tree, *first, &map)?,
                });
            }
            let info = copy_within(&mut stream.tree, source.0, &map)?;
            set_field_uint(&mut stream.tree, info, "identifier", new_id);
            stream.objects.push(Object {
                identifier: new_id,
                info,
                messages,
            });
        } else {
            let stream = stream_at(package, stream_index).expect("stream");
            let object = &stream.objects[object_index];
            let mut tree = Tree::new(&SCHEMA);
            let mut messages = Vec::new();
            for message in &object.messages {
                messages.push(ObjectMessage {
                    message_type: message.message_type,
                    first: copy_chain(&stream.tree, message.first, &mut tree, &map)?,
                });
            }
            let info = copy_chain(&stream.tree, object.info, &mut tree, &map)?;
            set_field_uint(&mut tree, info, "identifier", new_id);
            let locator = component_locator(package, &name).unwrap_or_else(|| {
                name.trim_start_matches("Index/")
                    .trim_end_matches(".iwa")
                    .to_string()
            });
            let base = locator.split('-').next().unwrap_or(&locator).to_string();
            new_streams.push((
                Stream {
                    name: format!("Index/{base}-{new_id}.iwa"),
                    tree,
                    objects: vec![Object {
                        identifier: new_id,
                        info,
                        messages,
                    }],
                },
                source_object,
                new_id,
            ));
        }
    }

    // Register the clone in the engine's shared containers and every shared
    // engine object that lists the table's owners.
    {
        let Entry::Stream(stream) = &mut package.entries[engine_stream] else {
            unreachable!()
        };
        let shared: Vec<(u64, u32, u32)> = stream
            .objects
            .iter()
            .filter(|object| !map.ids.values().any(|new| *new == object.identifier))
            .filter(|object| !map.ids.contains_key(&object.identifier))
            .map(|object| (object.identifier, object.info, object.messages[0].first))
            .collect();
        for (id, info, first) in shared {
            if id != engine_id
                && !matches!(
                    stream
                        .objects
                        .iter()
                        .find(|o| o.identifier == id)
                        .and_then(first_type),
                    Some(FORMULA_OWNER_DEPENDENCIES) | Some(6366)
                )
            {
                continue;
            }
            if register_clone(&mut stream.tree, first, &map)? {
                let references = chain_references(&stream.tree, first);
                add_object_references(&mut stream.tree, info, &references)?;
            }
        }
    }

    // Components for the new streams, and UUID-map entries for cloned objects.
    for (_, old_id, new_id) in &new_streams {
        add_component_clone(package, *old_id, *new_id)?;
    }
    add_uuid_map_clones(package, &map)?;
    for (stream, _, _) in new_streams {
        package.entries.push(Entry::Stream(stream));
    }
    Ok(map.id(proto.attach_id))
}

impl Object {
    /// The info chain and (type, chain) of each message, for copying.
    fn clone_shape(&self) -> (u32, Vec<(u32, u32)>) {
        (
            self.info,
            self.messages
                .iter()
                .map(|message| (message.message_type, message.first))
                .collect(),
        )
    }
}

/// The metadata locator (or preferred locator) of the component stored in
/// stream `name`.
fn component_locator(package: &Package, name: &str) -> Option<String> {
    let (tree, first) = metadata_view(package)?;
    for (_, field) in tree.chain(first) {
        if tree.field(field).map(|f| f.name) != Some("components") {
            continue;
        }
        let Node::Message(component) = field.value else {
            continue;
        };
        let locator = str_field(tree, component, "locator")
            .filter(|locator| !locator.is_empty())
            .or_else(|| str_field(tree, component, "preferred_locator"))?;
        if format!("Index/{locator}.iwa") == name {
            return str_field(tree, component, "preferred_locator").map(str::to_string);
        }
    }
    None
}

fn metadata_view(package: &Package) -> Option<(&Tree, u32)> {
    for entry in &package.entries {
        if let Entry::Stream(stream) = entry
            && let Some(object) = stream
                .objects
                .iter()
                .find(|object| first_type(object) == Some(PACKAGE_METADATA))
        {
            return Some((&stream.tree, object.messages.first()?.first));
        }
    }
    None
}

/// Adds a component for a cloned single-object stream: a copy of the source
/// object's component with the new identifier and locator, and without the
/// per-object lists (the reconcile pass fills external references back in).
fn add_component_clone(
    package: &mut Package,
    old_id: u64,
    new_id: u64,
) -> Result<(), PackageError> {
    let (stream, metadata_first) =
        metadata_message(package).ok_or_else(|| malformed("PackageMetadata is missing"))?;
    let tree = &mut stream.tree;
    let mut source = None;
    let mut last_component = None;
    for (index, field) in tree.chain(metadata_first) {
        if tree.field(field).map(|f| f.name) != Some("components") {
            continue;
        }
        last_component = Some(index);
        if let Node::Message(component) = field.value
            && field_value(tree, component, "identifier") == Some(Node::Uint(old_id))
        {
            source = Some((index, component));
        }
    }
    let (source_entry, component) =
        source.ok_or_else(|| malformed("component to clone is missing"))?;
    let last_component = last_component.expect("a component");
    let identity = CloneMap {
        ids: HashMap::new(),
        families: HashMap::new(),
        owners: HashMap::new(),
    };
    let copied = copy_within(tree, component, &identity)?;
    // Rebuild without the per-object lists, with the new identity.
    let component_ref = child_message(message_ref("TSP.PackageMetadata")?, "components")?;
    let preferred = str_field(tree, copied, "preferred_locator")
        .unwrap_or("")
        .to_string();
    let base = preferred
        .split('-')
        .next()
        .unwrap_or(&preferred)
        .to_string();
    let kept: Vec<TreeEntry> = tree.chain(copied).map(|(_, entry)| *entry).collect();
    let mut chain = Chain::new();
    for entry in kept {
        let name = tree.field(&entry).map(|f| f.name);
        match name {
            Some(
                "external_references" | "object_uuid_map_entries" | "data_references" | "locator",
            ) => continue,
            Some("identifier") => {
                push_field(
                    tree,
                    &mut chain,
                    component_ref,
                    "identifier",
                    Node::Uint(new_id),
                )?;
                let span = tree
                    .push_bytes(format!("{base}-{new_id}").as_bytes())
                    .map_err(tree_error)?;
                push_field(tree, &mut chain, component_ref, "locator", Node::Str(span))?;
            }
            _ => {
                let mut copy = entry;
                copy.next = NONE;
                tree.push(&mut chain, copy).map_err(tree_error)?;
            }
        }
    }
    let mut holder = tree.entries[source_entry as usize];
    holder.value = Node::Message(chain.first);
    holder.next = tree.entries[last_component as usize].next;
    let mut scratch = Chain::new();
    let new_index = tree.push(&mut scratch, holder).map_err(tree_error)?;
    tree.entries[new_index as usize].next = holder.next;
    tree.entries[last_component as usize].next = new_index;
    Ok(())
}

/// For every `object_uuid_map_entries` entry naming a cloned object, adds an
/// entry for the clone with a remapped (or, outside the table's family, fresh)
/// UUID, in the same component.
fn add_uuid_map_clones(package: &mut Package, map: &CloneMap) -> Result<(), PackageError> {
    let (stream, metadata_first) =
        metadata_message(package).ok_or_else(|| malformed("PackageMetadata is missing"))?;
    let tree = &mut stream.tree;
    let components: Vec<u32> = tree
        .chain(metadata_first)
        .filter(|(_, field)| tree.field(field).map(|f| f.name) == Some("components"))
        .filter_map(|(_, field)| match field.value {
            Node::Message(first) => Some(first),
            _ => None,
        })
        .collect();
    for component in components {
        let entries: Vec<(u32, TreeEntry)> = tree.chain(component).map(|(i, e)| (i, *e)).collect();
        for (index, entry) in entries {
            if tree.field(&entry).map(|f| f.name) != Some("object_uuid_map_entries") {
                continue;
            }
            let Node::Message(first) = entry.value else {
                continue;
            };
            let Some(Node::Uint(id)) = field_value(tree, first, "identifier") else {
                continue;
            };
            let Some(&new_id) = map.ids.get(&id) else {
                continue;
            };
            let copied = copy_within(tree, first, map)?;
            set_field_uint(tree, copied, "identifier", new_id);
            // A UUID outside the table's family must still be unique.
            if let Some(uuid) = message_field(tree, copied, "uuid") {
                let unchanged = chain_uuids(tree, uuid).iter().any(|at| match at {
                    UuidAt::Pair(_, bytes) => map.uuid(*bytes).is_none(),
                    _ => false,
                });
                if unchanged {
                    let (lower, upper) = object_uuid(new_id ^ 0xC10E);
                    set_field_uint(tree, uuid, "lower", lower);
                    set_field_uint(tree, uuid, "upper", upper);
                }
            }
            let mut copy = entry;
            copy.value = Node::Message(copied);
            copy.next = tree.entries[index as usize].next;
            let mut scratch = Chain::new();
            let new_index = tree.push(&mut scratch, copy).map_err(tree_error)?;
            tree.entries[new_index as usize].next = copy.next;
            tree.entries[index as usize].next = new_index;
        }
    }
    Ok(())
}

fn stream_at(package: &Package, index: usize) -> Option<&Stream> {
    match &package.entries[index] {
        Entry::Stream(stream) => Some(stream),
        _ => None,
    }
}

/// Adds to `cluster` every object reachable from `roots` through archive
/// references, short of the shared stylesheet, the document stream, and the
/// calculation engine archive.
fn extend_cluster(
    package: &Package,
    location: &HashMap<u64, (usize, usize)>,
    cluster: &mut Vec<u64>,
    roots: Vec<u64>,
) {
    let mut stack = roots;
    while let Some(id) = stack.pop() {
        if cluster.contains(&id) {
            continue;
        }
        let Some(&(stream_index, object_index)) = location.get(&id) else {
            continue;
        };
        let stream = stream_at(package, stream_index).expect("stream");
        let object = &stream.objects[object_index];
        if stream.name == "Index/DocumentStylesheet.iwa"
            || stream.name == "Index/Document.iwa"
            || first_type(object) == Some(CALCULATION_ENGINE)
        {
            continue;
        }
        cluster.push(id);
        stack.extend(archive_references(&stream.tree, object.info));
    }
}

// ----- merged cells -----

const CELL_RECORD_TILE: u32 = 4009;
const RANGE_PRECEDENTS_TILE: u32 = 4010;
/// The calculation-engine function a merge formula applies to its range.
const MERGE_FUNCTION: u64 = 168;

/// Writes merged regions the way Pages stores them: one formula per region in
/// the table's merge owner (`range(top-left, bottom-right)` passed to the merge
/// function), each recorded as a formula cell of the merge owner that depends
/// on its range of the table, in the owner's dependency object, the engine's
/// owner info, and a cell-record and a range-precedents tile.
fn write_merges(
    package: &mut Package,
    table: &TemplateTable,
    merges: &[(usize, usize, usize, usize)],
    next_id: &mut u64,
) -> Result<(), PackageError> {
    let stream = stream_containing(package, table.model_id)?;
    let tree = &mut stream.tree;
    let model_first = stream
        .objects
        .iter()
        .find(|object| object.identifier == table.model_id)
        .map(|object| object.messages[0].first)
        .ok_or_else(|| malformed("table model is missing"))?;

    // The table's and its merge owner's UUIDs, and their engine owners.
    let table_uuid = str_field(tree, model_first, "table_id")
        .and_then(|text| parse_uuid_text(text.as_bytes()))
        .ok_or_else(|| malformed("table model has no table_id"))?;
    let merge_owner = message_field(tree, model_first, "merge_owner")
        .ok_or_else(|| malformed("table model has no merge owner"))?;
    let merge_uuid = message_field(tree, merge_owner, "owner_id")
        .and_then(|id| {
            chain_uuids(tree, id).into_iter().find_map(|at| match at {
                UuidAt::Words(_, bytes) => Some(bytes),
                _ => None,
            })
        })
        .ok_or_else(|| malformed("merge owner has no id"))?;
    let owner_of = |tree: &Tree, uuid: [u8; 16]| -> Option<(u64, u64)> {
        stream_objects_by_type(&stream.objects, FORMULA_OWNER_DEPENDENCIES)
            .into_iter()
            .find_map(|(id, first)| {
                let uid = message_field(tree, first, "formula_owner_uid")?;
                let matches = chain_uuids(tree, uid).iter().any(|at| match at {
                    UuidAt::Pair(_, bytes) => *bytes == uuid,
                    _ => false,
                });
                if !matches {
                    return None;
                }
                match field_value(tree, first, "internal_formula_owner_id")? {
                    Node::Uint(internal) => Some((id, internal)),
                    _ => None,
                }
            })
    };
    let (_, table_owner) =
        owner_of(tree, table_uuid).ok_or_else(|| malformed("table owner is not registered"))?;
    let (merge_deps, merge_internal) =
        owner_of(tree, merge_uuid).ok_or_else(|| malformed("merge owner is not registered"))?;

    let cell_tile_id = *next_id;
    let range_tile_id = *next_id + 1;
    *next_id += 2;

    // 1. The formulas, in the model's merge-owner formula store.
    let model = message_ref("TST.TableModelArchive")?;
    let owner_ref = child_message(model, "merge_owner")?;
    let store_ref = child_message(owner_ref, "formula_store")?;
    let pair_ref = child_message(store_ref, "formulas")?;
    let formula_ref = child_message(pair_ref, "formula")?;
    let array_ref = child_message(formula_ref, "AST_node_array")?;
    let node_ref = child_message(array_ref, "AST_node")?;
    let column_ref = child_message(node_ref, "AST_column")?;
    let row_ref = child_message(node_ref, "AST_row")?;
    let extra_ref = child_message(node_ref, "AST_cross_table_reference_extra_info")?;
    let table_id_ref = child_message(extra_ref, "table_id")?;
    let mut store = Chain::new();
    push_field(
        tree,
        &mut store,
        store_ref,
        "next_formula_index",
        Node::Uint(merges.len() as u64),
    )?;
    for (index, &(row, column, rows, columns)) in merges.iter().enumerate() {
        let mut nodes = Chain::new();
        for (c, r) in [(column, row), (column + columns - 1, row + rows - 1)] {
            let mut col = Chain::new();
            push_field(tree, &mut col, column_ref, "column", Node::Uint(c as u64))?;
            push_field(tree, &mut col, column_ref, "absolute", Node::Bool(true))?;
            let mut rw = Chain::new();
            push_field(tree, &mut rw, row_ref, "row", Node::Uint(r as u64))?;
            push_field(tree, &mut rw, row_ref, "absolute", Node::Bool(true))?;
            let mut words = Chain::new();
            for (slot, name) in ["uuid_w0", "uuid_w1", "uuid_w2", "uuid_w3"]
                .iter()
                .enumerate()
            {
                let word =
                    u32::from_le_bytes(table_uuid[slot * 4..slot * 4 + 4].try_into().expect("4"));
                push_field(
                    tree,
                    &mut words,
                    table_id_ref,
                    name,
                    Node::Uint(u64::from(word)),
                )?;
            }
            let mut extra = Chain::new();
            push_field(
                tree,
                &mut extra,
                extra_ref,
                "table_id",
                Node::Message(words.first),
            )?;
            let mut node = Chain::new();
            push_field(tree, &mut node, node_ref, "AST_node_type", Node::Uint(36))?;
            push_field(
                tree,
                &mut node,
                node_ref,
                "AST_column",
                Node::Message(col.first),
            )?;
            push_field(
                tree,
                &mut node,
                node_ref,
                "AST_row",
                Node::Message(rw.first),
            )?;
            push_field(
                tree,
                &mut node,
                node_ref,
                "AST_cross_table_reference_extra_info",
                Node::Message(extra.first),
            )?;
            push_field(
                tree,
                &mut nodes,
                array_ref,
                "AST_node",
                Node::Message(node.first),
            )?;
        }
        let mut colon = Chain::new();
        push_field(tree, &mut colon, node_ref, "AST_node_type", Node::Uint(29))?;
        push_field(
            tree,
            &mut nodes,
            array_ref,
            "AST_node",
            Node::Message(colon.first),
        )?;
        let mut function = Chain::new();
        push_field(
            tree,
            &mut function,
            node_ref,
            "AST_node_type",
            Node::Uint(16),
        )?;
        push_field(
            tree,
            &mut function,
            node_ref,
            "AST_function_node_index",
            Node::Uint(MERGE_FUNCTION),
        )?;
        push_field(
            tree,
            &mut function,
            node_ref,
            "AST_function_node_numArgs",
            Node::Uint(1),
        )?;
        push_field(
            tree,
            &mut nodes,
            array_ref,
            "AST_node",
            Node::Message(function.first),
        )?;
        let mut formula = Chain::new();
        push_field(
            tree,
            &mut formula,
            formula_ref,
            "AST_node_array",
            Node::Message(nodes.first),
        )?;
        let mut pair = Chain::new();
        push_field(
            tree,
            &mut pair,
            pair_ref,
            "formula_index",
            Node::Uint(index as u64),
        )?;
        push_field(
            tree,
            &mut pair,
            pair_ref,
            "formula",
            Node::Message(formula.first),
        )?;
        push_field(
            tree,
            &mut store,
            store_ref,
            "formulas",
            Node::Message(pair.first),
        )?;
    }
    replace_message_field(tree, merge_owner, owner_ref, "formula_store", store.first)?;

    // 2. The dependency records, in the owner's dependency object and the
    // engine's owner info (which also flags each cell as a formula).
    let deps_ref = message_ref("TSCE.FormulaOwnerDependenciesArchive")?;
    let deps_first = stream
        .objects
        .iter()
        .find(|object| object.identifier == merge_deps)
        .map(|object| (object.messages[0].first, object.info))
        .ok_or_else(|| malformed("merge owner dependencies are missing"))?;
    let cells = build_merge_cells(tree, deps_ref, merges.len(), false)?;
    replace_message_field(tree, deps_first.0, deps_ref, "cell_dependencies", cells)?;
    let ranges = build_merge_ranges(tree, deps_ref, merges, table_owner)?;
    replace_message_field(tree, deps_first.0, deps_ref, "range_dependencies", ranges)?;
    let tiled_cells_ref = child_message(deps_ref, "tiled_cell_dependencies")?;
    let mut tiled_cells = Chain::new();
    push_field(
        tree,
        &mut tiled_cells,
        tiled_cells_ref,
        "cell_record_tiles",
        Node::Reference(cell_tile_id),
    )?;
    replace_message_field(
        tree,
        deps_first.0,
        deps_ref,
        "tiled_cell_dependencies",
        tiled_cells.first,
    )?;
    let tiled_ranges_ref = child_message(deps_ref, "tiled_range_dependencies")?;
    let mut tiled_ranges = Chain::new();
    push_field(
        tree,
        &mut tiled_ranges,
        tiled_ranges_ref,
        "range_precedents_tile",
        Node::Reference(range_tile_id),
    )?;
    replace_message_field(
        tree,
        deps_first.0,
        deps_ref,
        "tiled_range_dependencies",
        tiled_ranges.first,
    )?;
    add_object_references(tree, deps_first.1, &[cell_tile_id, range_tile_id])?;

    let engine_first = stream
        .objects
        .iter()
        .find(|object| first_type(object) == Some(CALCULATION_ENGINE))
        .map(|object| object.messages[0].first)
        .ok_or_else(|| malformed("calculation engine is missing"))?;
    if let Some(tracker) = message_field(tree, engine_first, "dependency_tracker") {
        let info_ref = child_message(
            child_message(
                message_ref("TSCE.CalculationEngineArchive")?,
                "dependency_tracker",
            )?,
            "formula_owner_info",
        )?;
        let infos: Vec<u32> = tree
            .chain(tracker)
            .filter(|(_, entry)| tree.field(entry).map(|f| f.name) == Some("formula_owner_info"))
            .filter_map(|(_, entry)| match entry.value {
                Node::Message(first) => Some(first),
                _ => None,
            })
            .collect();
        for info in infos {
            let is_merge = message_field(tree, info, "formula_owner_id").is_some_and(|id| {
                chain_uuids(tree, id)
                    .iter()
                    .any(|at| matches!(at, UuidAt::Words(_, b) if *b == merge_uuid))
            });
            if is_merge {
                let cells = build_merge_cells(tree, info_ref, merges.len(), true)?;
                replace_message_field(tree, info, info_ref, "cell_dependencies", cells)?;
                let ranges = build_merge_ranges(tree, info_ref, merges, table_owner)?;
                replace_message_field(tree, info, info_ref, "range_dependencies", ranges)?;
            }
        }
        if let Some(index) = field_entry(tree, tracker, "number_of_formulas")
            && let Node::Uint(count) = tree.entries[index as usize].value
        {
            tree.entries[index as usize].value = Node::Uint(count + merges.len() as u64);
        }
    }

    // 3. The tiles.
    let cell_tile_ref = message_ref("TSCE.CellRecordTileArchive")?;
    let record_ref = child_message(cell_tile_ref, "cell_records")?;
    let mut cell_tile = Chain::new();
    push_field(
        tree,
        &mut cell_tile,
        cell_tile_ref,
        "internal_owner_id",
        Node::Uint(merge_internal),
    )?;
    push_field(
        tree,
        &mut cell_tile,
        cell_tile_ref,
        "tile_column_begin",
        Node::Uint(0),
    )?;
    push_field(
        tree,
        &mut cell_tile,
        cell_tile_ref,
        "tile_row_begin",
        Node::Uint(0),
    )?;
    for index in 0..merges.len() {
        let mut record = Chain::new();
        push_field(
            tree,
            &mut record,
            record_ref,
            "column",
            Node::Uint(index as u64),
        )?;
        push_field(tree, &mut record, record_ref, "row", Node::Uint(0))?;
        push_field(
            tree,
            &mut record,
            record_ref,
            "expanded_edges",
            Node::Message(NONE),
        )?;
        push_field(
            tree,
            &mut cell_tile,
            cell_tile_ref,
            "cell_records",
            Node::Message(record.first),
        )?;
    }
    let range_tile_ref = message_ref("TSCE.RangePrecedentsTileArchive")?;
    let from_to_ref = child_message(range_tile_ref, "from_to_range")?;
    let from_ref = child_message(from_to_ref, "from_coord")?;
    let rect_ref = child_message(from_to_ref, "refers_to_rect")?;
    let origin_ref = child_message(rect_ref, "origin")?;
    let size_ref = child_message(rect_ref, "size")?;
    let mut range_tile = Chain::new();
    push_field(
        tree,
        &mut range_tile,
        range_tile_ref,
        "to_owner_id",
        Node::Uint(table_owner),
    )?;
    for (index, &(row, column, rows, columns)) in merges.iter().enumerate() {
        let mut from = Chain::new();
        push_field(
            tree,
            &mut from,
            from_ref,
            "column",
            Node::Uint(index as u64),
        )?;
        push_field(tree, &mut from, from_ref, "row", Node::Uint(0))?;
        let mut origin = Chain::new();
        push_field(
            tree,
            &mut origin,
            origin_ref,
            "column",
            Node::Uint(column as u64),
        )?;
        push_field(tree, &mut origin, origin_ref, "row", Node::Uint(row as u64))?;
        let mut size = Chain::new();
        if columns > 1 {
            push_field(
                tree,
                &mut size,
                size_ref,
                "num_columns",
                Node::Uint(columns as u64),
            )?;
        }
        if rows > 1 {
            push_field(
                tree,
                &mut size,
                size_ref,
                "num_rows",
                Node::Uint(rows as u64),
            )?;
        }
        let mut rect = Chain::new();
        push_field(
            tree,
            &mut rect,
            rect_ref,
            "origin",
            Node::Message(origin.first),
        )?;
        push_field(tree, &mut rect, rect_ref, "size", Node::Message(size.first))?;
        let mut entry = Chain::new();
        push_field(
            tree,
            &mut entry,
            from_to_ref,
            "from_coord",
            Node::Message(from.first),
        )?;
        push_field(
            tree,
            &mut entry,
            from_to_ref,
            "refers_to_rect",
            Node::Message(rect.first),
        )?;
        push_field(
            tree,
            &mut range_tile,
            range_tile_ref,
            "from_to_range",
            Node::Message(entry.first),
        )?;
    }
    for (id, kind, first) in [
        (cell_tile_id, CELL_RECORD_TILE, cell_tile.first),
        (range_tile_id, RANGE_PRECEDENTS_TILE, range_tile.first),
    ] {
        let info = build_archive_info(tree, id, kind)?;
        stream.objects.push(Object {
            identifier: id,
            info,
            messages: vec![ObjectMessage {
                message_type: kind,
                first,
            }],
        });
    }
    Ok(())
}

/// A dependency map's `cell_dependencies`: one record per merge formula,
/// at (formula index, 0). The engine's owner info also flags each a formula.
fn build_merge_cells(
    tree: &mut Tree,
    parent: MessageRef,
    count: usize,
    legacy: bool,
) -> Result<u32, PackageError> {
    let cells_ref = child_message(parent, "cell_dependencies")?;
    let record_ref = child_message(cells_ref, "cell_record")?;
    let mut cells = Chain::new();
    for index in 0..count {
        let mut record = Chain::new();
        push_field(
            tree,
            &mut record,
            record_ref,
            "column",
            Node::Uint(index as u64),
        )?;
        push_field(tree, &mut record, record_ref, "row", Node::Uint(0))?;
        if legacy {
            push_field(
                tree,
                &mut record,
                record_ref,
                "contains_a_formula",
                Node::Bool(true),
            )?;
            push_field(tree, &mut record, record_ref, "edges", Node::Message(NONE))?;
        } else {
            push_field(
                tree,
                &mut record,
                record_ref,
                "expanded_edges",
                Node::Message(NONE),
            )?;
        }
        push_field(
            tree,
            &mut cells,
            cells_ref,
            "cell_record",
            Node::Message(record.first),
        )?;
    }
    Ok(cells.first)
}

/// A dependency map's `range_dependencies`: each merge formula depends on its
/// region of the table (owner `table_owner`).
fn build_merge_ranges(
    tree: &mut Tree,
    parent: MessageRef,
    merges: &[(usize, usize, usize, usize)],
    table_owner: u64,
) -> Result<u32, PackageError> {
    let ranges_ref = child_message(parent, "range_dependencies")?;
    let back_ref = child_message(ranges_ref, "back_dependency")?;
    let reference_ref = child_message(back_ref, "internal_range_reference")?;
    let range_ref = child_message(reference_ref, "range")?;
    let mut ranges = Chain::new();
    for (index, &(row, column, rows, columns)) in merges.iter().enumerate() {
        let mut range = Chain::new();
        push_field(
            tree,
            &mut range,
            range_ref,
            "top_left_column",
            Node::Uint(column as u64),
        )?;
        push_field(
            tree,
            &mut range,
            range_ref,
            "top_left_row",
            Node::Uint(row as u64),
        )?;
        push_field(
            tree,
            &mut range,
            range_ref,
            "bottom_right_column",
            Node::Uint((column + columns - 1) as u64),
        )?;
        push_field(
            tree,
            &mut range,
            range_ref,
            "bottom_right_row",
            Node::Uint((row + rows - 1) as u64),
        )?;
        let mut reference = Chain::new();
        push_field(
            tree,
            &mut reference,
            reference_ref,
            "owner_id",
            Node::Uint(table_owner),
        )?;
        push_field(
            tree,
            &mut reference,
            reference_ref,
            "range",
            Node::Message(range.first),
        )?;
        let mut back = Chain::new();
        push_field(tree, &mut back, back_ref, "cell_coord_row", Node::Uint(0))?;
        push_field(
            tree,
            &mut back,
            back_ref,
            "cell_coord_column",
            Node::Uint(index as u64),
        )?;
        push_field(
            tree,
            &mut back,
            back_ref,
            "internal_range_reference",
            Node::Message(reference.first),
        )?;
        push_field(
            tree,
            &mut ranges,
            ranges_ref,
            "back_dependency",
            Node::Message(back.first),
        )?;
    }
    Ok(ranges.first)
}

/// Sets a message field of `first`'s chain to `value` (a chain start),
/// replacing it in place or inserting it in field-number order.
fn replace_message_field(
    tree: &mut Tree,
    first: u32,
    message: MessageRef,
    name: &str,
    value: u32,
) -> Result<(), PackageError> {
    if let Some(index) = field_entry(tree, first, name) {
        tree.entries[index as usize].value = Node::Message(value);
        return Ok(());
    }
    let (slot, field) = message
        .slot_named(name)
        .ok_or_else(|| malformed("field is not in the schema"))?;
    let mut previous = None;
    for (index, entry) in tree.chain(first) {
        if entry.number < field.number {
            previous = Some(index);
        }
    }
    let previous = previous.ok_or_else(|| malformed("cannot place field ahead of a chain"))?;
    let mut chain = Chain::new();
    let new_index = tree
        .push_known(
            &mut chain,
            message,
            slot,
            field,
            field.number,
            Node::Message(value),
        )
        .map_err(tree_error)?;
    tree.entries[new_index as usize].next = tree.entries[previous as usize].next;
    tree.entries[previous as usize].next = new_index;
    Ok(())
}

fn stream_objects_by_type(objects: &[Object], kind: u32) -> Vec<(u64, u32)> {
    objects
        .iter()
        .filter(|object| first_type(object) == Some(kind))
        .map(|object| (object.identifier, object.messages[0].first))
        .collect()
}

/// Sets the table owner's `total_range_for_table` and `body_range_for_table`
/// (under both spanning dependency maps) to the table's real size: the
/// calculation engine resolves every range into the table against these, so a
/// resized table keeping the template's would put cells outside its grid.
fn set_owner_table_ranges(
    package: &mut Package,
    table: &TemplateTable,
    mark: &TableMark,
) -> Result<(), PackageError> {
    let stream = stream_containing(package, table.model_id)?;
    let model_first = stream
        .objects
        .iter()
        .find(|object| object.identifier == table.model_id)
        .map(|object| object.messages[0].first)
        .ok_or_else(|| malformed("table model is missing"))?;
    let Some(table_uuid) = str_field(&stream.tree, model_first, "table_id")
        .and_then(|text| parse_uuid_text(text.as_bytes()))
    else {
        return Ok(());
    };
    let owner = stream_objects_by_type(&stream.objects, FORMULA_OWNER_DEPENDENCIES)
        .into_iter()
        .find(|(_, first)| {
            message_field(&stream.tree, *first, "formula_owner_uid").is_some_and(|uid| {
                chain_uuids(&stream.tree, uid)
                    .iter()
                    .any(|at| matches!(at, UuidAt::Pair(_, bytes) if *bytes == table_uuid))
            })
        });
    let Some((_, owner_first)) = owner else {
        return Ok(());
    };
    let tree = &mut stream.tree;
    let last_column = mark.columns.saturating_sub(1) as u64;
    let last_row = mark.rows.saturating_sub(1) as u64;
    let body_top = u64::from(mark.header_rows).min(last_row);
    for spanning in ["spanning_column_dependencies", "spanning_row_dependencies"] {
        let Some(map) = message_field(tree, owner_first, spanning) else {
            continue;
        };
        for (name, top) in [
            ("total_range_for_table", 0),
            ("body_range_for_table", body_top),
        ] {
            let Some(range) = message_field(tree, map, name) else {
                continue;
            };
            set_field_uint(tree, range, "top_left_column", 0);
            set_field_uint(tree, range, "top_left_row", top);
            set_field_uint(tree, range, "bottom_right_column", last_column);
            set_field_uint(tree, range, "bottom_right_row", last_row);
        }
    }
    Ok(())
}

// ----- headers and footers -----

const NUMBER_ATTACHMENT: u32 = 2043;

#[derive(Clone, Copy, PartialEq)]
enum PageVariant {
    Default,
    First,
    Even,
}

/// One header or footer area to write: which page template, header or footer,
/// which of the three areas (left, center, right), and its content.
struct PageArea {
    variant: PageVariant,
    footer: bool,
    position: usize,
    content: CellContent,
}

/// A section's headers and footers as Pages areas: each paragraph goes to the
/// left, center, or right area by its alignment, the way Pages lays out a
/// header natively.
fn page_areas(document: &Document, section: &crate::document::Section) -> Vec<PageArea> {
    let mut areas = Vec::new();
    for (footer, variants) in [(false, &section.headers), (true, &section.footers)] {
        for (variant, blocks) in [
            (PageVariant::Default, &variants.default),
            (PageVariant::First, &variants.first),
            (PageVariant::Even, &variants.even),
        ] {
            let Some(blocks) = blocks else {
                continue;
            };
            let mut buckets: [Vec<Line<'_>>; 3] = [Vec::new(), Vec::new(), Vec::new()];
            for block in blocks {
                let Block::Paragraph(paragraph) = block else {
                    // A table in a header keeps its text, row by row, left.
                    buckets[0].extend(block_lines(std::slice::from_ref(block)));
                    continue;
                };
                let position = match document.paragraph_properties(paragraph).alignment {
                    Some(crate::document::Alignment::Center) => 1,
                    Some(crate::document::Alignment::Right) => 2,
                    _ => 0,
                };
                buckets[position].push(vec![vec![paragraph]]);
            }
            for (position, lines) in buckets.iter().enumerate() {
                let content = flatten_lines(document, lines, &mut ListCounters::default());
                if content.text.trim().is_empty() && content.fields.is_empty() {
                    continue;
                }
                areas.push(PageArea {
                    variant,
                    footer,
                    position,
                    content,
                });
            }
        }
    }
    areas
}

/// Writes each area into the matching storage of the template's section page
/// templates (odd for the default, first, even), keeping the storage's own
/// paragraph style, and turns on first-page or odd/even variation when used.
fn write_page_areas(
    package: &mut Package,
    areas: &[PageArea],
    formats: &HashMap<Format, u64>,
    styles: CellStyles,
    paras: &ParaStyles,
    next_id: &mut u64,
) -> Result<(), PackageError> {
    if areas.is_empty() {
        return Ok(());
    }
    // The section and its page templates.
    let (section_id, templates) = {
        let stream = document_stream(package)?;
        let mut found = None;
        for object in &stream.objects {
            let first = object.messages[0].first;
            let reference = |name| match field_value(&stream.tree, first, name) {
                Some(Node::Reference(id)) => Some(id),
                _ => None,
            };
            if let Some(odd) = reference("odd_section_template_page") {
                found = Some((
                    object.identifier,
                    [
                        odd,
                        reference("first_section_template_page").unwrap_or(odd),
                        reference("even_section_template_page").unwrap_or(odd),
                    ],
                ));
                break;
            }
        }
        found.ok_or_else(|| malformed("section has no page templates"))?
    };
    for area in areas {
        let template = templates[match area.variant {
            PageVariant::Default => 0,
            PageVariant::First => 1,
            PageVariant::Even => 2,
        }];
        let storages: Vec<u64> = {
            let (tree, first) = object_message(package, template)
                .ok_or_else(|| malformed("page template is missing"))?;
            let name = if area.footer { "footers" } else { "headers" };
            tree.chain(first)
                .filter(|(_, entry)| tree.field(entry).map(|f| f.name) == Some(name))
                .filter_map(|(_, entry)| match entry.value {
                    Node::Reference(id) => Some(id),
                    _ => None,
                })
                .collect()
        };
        let Some(&storage_id) = storages.get(area.position) else {
            continue;
        };
        // Keep the area's own paragraph style (the template's header/footer).
        let paragraph = object_message(package, storage_id)
            .and_then(|(tree, first)| {
                let table = message_field(tree, first, "table_para_style")?;
                let entry = message_field(tree, table, "entries")?;
                reference_of(field_value(tree, entry, "object"))
            })
            .unwrap_or(styles.paragraph);
        let area_styles = CellStyles {
            paragraph,
            ..styles
        };
        let stream = stream_containing(package, storage_id)?;
        let attachments = create_number_attachments(stream, &area.content.fields, next_id)?;
        let (message, refs) = build_text_storage(
            &mut stream.tree,
            &area.content,
            formats,
            area_styles,
            Some(1),
            &attachments,
            paras,
        )?;
        let object = stream
            .objects
            .iter_mut()
            .find(|object| object.identifier == storage_id)
            .ok_or_else(|| malformed("header storage is missing"))?;
        object.messages[0].first = message;
        let info = object.info;
        add_object_references(&mut stream.tree, info, &refs)?;
    }
    // Variation flags on the section.
    let first_used = areas.iter().any(|area| area.variant == PageVariant::First);
    let even_used = areas.iter().any(|area| area.variant == PageVariant::Even);
    let stream = document_stream(package)?;
    let first = stream
        .objects
        .iter()
        .find(|object| object.identifier == section_id)
        .map(|object| object.messages[0].first)
        .ok_or_else(|| malformed("section is missing"))?;
    for (name, value) in [
        ("section_template_first_page_different", first_used),
        ("section_template_even_odd_pages_different", even_used),
        ("inherit_previous_header_footer", false),
    ] {
        if let Some(index) = field_entry(&stream.tree, first, name) {
            stream.tree.entries[index as usize].value = Node::Bool(value);
        }
    }
    Ok(())
}

/// Creates a `TSWP.NumberAttachmentArchive` (page number or page count,
/// decimal) in `stream` for each field, returning (offset, object id) pairs.
fn create_number_attachments(
    stream: &mut Stream,
    fields: &[(u32, u64)],
    next_id: &mut u64,
) -> Result<Vec<(u32, u64)>, PackageError> {
    let archive = message_ref("TSWP.NumberAttachmentArchive")?;
    let base = child_message(archive, "super")?;
    let mut out = Vec::new();
    for &(offset, kind) in fields {
        let id = *next_id;
        *next_id += 1;
        let tree = &mut stream.tree;
        let mut super_chain = Chain::new();
        push_field(tree, &mut super_chain, base, "kind", Node::Uint(kind))?;
        let mut chain = Chain::new();
        push_field(
            tree,
            &mut chain,
            archive,
            "super",
            Node::Message(super_chain.first),
        )?;
        push_field(tree, &mut chain, archive, "number_format", Node::Uint(0))?;
        let name = tree.push_bytes(b"decimal").map_err(tree_error)?;
        push_field(
            tree,
            &mut chain,
            archive,
            "number_format_name",
            Node::Str(name),
        )?;
        let info = build_archive_info(tree, id, NUMBER_ATTACHMENT)?;
        stream.objects.push(Object {
            identifier: id,
            info,
            messages: vec![ObjectMessage {
                message_type: NUMBER_ATTACHMENT,
                first: chain.first,
            }],
        });
        out.push((offset, id));
    }
    Ok(out)
}

/// Records `object_id` as a user of data `data_id` in the component's data
/// references, as Pages does when several images share one data file.
fn add_data_user(package: &mut Package, data_id: u64, object_id: u64) -> Result<(), PackageError> {
    let (stream, meta_first) =
        metadata_message(package).ok_or_else(|| malformed("package metadata is missing"))?;
    let tree = &mut stream.tree;
    let Some((_, reference)) = component_data_reference(tree, meta_first, data_id) else {
        return Ok(());
    };
    let Some(list) = message_field(tree, reference, "object_reference_list") else {
        return Ok(());
    };
    let copy = clone_chain(tree, list, &mut |_| {})?;
    set_field_uint(tree, copy, "object_identifier", object_id);
    let components = message_of(
        message_ref("TSP.PackageMetadata")?
            .field_named("components")
            .ok_or_else(|| malformed("metadata has no components"))?
            .kind,
    )?;
    let reference_ref = child_message(components, "data_references")?;
    append_message_field(
        tree,
        reference_ref,
        reference,
        "object_reference_list",
        copy,
    )?;
    Ok(())
}

/// A rough upper-bound estimate of a table's tallest row, in points: each
/// cell's paragraphs wrapped to the cell's width at their font size.
fn max_row_height(document: &Document, table: &crate::document::Table, widths: &[f32]) -> f32 {
    let mut tallest: f32 = 0.0;
    for row in &table.rows {
        let mut row_height: f32 = row.height.unwrap_or(0.0);
        for (column, cell) in row.cells.iter().enumerate() {
            let span = cell.column_span.max(1) as usize;
            let width: f32 = widths.iter().skip(column).take(span).sum::<f32>().max(24.0);
            if cell.merge != crate::document::Merge::Origin {
                continue;
            }
            let mut paragraphs = Vec::new();
            collect_paragraphs(&cell.blocks, &mut paragraphs);
            let mut height = 0.0;
            for paragraph in paragraphs {
                let mut characters = 0usize;
                let mut size: f32 = 11.0;
                for run in &paragraph.runs {
                    if let Inline::Text(span) = run.content {
                        characters += document.text(span).chars().count();
                        if let Some(points) = document.effective_run(paragraph, run).size {
                            size = size.max(points);
                        }
                    }
                }
                // Deliberately generous (wide average glyphs, the paragraph
                // spacing Pages' styles add): unwrapping a table that would
                // have fitted costs only its box, while a clipped row hides text.
                let per_line = ((width - 12.0).max(16.0) / (size * 0.6)).max(1.0);
                let lines = (characters as f32 / per_line).ceil().max(1.0);
                height += lines * size * 1.35 + size * 1.6;
            }
            row_height = row_height.max(height);
        }
        tallest = tallest.max(row_height);
    }
    tallest
}

// ----- paragraph formatting -----

/// Paragraph-style variations by (parent style, formatting).
type ParaStyles = HashMap<(u64, ParaFormat), u64>;

/// A paragraph's formatting, as Pages paragraph properties: alignment (Pages'
/// numbering), indents and spacing in hundredths of a point, and line spacing
/// as (mode, amount x 100). Hundredths keep it hashable.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
struct ParaFormat {
    alignment: Option<u8>,
    first_line_indent: Option<i32>,
    left_indent: Option<i32>,
    right_indent: Option<i32>,
    space_before: Option<i32>,
    space_after: Option<i32>,
    line_spacing: Option<(u8, i32)>,
}

impl ParaFormat {
    fn overrides(&self) -> u64 {
        [
            self.alignment.is_some(),
            self.first_line_indent.is_some(),
            self.left_indent.is_some(),
            self.right_indent.is_some(),
            self.space_before.is_some(),
            self.space_after.is_some(),
            self.line_spacing.is_some(),
        ]
        .iter()
        .filter(|set| **set)
        .count() as u64
    }
}

/// A paragraph's effective formatting (its style's, then its own), so Pages
/// lays it out as Word does even where Pages' own style defaults differ.
fn para_format(document: &Document, paragraph: &Paragraph) -> ParaFormat {
    let properties = document.effective_paragraph(paragraph);
    let hundredths = |value: Option<f32>| {
        value
            .filter(|value| value.is_finite())
            .map(|value| (value * 100.0).round() as i32)
    };
    ParaFormat {
        alignment: properties.alignment.map(|alignment| match alignment {
            crate::document::Alignment::Left => 0,
            crate::document::Alignment::Right => 1,
            crate::document::Alignment::Center => 2,
            crate::document::Alignment::Justify => 3,
        }),
        // Pages measures the first line from the margin, not from the left
        // indent: a paragraph indented as a whole starts its first line there too.
        // A list paragraph takes its indents from its list style, as Pages
        // imports a Word list; its own are zero.
        first_line_indent: if paragraph.list.is_some() {
            Some(0)
        } else {
            hundredths(properties.first_line_indent.or(properties.left_indent))
        },
        left_indent: if paragraph.list.is_some() {
            Some(0)
        } else {
            hundredths(properties.left_indent)
        },
        right_indent: hundredths(properties.right_indent),
        space_before: hundredths(properties.space_before),
        space_after: hundredths(properties.space_after),
        line_spacing: properties.line_spacing.and_then(|spacing| {
            let (mode, amount) = match spacing {
                crate::document::LineSpacing::Relative(amount) => (0, amount),
                crate::document::LineSpacing::Minimum(amount) => (1, amount),
                crate::document::LineSpacing::Exact(amount) => (2, amount),
            };
            amount
                .is_finite()
                .then(|| (mode, (amount * 100.0).round() as i32))
        }),
    }
}

/// Creates a `TSWP.ParagraphStyleArchive` variation (in the document stream)
/// for each distinct (parent, formatting) that sets anything, as Pages writes
/// a paragraph's direct formatting.
fn synthesize_para_styles(
    package: &mut Package,
    needed: Vec<(u64, ParaFormat)>,
    stylesheet: u64,
    next_id: &mut u64,
) -> Result<(ParaStyles, Vec<u64>), PackageError> {
    let archive = message_ref("TSWP.ParagraphStyleArchive")?;
    let base = child_message(archive, "super")?;
    let properties = child_message(archive, "para_properties")?;
    let spacing = child_message(properties, "line_spacing")?;
    let mut map: ParaStyles = HashMap::new();
    let mut created = Vec::new();
    let stream = document_stream(package)?;
    for key in needed {
        let (parent, format) = key;
        if format.overrides() == 0 || map.contains_key(&key) {
            continue;
        }
        let id = *next_id;
        *next_id += 1;
        let tree = &mut stream.tree;
        let mut style = Chain::new();
        push_field(tree, &mut style, base, "parent", Node::Reference(parent))?;
        push_field(tree, &mut style, base, "is_variation", Node::Bool(true))?;
        push_field(
            tree,
            &mut style,
            base,
            "stylesheet",
            Node::Reference(stylesheet),
        )?;
        let mut props = Chain::new();
        let points = |value: i32| Node::Float(value as f32 / 100.0);
        if let Some(alignment) = format.alignment {
            push_field(
                tree,
                &mut props,
                properties,
                "alignment",
                Node::Uint(u64::from(alignment)),
            )?;
        }
        if let Some(value) = format.first_line_indent {
            push_field(
                tree,
                &mut props,
                properties,
                "first_line_indent",
                points(value),
            )?;
        }
        if let Some(value) = format.left_indent {
            push_field(tree, &mut props, properties, "left_indent", points(value))?;
        }
        if let Some((mode, amount)) = format.line_spacing {
            let mut line = Chain::new();
            push_field(
                tree,
                &mut line,
                spacing,
                "mode",
                Node::Uint(u64::from(mode)),
            )?;
            push_field(tree, &mut line, spacing, "amount", points(amount))?;
            push_field(
                tree,
                &mut props,
                properties,
                "line_spacing",
                Node::Message(line.first),
            )?;
        }
        if let Some(value) = format.right_indent {
            push_field(tree, &mut props, properties, "right_indent", points(value))?;
        }
        if let Some(value) = format.space_after {
            push_field(tree, &mut props, properties, "space_after", points(value))?;
        }
        if let Some(value) = format.space_before {
            push_field(tree, &mut props, properties, "space_before", points(value))?;
        }
        let mut chain = Chain::new();
        push_field(
            tree,
            &mut chain,
            archive,
            "super",
            Node::Message(style.first),
        )?;
        push_field(
            tree,
            &mut chain,
            archive,
            "override_count",
            Node::Uint(format.overrides()),
        )?;
        push_field(
            tree,
            &mut chain,
            archive,
            "char_properties",
            Node::Message(NONE),
        )?;
        push_field(
            tree,
            &mut chain,
            archive,
            "para_properties",
            Node::Message(props.first),
        )?;
        let info = build_archive_info(tree, id, PARAGRAPH_STYLE)?;
        add_object_references(tree, info, &[parent, stylesheet])?;
        stream.objects.push(Object {
            identifier: id,
            info,
            messages: vec![ObjectMessage {
                message_type: PARAGRAPH_STYLE,
                first: chain.first,
            }],
        });
        map.insert(key, id);
        created.push(id);
    }
    let pairs: Vec<(u64, u64)> = map.iter().map(|((parent, _), id)| (*parent, *id)).collect();
    register_in_stylesheet(package, stylesheet, &pairs)?;
    Ok((map, created))
}

/// Registers style variations with the stylesheet, as Pages does: each in its
/// `styles` list and under its parent in `parent_to_children_style_map`. Pages
/// repairs a paragraph that points at an unregistered style on load, dropping
/// the formatting.
fn register_in_stylesheet(
    package: &mut Package,
    stylesheet: u64,
    pairs: &[(u64, u64)],
) -> Result<(), PackageError> {
    if pairs.is_empty() {
        return Ok(());
    }
    let sheet_ref = message_ref("TSS.StylesheetArchive")?;
    let map_ref = child_message(sheet_ref, "parent_to_children_style_map")?;
    let stream = stream_containing(package, stylesheet)?;
    let (first, info) = stream
        .objects
        .iter()
        .find(|object| object.identifier == stylesheet)
        .map(|object| (object.messages[0].first, object.info))
        .ok_or_else(|| malformed("stylesheet is missing"))?;
    let tree = &mut stream.tree;
    // Rebuild the chain in field order with the new entries alongside.
    let mut fields: Vec<(u32, Node)> = tree
        .chain(first)
        .map(|(_, entry)| (entry.number, entry.value))
        .collect();
    let styles_number = sheet_ref
        .slot_named("styles")
        .map(|(_, f)| f.number)
        .ok_or_else(|| malformed("no styles field"))?;
    let map_number = sheet_ref
        .slot_named("parent_to_children_style_map")
        .map(|(_, f)| f.number)
        .ok_or_else(|| malformed("no map field"))?;
    for (_, child) in pairs {
        fields.push((styles_number, Node::Reference(*child)));
    }
    let mut parents: Vec<u64> = pairs.iter().map(|(parent, _)| *parent).collect();
    parents.sort_unstable();
    parents.dedup();
    for parent in parents {
        let children: Vec<u64> = pairs
            .iter()
            .filter(|(p, _)| *p == parent)
            .map(|(_, c)| *c)
            .collect();
        // Extend the parent's existing entry if it has one.
        let existing = fields.iter().find_map(|(number, value)| match value {
            Node::Message(entry)
                if *number == map_number
                    && field_value(tree, *entry, "parent") == Some(Node::Reference(parent)) =>
            {
                Some(*entry)
            }
            _ => None,
        });
        match existing {
            Some(entry) => {
                for child in children {
                    append_message_reference(tree, map_ref, entry, "children", child)?;
                }
            }
            None => {
                let mut entry = Chain::new();
                push_field(tree, &mut entry, map_ref, "parent", Node::Reference(parent))?;
                for child in children {
                    push_field(
                        tree,
                        &mut entry,
                        map_ref,
                        "children",
                        Node::Reference(child),
                    )?;
                }
                fields.push((map_number, Node::Message(entry.first)));
            }
        }
    }
    fields.sort_by_key(|(number, _)| *number);
    let mut chain = Chain::new();
    for (number, value) in fields {
        let slot = sheet_ref
            .slot(number)
            .ok_or_else(|| malformed("stylesheet field without a slot"))?;
        let field = sheet_ref
            .field_at(slot)
            .ok_or_else(|| malformed("stylesheet slot out of range"))?;
        tree.push_known(&mut chain, sheet_ref, slot, field, number, value)
            .map_err(tree_error)?;
    }
    let refs: Vec<u64> = pairs.iter().map(|(_, child)| *child).collect();
    add_object_references(tree, info, &refs)?;
    if let Some(object) = stream
        .objects
        .iter_mut()
        .find(|object| object.identifier == stylesheet)
    {
        object.messages[0].first = chain.first;
    }
    Ok(())
}

/// Appends a reference to a repeated field at the end of a chain.
fn append_message_reference(
    tree: &mut Tree,
    parent: MessageRef,
    first: u32,
    name: &str,
    id: u64,
) -> Result<(), PackageError> {
    let (slot, field) = parent
        .slot_named(name)
        .ok_or_else(|| malformed("field is not in the schema"))?;
    let mut last = first;
    for (index, _) in tree.chain(first) {
        last = index;
    }
    let mut chain = Chain { first, last };
    tree.push_known(
        &mut chain,
        parent,
        slot,
        field,
        field.number,
        Node::Reference(id),
    )
    .map_err(tree_error)?;
    Ok(())
}

/// The paragraph style of the template storage a header or footer area is
/// written into (its own "Header"/"Footer" style), the parent of its
/// paragraphs' variations.
fn area_paragraph_style(package: &Package, area: &PageArea) -> Option<u64> {
    for entry in &package.entries {
        let Entry::Stream(stream) = entry else {
            continue;
        };
        for object in &stream.objects {
            let first = object.messages.first()?.first;
            let odd = match field_value(&stream.tree, first, "odd_section_template_page") {
                Some(Node::Reference(id)) => id,
                _ => continue,
            };
            let name = match area.variant {
                PageVariant::Default => None,
                PageVariant::First => Some("first_section_template_page"),
                PageVariant::Even => Some("even_section_template_page"),
            };
            let template = name
                .and_then(|name| reference_of(field_value(&stream.tree, first, name)))
                .unwrap_or(odd);
            let (template_tree, template_first) = object_message(package, template)?;
            let field = if area.footer { "footers" } else { "headers" };
            let storage = template_tree
                .chain(template_first)
                .filter(|(_, entry)| template_tree.field(entry).map(|f| f.name) == Some(field))
                .filter_map(|(_, entry)| match entry.value {
                    Node::Reference(id) => Some(id),
                    _ => None,
                })
                .nth(area.position)?;
            let (storage_tree, storage_first) = object_message(package, storage)?;
            let table = message_field(storage_tree, storage_first, "table_para_style")?;
            let entry = message_field(storage_tree, table, "entries")?;
            return reference_of(field_value(storage_tree, entry, "object"));
        }
    }
    None
}

// ----- text boxes -----

const SHAPE_INFO: u32 = 2011;
const SHAPE_STYLE: u32 = 2025;
const STANDIN_CAPTION: u32 = 3097;
const FLOATING_DRAWABLES: u32 = 10010;
const DRAWABLES_ZORDER: u32 = 10015;

/// A Word text box to write as a Pages shape floating on its page.
struct TextBox {
    fill: Option<crate::document::Color>,
    line: Option<crate::document::Border>,
    geometry: crate::document::ShapeGeometry,
    flip: (bool, bool),
    page: u32,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    content: CellContent,
}

/// The document's text boxes, their text flattened like a cell's.
fn text_boxes(document: &Document) -> Vec<TextBox> {
    document
        .floating
        .iter()
        .filter_map(|floating| match &floating.content {
            crate::document::FloatingContent::TextBox {
                blocks,
                fill,
                line,
                geometry,
                flip,
            } => {
                let mut content =
                    flatten_lines(document, &block_lines(blocks), &mut ListCounters::default());
                let has_text = !content.text.trim().is_empty() || !content.fields.is_empty();
                // A line keeps its thin extent; a box needs room for its text.
                let least = if *geometry == crate::document::ShapeGeometry::Line {
                    1.0
                } else {
                    12.0
                };
                // Pages judges a shape's empty text storage invalid and
                // repairs it on load; a lone space keeps it well formed.
                if content.text.is_empty() {
                    content.text.push(' ');
                }
                (has_text || fill.is_some() || line.is_some()).then(|| TextBox {
                    fill: *fill,
                    line: *line,
                    geometry: *geometry,
                    flip: *flip,
                    page: floating.page,
                    x: floating.x,
                    y: floating.y,
                    width: floating.width.max(least),
                    height: floating.height.max(least),
                    content,
                })
            }
            _ => None,
        })
        .collect()
}

/// Writes each text box as a Pages shape (the "Body" shape style, a text
/// storage of its own) floating on its page at its position, listed in the
/// document's floating drawables and z-order as Pages lists its own.
fn write_text_boxes(
    package: &mut Package,
    boxes: &[TextBox],
    formats: &HashMap<Format, u64>,
    styles: CellStyles,
    paras: &ParaStyles,
    next_id: &mut u64,
) -> Result<(), PackageError> {
    if boxes.is_empty() {
        return Ok(());
    }
    let shape_style = shape_style_identified(package, "textbox-0-shapestyle")
        .ok_or_else(|| malformed("template has no text box shape style"))?;
    // A filled or outlined shape gets a variation of the text-box style with
    // that fill and stroke: text set in white on a coloured shape would
    // otherwise vanish, and a shape without text would not show at all.
    let mut looks: Vec<ShapeLook> = boxes
        .iter()
        .map(|text_box| shape_look(text_box.fill, text_box.line))
        .filter(|look| look.0.is_some() || look.1.is_some())
        .collect();
    looks.sort_by_key(|(fill, line)| {
        let color = |color: Option<crate::document::Color>| {
            color.map(|color| (color.red, color.green, color.blue))
        };
        (
            color(*fill),
            line.map(|(width, stroke)| (width, color(stroke))),
        )
    });
    looks.dedup();
    let fill_styles = match stylesheet_identifier(package) {
        Some(sheet) if !looks.is_empty() => {
            create_fill_styles(package, shape_style, &looks, sheet, next_id)?
        }
        _ => HashMap::new(),
    };
    let stream = document_stream(package)?;
    let (floating_id, zorder_id) = {
        let find = |kind| {
            stream
                .objects
                .iter()
                .find(|object| first_type(object) == Some(kind))
                .map(|object| object.identifier)
        };
        (
            find(FLOATING_DRAWABLES).ok_or_else(|| malformed("no floating drawables"))?,
            find(DRAWABLES_ZORDER).ok_or_else(|| malformed("no drawables z-order"))?,
        )
    };
    let shape_info = message_ref("TSWP.ShapeInfoArchive")?;
    let shape = child_message(shape_info, "super")?;
    let drawable = child_message(shape, "super")?;
    let geometry = child_message(drawable, "geometry")?;
    let point = message_ref("TSP.Point")?;
    let size = message_ref("TSP.Size")?;
    let wrap = child_message(drawable, "exterior_text_wrap")?;
    let path_source = child_message(shape, "pathsource")?;
    let bezier = child_message(path_source, "bezier_path_source")?;
    let caption = message_ref("TSD.StandinCaptionArchive")?;
    let _ = caption;

    let mut placed: Vec<(u32, u64)> = Vec::new();
    for text_box in boxes {
        let storage_id = *next_id;
        let title_id = *next_id + 1;
        let caption_id = *next_id + 2;
        let shape_id = *next_id + 3;
        *next_id += 4;
        let attachments = create_number_attachments(stream, &text_box.content.fields, next_id)?;
        let tree = &mut stream.tree;
        let (storage_first, storage_refs) = build_text_storage(
            tree,
            &text_box.content,
            formats,
            styles,
            None,
            &attachments,
            paras,
        )?;
        let mut objects = vec![(storage_id, STORAGE_ARCHIVE, storage_first, storage_refs)];
        for id in [title_id, caption_id] {
            objects.push((id, STANDIN_CAPTION, NONE, Vec::new()));
        }

        // The drawable: geometry at the page position, wrap, captions.
        let mut position = Chain::new();
        push_field(tree, &mut position, point, "x", Node::Float(text_box.x))?;
        push_field(tree, &mut position, point, "y", Node::Float(text_box.y))?;
        let mut extent = Chain::new();
        push_field(
            tree,
            &mut extent,
            size,
            "width",
            Node::Float(text_box.width),
        )?;
        push_field(
            tree,
            &mut extent,
            size,
            "height",
            Node::Float(text_box.height),
        )?;
        let mut geo = Chain::new();
        push_field(
            tree,
            &mut geo,
            geometry,
            "position",
            Node::Message(position.first),
        )?;
        push_field(
            tree,
            &mut geo,
            geometry,
            "size",
            Node::Message(extent.first),
        )?;
        push_field(tree, &mut geo, geometry, "flags", Node::Uint(3))?;
        push_field(tree, &mut geo, geometry, "angle", Node::Float(0.0))?;
        let mut wrap_chain = Chain::new();
        push_field(tree, &mut wrap_chain, wrap, "type", Node::Uint(4))?;
        push_field(tree, &mut wrap_chain, wrap, "direction", Node::Uint(2))?;
        push_field(tree, &mut wrap_chain, wrap, "fit_type", Node::Uint(1))?;
        push_field(tree, &mut wrap_chain, wrap, "margin", Node::Float(12.0))?;
        push_field(
            tree,
            &mut wrap_chain,
            wrap,
            "alpha_threshold",
            Node::Float(0.5),
        )?;
        push_field(
            tree,
            &mut wrap_chain,
            wrap,
            "is_html_wrap",
            Node::Bool(false),
        )?;
        let mut draw = Chain::new();
        push_field(
            tree,
            &mut draw,
            drawable,
            "geometry",
            Node::Message(geo.first),
        )?;
        push_field(
            tree,
            &mut draw,
            drawable,
            "exterior_text_wrap",
            Node::Message(wrap_chain.first),
        )?;
        push_field(tree, &mut draw, drawable, "locked", Node::Bool(false))?;
        push_field(
            tree,
            &mut draw,
            drawable,
            "aspect_ratio_locked",
            Node::Bool(false),
        )?;
        push_field(
            tree,
            &mut draw,
            drawable,
            "title",
            Node::Reference(title_id),
        )?;
        push_field(
            tree,
            &mut draw,
            drawable,
            "caption",
            Node::Reference(caption_id),
        )?;
        push_field(tree, &mut draw, drawable, "title_hidden", Node::Bool(false))?;
        push_field(
            tree,
            &mut draw,
            drawable,
            "caption_hidden",
            Node::Bool(false),
        )?;

        // The shape: style and a rectangular path at the box's size.
        let mut natural = Chain::new();
        push_field(
            tree,
            &mut natural,
            size,
            "width",
            Node::Float(text_box.width),
        )?;
        push_field(
            tree,
            &mut natural,
            size,
            "height",
            Node::Float(text_box.height),
        )?;
        let path = build_shape_path(tree, text_box.geometry, text_box.width, text_box.height)?;
        let mut bezier_chain = Chain::new();
        push_field(
            tree,
            &mut bezier_chain,
            bezier,
            "naturalSize",
            Node::Message(natural.first),
        )?;
        push_field(tree, &mut bezier_chain, bezier, "path", Node::Message(path))?;
        let mut source = Chain::new();
        push_field(
            tree,
            &mut source,
            path_source,
            "horizontalFlip",
            Node::Bool(text_box.flip.0),
        )?;
        push_field(
            tree,
            &mut source,
            path_source,
            "verticalFlip",
            Node::Bool(text_box.flip.1),
        )?;
        push_field(
            tree,
            &mut source,
            path_source,
            "bezier_path_source",
            Node::Message(bezier_chain.first),
        )?;
        let mut shape_chain = Chain::new();
        push_field(
            tree,
            &mut shape_chain,
            shape,
            "super",
            Node::Message(draw.first),
        )?;
        let style = fill_styles
            .get(&shape_look(text_box.fill, text_box.line))
            .copied()
            .unwrap_or(shape_style);
        push_field(
            tree,
            &mut shape_chain,
            shape,
            "style",
            Node::Reference(style),
        )?;
        push_field(
            tree,
            &mut shape_chain,
            shape,
            "pathsource",
            Node::Message(source.first),
        )?;
        push_field(
            tree,
            &mut shape_chain,
            shape,
            "strokePatternOffsetDistance",
            Node::Float(0.0),
        )?;
        let mut info_chain = Chain::new();
        push_field(
            tree,
            &mut info_chain,
            shape_info,
            "super",
            Node::Message(shape_chain.first),
        )?;
        push_field(
            tree,
            &mut info_chain,
            shape_info,
            "deprecated_storage",
            Node::Reference(storage_id),
        )?;
        push_field(
            tree,
            &mut info_chain,
            shape_info,
            "owned_storage",
            Node::Reference(storage_id),
        )?;
        push_field(
            tree,
            &mut info_chain,
            shape_info,
            "is_text_box",
            Node::Bool(true),
        )?;
        objects.push((
            shape_id,
            SHAPE_INFO,
            info_chain.first,
            vec![title_id, caption_id, style, storage_id],
        ));

        for (id, kind, first, refs) in objects {
            let info = build_archive_info(tree, id, kind)?;
            add_object_references(tree, info, &refs)?;
            stream.objects.push(Object {
                identifier: id,
                info,
                messages: vec![ObjectMessage {
                    message_type: kind,
                    first,
                }],
            });
        }
        placed.push((text_box.page, shape_id));
    }

    // List each shape under its page, and on top in the z-order.
    let floating = message_ref("TP.FloatingDrawablesArchive")?;
    let group = child_message(floating, "page_groups")?;
    let entry = child_message(group, "drawables")?;
    let zorder = message_ref("TP.DrawablesZOrderArchive")?;
    let tree = &mut stream.tree;
    let (floating_first, floating_info) = stream
        .objects
        .iter()
        .find(|object| object.identifier == floating_id)
        .map(|object| (object.messages[0].first, object.info))
        .ok_or_else(|| malformed("floating drawables missing"))?;
    let mut pages: Vec<u32> = placed.iter().map(|(page, _)| *page).collect();
    pages.sort_unstable();
    pages.dedup();
    let mut fields: Vec<(u32, Node)> = tree
        .chain(floating_first)
        .map(|(_, e)| (e.number, e.value))
        .collect();
    let groups_number = floating
        .slot_named("page_groups")
        .map(|(_, f)| f.number)
        .ok_or_else(|| malformed("no page_groups"))?;
    for page in pages {
        let mut group_chain = Chain::new();
        push_field(
            tree,
            &mut group_chain,
            group,
            "page_index",
            Node::Uint(u64::from(page)),
        )?;
        for (_, shape_id) in placed.iter().filter(|(p, _)| *p == page) {
            let mut item = Chain::new();
            push_field(
                tree,
                &mut item,
                entry,
                "drawable",
                Node::Reference(*shape_id),
            )?;
            push_field(
                tree,
                &mut group_chain,
                group,
                "drawables",
                Node::Message(item.first),
            )?;
        }
        fields.push((groups_number, Node::Message(group_chain.first)));
    }
    let mut chain = Chain::new();
    for (number, value) in fields {
        let slot = floating
            .slot(number)
            .ok_or_else(|| malformed("floating field without slot"))?;
        let field = floating
            .field_at(slot)
            .ok_or_else(|| malformed("floating slot out of range"))?;
        tree.push_known(&mut chain, floating, slot, field, number, value)
            .map_err(tree_error)?;
    }
    let shape_ids: Vec<u64> = placed.iter().map(|(_, id)| *id).collect();
    add_object_references(tree, floating_info, &shape_ids)?;
    if let Some(object) = stream
        .objects
        .iter_mut()
        .find(|object| object.identifier == floating_id)
    {
        object.messages[0].first = chain.first;
    }
    let (zorder_first, zorder_info) = stream
        .objects
        .iter()
        .find(|object| object.identifier == zorder_id)
        .map(|object| (object.messages[0].first, object.info))
        .ok_or_else(|| malformed("z-order missing"))?;
    for id in &shape_ids {
        append_message_reference(&mut stream.tree, zorder, zorder_first, "drawables", *id)?;
    }
    add_object_references(&mut stream.tree, zorder_info, &shape_ids)?;
    Ok(())
}

/// The template's shape style with style identifier `identifier` (at
/// `super.super.style_identifier`), such as Pages' "textbox-0-shapestyle".
fn shape_style_identified(package: &Package, identifier: &str) -> Option<u64> {
    for entry in &package.entries {
        let Entry::Stream(stream) = entry else {
            continue;
        };
        for object in &stream.objects {
            if first_type(object) != Some(SHAPE_STYLE) {
                continue;
            }
            let first = object.messages.first()?.first;
            let found = message_field(&stream.tree, first, "super")
                .and_then(|base| message_field(&stream.tree, base, "super"))
                .and_then(|style| str_field(&stream.tree, style, "style_identifier"));
            if found == Some(identifier) {
                return Some(object.identifier);
            }
        }
    }
    None
}

/// Creates, in the document stream, a variation of `parent` (a shape style)
/// per colour with only its fill overridden, registered with the stylesheet.
fn create_fill_styles(
    package: &mut Package,
    parent: u64,
    looks: &[ShapeLook],
    stylesheet: u64,
    next_id: &mut u64,
) -> Result<HashMap<ShapeLook, u64>, PackageError> {
    let archive = message_ref("TSWP.ShapeStyleArchive")?;
    let drawing = child_message(archive, "super")?;
    let base = child_message(drawing, "super")?;
    let properties = child_message(drawing, "shape_properties")?;
    let fill = child_message(properties, "fill")?;
    let stroke = child_message(properties, "stroke")?;
    let mut out = HashMap::new();
    let stream = document_stream(package)?;
    for look in looks {
        let (color, line) = *look;
        let id = *next_id;
        *next_id += 1;
        let tree = &mut stream.tree;
        let mut style = Chain::new();
        push_field(tree, &mut style, base, "parent", Node::Reference(parent))?;
        push_field(tree, &mut style, base, "is_variation", Node::Bool(true))?;
        push_field(
            tree,
            &mut style,
            base,
            "stylesheet",
            Node::Reference(stylesheet),
        )?;
        let mut props = Chain::new();
        let mut overrides = 0;
        if let Some(color) = color {
            let color_first = build_color(tree, color)?;
            let mut fill_chain = Chain::new();
            push_field(
                tree,
                &mut fill_chain,
                fill,
                "color",
                Node::Message(color_first),
            )?;
            push_field(
                tree,
                &mut props,
                properties,
                "fill",
                Node::Message(fill_chain.first),
            )?;
            overrides += 1;
        }
        if let Some((width, stroke_color)) = line {
            let black = crate::document::Color {
                red: 0,
                green: 0,
                blue: 0,
            };
            let first = build_solid_stroke(
                tree,
                stroke,
                f32::from_bits(width),
                stroke_color.unwrap_or(black),
            )?;
            push_field(tree, &mut props, properties, "stroke", Node::Message(first))?;
            overrides += 1;
        }
        let mut drawing_chain = Chain::new();
        push_field(
            tree,
            &mut drawing_chain,
            drawing,
            "super",
            Node::Message(style.first),
        )?;
        push_field(
            tree,
            &mut drawing_chain,
            drawing,
            "override_count",
            Node::Uint(overrides),
        )?;
        push_field(
            tree,
            &mut drawing_chain,
            drawing,
            "shape_properties",
            Node::Message(props.first),
        )?;
        let mut chain = Chain::new();
        push_field(
            tree,
            &mut chain,
            archive,
            "super",
            Node::Message(drawing_chain.first),
        )?;
        push_field(tree, &mut chain, archive, "override_count", Node::Uint(0))?;
        let info = build_archive_info(tree, id, SHAPE_STYLE)?;
        add_object_references(tree, info, &[parent, stylesheet])?;
        stream.objects.push(Object {
            identifier: id,
            info,
            messages: vec![ObjectMessage {
                message_type: SHAPE_STYLE,
                first: chain.first,
            }],
        });
        out.insert(*look, id);
    }
    let pairs: Vec<(u64, u64)> = out.values().map(|id| (parent, *id)).collect();
    register_in_stylesheet(package, stylesheet, &pairs)?;
    Ok(out)
}

/// Sets the refcount of the entry with `key` in a table data list.
fn set_list_refcount(
    package: &mut Package,
    list_id: u64,
    key: u64,
    refcount: u64,
) -> Result<(), PackageError> {
    let stream = stream_containing(package, list_id)?;
    let first = stream
        .objects
        .iter()
        .find(|object| object.identifier == list_id)
        .map(|object| object.messages[0].first)
        .ok_or_else(|| malformed("data list is missing"))?;
    let entries: Vec<u32> = stream
        .tree
        .chain(first)
        .filter(|(_, entry)| stream.tree.field(entry).map(|f| f.name) == Some("entries"))
        .filter_map(|(_, entry)| match entry.value {
            Node::Message(child) => Some(child),
            _ => None,
        })
        .collect();
    for entry in entries {
        if field_value(&stream.tree, entry, "key") == Some(Node::Uint(key)) {
            set_field_uint(&mut stream.tree, entry, "refcount", refcount);
        }
    }
    Ok(())
}

/// Sets `PackageMetadata.write_version` (major, minor, patch).
fn set_write_version(package: &mut Package, version: [u64; 3]) -> Result<(), PackageError> {
    let (stream, first) =
        metadata_message(package).ok_or_else(|| malformed("PackageMetadata is missing"))?;
    let slots: Vec<u32> = stream
        .tree
        .chain(first)
        .filter(|(_, entry)| stream.tree.field(entry).map(|f| f.name) == Some("write_version"))
        .map(|(index, _)| index)
        .collect();
    for (index, value) in slots.iter().zip(version) {
        stream.tree.entries[*index as usize].value = Node::Uint(value);
    }
    Ok(())
}

/// Replaces the first entry of `Metadata/BuildVersionHistory.plist` (the
/// document's origin) with `origin`.
fn set_origin(package: &mut Package, origin: &str) {
    for entry in &mut package.entries {
        let Entry::File { name, bytes } = entry else {
            continue;
        };
        if name != "Metadata/BuildVersionHistory.plist" {
            continue;
        }
        let Ok(text) = std::str::from_utf8(bytes) else {
            return;
        };
        let Some(start) = text.find("<string>") else {
            return;
        };
        let Some(end) = text[start..].find("</string>").map(|offset| start + offset) else {
            return;
        };
        let replaced = format!("{}<string>{origin}{}", &text[..start], &text[end..]);
        *bytes = replaced.into_bytes();
        return;
    }
}

/// Creates, in the stylesheet's stream, a variation of table style `parent`
/// with banded rows off, registered with the stylesheet.
fn create_unbanded_table_style(
    package: &mut Package,
    parent: u64,
    stylesheet: u64,
    next_id: &mut u64,
) -> Result<u64, PackageError> {
    let archive = message_ref("TST.TableStyleArchive")?;
    let base = child_message(archive, "super")?;
    let properties = child_message(archive, "table_properties")?;
    let id = *next_id;
    *next_id += 1;
    let stream = stream_containing(package, stylesheet)?;
    let tree = &mut stream.tree;
    let mut style = Chain::new();
    push_field(tree, &mut style, base, "parent", Node::Reference(parent))?;
    push_field(tree, &mut style, base, "is_variation", Node::Bool(true))?;
    push_field(
        tree,
        &mut style,
        base,
        "stylesheet",
        Node::Reference(stylesheet),
    )?;
    let mut props = Chain::new();
    push_field(
        tree,
        &mut props,
        properties,
        "banded_rows",
        Node::Bool(false),
    )?;
    let mut chain = Chain::new();
    push_field(
        tree,
        &mut chain,
        archive,
        "super",
        Node::Message(style.first),
    )?;
    push_field(tree, &mut chain, archive, "override_count", Node::Uint(1))?;
    push_field(
        tree,
        &mut chain,
        archive,
        "table_properties",
        Node::Message(props.first),
    )?;
    let info = build_archive_info(tree, id, TABLE_STYLE)?;
    add_object_references(tree, info, &[parent, stylesheet])?;
    stream.objects.push(Object {
        identifier: id,
        info,
        messages: vec![ObjectMessage {
            message_type: TABLE_STYLE,
            first: chain.first,
        }],
    });
    register_in_stylesheet(package, stylesheet, &[(parent, id)])?;
    Ok(id)
}

// ----- table borders -----

const STROKE_LAYER: u32 = 6306;

/// Stroke layer objects for a table's sidecar, by side.
#[derive(Default)]
struct StrokeLayers {
    left: Vec<u64>,
    right: Vec<u64>,
    top: Vec<u64>,
    bottom: Vec<u64>,
}

impl StrokeLayers {
    fn all(&self) -> impl Iterator<Item = u64> + '_ {
        self.left
            .iter()
            .chain(&self.right)
            .chain(&self.top)
            .chain(&self.bottom)
            .copied()
    }
}

/// Creates the stroke layers for a table's borders in the sidecar's stream:
/// a line where the source draws one (solid), an empty stroke where it draws
/// none, one run per stretch of identical cell edges along a column or row.
fn build_stroke_layers(
    package: &mut Package,
    sidecar_id: u64,
    mark: &TableMark,
    borders: crate::document::TableBorders,
    next_id: &mut u64,
) -> Result<StrokeLayers, PackageError> {
    let layer = message_ref("TST.StrokeLayerArchive")?;
    let run = child_message(layer, "stroke_runs")?;
    let stroke = child_message(run, "stroke")?;
    let pattern = child_message(stroke, "pattern")?;
    let stream = stream_containing(package, sidecar_id)?;
    let (columns, rows) = (mark.columns, mark.rows);
    let mut layers = StrokeLayers::default();
    let edges = |index: usize| mark.cell_borders.get(index).copied().unwrap_or_default();
    // The vertical line left of grid column `x` in row `r`, and the
    // horizontal line above grid row `y` in column `c`.
    let vertical = |x: usize, r: usize| {
        let table = if x == 0 {
            borders.left
        } else if x == columns {
            borders.right
        } else {
            borders.inside_vertical
        };
        let before = (x > 0).then(|| edges(r * columns + x - 1).right).flatten();
        let after = (x < columns).then(|| edges(r * columns + x).left).flatten();
        edge_line(table, before, after)
    };
    let horizontal = |y: usize, c: usize| {
        let table = if y == 0 {
            borders.top
        } else if y == rows {
            borders.bottom
        } else {
            borders.inside_horizontal
        };
        let before = (y > 0)
            .then(|| edges((y - 1) * columns + c).bottom)
            .flatten();
        let after = (y < rows).then(|| edges(y * columns + c).top).flatten();
        edge_line(table, before, after)
    };
    // (side, index, the line on each cell edge along it)
    let mut plan: Vec<(u8, usize, Vec<Option<crate::document::Border>>)> = Vec::new();
    for column in 0..columns {
        plan.push((0, column, (0..rows).map(|r| vertical(column, r)).collect()));
        plan.push((
            1,
            column,
            (0..rows).map(|r| vertical(column + 1, r)).collect(),
        ));
    }
    for row in 0..rows {
        plan.push((2, row, (0..columns).map(|c| horizontal(row, c)).collect()));
        plan.push((
            3,
            row,
            (0..columns).map(|c| horizontal(row + 1, c)).collect(),
        ));
    }
    for (side, index, segments) in plan {
        let tree = &mut stream.tree;
        let mut runs: Vec<u32> = Vec::new();
        let mut origin = 0;
        while origin < segments.len() {
            let line = segments[origin];
            let length = segments[origin..]
                .iter()
                .take_while(|other| **other == line)
                .count();
            runs.push(build_stroke_run(
                tree,
                (run, stroke, pattern),
                origin,
                length,
                line,
            )?);
            origin += length;
        }
        let mut chain = Chain::new();
        push_field(
            tree,
            &mut chain,
            layer,
            "row_column_index",
            Node::Uint(index as u64),
        )?;
        for first in runs {
            push_field(tree, &mut chain, layer, "stroke_runs", Node::Message(first))?;
        }
        let id = *next_id;
        *next_id += 1;
        let info = build_archive_info(tree, id, STROKE_LAYER)?;
        stream.objects.push(Object {
            identifier: id,
            info,
            messages: vec![ObjectMessage {
                message_type: STROKE_LAYER,
                first: chain.first,
            }],
        });
        match side {
            0 => layers.left.push(id),
            1 => layers.right.push(id),
            2 => layers.top.push(id),
            _ => layers.bottom.push(id),
        }
    }
    Ok(layers)
}

/// The line on one shared cell edge: the cells' own statements over the
/// table's, and where the two cells disagree, the heavier line.
fn edge_line(
    table: Option<crate::document::Border>,
    before: Option<Option<crate::document::Border>>,
    after: Option<Option<crate::document::Border>>,
) -> Option<crate::document::Border> {
    match (before, after) {
        (None, None) => table,
        (Some(line), None) | (None, Some(line)) => line,
        (Some(a), Some(b)) => match (a, b) {
            (Some(a), Some(b)) => Some(if b.width > a.width { b } else { a }),
            (line, None) | (None, line) => line,
        },
    }
}

/// One stroke run of a stroke layer: `length` cells from `origin`, solid
/// (pattern type 1) where there is a line, empty (type 2) where there is none.
fn build_stroke_run(
    tree: &mut Tree,
    (run, stroke, pattern): (MessageRef, MessageRef, MessageRef),
    origin: usize,
    length: usize,
    line: Option<crate::document::Border>,
) -> Result<u32, PackageError> {
    let black = crate::document::Color {
        red: 0,
        green: 0,
        blue: 0,
    };
    let color = build_color(tree, line.and_then(|line| line.color).unwrap_or(black))?;
    let mut pattern_chain = Chain::new();
    push_field(
        tree,
        &mut pattern_chain,
        pattern,
        "type",
        Node::Uint(if line.is_some() { 1 } else { 2 }),
    )?;
    push_field(tree, &mut pattern_chain, pattern, "phase", Node::Float(0.0))?;
    push_field(tree, &mut pattern_chain, pattern, "count", Node::Uint(0))?;
    for _ in 0..6 {
        push_field(
            tree,
            &mut pattern_chain,
            pattern,
            "pattern",
            Node::Float(0.0),
        )?;
    }
    let mut stroke_chain = Chain::new();
    push_field(
        tree,
        &mut stroke_chain,
        stroke,
        "color",
        Node::Message(color),
    )?;
    push_field(
        tree,
        &mut stroke_chain,
        stroke,
        "width",
        Node::Float(line.map_or(1.0, |line| line.width)),
    )?;
    push_field(tree, &mut stroke_chain, stroke, "cap", Node::Uint(0))?;
    push_field(tree, &mut stroke_chain, stroke, "join", Node::Uint(0))?;
    push_field(
        tree,
        &mut stroke_chain,
        stroke,
        "miter_limit",
        Node::Float(4.0),
    )?;
    push_field(
        tree,
        &mut stroke_chain,
        stroke,
        "pattern",
        Node::Message(pattern_chain.first),
    )?;
    let mut run_chain = Chain::new();
    push_field(
        tree,
        &mut run_chain,
        run,
        "origin",
        Node::Uint(origin as u64),
    )?;
    push_field(
        tree,
        &mut run_chain,
        run,
        "length",
        Node::Uint(length as u64),
    )?;
    push_field(
        tree,
        &mut run_chain,
        run,
        "stroke",
        Node::Message(stroke_chain.first),
    )?;
    push_field(tree, &mut run_chain, run, "order", Node::Uint(1))?;
    Ok(run_chain.first)
}

// ----- list styles -----

/// Pages' list label types.
const LABEL_NONE: u64 = 0;
const LABEL_BULLET: u64 = 2;
const LABEL_NUMBER: u64 = 3;
/// Pages keeps nine list levels.
const LIST_LEVELS: usize = 9;

/// Creates a `TSWP.ListStyleArchive` variation (in the stylesheet's stream)
/// for each Word list the body's paragraphs use: per level its label (bullet
/// text or number style), where the label sits, and the gap to the text, as
/// Pages imports a Word list. Keyed by the model's list style.
fn synthesize_list_styles(
    package: &mut Package,
    document: &Document,
    body: &Body,
    lists: &HashMap<String, u64>,
    stylesheet: u64,
    next_id: &mut u64,
) -> Result<HashMap<usize, u64>, PackageError> {
    let archive = message_ref("TSWP.ListStyleArchive")?;
    let base = child_message(archive, "super")?;
    let geometry = child_message(archive, "geometries")?;
    let mut used: Vec<&ListItem> = body
        .paragraphs
        .iter()
        .filter_map(|mark| mark.list.as_ref())
        .collect();
    used.sort_by_key(|item| item.style);
    used.dedup_by_key(|item| item.style);
    // The em the text gap is measured in: the body text's size.
    let em = default_font_size(document);
    let mut out = HashMap::new();
    let mut pairs = Vec::new();
    let stream = stream_containing(package, stylesheet)?;
    for item in used {
        let Some(style) = document.styles.list.get(item.style) else {
            continue;
        };
        let Some(parent) = list_style_for(document, lists, Some(item)) else {
            continue;
        };
        let level = |index: usize| style.levels.get(index).or_else(|| style.levels.last());
        let tree = &mut stream.tree;
        let id = *next_id;
        *next_id += 1;
        let mut super_chain = Chain::new();
        push_field(
            tree,
            &mut super_chain,
            base,
            "parent",
            Node::Reference(parent),
        )?;
        push_field(
            tree,
            &mut super_chain,
            base,
            "is_variation",
            Node::Bool(true),
        )?;
        push_field(
            tree,
            &mut super_chain,
            base,
            "stylesheet",
            Node::Reference(stylesheet),
        )?;
        let mut chain = Chain::new();
        push_field(
            tree,
            &mut chain,
            archive,
            "super",
            Node::Message(super_chain.first),
        )?;
        push_field(tree, &mut chain, archive, "override_count", Node::Uint(8))?;
        for index in 0..LIST_LEVELS {
            let kind = match level(index).map(|level| &level.label) {
                Some(ListLabel::Text(_)) => LABEL_BULLET,
                Some(ListLabel::Number(_)) => LABEL_NUMBER,
                _ => LABEL_NONE,
            };
            push_field(tree, &mut chain, archive, "label_types", Node::Uint(kind))?;
        }
        for index in 0..LIST_LEVELS {
            let gap =
                level(index).map_or(18.0, |level| (level.indent - level.label_indent).max(0.0));
            push_field(
                tree,
                &mut chain,
                archive,
                "text_indents",
                Node::Float((gap / em).max(0.5)),
            )?;
        }
        for index in 0..LIST_LEVELS {
            let at = level(index).map_or(0.0, |level| level.label_indent.max(0.0));
            push_field(tree, &mut chain, archive, "indents", Node::Float(at))?;
        }
        for _ in 0..LIST_LEVELS {
            let mut geometry_chain = Chain::new();
            push_field(
                tree,
                &mut geometry_chain,
                geometry,
                "scale",
                Node::Float(1.0),
            )?;
            push_field(
                tree,
                &mut geometry_chain,
                geometry,
                "baseline_offset",
                Node::Float(0.0),
            )?;
            push_field(
                tree,
                &mut geometry_chain,
                geometry,
                "scale_with_text",
                Node::Bool(true),
            )?;
            push_field(
                tree,
                &mut chain,
                archive,
                "geometries",
                Node::Message(geometry_chain.first),
            )?;
        }
        for index in 0..LIST_LEVELS {
            let text = match level(index).map(|level| &level.label) {
                Some(ListLabel::Text(text)) => text.clone(),
                _ => String::new(),
            };
            let span = tree.push_bytes(text.as_bytes()).map_err(tree_error)?;
            push_field(tree, &mut chain, archive, "strings", Node::Str(span))?;
        }
        for index in 0..LIST_LEVELS {
            let number = match level(index).map(|level| &level.label) {
                Some(ListLabel::Number(format)) => pages_number_type(format),
                _ => 0,
            };
            push_field(
                tree,
                &mut chain,
                archive,
                "number_types",
                Node::Uint(number),
            )?;
        }
        for index in 0..LIST_LEVELS {
            let tiered = matches!(
                level(index).map(|level| &level.label),
                Some(ListLabel::Number(format)) if format.tiered
            );
            push_field(
                tree,
                &mut chain,
                archive,
                "tiered_numbers",
                Node::Bool(tiered),
            )?;
        }
        let info = build_archive_info(tree, id, LIST_STYLE)?;
        add_object_references(tree, info, &[parent, stylesheet])?;
        stream.objects.push(Object {
            identifier: id,
            info,
            messages: vec![ObjectMessage {
                message_type: LIST_STYLE,
                first: chain.first,
            }],
        });
        out.insert(item.style, id);
        pairs.push((parent, id));
    }
    register_in_stylesheet(package, stylesheet, &pairs)?;
    Ok(out)
}

/// Pages' number style for a Word number format: its kind and whether it
/// reads `1.`, `(1)`, or `1)` (Pages has no other punctuation; a bare
/// number or another pattern takes the period form).
fn pages_number_type(format: &crate::document::NumberFormat) -> u64 {
    let base = match format.kind {
        NumberKind::Decimal => 0,
        NumberKind::UpperRoman => 3,
        NumberKind::LowerRoman => 6,
        NumberKind::UpperLetter => 9,
        NumberKind::LowerLetter => 12,
    };
    let pattern = format.pattern.trim();
    let punctuation = if pattern == "(%1)" {
        1
    } else if pattern == "%1)" {
        2
    } else {
        0
    };
    base + punctuation
}

/// The body text's font size in points: the default paragraph style's (or
/// its bases'), else 12.
fn default_font_size(document: &Document) -> f32 {
    let mut next = document.styles.default_paragraph;
    for _ in 0..16 {
        let Some(style) = next.and_then(|id| document.styles.paragraph.get(id)) else {
            break;
        };
        if let Some(size) = style.run.size {
            return size;
        }
        next = style.parent;
    }
    12.0
}

// ----- shapes -----

/// A shape's look in Pages: its fill, and its outline (width as bits, colour).
type ShapeLook = (
    Option<crate::document::Color>,
    Option<(u32, Option<crate::document::Color>)>,
);

fn shape_look(
    fill: Option<crate::document::Color>,
    line: Option<crate::document::Border>,
) -> ShapeLook {
    (fill, line.map(|line| (line.width.to_bits(), line.color)))
}

/// A solid `TSD.StrokeArchive` of `width` points in `color`.
fn build_solid_stroke(
    tree: &mut Tree,
    stroke: MessageRef,
    width: f32,
    color: crate::document::Color,
) -> Result<u32, PackageError> {
    let pattern = child_message(stroke, "pattern")?;
    let color_first = build_color(tree, color)?;
    let mut pattern_chain = Chain::new();
    push_field(tree, &mut pattern_chain, pattern, "type", Node::Uint(1))?;
    push_field(tree, &mut pattern_chain, pattern, "phase", Node::Float(0.0))?;
    push_field(tree, &mut pattern_chain, pattern, "count", Node::Uint(0))?;
    for _ in 0..6 {
        push_field(
            tree,
            &mut pattern_chain,
            pattern,
            "pattern",
            Node::Float(0.0),
        )?;
    }
    let mut chain = Chain::new();
    push_field(
        tree,
        &mut chain,
        stroke,
        "color",
        Node::Message(color_first),
    )?;
    push_field(tree, &mut chain, stroke, "width", Node::Float(width))?;
    push_field(tree, &mut chain, stroke, "cap", Node::Uint(0))?;
    push_field(tree, &mut chain, stroke, "join", Node::Uint(0))?;
    push_field(tree, &mut chain, stroke, "miter_limit", Node::Float(4.0))?;
    push_field(
        tree,
        &mut chain,
        stroke,
        "pattern",
        Node::Message(pattern_chain.first),
    )?;
    Ok(chain.first)
}

/// A path element: move to (1), line to (2), curve to (4, two control
/// points then the end), or close (5).
enum PathStep {
    Move(f32, f32),
    Line(f32, f32),
    Curve([(f32, f32); 3]),
    Close,
}

/// The outline of a preset shape in a `width` x `height` box, as a `TSP.Path`.
fn build_shape_path(
    tree: &mut Tree,
    geometry: crate::document::ShapeGeometry,
    width: f32,
    height: f32,
) -> Result<u32, PackageError> {
    use crate::document::ShapeGeometry as G;
    let (w, h) = (width, height);
    let polygon = |points: &[(f32, f32)]| {
        let mut steps: Vec<PathStep> = points
            .iter()
            .enumerate()
            .map(|(index, (x, y))| {
                if index == 0 {
                    PathStep::Move(*x, *y)
                } else {
                    PathStep::Line(*x, *y)
                }
            })
            .collect();
        steps.push(PathStep::Close);
        steps
    };
    // A regular polygon or star inscribed in the box, from the top.
    let around = |count: usize, inner: Option<f32>| {
        let total = if inner.is_some() { count * 2 } else { count };
        (0..total)
            .map(|index| {
                let angle = -std::f32::consts::FRAC_PI_2
                    + index as f32 * std::f32::consts::TAU / total as f32;
                let radius = match inner {
                    Some(ratio) if index % 2 == 1 => ratio,
                    _ => 1.0,
                };
                (
                    w / 2.0 + angle.cos() * radius * w / 2.0,
                    h / 2.0 + angle.sin() * radius * h / 2.0,
                )
            })
            .collect::<Vec<_>>()
    };
    // The cubic control distance that draws a quarter ellipse.
    const K: f32 = 0.552_284_8;
    let steps = match geometry {
        G::Rectangle => polygon(&[(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)]),
        G::Ellipse => {
            let (rx, ry, cx, cy) = (w / 2.0, h / 2.0, w / 2.0, h / 2.0);
            vec![
                PathStep::Move(cx, 0.0),
                PathStep::Curve([(cx + K * rx, 0.0), (w, cy - K * ry), (w, cy)]),
                PathStep::Curve([(w, cy + K * ry), (cx + K * rx, h), (cx, h)]),
                PathStep::Curve([(cx - K * rx, h), (0.0, cy + K * ry), (0.0, cy)]),
                PathStep::Curve([(0.0, cy - K * ry), (cx - K * rx, 0.0), (cx, 0.0)]),
                PathStep::Close,
            ]
        }
        G::RoundedRectangle => {
            let r = w.min(h) / 6.0;
            let k = K * r;
            vec![
                PathStep::Move(r, 0.0),
                PathStep::Line(w - r, 0.0),
                PathStep::Curve([(w - r + k, 0.0), (w, r - k), (w, r)]),
                PathStep::Line(w, h - r),
                PathStep::Curve([(w, h - r + k), (w - r + k, h), (w - r, h)]),
                PathStep::Line(r, h),
                PathStep::Curve([(r - k, h), (0.0, h - r + k), (0.0, h - r)]),
                PathStep::Line(0.0, r),
                PathStep::Curve([(0.0, r - k), (r - k, 0.0), (r, 0.0)]),
                PathStep::Close,
            ]
        }
        G::Triangle => polygon(&[(w / 2.0, 0.0), (w, h), (0.0, h)]),
        G::RightTriangle => polygon(&[(0.0, 0.0), (w, h), (0.0, h)]),
        G::Diamond => polygon(&[(w / 2.0, 0.0), (w, h / 2.0), (w / 2.0, h), (0.0, h / 2.0)]),
        G::Pentagon => polygon(&around(5, None)),
        G::Hexagon => polygon(&[
            (w * 0.25, 0.0),
            (w * 0.75, 0.0),
            (w, h / 2.0),
            (w * 0.75, h),
            (w * 0.25, h),
            (0.0, h / 2.0),
        ]),
        G::Octagon => {
            let (a, b) = (w * 0.29, h * 0.29);
            polygon(&[
                (a, 0.0),
                (w - a, 0.0),
                (w, b),
                (w, h - b),
                (w - a, h),
                (a, h),
                (0.0, h - b),
                (0.0, b),
            ])
        }
        G::Star => polygon(&around(5, Some(0.382))),
        G::RightArrow | G::LeftArrow => {
            let head = (h * 0.5).min(w);
            let points = [
                (0.0, h * 0.25),
                (w - head, h * 0.25),
                (w - head, 0.0),
                (w, h / 2.0),
                (w - head, h),
                (w - head, h * 0.75),
                (0.0, h * 0.75),
            ];
            if geometry == G::LeftArrow {
                polygon(&points.map(|(x, y)| (w - x, y)))
            } else {
                polygon(&points)
            }
        }
        G::UpArrow | G::DownArrow => {
            let head = (w * 0.5).min(h);
            let points = [
                (w * 0.25, h),
                (w * 0.25, head),
                (0.0, head),
                (w / 2.0, 0.0),
                (w, head),
                (w * 0.75, head),
                (w * 0.75, h),
            ];
            if geometry == G::DownArrow {
                polygon(&points.map(|(x, y)| (x, h - y)))
            } else {
                polygon(&points)
            }
        }
        G::Line => {
            // A line Word draws flat (or upright) keeps to the middle of its
            // thin box; otherwise it runs corner to corner.
            let (from, to) = if h <= 2.0 {
                ((0.0, h / 2.0), (w, h / 2.0))
            } else if w <= 2.0 {
                ((w / 2.0, 0.0), (w / 2.0, h))
            } else {
                ((0.0, 0.0), (w, h))
            };
            vec![PathStep::Move(from.0, from.1), PathStep::Line(to.0, to.1)]
        }
    };
    let path = message_ref("TSP.Path")?;
    let element = message_ref("TSP.Path.Element")?;
    let point = message_ref("TSP.Point")?;
    let mut chain = Chain::new();
    for step in steps {
        let (kind, points): (u64, Vec<(f32, f32)>) = match step {
            PathStep::Move(x, y) => (1, vec![(x, y)]),
            PathStep::Line(x, y) => (2, vec![(x, y)]),
            PathStep::Curve(points) => (4, points.to_vec()),
            PathStep::Close => (5, Vec::new()),
        };
        let mut element_chain = Chain::new();
        push_field(tree, &mut element_chain, element, "type", Node::Uint(kind))?;
        for (x, y) in points {
            let mut point_chain = Chain::new();
            push_field(tree, &mut point_chain, point, "x", Node::Float(x))?;
            push_field(tree, &mut point_chain, point, "y", Node::Float(y))?;
            push_field(
                tree,
                &mut element_chain,
                element,
                "points",
                Node::Message(point_chain.first),
            )?;
        }
        push_field(
            tree,
            &mut chain,
            path,
            "elements",
            Node::Message(element_chain.first),
        )?;
    }
    Ok(chain.first)
}

/// Turns off the rules the template draws under its headings: the source's
/// paragraphs never asked for them.
fn clear_template_rules(package: &mut Package) {
    for entry in &mut package.entries {
        let Entry::Stream(stream) = entry else {
            continue;
        };
        let firsts: Vec<u32> = stream
            .objects
            .iter()
            .filter(|object| first_type(object) == Some(PARAGRAPH_STYLE))
            .map(|object| object.messages[0].first)
            .collect();
        for first in firsts {
            let tree = &mut stream.tree;
            if let Some(properties) = message_field(tree, first, "para_properties")
                && let Some(index) = field_entry(tree, properties, "deprecated_borders")
            {
                tree.entries[index as usize].value = Node::Uint(0);
            }
        }
    }
}

/// A link target as a URL Pages can parse: characters outside printable
/// ASCII (a Cyrillic path, a space) are percent-encoded as UTF-8, as a
/// browser sends them; Pages fails an address it cannot read as a URL.
fn encode_url(url: &str) -> String {
    let mut out = String::with_capacity(url.len());
    for character in url.chars() {
        if character.is_ascii_graphic() {
            out.push(character);
        } else {
            let mut buffer = [0u8; 4];
            for byte in character.encode_utf8(&mut buffer).bytes() {
                out.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    out
}

#[cfg(test)]
mod url_tests {
    #[test]
    fn non_ascii_link_targets_are_percent_encoded() {
        assert_eq!(
            super::encode_url("http://ru.wikipedia.org/wiki/Уз b"),
            "http://ru.wikipedia.org/wiki/%D0%A3%D0%B7%20b"
        );
        assert_eq!(
            super::encode_url("https://example.com/?a=1#x"),
            "https://example.com/?a=1#x"
        );
    }
}
