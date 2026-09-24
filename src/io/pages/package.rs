//! A Pages package as a whole: every ZIP entry, with the IWA streams
//! decoded into schema-named object trees and everything else kept as
//! bytes. Reading and writing are inverse: the objects re-encode to the
//! exact bytes of the streams they came from (the ZIP and Snappy layers
//! are rebuilt, so those bytes differ, their contents do not).

use std::collections::HashMap;
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
}

/// Object types the document walk starts from.
const ROOT_TYPES: [u32; 3] = [10000, 11006, 4000];
/// Object types whose references are not followed: hubs that list every
/// style, theme preset, or view state in the package.
const HUB_TYPES: [u32; 7] = [401, 10001, 210, 10133, 10131, 10147, 213];

impl Package {
    pub fn read(bytes: &[u8]) -> Result<Package, PackageError> {
        Package::read_scope(bytes, Scope::Everything)
    }

    pub fn read_scope(bytes: &[u8], scope: Scope) -> Result<Package, PackageError> {
        let archive = ZipArchive::parse(bytes)?;
        // Every entry is read first; the reachable set needs all streams'
        // object headers before any stream is decoded.
        let mut raw: Vec<(String, Vec<u8>, bool)> = Vec::new();
        for entry in archive.entries() {
            if entry.is_directory() {
                continue;
            }
            let mut data = Vec::new();
            archive.read(entry, &mut data)?;
            let is_stream = entry.name.ends_with(".iwa");
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
        let reachable = match scope {
            Scope::Everything => None,
            Scope::Document => Some(reachable_objects(&parsed)),
        };
        let mut entries = Vec::with_capacity(raw.len());
        for ((name, bytes, _), objects) in raw.iter().zip(parsed) {
            match objects {
                Some(objects) => {
                    let keep = |identifier: u64| {
                        reachable
                            .as_ref()
                            .is_none_or(|set| set.contains_key(&identifier))
                    };
                    entries.push(Entry::Stream(decode_objects(name, bytes, objects, keep)?));
                }
                None => entries.push(Entry::File {
                    name: name.clone(),
                    bytes: bytes.clone(),
                }),
            }
        }
        Ok(Package { entries })
    }

    /// Writes the package as a ZIP with stored entries, as Pages does;
    /// streams are re-encoded and Snappy-compressed in 64 KiB chunks.
    pub fn write<W: std::io::Write>(&self, sink: W) -> Result<W, PackageError> {
        let mut zip = ZipWriter::new(sink);
        let mut encoded = Vec::new();
        let mut chunked = Vec::new();
        for entry in &self.entries {
            match entry {
                Entry::File { name, bytes } => zip.add(name, bytes)?,
                Entry::Stream(stream) => {
                    encoded.clear();
                    encode_stream(stream, &mut encoded)?;
                    chunked.clear();
                    for chunk in encoded.chunks(CHUNK_SIZE) {
                        let mut block = Vec::with_capacity(chunk.len() + 8);
                        compress_block(chunk, &mut block);
                        chunked.push(0);
                        chunked.extend_from_slice(&(block.len() as u32).to_le_bytes()[..3]);
                        chunked.extend_from_slice(&block);
                    }
                    zip.add(&stream.name, &chunked)?;
                }
            }
        }
        Ok(zip.finish()?)
    }
}

/// The identifiers a document reader reaches (see `Scope::Document`).
/// The maps use the `u64 -> usize` shape the rest of the crate already
/// instantiates, so they add no code to the binary.
fn reachable_objects(streams: &[Option<Vec<IwaObject<'_>>>]) -> HashMap<u64, usize> {
    let objects: Vec<&IwaObject<'_>> = streams.iter().flatten().flatten().collect();
    let mut index: HashMap<u64, usize> = HashMap::with_capacity(objects.len());
    let mut pending: Vec<u64> = Vec::new();
    for (position, object) in objects.iter().enumerate() {
        index.insert(object.identifier, position);
        if ROOT_TYPES.contains(&object.message_type().unwrap_or(0)) {
            pending.push(object.identifier);
        }
    }
    let mut reachable: HashMap<u64, usize> = HashMap::new();
    while let Some(identifier) = pending.pop() {
        let Some(position) = index.get(&identifier) else {
            continue;
        };
        if reachable.insert(identifier, *position).is_some() {
            continue;
        }
        let object = objects[*position];
        if HUB_TYPES.contains(&object.message_type().unwrap_or(0)) {
            continue;
        }
        for message in &object.messages {
            pending.extend(message.object_references.iter().copied());
        }
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
    decode_objects(name, &bytes, raw_objects, |_| true)
}

/// Decodes the objects `keep` selects into a stream; the others keep
/// their identifier with no messages, so lookups by identifier still
/// resolve and readers see them as absent.
fn decode_objects(
    name: &str,
    bytes: &[u8],
    raw_objects: Vec<IwaObject<'_>>,
    keep: impl Fn(u64) -> bool,
) -> Result<Stream, PackageError> {
    let info_schema = SCHEMA.message("TSP.ArchiveInfo");
    let mut tree = Tree::new(&SCHEMA);
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
    let tree = &stream.tree;
    let tree_error = |error| PackageError::Tree {
        stream: stream.name.clone(),
        error,
    };
    let mut info_bytes = Vec::new();
    let mut lengths = Vec::new();
    for object in &stream.objects {
        lengths.clear();
        for message in &object.messages {
            lengths.push(tree.size(message.first));
        }
        // The info chain is encoded with lengths patched in place, on a
        // copy of the tree's entries only where needed: patch, encode, restore.
        info_bytes.clear();
        encode_info(tree, object.info, &lengths, &mut info_bytes).map_err(|what| {
            PackageError::Malformed {
                stream: stream.name.clone(),
                what,
            }
        })?;
        write_varint(out, info_bytes.len() as u64);
        out.extend_from_slice(&info_bytes);
        for message in &object.messages {
            tree.encode(message.first, out).map_err(tree_error)?;
        }
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
