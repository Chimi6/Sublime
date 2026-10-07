//! Numbers documents written from tables of text: a sheet per input sheet,
//! each holding its tables, from a blank document Numbers saved
//! (`numbers_template.numbers`). Its one sheet and table are the
//! prototypes: further tables are clones of the table's cluster (as the
//! Pages writer clones a table), further sheets clones of the sheet with
//! its header, footer, and guide storages. Cells are typed by their text
//! under the workbook writer's read-back rule: plain decimals are numbers,
//! ISO 8601 dates and dates with times are dates, `TRUE` and `FALSE` are
//! booleans, everything else is text. A table may name its cells' number
//! and date formats (from a workbook's), and its merged ranges.

use super::*;

const NUMBERS_TEMPLATE: &[u8] = include_bytes!("../numbers_template.numbers");
/// `TN.DocumentArchive` and `TN.SheetArchive`, which the Pages registry
/// leaves out: their messages are kept as raw fields.
const NUMBERS_DOCUMENT: u32 = 1;
const NUMBERS_SHEET: u32 = 2;
const TREE_NODE: u32 = 205;
/// Raw field numbers: the document's sheets and sidebar order, a sheet's
/// name and drawables.
const DOCUMENT_SHEETS: u32 = 1;
const DOCUMENT_SIDEBAR: u32 = 5;
const SHEET_NAME: u32 = 1;
const SHEET_DRAWABLES: u32 = 2;
/// Space between tables stacked on one sheet, in points.
const TABLE_GAP: f32 = 30.0;
/// Days from Excel's day zero (1899-12-30) to 2001-01-01, Numbers' epoch.
const EXCEL_DAYS_TO_2001: f64 = 36_892.0;

/// A sheet to write: its name and its tables.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NumbersSheet {
    pub name: String,
    pub tables: Vec<NumbersTable>,
}

/// A table to write: its name and rows of cell text, the first row its
/// header; the formats its cells show in, and its merged ranges.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NumbersTable {
    pub name: String,
    pub rows: NumbersRows,
    /// Number and date formats the cells name.
    pub formats: Vec<cell_format::Format>,
    /// Per row, each cell's format: its index in `formats` from 1, or 0
    /// for none. A number takes a number format, a date a date format;
    /// another pairing is left unformatted. Empty when no cell has one.
    pub cell_formats: Vec<Vec<u16>>,
    /// Merged ranges as (row, column, rows, columns) from 0.
    pub merges: Vec<(usize, usize, usize, usize)>,
}

/// Rows of cell text, held compactly: every cell's text in one buffer and
/// where each cell and each row ends. A large table is millions of cells,
/// and a `String` each would cost some forty bytes before its text.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NumbersRows {
    text: String,
    cell_ends: Vec<usize>,
    row_ends: Vec<usize>,
}

impl NumbersRows {
    pub fn new() -> NumbersRows {
        NumbersRows::default()
    }

    /// Appends a row of cells.
    pub fn push_row<'a>(&mut self, cells: impl IntoIterator<Item = &'a str>) {
        for cell in cells {
            self.text.push_str(cell);
            self.cell_ends.push(self.text.len());
        }
        self.row_ends.push(self.cell_ends.len());
    }

    pub fn len(&self) -> usize {
        self.row_ends.len()
    }

    pub fn is_empty(&self) -> bool {
        self.row_ends.is_empty()
    }

    /// The cells of row `index`, empty past the last row.
    pub fn row(&self, index: usize) -> impl Iterator<Item = &str> + '_ {
        let cells = match index {
            0 => 0..self.row_ends.first().copied().unwrap_or(0),
            _ => match (self.row_ends.get(index - 1), self.row_ends.get(index)) {
                (Some(&start), Some(&end)) => start..end,
                _ => 0..0,
            },
        };
        cells.map(move |cell| {
            let start = if cell == 0 { 0 } else { self.cell_ends[cell - 1] };
            &self.text[start..self.cell_ends[cell]]
        })
    }

    /// The most cells a row has.
    pub fn width(&self) -> usize {
        let mut start = 0;
        let mut widest = 0;
        for &end in &self.row_ends {
            widest = widest.max(end - start);
            start = end;
        }
        widest
    }

    /// Every cell's text, in order (the output identity's digest).
    fn text(&self) -> &str {
        &self.text
    }
}

