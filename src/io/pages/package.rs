//! A Pages package as a whole: every ZIP entry, with the IWA streams
//! decoded into schema-named object trees and everything else kept as
//! bytes. Reading and writing are inverse: the objects re-encode to the
//! exact bytes of the streams they came from (the ZIP and Snappy layers
//! are rebuilt, so those bytes differ, their contents do not).

use std::collections::{HashMap, HashSet};
use std::fmt;

use super::schema::SCHEMA;
use super::{message_schema, type_name};
use crate::io::iwa::{IwaError, IwaObject, decompress_stream, parse_objects};
use crate::io::protobuf::tree::{NONE, Node, Tree, TreeError, write_varint};
use crate::io::snappy::compress_block;
use crate::io::zip::{ZipArchive, ZipError, ZipWriter};

/// Uncompressed bytes per IWA chunk when writing.
const CHUNK_SIZE: usize = 64 * 1024;

#[derive(Debug)]
pub enum PackageError {
    Zip(ZipError),
    Iwa {
        stream: String,
        error: IwaError,
    },
    Tree {
        stream: String,
        error: TreeError,
    },
    Io(std::io::Error),
    /// The stream's headers do not describe its objects.
    Malformed {
        stream: String,
        what: &'static str,
    },
}

impl fmt::Display for PackageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PackageError::Zip(error) => write!(formatter, "{error}"),
            PackageError::Iwa { stream, error } => write!(formatter, "{stream}: {error}"),
            PackageError::Tree { stream, error } => write!(formatter, "{stream}: {error}"),
            PackageError::Io(error) => write!(formatter, "{error}"),
            PackageError::Malformed { stream, what } => write!(formatter, "{stream}: {what}"),
        }
    }
}

impl std::error::Error for PackageError {}

impl From<ZipError> for PackageError {
    fn from(error: ZipError) -> Self {
        PackageError::Zip(error)
    }
}

impl From<std::io::Error> for PackageError {
    fn from(error: std::io::Error) -> Self {
        PackageError::Io(error)
    }
}

/// One message of an object: its registry type and its fields, as the
/// first entry of a chain in the stream's tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectMessage {
    pub message_type: u32,
    pub first: u32,
}

/// One object: its `ArchiveInfo` header chain and its messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    pub identifier: u64,
    pub info: u32,
    pub messages: Vec<ObjectMessage>,
}

/// A decoded `.iwa` entry: its objects and the tree that holds them.
pub struct Stream {
    pub name: String,
    pub tree: Tree,
    pub objects: Vec<Object>,
}

pub enum Entry {
    Stream(Stream),
    File { name: String, bytes: Vec<u8> },
}

impl Entry {
    pub fn name(&self) -> &str {
        match self {
            Entry::Stream(stream) => &stream.name,
            Entry::File { name, .. } => name,
        }
    }
}

#[derive(Default)]
pub struct Package {
    pub entries: Vec<Entry>,
}

/// How much of the package to decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Every object of every stream: the lossless form.
    Everything,
    /// The objects a document reader reaches from the document root, the
    /// package metadata, and the calculation engine, following object
    /// references but never through the hubs (the stylesheet, the theme,
    /// view and layout state) whose members are reached one by one from
    /// the text. A typical document uses seventy of its six hundred
    /// objects; the rest are presets and are left undecoded.
    Document,
    /// What a workbook reader needs: every object but the skipped types
    /// and the tables' row headers (sizes, which a cell's text does not
    /// need), each stream decoded and freed in turn. Not a walk from the
    /// root: older Numbers files leave references out of their object
    /// headers, so a walk misses tables' cells.
    Workbook,
}

