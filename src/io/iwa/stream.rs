//! IWA chunk framing and the object stream.
//!
//! Framing: a sequence of chunks, each `[type: 1 byte][length: 3 bytes
//! little-endian]` followed by that many bytes of one raw Snappy block.
//! Only type 0 is known. The decompressed chunks concatenate into the
//! protobuf stream.
//!
//! Stream: repeated `[varint length][ArchiveInfo][payload messages]`.
//! `ArchiveInfo` (TSP): field 1 `identifier` (uint64), field 2
//! `message_infos` (repeated `MessageInfo`). `MessageInfo`: field 1
//! `type` (uint32), field 2 `version` (packed uint32), field 3 `length`
//! (uint32), field 4 `field_infos`, field 5 `object_references` (packed
//! uint64), field 6 `data_references` (packed uint64). These are the two
//! schemas the container needs; everything else is a payload.

use std::fmt;

use crate::io::protobuf::reader::packed_varints;
use crate::io::protobuf::{FieldReader, ProtobufError, Value, read_varint};
use crate::io::snappy::{SnappyError, decompress_block};

/// Largest decompressed stream accepted, to bound memory on corrupt input.
const STREAM_LIMIT: usize = 1 << 31;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IwaError {
    Truncated {
        offset: usize,
        what: &'static str,
    },
    UnknownChunkType {
        offset: usize,
        kind: u8,
    },
    Snappy {
        offset: usize,
        error: SnappyError,
    },
    Protobuf {
        object_offset: usize,
        error: ProtobufError,
    },
    /// An object whose payloads extend past the stream.
    ObjectTruncated {
        object_offset: usize,
    },
}

impl fmt::Display for IwaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IwaError::Truncated { offset, what } => {
                write!(formatter, "IWA {what} truncated at byte {offset}")
            }
            IwaError::UnknownChunkType { offset, kind } => write!(
                formatter,
                "IWA chunk of unknown type {kind} at byte {offset}"
            ),
            IwaError::Snappy { offset, error } => {
                write!(formatter, "IWA chunk at byte {offset}: {error}")
            }
            IwaError::Protobuf {
                object_offset,
                error,
            } => write!(formatter, "IWA object at byte {object_offset}: {error}"),
            IwaError::ObjectTruncated { object_offset } => write!(
                formatter,
                "IWA object at byte {object_offset} runs past the stream"
            ),
        }
    }
}

impl std::error::Error for IwaError {}

/// Decompresses every chunk of an `.iwa` file into one protobuf stream.
pub fn decompress_stream(iwa: &[u8]) -> Result<Vec<u8>, IwaError> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    while offset < iwa.len() {
        let header = iwa.get(offset..offset + 4).ok_or(IwaError::Truncated {
            offset,
            what: "chunk header",
        })?;
        if header[0] != 0 {
            return Err(IwaError::UnknownChunkType {
                offset,
                kind: header[0],
            });
        }
        let length =
            usize::from(header[1]) | (usize::from(header[2]) << 8) | (usize::from(header[3]) << 16);
        let block = iwa
            .get(offset + 4..offset + 4 + length)
            .ok_or(IwaError::Truncated {
                offset,
                what: "chunk",
            })?;
        let remaining_limit = STREAM_LIMIT.saturating_sub(out.len());
        decompress_block(block, &mut out, remaining_limit)
            .map_err(|error| IwaError::Snappy { offset, error })?;
        offset += 4 + length;
    }
    Ok(out)
}

/// One message of an object: its type id and its bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageInfo<'a> {
    pub message_type: u32,
    pub versions: Vec<u32>,
    pub object_references: Vec<u64>,
    pub data_references: Vec<u64>,
    /// The serialized message, to be read with a schema.
    pub payload: &'a [u8],
}

/// One object of the stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IwaObject<'a> {
    pub identifier: u64,
    pub messages: Vec<MessageInfo<'a>>,
    /// Byte offset of the object in the decompressed stream.
    pub offset: usize,
}

impl IwaObject<'_> {
    /// Type of the first message, which is the object's own type.
    pub fn message_type(&self) -> Option<u32> {
        self.messages.first().map(|message| message.message_type)
    }
}

