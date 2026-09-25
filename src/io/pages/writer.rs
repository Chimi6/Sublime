//! Writes a Pages package from the document model. Apple's object graph is
//! intricate and interdependent, so a real blank document is the scaffolding
//! (see `DOCS/formats/pages.md`): the template carries the stylesheet, theme,
//! section, and settings, and the writer replaces only the body text storage,
//! setting the text and one paragraph-style run per paragraph. Everything the
//! template already holds (the named styles, page setup) is reused.

use std::collections::HashMap;

use super::package::{Entry, Object, Package, PackageError, Stream};
use super::schema::SCHEMA;
use crate::document::{
    Block, Document, Inline, ListItem, ListLabel, NumberKind, Paragraph, RunProperties,
};
use crate::io::protobuf::schema::MessageRef;
use crate::io::protobuf::tree::{Chain, NONE, Node, Tree, TreeError};

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
    let (new_first, list_refs) = build_storage(
        &mut stream.tree,
        old_first,
        document,
        &body,
        styles,
        formats,
        lists,
        &anchors,
    )?;
    stream.objects[index].messages[0].first = new_first;
    let info = stream.objects[index].info;
    let mut references: Vec<u64> = list_refs;
    references.extend(anchors.iter().map(|(_, id)| *id));
    add_object_references(&mut stream.tree, info, &references)?;
    Ok(())
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
) -> Result<(u32, Vec<u64>), PackageError> {
    let storage = message_ref("TSWP.StorageArchive")?;
    // The list tables are only rewritten when the document has a list;
    // otherwise the template's own defaults (no list, level 0) are kept.
    let has_lists = body.paragraphs.iter().any(|mark| mark.list.is_some());
    let has_tables = !anchors.is_empty();
    // Fields the writer owns; everything else is copied from the template.
    let mut owned = vec!["text", "table_para_style", "table_char_style"];
    if has_lists {
        owned.extend(["table_list_style", "table_para_data", "table_para_starts"]);
    }
    if has_tables {
        owned.push("table_attachment");
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

/// The document flattened for the storage: the text, the paragraph and
/// character marks, and the anchored tables.
struct Body {
    text: String,
    paragraphs: Vec<ParagraphMark>,
    char_marks: Vec<CharMark>,
    tables: Vec<TableMark>,
}

/// Flattens the document into one text string (paragraphs joined by `\n`)
/// with paragraph and character marks. Each table becomes one anchor
/// paragraph holding a `U+FFFC`, with its grid recorded for later synthesis.
fn flatten(document: &Document) -> Body {
    let mut walk = Walk {
        text: String::new(),
        paragraphs: Vec::new(),
        char_marks: Vec::new(),
        tables: Vec::new(),
        offset: 0,
        current: Format::default(),
    };
    for section in &document.sections {
        walk.blocks(document, &section.blocks);
    }
    Body {
        text: walk.text,
        paragraphs: walk.paragraphs,
        char_marks: walk.char_marks,
        tables: walk.tables,
    }
}

struct Walk {
    text: String,
    paragraphs: Vec<ParagraphMark>,
    char_marks: Vec<CharMark>,
    tables: Vec<TableMark>,
    offset: u32,
    current: Format,
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
        self.text.push(ATTACHMENT);
        self.offset += 1;
    }

    fn paragraph(&mut self, document: &Document, paragraph: &Paragraph) {
        if !self.paragraphs.is_empty() {
            self.mark(Format::default());
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
            let piece = match run.content {
                Inline::Text(span) => document.text(span),
                Inline::Tab => "\t",
                _ => continue,
            };
            if piece.is_empty() {
                continue;
            }
            self.mark(run_format(document, run));
            self.text.push_str(piece);
            self.offset += utf16_len(piece);
        }
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

/// Maps the template's registered bold and italic character styles to a
/// formatting, so a run can point at one by reference. Only styles whose
/// only formatting is bold and italic are taken, so a colored preset like
/// "Red Bold" is not mistaken for plain bold.
fn collect_char_formats(package: &Package) -> HashMap<Format, u64> {
    let mut formats = HashMap::new();
    for entry in &package.entries {
        let Entry::Stream(stream) = entry else {
            continue;
        };
        for object in &stream.objects {
            let Some(message) = object.messages.first() else {
                continue;
            };
            if message.message_type != CHARACTER_STYLE {
                continue;
            }
            let Some(properties) = message_field(&stream.tree, message.first, "char_properties")
            else {
                continue;
            };
            let mut format = Format::default();
            // Other overrides that would change a run's look beyond emphasis;
            // a style that carries them is a preset ("Red Bold", a heading),
            // not the plain bold or italic the writer wants to reuse.
            let mut has_other = false;
            for (_, field_entry) in stream.tree.chain(properties) {
                match stream.tree.field(field_entry).map(|field| field.name) {
                    Some("bold") => format.bold = matches!(field_entry.value, Node::Bool(true)),
                    Some("italic") => format.italic = matches!(field_entry.value, Node::Bool(true)),
                    // The bold or italic font name is the weighted face and is
                    // welcome; every other visual property disqualifies it.
                    Some("font_name")
                    | Some("compatibility_font_name_null")
                    | Some("tsd_fill_should_fill_text_container") => {}
                    Some(_) => has_other = true,
                    None => {}
                }
            }
            // A char style whose only look is bold or italic: the theme's own
            // emphasis style, which Pages renders when a run points at it.
            if !format.is_plain() && !has_other {
                formats.entry(format).or_insert(object.identifier);
            }
        }
    }
    formats
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