/// Object types the document walk starts from: the Pages document, the
/// package metadata, the calculation engine, and the Numbers document
/// (`TN.DocumentArchive`, a type Pages does not use).
const ROOT_TYPES: [u32; 4] = [10000, 11006, 4000, 1];
/// Object types whose references are not followed: hubs that list every
/// style, theme preset, or view state in the package.
const HUB_TYPES: [u32; 7] = [401, 10001, 210, 10133, 10131, 10147, 213];
/// Object types no reader looks at, left undecoded: the formula engine's
/// cell records, reference and name tracking, the tables' row and column
/// identity maps, header-name caches, filters, and categories. A large
/// spreadsheet's engine is mostly these.
/// `TST.HeaderStorageBucket`: a table's row or column sizes.
const ROW_HEADERS: u32 = 6006;
const SKIPPED_TYPES: [u32; 11] = [
    4003, 4004, 4009, 6220, 6267, 6316, 6317, 6365, 6366, 6373, 6383,
];

/// `TST.TableModelArchive`, `TST.TableInfoArchive`, `TST.SummaryModelArchive`,
/// `TST.ColumnRowUIDMapArchive`, and `TST.GroupByArchive`.
const TABLE_MODEL: u32 = 6001;
const TABLE_INFO: u32 = 6000;
const SUMMARY_MODEL: u32 = 6316;
const UID_MAP: u32 = 6267;
const GROUP_BY: u32 = 6373;

/// The objects of skipped types a table's view needs, among a stream's
/// objects (Numbers keeps a table's model, info, and maps in one stream):
/// for each table whose info names a view (`view_column_row_uids`, which
/// only pivot and categorised tables have), the uid maps of its info,
/// model, and summary model, and the summary model, which holds a pivot's
/// grand totals; and, when any does, the stream's categories
/// (`TST.GroupByArchive`, small for a table without them). Found by the
/// references each object's header lists.
fn view_objects(objects: &[crate::io::iwa::IwaObject<'_>]) -> HashSet<u64> {
    let types: HashMap<u64, u32> = objects
        .iter()
        .map(|object| (object.identifier, object.message_type().unwrap_or(0)))
        .collect();
    let of_type =
        |id: &u64, wanted: &[u32]| types.get(id).is_some_and(|kind| wanted.contains(kind));
    let references = |object: &crate::io::iwa::IwaObject<'_>| -> Vec<u64> {
        object
            .messages
            .iter()
            .flat_map(|message| message.object_references.iter().copied())
            .collect()
    };
    let mut keep = HashSet::new();
    let mut models = HashSet::new();
    for object in objects {
        if object.message_type() != Some(TABLE_INFO) {
            continue;
        }
        let refs = references(object);
        if !refs.iter().any(|id| of_type(id, &[UID_MAP])) {
            continue;
        }
        models.extend(
            refs.iter()
                .copied()
                .filter(|id| of_type(id, &[TABLE_MODEL])),
        );
        keep.extend(
            refs.into_iter()
                .filter(|id| of_type(id, &[UID_MAP, SUMMARY_MODEL])),
        );
    }
    if keep.is_empty() {
        return keep;
    }
    for object in objects {
        let message_type = object.message_type();
        if message_type == Some(GROUP_BY) {
            keep.insert(object.identifier);
        }
        let named = models.contains(&object.identifier)
            || (message_type == Some(SUMMARY_MODEL) && keep.contains(&object.identifier));
        if named {
            keep.extend(
                references(object)
                    .into_iter()
                    .filter(|id| of_type(id, &[UID_MAP])),
            );
        }
    }
    keep
}

impl Package {
    pub fn read(bytes: &[u8]) -> Result<Package, PackageError> {
        Package::read_scope(bytes, Scope::Everything)
    }