/// Splits a decompressed stream into its objects.
pub fn parse_objects(stream: &[u8]) -> Result<Vec<IwaObject<'_>>, IwaError> {
    let mut objects = Vec::new();
    let mut offset = 0usize;
    while offset < stream.len() {
        let object_offset = offset;
        let (info_length, prefix) =
            read_varint(stream, offset).map_err(|error| IwaError::Protobuf {
                object_offset,
                error,
            })?;
        offset += prefix;
        let info_end = offset.saturating_add(info_length as usize);
        let info = stream
            .get(offset..info_end)
            .ok_or(IwaError::ObjectTruncated { object_offset })?;
        offset = info_end;
        let mut identifier = 0u64;
        let mut messages = Vec::new();
        let mut lengths = Vec::new();
        for field in FieldReader::new(info) {
            let field = field.map_err(|error| IwaError::Protobuf {
                object_offset,
                error,
            })?;
            match (field.number, field.value) {
                (1, Value::Varint(value)) => identifier = value,
                (2, Value::Bytes(bytes)) => {
                    let (message, length) =
                        parse_message_info(bytes).map_err(|error| IwaError::Protobuf {
                            object_offset,
                            error,
                        })?;
                    messages.push(message);
                    lengths.push(length);
                }
                _ => {}
            }
        }
        for (message, length) in messages.iter_mut().zip(lengths) {
            let end = offset.saturating_add(length);
            message.payload = stream
                .get(offset..end)
                .ok_or(IwaError::ObjectTruncated { object_offset })?;
            offset = end;
        }
        objects.push(IwaObject {
            identifier,
            messages,
            offset: object_offset,
        });
    }
    Ok(objects)
}

/// Reads a `MessageInfo`, returning it with an empty payload and the
/// declared payload length; `parse_objects` attaches the data.
fn parse_message_info(bytes: &[u8]) -> Result<(MessageInfo<'static>, usize), ProtobufError> {
    let mut message = MessageInfo {
        message_type: 0,
        versions: Vec::new(),
        object_references: Vec::new(),
        data_references: Vec::new(),
        payload: &[],
    };
    let mut length = 0usize;
    for field in FieldReader::new(bytes) {
        let field = field?;
        match (field.number, field.value) {
            (1, Value::Varint(value)) => message.message_type = value as u32,
            (2, Value::Varint(value)) => message.versions.push(value as u32),
            (2, Value::Bytes(packed)) => message.versions.extend(
                packed_varints(packed)?
                    .into_iter()
                    .map(|value| value as u32),
            ),
            (3, Value::Varint(value)) => length = value as usize,
            (5, Value::Varint(value)) => message.object_references.push(value),
            (5, Value::Bytes(packed)) => message.object_references.extend(packed_varints(packed)?),
            (6, Value::Varint(value)) => message.data_references.push(value),
            (6, Value::Bytes(packed)) => message.data_references.extend(packed_varints(packed)?),
            _ => {}
        }
    }
    Ok((message, length))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_and_objects_decode() {
        // One object: ArchiveInfo { identifier 7, message_infos [{type 10000, length 3}] } then payload 08 2A 00.
        let info = [0x08, 0x07, 0x12, 0x05, 0x08, 0x90, 0x4E, 0x18, 0x03];
        let mut stream = vec![info.len() as u8];
        stream.extend_from_slice(&info);
        stream.extend_from_slice(&[0x08, 0x2A, 0x00]);
        // Wrap in a Snappy literal block and an IWA chunk.
        let mut block = vec![stream.len() as u8, ((stream.len() as u8 - 1) << 2)];
        block.extend_from_slice(&stream);
        let mut iwa = vec![0, block.len() as u8, 0, 0];
        iwa.extend_from_slice(&block);
        let decompressed = decompress_stream(&iwa).unwrap();
        assert_eq!(decompressed, stream);
        let objects = parse_objects(&decompressed).unwrap();
        assert_eq!(objects.len(), 1);
        assert_eq!(objects[0].identifier, 7);
        assert_eq!(objects[0].message_type(), Some(10000));
        assert_eq!(objects[0].messages[0].payload, &[0x08, 0x2A, 0x00]);
    }

    #[test]
    fn bad_chunk_types_and_truncation_are_errors() {
        assert_eq!(
            decompress_stream(&[1, 0, 0, 0]),
            Err(IwaError::UnknownChunkType { offset: 0, kind: 1 })
        );
        assert!(matches!(
            decompress_stream(&[0, 5, 0, 0, 1]),
            Err(IwaError::Truncated { .. })
        ));
        assert!(matches!(
            parse_objects(&[0x09, 0x08]),
            Err(IwaError::ObjectTruncated { .. })
        ));
    }
}
