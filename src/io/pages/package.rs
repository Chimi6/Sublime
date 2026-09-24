//! A Pages package as a whole: every ZIP entry, with the IWA streams
//! decoded into schema-named object trees and everything else kept as
//! bytes. Reading and writing are inverse: the objects re-encode to the
//! exact bytes of the streams they came from (the ZIP and Snappy layers
//! are rebuilt, so those bytes differ, their contents do not).

use std::fmt;

use super::schema::SCHEMA;
use super::{message_schema, type_name};
use crate::io::iwa::{IwaError, decompress_stream, parse_objects};
use crate::io::protobuf::tree::{NONE, Node, Tree, TreeError, write_varint};
use crate::io::snappy::encode_literal_block;
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

impl Package {
    pub fn read(bytes: &[u8]) -> Result<Package, PackageError> {
        let archive = ZipArchive::parse(bytes)?;
        let mut entries = Vec::new();
        let mut data = Vec::new();
        for entry in archive.entries() {
            if entry.is_directory() {
                continue;
            }
            data.clear();
            archive.read(entry, &mut data)?;
            if entry.name.ends_with(".iwa") {
                entries.push(Entry::Stream(decode_stream(&entry.name, &data)?));
            } else {
                entries.push(Entry::File {
                    name: entry.name.clone(),
                    bytes: data.clone(),
                });
            }
        }
        Ok(Package { entries })
    }

    /// Writes the package as a ZIP with stored entries; streams are
    /// re-encoded and chunked as literal Snappy blocks.
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
                        encode_literal_block(chunk, &mut block);
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
    let info_schema = SCHEMA.message("TSP.ArchiveInfo");
    let mut tree = Tree::new(&SCHEMA);
    tree.entries.reserve(bytes.len() / 8);
    tree.text.reserve(bytes.len() / 2);
    let mut objects = Vec::with_capacity(raw_objects.len());
    for raw in raw_objects {
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