    pub fn read_scope(bytes: &[u8], scope: Scope) -> Result<Package, PackageError> {
        let archive = ZipArchive::parse(bytes)?;
        // A password-protected document carries its hint (`.iwph`) and
        // encrypts every stream.
        if archive.entries().iter().any(|entry| entry.name == ".iwph") {
            return Err(PackageError::Malformed {
                stream: ".iwph".to_string(),
                what: "the document is password-protected; remove the password in Pages or Numbers first",
            });
        }
        if let Some(package) = Package::read_bundle(&archive, scope)? {
            return Ok(package);
        }
        if scope == Scope::Workbook {
            return Package::read_workbook(&archive);
        }
        if scope == Scope::Document {
            return Package::read_document(&archive);
        }
        // Every entry is read first; the reachable set needs all streams'
        // object headers before any stream is decoded.
        let mut raw: Vec<(String, Vec<u8>, bool)> = Vec::new();
        for entry in archive.entries() {
            if entry.is_directory() {
                continue;
            }
            let mut data = Vec::new();
            archive.read(entry, &mut data)?;
            // A stream Apple compressed with LZFSE (`bvxn`, the operation
            // log of a shared document) is kept as its bytes: nothing the
            // readers use is in it, and it is written back as it was.
            let is_stream = entry.name.ends_with(".iwa") && !data.starts_with(b"bvx");
            if is_stream {
                let decompressed = decompress_stream(&data).map_err(|error| PackageError::Iwa {
                    stream: entry.name.clone(),
                    error,
                })?;
                raw.push((entry.name.clone(), decompressed, true));
            } else {
                raw.push((entry.name.clone(), data, false));
            }
        }
        let mut parsed: Vec<Option<Vec<IwaObject<'_>>>> = Vec::with_capacity(raw.len());
        for (name, bytes, is_stream) in &raw {
            if *is_stream {
                let objects = parse_objects(bytes).map_err(|error| PackageError::Iwa {
                    stream: name.clone(),
                    error,
                })?;
                parsed.push(Some(objects));
            } else {
                parsed.push(None);
            }
        }
        let mut entries = Vec::with_capacity(raw.len());
        for ((name, bytes, _), objects) in raw.iter().zip(parsed) {
            match objects {
                Some(objects) => {
                    entries.push(Entry::Stream(decode_objects(
                        name,
                        bytes,
                        objects,
                        |_| true,
                        &[],
                    )?));
                }
                None => entries.push(Entry::File {
                    name: name.clone(),
                    bytes: bytes.clone(),
                }),
            }
        }
        Ok(Package { entries })
    }

    /// `Scope::Document`: the objects a document reader reaches, in two
    /// passes over the streams so no two are held decompressed at once:
    /// the first keeps each object's identifier, type, and references for
    /// the walk; the second decompresses each stream again and decodes the
    /// reachable objects alone.
    fn read_document(archive: &ZipArchive<'_>) -> Result<Package, PackageError> {
        let read_stream = |entry: &crate::io::zip::Entry| -> Result<Option<Vec<u8>>, PackageError> {
            let mut data = Vec::new();
            archive.read(entry, &mut data)?;
            // A stream Apple compressed with LZFSE (`bvxn`, the operation
            // log of a shared document) is kept as its bytes: nothing the
            // readers use is in it, and it is written back as it was.
            if !entry.name.ends_with(".iwa") || data.starts_with(b"bvx") {
                return Ok(None);
            }
            decompress_stream(&data)
                .map(Some)
                .map_err(|error| PackageError::Iwa {
                    stream: entry.name.clone(),
                    error,
                })
        };
        let mut headers: Vec<ObjectHeader> = Vec::new();
        for entry in archive.entries() {
            if entry.is_directory() {
                continue;
            }
            let Some(bytes) = read_stream(entry)? else {
                continue;
            };
            let objects = parse_objects(&bytes).map_err(|error| PackageError::Iwa {
                stream: entry.name.clone(),
                error,
            })?;
            headers.extend(objects.iter().map(|object| {
                ObjectHeader {
                    identifier: object.identifier,
                    message_type: object.message_type().unwrap_or(0),
                    references: object
                        .messages
                        .iter()
                        .flat_map(|message| message.object_references.iter().copied())
                        .collect(),
                }
            }));
        }
        let reachable = reachable_objects(&headers);
        drop(headers);
        let deferred = deferred_fields();
        let mut entries = Vec::new();
        for entry in archive.entries() {
            if entry.is_directory() {
                continue;
            }
            match read_stream(entry)? {
                Some(bytes) => {
                    let objects = parse_objects(&bytes).map_err(|error| PackageError::Iwa {
                        stream: entry.name.clone(),
                        error,
                    })?;
                    entries.push(Entry::Stream(decode_objects(
                        &entry.name,
                        &bytes,
                        objects,
                        |identifier| reachable.contains(&identifier),
                        &deferred,
                    )?));
                }
                None => {
                    let mut bytes = Vec::new();
                    archive.read(entry, &mut bytes)?;
                    entries.push(Entry::File {
                        name: entry.name.clone(),
                        bytes,
                    });
                }
            }
        }
        Ok(Package { entries })
    }