impl From<Vec<Vec<String>>> for NumbersRows {
    fn from(rows: Vec<Vec<String>>) -> NumbersRows {
        let mut compact = NumbersRows::new();
        for row in &rows {
            compact.push_row(row.iter().map(String::as_str));
        }
        compact
    }
}

/// Writes `sheets` as a Numbers package into `sink`.
pub fn write_numbers_to(
    mut sheets: Vec<NumbersSheet>,
    sink: &mut dyn std::io::Write,
) -> Result<(), PackageError> {
    // Identities vary with the content, as the Pages writer's do.
    let digest = sheets
        .iter()
        .flat_map(|sheet| sheet.tables.iter())
        .flat_map(|table| table.rows.text().bytes())
        .fold(0xCBF2_9CE4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01B3)
        });
    OUTPUT_SALT.store(digest, std::sync::atomic::Ordering::Relaxed);
    forget_object_positions();
    let mut package = Package::read(NUMBERS_TEMPLATE)?;

    // Every sheet holds at least one table, and the document one sheet.
    if sheets.is_empty() {
        sheets.push(NumbersSheet {
            name: "Sheet 1".to_string(),
            tables: Vec::new(),
        });
    }
    for sheet in &mut sheets {
        if sheet.tables.is_empty() {
            sheet.tables.push(NumbersTable {
                name: "Table 1".to_string(),
                ..NumbersTable::default()
            });
        }
    }

    let document_id = object_of_type(&package, NUMBERS_DOCUMENT)
        .ok_or_else(|| malformed("template has no document"))?;
    let sheet_id = raw_references(&package, document_id, DOCUMENT_SHEETS)
        .first()
        .copied()
        .ok_or_else(|| malformed("template has no sheet"))?;
    let proto_info = raw_references(&package, sheet_id, SHEET_DRAWABLES)
        .first()
        .copied()
        .ok_or_else(|| malformed("template sheet has no table"))?;
    let proto = template_table(&package, proto_info, proto_info)
        .ok_or_else(|| malformed("template table is incomplete"))?;
    let mut next_id = max_identifier(&package) + 1;

    // Tables: the prototype, then its clones.
    let total: usize = sheets.iter().map(|sheet| sheet.tables.len()).sum();
    let mut infos = vec![proto_info];
    infos.extend(clone_template_tables(
        &mut package,
        &proto,
        total - 1,
        &mut next_id,
    )?);
    // Sheets: the prototype, then its clones (without its table).
    let mut sheet_ids = vec![sheet_id];
    sheet_ids.extend(clone_sheets(
        &mut package,
        sheet_id,
        proto_info,
        sheets.len() - 1,
        &mut next_id,
    )?);

    // Fill each table and place it on its sheet.
    let mut assigned = infos.iter();
    let mut sheet_tables: Vec<Vec<u64>> = Vec::new();
    for (sheet, &sheet_id) in sheets.iter_mut().zip(&sheet_ids) {
        let mut ids = Vec::new();
        let mut top: Option<f32> = None;
        for table in &mut sheet.tables {
            let Some(&info) = assigned.next() else {
                break;
            };
            let template = template_table(&package, info, info)
                .ok_or_else(|| malformed("cloned table is incomplete"))?;
            // The table's rows move into its mark, and go with it.
            let mark = data_mark(&package, &template, table);
            reuse_table(
                &mut package,
                &template,
                &mark,
                &HashMap::new(),
                None,
                &HashMap::new(),
                &mut HashMap::new(),
                &mut HashMap::new(),
                &mut next_id,
            )?;
            set_table_name(&mut package, template.model_id, &table.name)?;
            // Tables on one sheet stack downwards from the template's place.
            let height: f32 = mark.heights.iter().sum();
            let y = place_table(&mut package, info, sheet_id, top)?;
            top = Some(y + height + TABLE_GAP);
            ids.push(info);
        }
        set_raw_sheet(&mut package, sheet_id, &sheet.name, &ids)?;
        sheet_tables.push(ids);
    }
    set_raw_references(&mut package, document_id, DOCUMENT_SHEETS, &sheet_ids)?;
    rebuild_sidebar(
        &mut package,
        document_id,
        &sheet_ids,
        &sheet_tables,
        &mut next_id,
    )?;

    reconcile_components(&mut package)?;
    let highest = max_identifier(&package);
    set_last_object_identifier(&mut package, highest)?;
    renew_document_identity(&mut package);
    package.write(sink)?;
    Ok(())
}

