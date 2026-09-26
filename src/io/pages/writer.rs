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
    Block, Document, Id, Inline, ListItem, ListLabel, MediaId, NumberKind, Paragraph, RunProperties,
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
    // Rewrite one template table per model table, up to the number the
    // template carries; each returns the attachment the body anchors it with.
    let template_tables = collect_template_tables(package);
    let mut anchors: Vec<(u32, u64)> = Vec::new();
    for (mark, table) in body.tables.iter().zip(&template_tables) {
        reuse_table(package, table, mark)?;
        anchors.push((mark.offset, table.attach_id));
    }
    // The template carries one image as a prototype: the first model image
    // reuses it; any others clone it so every image reaches Pages, each with its
    // own objects and data files.
    if let Some(prototype) = collect_template_images(package).first().copied() {
        let mut next_object_id = max_identifier(package) + 1;
        let mut next_data_id = max_data_id(package) + 1;
        for (index, mark) in body.images.iter().enumerate() {
            let bytes = document.media[mark.media].bytes.clone();
            let attach_id = if index == 0 {
                reuse_image(package, &prototype, mark, &bytes)?;
                prototype.attach_id
            } else {
                clone_image(
                    package,
                    &prototype,
                    mark,
                    &bytes,
                    &mut next_object_id,
                    &mut next_data_id,
                )?
            };
            anchors.push((mark.offset, attach_id));
        }
    }
    // Attachments anchor by ascending character offset in one table.
    anchors.sort_by_key(|(offset, _)| *offset);

    // Identifiers are unique across the whole package, so a new hyperlink
    // object takes the next id after the highest any stream already uses.
    let mut next_id = max_identifier(package) + 1;

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
        formats,
        lists,
        &anchors,
        &smart_entries,
    )?;
    stream.objects[index].messages[0].first = new_first;
    let info = stream.objects[index].info;
    let mut references: Vec<u64> = list_refs;
    references.extend(anchors.iter().map(|(_, id)| *id));
    references.extend(link_objects.iter().map(|object| object.identifier));
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