    /// `Scope::Workbook`: each stream decompressed, decoded without the
    /// skipped types and the row headers, and freed before the next, since
    /// the workbook reads every other object and needs no walk.
    fn read_workbook(archive: &ZipArchive<'_>) -> Result<Package, PackageError> {
        let deferred = deferred_fields();
        let mut entries = Vec::new();
        for entry in archive.entries() {
            if entry.is_directory() {
                continue;
            }
            let mut data = Vec::new();
            archive.read(entry, &mut data)?;
            if !entry.name.ends_with(".iwa") || data.starts_with(b"bvx") {
                entries.push(Entry::File {
                    name: entry.name.clone(),
                    bytes: data,
                });
                continue;
            }
            let bytes = decompress_stream(&data).map_err(|error| PackageError::Iwa {
                stream: entry.name.clone(),
                error,
            })?;
            drop(data);
            let objects = parse_objects(&bytes).map_err(|error| PackageError::Iwa {
                stream: entry.name.clone(),
                error,
            })?;
            let views = view_objects(&objects);
            let skipped: Vec<u64> = objects
                .iter()
                .filter(|object| {
                    let message_type = object.message_type().unwrap_or(0);
                    (SKIPPED_TYPES.contains(&message_type) || message_type == ROW_HEADERS)
                        && !views.contains(&object.identifier)
                })
                .map(|object| object.identifier)
                .collect();
            let keep = |identifier: u64| !skipped.contains(&identifier);
            entries.push(Entry::Stream(decode_objects(
                &entry.name,
                &bytes,
                objects,
                keep,
                &deferred,
            )?));
        }
        Ok(Package { entries })
    }

    /// A package saved as a folder and then zipped (`Name.pages/Index.zip`
    /// beside `Name.pages/Data/...`): the objects are in the inner
    /// `Index.zip`. The outer files join the package under the names they
    /// have inside the folder.
    fn read_bundle(
        archive: &ZipArchive<'_>,
        scope: Scope,
    ) -> Result<Option<Package>, PackageError> {
        let entries = archive.entries();
        if entries.iter().any(|entry| entry.name.ends_with(".iwa")) {
            return Ok(None);
        }
        let Some(index) = entries
            .iter()
            .find(|entry| entry.name == "Index.zip" || entry.name.ends_with("/Index.zip"))
        else {
            return Ok(None);
        };
        let prefix = &index.name[..index.name.len() - "Index.zip".len()];
        let mut inner = Vec::new();
        archive.read(index, &mut inner)?;
        let mut package = Package::read_scope(&inner, scope)?;
        for entry in entries {
            if entry.is_directory() || entry.name == index.name {
                continue;
            }
            let Some(name) = entry.name.strip_prefix(prefix) else {
                continue;
            };
            let mut bytes = Vec::new();
            archive.read(entry, &mut bytes)?;
            package.entries.push(Entry::File {
                name: name.to_string(),
                bytes,
            });
        }
        Ok(Some(package))
    }

