//! A Pages package as a whole: every ZIP entry, with the IWA streams
//! decoded into schema-named object trees and everything else kept as
//! bytes. Reading and writing are inverse: the objects re-encode to the
//! exact bytes of the streams they came from (the ZIP and Snappy layers
//! are rebuilt, so those bytes differ, their contents do not).

use std::fmt;

use super::schema::SCHEMA;
use super::{message_schema, type_name};
use crate::io::iwa::{IwaError, decompress_stream, parse_objects};
use crate::io::protobuf::tree::{self, Fields, Node, TreeError, write_varint};
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

/// One message of an object: its registry type and its decoded fields.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectMessage {
    pub message_type: u32,
    pub fields: Fields,
}

/// One object: its `ArchiveInfo` header as a tree, and its messages.
#[derive(Debug, Clone, PartialEq)]
pub struct Object {
    pub identifier: u64,
    pub info: Fields,
    pub messages: Vec<ObjectMessage>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    Stream { name: String, objects: Vec<Object> },
    File { name: String, bytes: Vec<u8> },
}

impl Entry {
    pub fn name(&self) -> &str {
        match self {
            Entry::Stream { name, .. } | Entry::File { name, .. } => name,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
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
                let objects = decode_stream(&entry.name, &data)?;
                entries.push(Entry::Stream {
                    name: entry.name.clone(),
                    objects,
                });
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
        let mut stream = Vec::new();
        let mut chunked = Vec::new();
        for entry in &self.entries {
            match entry {
                Entry::File { name, bytes } => zip.add(name, bytes)?,
                Entry::Stream { name, objects } => {
                    stream.clear();
                    encode_stream(name, objects, &mut stream)?;
                    chunked.clear();
                    for chunk in stream.chunks(CHUNK_SIZE) {
                        let mut block = Vec::with_capacity(chunk.len() + 8);
                        encode_literal_block(chunk, &mut block);
                        chunked.push(0);
                        chunked.extend_from_slice(&(block.len() as u32).to_le_bytes()[..3]);
                        chunked.extend_from_slice(&block);
                    }
                    if stream.is_empty() {
                        chunked.clear();
                    }
                    zip.add(name, &chunked)?;
                }
            }
        }
        Ok(zip.finish()?)
    }
}

/// Decodes one `.iwa` entry's objects.
pub fn decode_stream(name: &str, compressed: &[u8]) -> Result<Vec<Object>, PackageError> {
    let stream = decompress_stream(compressed).map_err(|error| PackageError::Iwa {
        stream: name.to_string(),
        error,
    })?;
    let raw_objects = parse_objects(&stream).map_err(|error| PackageError::Iwa {
        stream: name.to_string(),
        error,
    })?;
    let info_schema = SCHEMA.message("TSP.ArchiveInfo");
    let mut objects = Vec::with_capacity(raw_objects.len());
    for raw in raw_objects {
        let info =
            tree::decode(raw.info, info_schema, &SCHEMA).map_err(|error| PackageError::Tree {
                stream: name.to_string(),
                error,
            })?;
        let mut messages = Vec::with_capacity(raw.messages.len());
        for message in &raw.messages {
            let schema = message_schema(message.message_type);
            let fields = tree::decode(message.payload, schema, &SCHEMA).map_err(|error| {
                PackageError::Tree {
                    stream: name.to_string(),
                    error,
                }
            })?;
            messages.push(ObjectMessage {
                message_type: message.message_type,
                fields,
            });
        }
        objects.push(Object {
            identifier: raw.identifier,
            info,
            messages,
        });
    }
    Ok(objects)
}

/// Re-encodes objects into one decompressed stream. The `length` of each
/// `MessageInfo` is set from the encoded payload.
pub fn encode_stream(
    name: &str,
    objects: &[Object],
    out: &mut Vec<u8>,
) -> Result<(), PackageError> {
    let mut payloads: Vec<Vec<u8>> = Vec::new();
    let mut info_bytes = Vec::new();
    for object in objects {
        payloads.clear();
        for message in &object.messages {
            let mut payload = Vec::new();
            tree::encode(&message.fields, &mut payload).map_err(|error| PackageError::Tree {
                stream: name.to_string(),
                error,
            })?;
            payloads.push(payload);
        }
        let mut info = object.info.clone();
        set_lengths(&mut info, &payloads).map_err(|what| PackageError::Malformed {
            stream: name.to_string(),
            what,
        })?;
        info_bytes.clear();
        tree::encode(&info, &mut info_bytes).map_err(|error| PackageError::Tree {
            stream: name.to_string(),
            error,
        })?;
        write_varint(out, info_bytes.len() as u64);
        out.extend_from_slice(&info_bytes);
        for payload in &payloads {
            out.extend_from_slice(payload);
        }
    }
    Ok(())
}

/// `ArchiveInfo.message_infos` is field 2; `MessageInfo.length` is field 3.
fn set_lengths(info: &mut Fields, payloads: &[Vec<u8>]) -> Result<(), &'static str> {
    let mut index = 0usize;
    for entry in &mut info.entries {
        if entry.number != 2 {
            continue;
        }
        let payload = payloads
            .get(index)
            .ok_or("more message headers than messages")?;
        index += 1;
        let message_info = match &mut entry.value {
            Node::Message(fields) => fields,
            _ => return Err("message header is not a message"),
        };
        let length_entry = message_info
            .entries
            .iter_mut()
            .find(|entry| entry.number == 3)
            .ok_or("message header without a length")?;
        length_entry.value = Node::Uint(payload.len() as u64);
    }
    if index != payloads.len() {
        return Err("fewer message headers than messages");
    }
    Ok(())
}

/// Name of an object's first message type.
pub fn object_type_name(object: &Object) -> &'static str {
    object
        .messages
        .first()
        .and_then(|message| type_name(message.message_type))
        .unwrap_or("?")
}