/// A table's cells as a data table mark: typed where the text reads back
/// as a number, a date, or a boolean; the first row the header.
fn data_mark(package: &Package, template: &TemplateTable, table: &mut NumbersTable) -> TableMark {
    let source = std::mem::take(&mut table.rows);
    let mut named = std::mem::take(&mut table.formats);
    let cell_formats = std::mem::take(&mut table.cell_formats);
    let merges = std::mem::take(&mut table.merges);
    let rows = source.len().max(1);
    let columns = source.width().max(1);
    let width = object_message(package, template.model_id)
        .and_then(
            |(tree, first)| match field_value(tree, first, "default_column_width") {
                Some(Node::Double(value)) => Some(value as f32),
                Some(Node::Float(value)) => Some(value),
                _ => None,
            },
        )
        .unwrap_or(100.0);
    let count = rows * columns;
    let mut data = Vec::with_capacity(count);
    // Merged ranges within the table, their covered cells left empty.
    let merges: Vec<(usize, usize, usize, usize)> = merges
        .into_iter()
        .filter(|&(row, column, height, width)| {
            (height > 1 || width > 1) && row + height <= rows && column + width <= columns
        })
        .collect();
    let mut covered = HashSet::new();
    for &(row, column, height, width) in &merges {
        for r in row..row + height {
            for c in column..column + width {
                if (r, c) != (row, column) {
                    covered.insert((r, c));
                }
            }
        }
    }
    // A date's format: the one it names, else the ISO form it was
    // written in (`DATA_DATE_FORMATS`), added to the list when used.
    let mut iso_keys = [0u16; 2];
    for row in 0..rows {
        let mut cells = source.row(row);
        for column in 0..columns {
            let text = cells.next().unwrap_or("");
            if covered.contains(&(row, column)) {
                data.push(DataCell::Empty);
                continue;
            }
            let key = cell_formats
                .get(row)
                .and_then(|keys| keys.get(column))
                .copied()
                .unwrap_or(0);
            let kind = (key > 0)
                .then(|| named.get(usize::from(key) - 1))
                .flatten()
                .map(|format| format.kind);
            let is_date = kind == Some(cell_format::DATE);
            let value = typed_value(text, kind).map(|value| match value {
                Typed::Number(decimal, _) if kind.is_some() && !is_date => {
                    Typed::Number(decimal, key)
                }
                Typed::Date(seconds, _) if is_date => Typed::Date(seconds, key),
                Typed::Date(seconds, time) => {
                    let slot = usize::from(time != 0);
                    if iso_keys[slot] == 0 {
                        named.push(cell_format::Format {
                            kind: cell_format::DATE,
                            date_time_format: DATA_DATE_FORMATS[slot].to_string(),
                            ..cell_format::Format::default()
                        });
                        iso_keys[slot] = named.len() as u16;
                    }
                    Typed::Date(seconds, iso_keys[slot])
                }
                other => other,
            });
            data.push(match value {
                Some(value) => DataCell::Value(value),
                None if text.is_empty() => DataCell::Empty,
                None => DataCell::Text(text.to_string()),
            });
        }
    }
    TableMark {
        offset: 0,
        rows,
        columns,
        header_rows: u32::from(rows > 1),
        // A data table's cells are `data`.
        cells: Vec::new(),
        widths: vec![width; columns],
        heights: vec![DEFAULT_ROW_HEIGHT; rows],
        merges,
        // A data table has no per-cell look: styled tables alone read these.
        backgrounds: Vec::new(),
        alignments: Vec::new(),
        paddings: Vec::new(),
        borders: None,
        cell_borders: Vec::new(),
        data,
        formats: named,
    }
}