    /// Writes the package as a ZIP with stored entries, as Pages does;
    /// streams are re-encoded and Snappy-compressed in 64 KiB chunks.
    pub fn write<W: std::io::Write>(&self, sink: W) -> Result<W, PackageError> {
        let mut zip = ZipWriter::new(sink);
        let mut encoded = Vec::new();
        let mut chunked = Vec::new();
        let mut block = Vec::new();
        // Each stream is compressed a chunk at a time as its objects are
        // encoded, so only the object being encoded (and not the whole
        // stream) is held uncompressed.
        let mut compress = |encoded: &mut Vec<u8>, chunked: &mut Vec<u8>, all: bool| {
            let mut start = 0;
            while encoded.len() - start >= CHUNK_SIZE || (all && start < encoded.len()) {
                let end = (start + CHUNK_SIZE).min(encoded.len());
                block.clear();
                compress_block(&encoded[start..end], &mut block);
                chunked.push(0);
                chunked.extend_from_slice(&(block.len() as u32).to_le_bytes()[..3]);
                chunked.extend_from_slice(&block);
                start = end;
            }
            encoded.drain(..start);
        };
        for entry in &self.entries {
            match entry {
                Entry::File { name, bytes } => zip.add(name, bytes)?,
                Entry::Stream(stream) => {
                    encoded.clear();
                    chunked.clear();
                    let mut scratch = ObjectScratch::default();
                    for object in &stream.objects {
                        encode_object(stream, object, &mut scratch, &mut encoded)?;
                        compress(&mut encoded, &mut chunked, false);
                    }
                    compress(&mut encoded, &mut chunked, true);
                    zip.add(&stream.name, &chunked)?;
                }
            }
        }
        Ok(zip.finish()?)
    }
}

/// An object's identifier, type, and references: what the reachability
/// walk needs of it.
struct ObjectHeader {
    identifier: u64,
    message_type: u32,
    references: Vec<u64>,
}

/// The identifiers a document reader reaches (see `Scope::Document`).
fn reachable_objects(objects: &[ObjectHeader]) -> HashSet<u64> {
    let mut index: HashMap<u64, usize> = HashMap::with_capacity(objects.len());
    let mut pending: Vec<u64> = Vec::new();
    for (position, object) in objects.iter().enumerate() {
        index.insert(object.identifier, position);
        if ROOT_TYPES.contains(&object.message_type) {
            pending.push(object.identifier);
        }
    }
    let mut reachable: HashSet<u64> = HashSet::new();
    while let Some(identifier) = pending.pop() {
        let Some(position) = index.get(&identifier) else {
            continue;
        };
        let object = &objects[*position];
        if SKIPPED_TYPES.contains(&object.message_type) || !reachable.insert(identifier) {
            continue;
        }
        if HUB_TYPES.contains(&object.message_type) {
            continue;
        }
        pending.extend(object.references.iter().copied());
    }
    reachable
}

/// Decodes one `.iwa` entry.
pub fn decode_stream(name: &str, compressed: &[u8]) -> Result<Stream, PackageError> {
    let bytes = decompress_stream(compressed).map_err(|error| PackageError::Iwa {
        stream: name.to_string(),
        error,
    })?;
    let raw_objects = parse_objects(&bytes).map_err(|error| PackageError::Iwa {
        stream: name.to_string(),
        error,
    })?;
    decode_objects(name, &bytes, raw_objects, |_| true, &[])
}

/// The attribute tables of `TSWP.StorageArchive`: the bulk of a text
/// stream, which the document reader parses itself (see
/// `Tree::deferred`).
#[inline(never)]
fn deferred_fields() -> Vec<(u16, u32)> {
    let Some(storage) = SCHEMA.message("TSWP.StorageArchive") else {
        return Vec::new();
    };
    [
        "table_para_style",
        "table_char_style",
        "table_list_style",
        "table_smartfield",
        "table_attachment",
        "table_footnote",
        "table_insertion",
        "table_deletion",
        "table_section",
        "table_layout_style",
        "table_para_data",
        "table_para_starts",
    ]
    .iter()
    .filter_map(|name| storage.slot_named(name))
    .map(|(_, field)| (storage.index(), field.number))
    // A table's layout cache, which no reader opens.
    .chain(SCHEMA.message("TST.TableInfoArchive").and_then(|info| {
        info.slot_named("layout_engine")
            .map(|(_, field)| (info.index(), field.number))
    }))
    // A data list's entries (a table's strings, styles, formats), which the
    // readers decode one at a time.
    .chain(SCHEMA.message("TST.TableDataList").and_then(|list| {
        list.slot_named("entries")
            .map(|(_, field)| (list.index(), field.number))
    }))
    // A tile's rows, which the table readers parse themselves (a row is nine
    // fields, and a large table has a million rows).
    .chain(SCHEMA.message("TST.Tile").and_then(|tile| {
        tile.slot_named("rowInfos")
            .map(|(_, field)| (tile.index(), field.number))
    }))
    .collect()
}