/// A table the template carries, by the identifiers of the objects a rewrite
/// touches: the drawable attachment the body anchors, the model whose sizes
/// change, and the tile, string table, and header buckets holding the cells.
struct TemplateTable {
    attach_id: u64,
    model_id: u64,
    tile_id: u64,
    string_id: u64,
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
    let col_bucket_id = reference_of(field_value(tree, store, "columnHeaders"))?;
    let tiles = message_field(tree, store, "tiles")?;
    let one_tile = message_field(tree, tiles, "tiles")?;
    let tile_id = reference_of(field_value(tree, one_tile, "tile"))?;
    let row_headers = message_field(tree, store, "rowHeaders")?;
    let row_bucket_id = reference_of(field_value(tree, row_headers, "buckets"))?;
    Some(TemplateTable {
        attach_id,
        model_id,
        tile_id,
        string_id,
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
fn reuse_table(
    package: &mut Package,
    table: &TemplateTable,
    mark: &TableMark,
) -> Result<(), PackageError> {
    rewrite_object(package, table.tile_id, |tree| build_tile(tree, mark))?;
    rewrite_object(package, table.string_id, |tree| {
        build_string_list(tree, &mark.cells)
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
            build_column_row_uids(tree, cols, rows)
        })?;
    }
    update_model_dims(package, table.model_id, mark)
}

/// A fresh column/row UID map with one distinct UUID per column and row and
/// an identity index mapping, so Pages sizes the table to exactly `columns`
/// by `rows`.
fn build_column_row_uids(
    tree: &mut Tree,
    columns: usize,
    rows: usize,
) -> Result<u32, PackageError> {
    let map = message_ref("TST.ColumnRowUIDMapArchive")?;
    let uuid = message_ref("TSP.UUID")?;
    let mut chain = Chain::new();
    for index in 0..columns {
        let mut value = Chain::new();
        push_field(
            tree,
            &mut value,
            uuid,
            "lower",
            Node::Uint(0x1000 + index as u64),
        )?;
        push_field(tree, &mut value, uuid, "upper", Node::Uint(0xC01))?;
        push_field(
            tree,
            &mut chain,
            map,
            "sorted_column_uids",
            Node::Message(value.first),
        )?;
    }
    for index in 0..columns {
        push_field(
            tree,
            &mut chain,
            map,
            "column_index_for_uid",
            Node::Uint(index as u64),
        )?;
    }
    for index in 0..columns {
        push_field(
            tree,
            &mut chain,
            map,
            "column_uid_for_index",
            Node::Uint(index as u64),
        )?;
    }
    for index in 0..rows {
        let mut value = Chain::new();
        push_field(
            tree,
            &mut value,
            uuid,
            "lower",
            Node::Uint(0x2000 + index as u64),
        )?;
        push_field(tree, &mut value, uuid, "upper", Node::Uint(0x201))?;
        push_field(
            tree,
            &mut chain,
            map,
            "sorted_row_uids",
            Node::Message(value.first),
        )?;
    }
    for index in 0..rows {
        push_field(
            tree,
            &mut chain,
            map,
            "row_index_for_uid",
            Node::Uint(index as u64),
        )?;
    }
    for index in 0..rows {
        push_field(
            tree,
            &mut chain,
            map,
            "row_uid_for_index",
            Node::Uint(index as u64),
        )?;
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
    next_object_id: &mut u64,
    next_data_id: &mut u64,
) -> Result<u64, PackageError> {
    let new_image_id = *next_object_id;
    let new_attach_id = *next_object_id + 1;
    *next_object_id += 2;
    let new_full = *next_data_id;
    *next_data_id += 1;
    let new_thumb = proto.thumb_id.map(|_| {
        let id = *next_data_id;
        *next_data_id += 1;
        id
    });

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
    Ok(new_attach_id)
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
    rebuild_message(
        tree,
        geometry,
        geometry_first,
        vec![("size", Node::Message(size))],
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
    // An inline attachment carries the offset fields with a not-a-number value
    // (as Pages writes them); the reader reads NaN as "in the text line".
    push_field(tree, &mut chain, attachment, "h_offset_type", Node::Uint(0))?;
    push_field(
        tree,
        &mut chain,
        attachment,
        "h_offset",
        Node::Float(f32::NAN),
    )?;
    push_field(tree, &mut chain, attachment, "v_offset_type", Node::Uint(0))?;
    push_field(
        tree,
        &mut chain,
        attachment,
        "v_offset",
        Node::Float(f32::NAN),
    )?;
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
    anchors: &[(u32, u64)],
    smart_entries: &[(u32, Option<u64>)],
) -> Result<(u32, Vec<u64>), PackageError> {
    let storage = message_ref("TSWP.StorageArchive")?;
    // The list tables are only rewritten when the document has a list;
    // otherwise the template's own defaults (no list, level 0) are kept.
    let has_lists = body.paragraphs.iter().any(|mark| mark.list.is_some());
    let has_tables = !anchors.is_empty();
    let has_links = smart_entries.iter().any(|(_, object)| object.is_some());
    // Fields the writer owns; everything else is copied from the template.
    let mut owned = vec!["text", "table_para_style", "table_char_style"];
    if has_lists {
        owned.extend(["table_list_style", "table_para_data", "table_para_starts"]);
    }
    if has_tables {
        owned.push("table_attachment");
    }
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
    if has_lists {
        // A list paragraph points at the template's Bullet or Numbered list
        // style and carries its nesting level; a plain one reverts to the
        // "None" style at level 0. Like the other tables, all three start at
        // offset 0. table_para_starts carries the number an ordered list
        // restarts at.
        let list_entries: Vec<(u32, Option<u64>)> = body
            .paragraphs
            .iter()
            .map(|mark| {
                (
                    mark.offset,
                    list_style_for(document, lists, mark.list.as_ref()),
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

    Ok((chain.first, references))
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
            italic: false,
        })
    {
        return Some(*id);
    }
    if format.italic
        && let Some(id) = formats.get(&Format {
            bold: false,
            italic: true,
        })
    {
        return Some(*id);
    }
    None
}

/// Character formatting the writer can carry, reused from the template's own
/// styles. Kept to bold and italic, which every theme provides.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
struct Format {
    bold: bool,
    italic: bool,
}

impl Format {
    fn is_plain(&self) -> bool {
        *self == Format::default()
    }
}

/// A paragraph start: its offset (UTF-16 units), style name, and list
/// membership (the model's item, resolved to a template list style later).
struct ParagraphMark {
    offset: u32,
    style_name: Option<String>,
    list: Option<ListItem>,
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

/// A table anchored at a `U+FFFC` character: its offset and its grid of cell
/// text, with the column widths, row heights, and header-row count.
struct TableMark {
    offset: u32,
    rows: usize,
    columns: usize,
    header_rows: u32,
    /// Row-major cell text, `rows * columns` entries.
    cells: Vec<String>,
    widths: Vec<f32>,
    heights: Vec<f32>,
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
    };
    for section in &document.sections {
        walk.blocks(document, &section.blocks);
    }
    Body {
        text: walk.text,
        paragraphs: walk.paragraphs,
        char_marks: walk.char_marks,
        link_marks: walk.link_marks,
        tables: walk.tables,
        images: walk.images,
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
}

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
        });
        let columns = table.columns.len();
        let rows = table.rows.len();
        let mut cells = vec![String::new(); rows * columns];
        for (r, row) in table.rows.iter().enumerate() {
            for (c, cell) in row.cells.iter().enumerate().take(columns) {
                cells[r * columns + c] = cell_text(document, cell);
            }
        }
        let widths = table.columns.clone();
        let heights = table
            .rows
            .iter()
            .map(|row| row.height.unwrap_or(0.0))
            .collect();
        self.tables.push(TableMark {
            offset: self.offset,
            rows,
            columns,
            header_rows: table.header_rows,
            cells,
            widths,
            heights,
        });
        self.mark(Format::default());
        self.link_mark(None);
        self.text.push(ATTACHMENT);
        self.offset += 1;
    }

    fn paragraph(&mut self, document: &Document, paragraph: &Paragraph) {
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
        });
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
            let piece = match run.content {
                Inline::Text(span) => document.text(span),
                Inline::Tab => "\t",
                _ => continue,
            };
            if piece.is_empty() {
                continue;
            }
            self.mark(run_format(document, run));
            self.link_mark(run.link);
            self.text.push_str(piece);
            self.offset += utf16_len(piece);
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
fn cell_text(document: &Document, cell: &crate::document::Cell) -> String {
    let mut text = String::new();
    for block in &cell.blocks {
        let Block::Paragraph(paragraph) = block else {
            continue;
        };
        if !text.is_empty() {
            text.push('\n');
        }
        for run in &paragraph.runs {
            if document.is_deleted(run) {
                continue;
            }
            match run.content {
                Inline::Text(span) => text.push_str(document.text(span)),
                Inline::Tab => text.push('\t'),
                _ => {}
            }
        }
    }
    text
}

/// A run's own bold and italic, independent of the paragraph.
fn run_format(document: &Document, run: &crate::document::Run) -> Format {
    let mut properties = RunProperties::default();
    if let Some(style) = run.style {
        properties.overlay(&document.character_style_run(style));
    }
    properties.overlay(&document.run_properties(run));
    Format {
        bold: properties.bold.unwrap_or(false),
        italic: properties.italic.unwrap_or(false),
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
fn build_string_list(tree: &mut Tree, cells: &[String]) -> Result<u32, PackageError> {
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
    for (index, text) in cells.iter().enumerate() {
        let mut entry = Chain::new();
        push_field(
            tree,
            &mut entry,
            entry_ref,
            "key",
            Node::Uint(index as u64 + 1),
        )?;
        push_field(tree, &mut entry, entry_ref, "refcount", Node::Uint(1))?;
        let span = tree.push_bytes(text.as_bytes()).map_err(tree_error)?;
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
fn build_tile(tree: &mut Tree, mark: &TableMark) -> Result<u32, PackageError> {
    let tile = message_ref("TST.Tile")?;
    let row_info = message_ref("TST.TileRowInfo")?;
    let mut chain = Chain::new();
    push_field(
        tree,
        &mut chain,
        tile,
        "maxColumn",
        Node::Uint(mark.columns.saturating_sub(1) as u64),
    )?;
    push_field(
        tree,
        &mut chain,
        tile,
        "maxRow",
        Node::Uint(mark.rows.saturating_sub(1) as u64),
    )?;
    push_field(
        tree,
        &mut chain,
        tile,
        "numCells",
        Node::Uint((mark.rows * mark.columns) as u64),
    )?;
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
    for row in 0..mark.rows {
        let mut offsets = Vec::with_capacity(mark.columns * 2);
        let mut buffer = Vec::with_capacity(mark.columns * 16);
        for column in 0..mark.columns {
            let key = (row * mark.columns + column) as u32 + 1;
            offsets.extend_from_slice(&(buffer.len() as u16).to_le_bytes());
            buffer.extend_from_slice(&cell_record_bytes(key));
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

/// A 16-byte storage-version-5 cell record for a plain-text cell whose text
/// is stringTable key `key`: marker 5, kind 3 (string), flag 0x8, the key.
fn cell_record_bytes(key: u32) -> [u8; 16] {
    let mut bytes = [0u8; 16];
    bytes[0] = 5;
    bytes[1] = 3;
    bytes[8] = 0x08;
    bytes[12..16].copy_from_slice(&key.to_le_bytes());
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