/// A cell's value when its text reads back as one (the workbook writer's
/// rule): a number, a date or date and time, or a boolean; a time alone
/// only when a date format shows it (as a time on Excel's day zero). A
/// number's format key is 0, a date's 1 when it has a time and 0 if not,
/// for the caller to replace. Under a number format (`format` the kind it
/// names) any plain decimal is a number: the workbook said so.
fn typed_value(text: &str, format: Option<u32>) -> Option<Typed> {
    if text.is_empty() {
        return None;
    }
    let date_format = format == Some(cell_format::DATE);
    let plain = |text: &str| {
        let digits = text.strip_prefix('-').unwrap_or(text);
        let (mantissa, power) = digits.split_once(['e', 'E']).unwrap_or((digits, "0"));
        let power = power.strip_prefix('+').unwrap_or(power);
        let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
        !whole.is_empty()
            && whole.bytes().all(|byte| byte.is_ascii_digit())
            && fraction.bytes().all(|byte| byte.is_ascii_digit())
            && power.parse::<i32>().is_ok()
    };
    // A number with an exponent reads back without it (`1e3` is 1000):
    // it stays text, unless a number format says it is a number.
    if (crate::io::xlsx::writer::is_number_literal(text) && !text.contains(['e', 'E']))
        || (format.is_some() && !date_format && plain(text))
    {
        return decimal128(text).map(|decimal| Typed::Number(decimal, 0));
    }
    if text == "TRUE" || text == "FALSE" {
        return Some(Typed::Boolean(text == "TRUE"));
    }
    match crate::io::xlsx::writer::iso_serial(text) {
        // A time alone has no date in Numbers; it stays text unless a
        // format shows only its time.
        Some((serial, style)) if style != 3 || date_format => {
            // Whole days and whole seconds, apart: a fractional serial
            // loses the last second.
            let days = serial.floor();
            let seconds = ((serial - days) * 86_400.0).round();
            Some(Typed::Date(
                (days - EXCEL_DAYS_TO_2001) * 86_400.0 + seconds,
                u16::from(style == 2),
            ))
        }
        _ => None,
    }
}

/// A plain decimal (`-12.5`, `3e4`) as decimal128: a 113-bit mantissa and a
/// biased power of ten, as Numbers stores numbers.
fn decimal128(text: &str) -> Option<[u8; 16]> {
    let (negative, text) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (number, power) = match text.split_once(['e', 'E']) {
        Some((number, power)) => (number, power.parse::<i32>().ok()?),
        None => (text, 0),
    };
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    let mut mantissa: u128 = 0;
    for digit in whole.bytes().chain(fraction.bytes()) {
        mantissa = mantissa
            .checked_mul(10)?
            .checked_add(u128::from(digit.checked_sub(b'0')?))?;
    }
    let exponent = power - fraction.len() as i32;
    let biased = u32::try_from(exponent + 0x1820).ok()?;
    let mut bytes = [0u8; 16];
    bytes[..14].copy_from_slice(&mantissa.to_le_bytes()[..14]);
    bytes[14] = (((biased & 0x7F) << 1) as u8) | ((mantissa >> 112) & 1) as u8;
    bytes[15] = ((biased >> 7) & 0x7F) as u8 | if negative && mantissa != 0 { 0x80 } else { 0 };
    Some(bytes)
}

/// The first object of `message_type`.
fn object_of_type(package: &Package, message_type: u32) -> Option<u64> {
    package.entries.iter().find_map(|entry| match entry {
        Entry::Stream(stream) => stream
            .objects
            .iter()
            .find(|object| first_type(object) == Some(message_type))
            .map(|object| object.identifier),
        _ => None,
    })
}

/// The references a raw (schema-less) message holds in field `number`.
fn raw_references(package: &Package, id: u64, number: u32) -> Vec<u64> {
    let Some((tree, first)) = object_message(package, id) else {
        return Vec::new();
    };
    tree.chain(first)
        .filter(|(_, entry)| entry.number == number)
        .filter_map(|(_, entry)| match entry.value {
            Node::RawBytes(span) => raw_reference(tree.bytes(span)),
            _ => None,
        })
        .collect()
}