/// Fields a reading scope leaves out of the deferred messages that hold
/// them: a tile row's legacy (pre-BNC) copy of its cells, which Pages and
/// Numbers keep beside the current one and no reader opens (half a row's
/// bytes).
fn stripped_fields() -> Vec<(u16, u32)> {
    let Some(row) = SCHEMA.message("TST.TileRowInfo") else {
        return Vec::new();
    };
    ["cell_storage_buffer_pre_bnc", "cell_offsets_pre_bnc"]
        .iter()
        .filter_map(|name| row.slot_named(name))
        .map(|(_, field)| (row.index(), field.number))
        .collect()
}

/// Decodes the objects `keep` selects into a stream; the others keep
/// their identifier with no messages, so lookups by identifier still
/// resolve and readers see them as absent.
fn decode_objects(
    name: &str,
    bytes: &[u8],
    raw_objects: Vec<IwaObject<'_>>,
    keep: impl Fn(u64) -> bool,
    deferred: &[(u16, u32)],
) -> Result<Stream, PackageError> {
    let info_schema = SCHEMA.message("TSP.ArchiveInfo");
    let mut tree = Tree::new(&SCHEMA);
    tree.deferred = deferred.to_vec();
    if !deferred.is_empty() {
        tree.stripped = stripped_fields();
    }
    tree.entries.reserve(bytes.len() / 8);
    tree.text.reserve(bytes.len() / 2);
    let mut objects = Vec::with_capacity(raw_objects.len());
    for raw in raw_objects {
        if !keep(raw.identifier) {
            objects.push(Object {
                identifier: raw.identifier,
                info: NONE,
                messages: Vec::new(),
            });
            continue;
        }
        let info = tree
            .decode(raw.info, info_schema)
            .map_err(|error| PackageError::Tree {
                stream: name.to_string(),
                error,
            })?;
        let mut messages = Vec::with_capacity(raw.messages.len());
        for message in &raw.messages {
            let schema = message_schema(message.message_type);
            let first =
                tree.decode(message.payload, schema)
                    .map_err(|error| PackageError::Tree {
                        stream: name.to_string(),
                        error,
                    })?;
            messages.push(ObjectMessage {
                message_type: message.message_type,
                first,
            });
        }
        objects.push(Object {
            identifier: raw.identifier,
            info,
            messages,
        });
    }
    Ok(Stream {
        name: name.to_string(),
        tree,
        objects,
    })
}

/// Re-encodes a stream's objects into one decompressed stream. Each
/// `MessageInfo.length` is set from its encoded message.
pub fn encode_stream(stream: &Stream, out: &mut Vec<u8>) -> Result<(), PackageError> {
    let mut scratch = ObjectScratch::default();
    for object in &stream.objects {
        encode_object(stream, object, &mut scratch, out)?;
    }
    Ok(())
}

/// Buffers `encode_object` reuses from one object to the next.
#[derive(Default)]
struct ObjectScratch {
    info_bytes: Vec<u8>,
    lengths: Vec<usize>,
}

/// Appends one object of a stream, its `ArchiveInfo` then its messages.
fn encode_object(
    stream: &Stream,
    object: &Object,
    scratch: &mut ObjectScratch,
    out: &mut Vec<u8>,
) -> Result<(), PackageError> {
    let tree = &stream.tree;
    let lengths = &mut scratch.lengths;
    lengths.clear();
    for message in &object.messages {
        lengths.push(tree.size(message.first));
    }
    // The info chain is encoded with lengths patched in place, on a
    // copy of the tree's entries only where needed: patch, encode, restore.
    let info_bytes = &mut scratch.info_bytes;
    info_bytes.clear();
    encode_info(tree, object.info, lengths, info_bytes).map_err(|what| {
        PackageError::Malformed {
            stream: stream.name.clone(),
            what,
        }
    })?;
    // Sized once: a text storage is megabytes, and growing to it by
    // doubling would hold up to twice that.
    out.reserve(10 + info_bytes.len() + lengths.iter().sum::<usize>());
    write_varint(out, info_bytes.len() as u64);
    out.extend_from_slice(info_bytes);
    for message in &object.messages {
        tree.encode(message.first, out)
            .map_err(|error| PackageError::Tree {
                stream: stream.name.clone(),
                error,
            })?;
    }
    Ok(())
}

/// Encodes an `ArchiveInfo` chain with each `MessageInfo.length` (field 3
/// of field 2) replaced by the given lengths. The tree is not modified:
/// the length entries are overridden while encoding.
fn encode_info(
    tree: &Tree,
    info: u32,
    lengths: &[usize],
    out: &mut Vec<u8>,
) -> Result<(), &'static str> {
    // Copy the chain into a scratch tree with patched lengths. The info
    // chains are a few entries long, so this is cheap.
    let mut scratch = Tree::new(tree.schema);
    let mut message_index = 0usize;
    let first = copy_chain(
        tree,
        info,
        &mut scratch,
        &mut |scratch_entry: &mut crate::io::protobuf::tree::Entry, depth| {
            // A MessageInfo's length is field 3 at depth 1.
            if depth == 1 && scratch_entry.number == 3 {
                let length = *lengths
                    .get(message_index)
                    .ok_or("more message headers than messages")?;
                message_index += 1;
                scratch_entry.value = Node::Uint(length as u64);
            }
            Ok(())
        },
    )?;
    if message_index != lengths.len() {
        return Err("fewer message headers than messages");
    }
    scratch
        .encode(first, out)
        .map_err(|_| "header re-encoding failed")
}

/// Copies a chain (and its nested chains) into `scratch`, letting `patch`
/// adjust each copied entry. Returns the first entry in `scratch`.
fn copy_chain(
    tree: &Tree,
    first: u32,
    scratch: &mut Tree,
    patch: &mut dyn FnMut(&mut crate::io::protobuf::tree::Entry, usize) -> Result<(), &'static str>,
) -> Result<u32, &'static str> {
    copy_chain_at(tree, first, scratch, patch, 0)
}

fn copy_chain_at(
    tree: &Tree,
    first: u32,
    scratch: &mut Tree,
    patch: &mut dyn FnMut(&mut crate::io::protobuf::tree::Entry, usize) -> Result<(), &'static str>,
    depth: usize,
) -> Result<u32, &'static str> {
    let mut chain = crate::io::protobuf::tree::Chain::new();
    for (_, entry) in tree.chain(first) {
        let mut copy = *entry;
        copy.next = NONE;
        copy.value = match entry.value {
            Node::Str(span) => Node::Str(
                scratch
                    .push_bytes(tree.bytes(span))
                    .map_err(|_| "too large")?,
            ),
            Node::Bytes(span) => Node::Bytes(
                scratch
                    .push_bytes(tree.bytes(span))
                    .map_err(|_| "too large")?,
            ),
            Node::RawBytes(span) => Node::RawBytes(
                scratch
                    .push_bytes(tree.bytes(span))
                    .map_err(|_| "too large")?,
            ),
            Node::Deferred(span) => Node::Deferred(
                scratch
                    .push_bytes(tree.bytes(span))
                    .map_err(|_| "too large")?,
            ),
            Node::Message(nested) => {
                Node::Message(copy_chain_at(tree, nested, scratch, patch, depth + 1)?)
            }
            other => other,
        };
        patch(&mut copy, depth)?;
        scratch.push(&mut chain, copy).map_err(|_| "too large")?;
    }
    Ok(chain.first)
}

/// Name of an object's first message type.
pub fn object_type_name(object: &Object) -> &'static str {
    object
        .messages
        .first()
        .and_then(|message| type_name(message.message_type))
        .unwrap_or("?")
}