/// The identifier in an encoded `TSP.Reference`.
fn raw_reference(bytes: &[u8]) -> Option<u64> {
    let (&tag, rest) = bytes.split_first()?;
    if tag != 0x08 {
        return None;
    }
    let mut value = 0u64;
    for (index, byte) in rest.iter().enumerate() {
        value |= u64::from(byte & 0x7F) << (7 * index);
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

/// An encoded `TSP.Reference` to `id`.
fn encoded_reference(id: u64) -> Vec<u8> {
    let mut out = vec![0x08];
    crate::io::protobuf::tree::write_varint(&mut out, id);
    out
}

/// Rewrites a raw message, replacing every field in `fields` by the given
/// encoded values and keeping the rest.
fn rewrite_raw(
    package: &mut Package,
    id: u64,
    fields: &[(u32, Vec<Vec<u8>>)],
) -> Result<(), PackageError> {
    rewrite_object_with(package, id, |tree, old_first| {
        let kept: Vec<(u32, Node)> = tree
            .chain(old_first)
            .filter(|(_, entry)| !fields.iter().any(|(number, _)| *number == entry.number))
            .map(|(_, entry)| (entry.number, entry.value))
            .collect();
        let mut chain = Chain::new();
        for (number, values) in fields {
            for value in values {
                let span = tree.push_bytes(value).map_err(tree_error)?;
                tree.push_unknown(&mut chain, *number, Node::RawBytes(span))
                    .map_err(tree_error)?;
            }
        }
        for (number, value) in kept {
            tree.push_unknown(&mut chain, number, value)
                .map_err(tree_error)?;
        }
        Ok(chain.first)
    })
}

fn set_raw_references(
    package: &mut Package,
    id: u64,
    number: u32,
    references: &[u64],
) -> Result<(), PackageError> {
    let values = references.iter().map(|id| encoded_reference(*id)).collect();
    rewrite_raw(package, id, &[(number, values)])?;
    add_object_refs(package, id, references)
}

/// A sheet's name and its tables.
fn set_raw_sheet(
    package: &mut Package,
    id: u64,
    name: &str,
    tables: &[u64],
) -> Result<(), PackageError> {
    let drawables = tables.iter().map(|id| encoded_reference(*id)).collect();
    rewrite_raw(
        package,
        id,
        &[
            (SHEET_NAME, vec![name.as_bytes().to_vec()]),
            (SHEET_DRAWABLES, drawables),
        ],
    )?;
    add_object_refs(package, id, tables)
}

fn set_table_name(package: &mut Package, model_id: u64, name: &str) -> Result<(), PackageError> {
    if name.is_empty() {
        return Ok(());
    }
    let stream = stream_containing(package, model_id)?;
    let first = find_object(&stream.objects, model_id)
        .map(|object| object.messages[0].first)
        .ok_or_else(|| malformed("table model is missing"))?;
    set_field_str(&mut stream.tree, first, "table_name", name)
}

/// Puts a table on its sheet (its drawable's parent) and, after the first,
/// below the one before (`top`). Returns the table's top.
fn place_table(
    package: &mut Package,
    info: u64,
    sheet: u64,
    top: Option<f32>,
) -> Result<f32, PackageError> {
    let stream = stream_containing(package, info)?;
    let first = find_object(&stream.objects, info)
        .and_then(|object| object.messages.first())
        .map(|message| message.first)
        .ok_or_else(|| malformed("table info is missing"))?;
    let drawable = message_field(&stream.tree, first, "super")
        .ok_or_else(|| malformed("table info has no drawable"))?;
    if let Some(index) = field_entry(&stream.tree, drawable, "parent") {
        stream.tree.entries[index as usize].value = Node::Reference(sheet);
    }
    let position = message_field(&stream.tree, drawable, "geometry")
        .and_then(|geometry| message_field(&stream.tree, geometry, "position"))
        .ok_or_else(|| malformed("table info has no position"))?;
    let y = match top {
        Some(top) => {
            set_field_float(&mut stream.tree, position, "y", top);
            top
        }
        None => match field_value(&stream.tree, position, "y") {
            Some(Node::Float(y)) => y,
            _ => 0.0,
        },
    };
    let info_chain = find_object(&stream.objects, info).map(|object| object.info);
    if let Some(info_chain) = info_chain {
        add_object_references(&mut stream.tree, info_chain, &[sheet])?;
    }
    Ok(y)
}

/// Clones the template sheet `count` times: the sheet and the objects only
/// it uses (header and footer storages, its guide storage), with fresh
/// identifiers and UUIDs, in the document stream. Its table is not copied;
/// each sheet is given its own.
fn clone_sheets(
    package: &mut Package,
    sheet: u64,
    table: u64,
    count: usize,
    next_id: &mut u64,
) -> Result<Vec<u64>, PackageError> {
    if count == 0 {
        return Ok(Vec::new());
    }
    let (stream_index, _) =
        locate_object(package, sheet).ok_or_else(|| malformed("template sheet is missing"))?;
    let Entry::Stream(stream) = &package.entries[stream_index] else {
        return Err(malformed("template sheet is not in a stream"));
    };
    // The cluster: the sheet and what it reaches within its own stream,
    // short of the document, other sheets, the sidebar, and its table.
    let positions: HashMap<u64, usize> = stream
        .objects
        .iter()
        .enumerate()
        .map(|(index, object)| (object.identifier, index))
        .collect();
    let mut cluster: Vec<u64> = Vec::new();
    let mut pending = vec![sheet];
    while let Some(id) = pending.pop() {
        if id == table || cluster.contains(&id) {
            continue;
        }
        let Some(&position) = positions.get(&id) else {
            continue;
        };
        let object = &stream.objects[position];
        if id != sheet
            && matches!(
                first_type(object),
                Some(NUMBERS_DOCUMENT | NUMBERS_SHEET | TREE_NODE)
            )
        {
            continue;
        }
        cluster.push(id);
        pending.extend(archive_references(&stream.tree, object.info));
    }
    // The UUID families the cluster's objects use.
    let mut family_keys: Vec<[u8; 12]> = Vec::new();
    for id in &cluster {
        let object = &stream.objects[positions[id]];
        for message in &object.messages {
            walk_chains(&stream.tree, message.first, &mut |tree, chain| {
                for uuid in chain_uuids(tree, chain) {
                    let bytes = match uuid {
                        UuidAt::Words(_, b) | UuidAt::Pair(_, b) | UuidAt::Text(b) => b,
                    };
                    if let Ok(family) = <[u8; 12]>::try_from(&bytes[4..])
                        && family.iter().any(|byte| *byte != 0)
                        && !family_keys.contains(&family)
                    {
                        family_keys.push(family);
                    }
                }
            });
        }
    }
    let mut sheets = Vec::with_capacity(count);
    let mut maps = Vec::with_capacity(count);
    for _ in 0..count {
        let mut families = HashMap::new();
        for family in &family_keys {
            let fresh = fresh_uuid(families.len() as u64 + *next_id);
            if let Some(fresh) = parse_uuid_text(&fresh)
                && let Ok(fresh) = <[u8; 12]>::try_from(&fresh[4..])
            {
                families.insert(*family, fresh);
            }
        }
        let mut map = CloneMap {
            ids: HashMap::new(),
            families,
            owners: HashMap::new(),
        };
        for id in &cluster {
            map.ids.insert(*id, *next_id);
            *next_id += 1;
        }
        let Entry::Stream(stream) = &mut package.entries[stream_index] else {
            return Err(malformed("template sheet is not in a stream"));
        };
        for id in &cluster {
            let position = positions[id];
            let source = stream.objects[position].clone_shape();
            let mut messages = Vec::new();
            for (message_type, first) in &source.1 {
                messages.push(ObjectMessage {
                    message_type: *message_type,
                    first: copy_within(&mut stream.tree, *first, &map)?,
                });
            }
            let info = copy_within(&mut stream.tree, source.0, &map)?;
            let new_id = map.id(*id);
            set_field_uint(&mut stream.tree, info, "identifier", new_id);
            stream.objects.push(Object {
                identifier: new_id,
                info,
                messages,
            });
        }
        // The sheet's message has no schema: its references (header and
        // footer storages, guide storage) are raw bytes the copy left
        // pointing at the template's objects.
        let sheet_clone = map.id(sheet);
        if let Some(object) = stream
            .objects
            .iter()
            .find(|object| object.identifier == sheet_clone)
        {
            let first = object.messages[0].first;
            let raw: Vec<(u32, u64)> = stream
                .tree
                .chain(first)
                .filter_map(|(index, entry)| match entry.value {
                    Node::RawBytes(span) => {
                        raw_reference(stream.tree.bytes(span)).map(|id| (index, id))
                    }
                    _ => None,
                })
                .collect();
            for (index, id) in raw {
                let mapped = map.id(id);
                if mapped != id {
                    let span = stream
                        .tree
                        .push_bytes(&encoded_reference(mapped))
                        .map_err(tree_error)?;
                    stream.tree.entries[index as usize].value = Node::RawBytes(span);
                }
            }
        }
        sheets.push(sheet_clone);
        maps.push(map);
    }
    add_uuid_map_clones(package, &maps)?;
    Ok(sheets)
}

/// The sidebar's tree: a node per sheet under the root, and under each a
/// node per table. The template's root and first sheet and table nodes are
/// kept; the rest are new.
fn rebuild_sidebar(
    package: &mut Package,
    document: u64,
    sheets: &[u64],
    tables: &[Vec<u64>],
    next_id: &mut u64,
) -> Result<(), PackageError> {
    let root = raw_references(package, document, DOCUMENT_SIDEBAR)
        .first()
        .copied()
        .ok_or_else(|| malformed("template has no sidebar"))?;
    let tree_children = |package: &Package, id: u64| -> Vec<u64> {
        object_message(package, id)
            .map(|(tree, first)| {
                tree.chain(first)
                    .filter(|(_, entry)| {
                        tree.field(entry).map(|field| field.name) == Some("children")
                    })
                    .filter_map(|(_, entry)| match entry.value {
                        Node::Reference(id) => Some(id),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let first_sheet_node = tree_children(package, root).first().copied();
    let first_table_node =
        first_sheet_node.and_then(|node| tree_children(package, node).first().copied());
    let mut sheet_nodes = Vec::with_capacity(sheets.len());
    for (index, (&sheet, sheet_tables)) in sheets.iter().zip(tables).enumerate() {
        let mut table_nodes = Vec::with_capacity(sheet_tables.len());
        for (position, &table) in sheet_tables.iter().enumerate() {
            let node = match (index, position, first_table_node) {
                (0, 0, Some(node)) => node,
                _ => new_node_id(next_id),
            };
            write_tree_node(package, node, Some(table), &[])?;
            table_nodes.push(node);
        }
        let node = match (index, first_sheet_node) {
            (0, Some(node)) => node,
            _ => new_node_id(next_id),
        };
        write_tree_node(package, node, Some(sheet), &table_nodes)?;
        sheet_nodes.push(node);
    }
    write_tree_node(package, root, None, &sheet_nodes)
}

fn new_node_id(next_id: &mut u64) -> u64 {
    let id = *next_id;
    *next_id += 1;
    id
}

/// Writes (or creates, in the document stream) a `TSK.TreeNode` naming
/// `object` with `children`.
fn write_tree_node(
    package: &mut Package,
    id: u64,
    object: Option<u64>,
    children: &[u64],
) -> Result<(), PackageError> {
    let node = message_ref("TSK.TreeNode")?;
    let build = |tree: &mut Tree| -> Result<u32, PackageError> {
        let mut chain = Chain::new();
        for child in children {
            push_field(tree, &mut chain, node, "children", Node::Reference(*child))?;
        }
        if let Some(object) = object {
            push_field(tree, &mut chain, node, "object", Node::Reference(object))?;
        }
        Ok(chain.first)
    };
    let mut references: Vec<u64> = children.to_vec();
    references.extend(object);
    if locate_object(package, id).is_some() {
        rewrite_object(package, id, build)?;
        return add_object_refs(package, id, &references);
    }
    let stream = document_stream(package)?;
    let first = build(&mut stream.tree)?;
    let info = build_archive_info(&mut stream.tree, id, TREE_NODE)?;
    add_object_references(&mut stream.tree, info, &references)?;
    stream.objects.push(Object {
        identifier: id,
        info,
        messages: vec![ObjectMessage {
            message_type: TREE_NODE,
            first,
        }],
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimals_encode_as_numbers_reads_them() {
        for text in [
            "0",
            "42",
            "-3",
            "1.5",
            "0.25",
            "123456789012345",
            "1e10",
            "-2.5E-3",
        ] {
            let bytes = decimal128(text).expect("encodes");
            let back = crate::io::pages::document::decimal128_text(&bytes).expect("decodes");
            let want: f64 = text.parse().unwrap();
            assert_eq!(back.parse::<f64>().unwrap(), want, "{text}");
        }
    }
}
