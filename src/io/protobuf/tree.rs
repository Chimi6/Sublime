//! Schema-driven decoding of a message into a tree of named values, and
//! encoding back. Fields the schema does not name are kept with their wire
//! type, so a tree always re-encodes to the bytes it came from.

use std::fmt;

use super::reader::{FieldReader, ProtobufError, Value, packed_varints, read_varint};
use super::schema::{Field, Kind, Message};

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Int(i64),
    Uint(u64),
    Bool(bool),
    Fixed32(u32),
    Fixed64(u64),
    Float(f32),
    Double(f64),
    Str(String),
    Bytes(Vec<u8>),
    Reference(u64),
    Message(Fields),
    /// Values of fields the schema does not describe, by wire type.
    RawVarint(u64),
    RawFixed32(u32),
    RawFixed64(u64),
    RawBytes(Vec<u8>),
}

/// One field occurrence, in wire order. A repeated field is several entries.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub number: u32,
    /// The schema field, when known.
    pub field: Option<&'static Field>,
    pub value: Node,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Fields {
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeError {
    Wire(ProtobufError),
    /// A value that the schema's kind cannot represent (bad UTF-8 in a
    /// string, a message that does not parse) is kept raw instead, so this
    /// only arises when re-encoding a tree that was built by hand.
    KindMismatch {
        number: u32,
    },
}

impl fmt::Display for TreeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TreeError::Wire(error) => write!(formatter, "{error}"),
            TreeError::KindMismatch { number } => {
                write!(formatter, "field {number} holds a value of the wrong kind")
            }
        }
    }
}

impl std::error::Error for TreeError {}

impl From<ProtobufError> for TreeError {
    fn from(error: ProtobufError) -> Self {
        TreeError::Wire(error)
    }
}

/// Decodes `bytes` as `message`; `table` resolves nested message kinds.
pub fn decode(
    bytes: &[u8],
    message: Option<&'static Message>,
    table: &'static [Message],
) -> Result<Fields, TreeError> {
    let mut entries = Vec::new();
    for field in FieldReader::new(bytes) {
        let field = field?;
        let schema = message.and_then(|message| message.field(field.number));
        match schema {
            Some(schema) => decode_known(field.number, schema, field.value, table, &mut entries),
            None => entries.push(Entry {
                number: field.number,
                field: None,
                value: raw(field.value),
            }),
        }
    }
    Ok(Fields { entries })
}

fn raw(value: Value<'_>) -> Node {
    match value {
        Value::Varint(value) => Node::RawVarint(value),
        Value::Fixed32(value) => Node::RawFixed32(value),
        Value::Fixed64(value) => Node::RawFixed64(value),
        Value::Bytes(bytes) => Node::RawBytes(bytes.to_vec()),
        // Groups are not used by iWork; keep them as their encoded bytes.
        Value::Group(bytes) => Node::RawBytes(bytes.to_vec()),
    }
}

fn decode_known(
    number: u32,
    schema: &'static Field,
    value: Value<'_>,
    table: &'static [Message],
    entries: &mut Vec<Entry>,
) {
    let push = |entries: &mut Vec<Entry>, node: Node| {
        entries.push(Entry {
            number,
            field: Some(schema),
            value: node,
        });
    };
    match (schema.kind, value) {
        (Kind::Int | Kind::Enum, Value::Varint(value)) => push(entries, Node::Int(value as i64)),
        (Kind::Uint, Value::Varint(value)) => push(entries, Node::Uint(value)),
        (Kind::Sint, Value::Varint(value)) => push(entries, Node::Int(zigzag_decode(value))),
        (Kind::Bool, Value::Varint(value)) => push(entries, Node::Bool(value != 0)),
        (Kind::Fixed32, Value::Fixed32(value)) => push(entries, Node::Fixed32(value)),
        (Kind::Fixed64, Value::Fixed64(value)) => push(entries, Node::Fixed64(value)),
        (Kind::Float, Value::Fixed32(value)) => push(entries, Node::Float(f32::from_bits(value))),
        (Kind::Double, Value::Fixed64(value)) => push(entries, Node::Double(f64::from_bits(value))),
        (Kind::String, Value::Bytes(bytes)) => match std::str::from_utf8(bytes) {
            Ok(text) => push(entries, Node::Str(text.to_string())),
            Err(_) => push(entries, Node::RawBytes(bytes.to_vec())),
        },
        (Kind::Bytes, Value::Bytes(bytes)) => push(entries, Node::Bytes(bytes.to_vec())),
        (Kind::Reference, Value::Bytes(bytes)) => match reference_identifier(bytes) {
            Some(identifier) => push(entries, Node::Reference(identifier)),
            None => push(entries, Node::RawBytes(bytes.to_vec())),
        },
        (Kind::Message(index), Value::Bytes(bytes)) => {
            let nested = table.get(usize::from(index));
            match decode(bytes, nested, table) {
                Ok(fields) => push(entries, Node::Message(fields)),
                Err(_) => push(entries, Node::RawBytes(bytes.to_vec())),
            }
        }
        // A packed repeated scalar arrives as one length-delimited value.
        (Kind::Int | Kind::Uint | Kind::Sint | Kind::Bool | Kind::Enum, Value::Bytes(bytes))
            if schema.repeated =>
        {
            match packed_varints(bytes) {
                Ok(values) => {
                    for value in values {
                        let node = match schema.kind {
                            Kind::Uint => Node::Uint(value),
                            Kind::Sint => Node::Int(zigzag_decode(value)),
                            Kind::Bool => Node::Bool(value != 0),
                            _ => Node::Int(value as i64),
                        };
                        push(entries, node);
                    }
                }
                Err(_) => push(entries, Node::RawBytes(bytes.to_vec())),
            }
        }
        (Kind::Float | Kind::Fixed32, Value::Bytes(bytes))
            if schema.repeated && bytes.len() % 4 == 0 =>
        {
            for chunk in bytes.chunks(4) {
                let bits = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                let node = if schema.kind == Kind::Float {
                    Node::Float(f32::from_bits(bits))
                } else {
                    Node::Fixed32(bits)
                };
                push(entries, node);
            }
        }
        (Kind::Double | Kind::Fixed64, Value::Bytes(bytes))
            if schema.repeated && bytes.len() % 8 == 0 =>
        {
            for chunk in bytes.chunks(8) {
                let mut array = [0u8; 8];
                array.copy_from_slice(chunk);
                let bits = u64::from_le_bytes(array);
                let node = if schema.kind == Kind::Double {
                    Node::Double(f64::from_bits(bits))
                } else {
                    Node::Fixed64(bits)
                };
                push(entries, node);
            }
        }
        (_, value) => entries.push(Entry {
            number,
            field: None,
            value: raw(value),
        }),
    }
}

/// `TSP.Reference` is `{ required uint64 identifier = 1 }`; anything else
/// in it would be lost, so only that exact shape becomes a reference.
fn reference_identifier(bytes: &[u8]) -> Option<u64> {
    if bytes.first() != Some(&0x08) {
        return None;
    }
    let (identifier, length) = read_varint(bytes, 1).ok()?;
    if 1 + length != bytes.len() {
        return None;
    }
    Some(identifier)
}

fn zigzag_decode(value: u64) -> i64 {
    ((value >> 1) as i64) ^ -((value & 1) as i64)
}

fn zigzag_encode(value: i64) -> u64 {
    ((value << 1) ^ (value >> 63)) as u64
}

// ----- encoding -----

pub fn write_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value as u8) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn write_tag(out: &mut Vec<u8>, number: u32, wire_type: u8) {
    write_varint(out, (u64::from(number) << 3) | u64::from(wire_type));
}

fn write_length_delimited(out: &mut Vec<u8>, number: u32, bytes: &[u8]) {
    write_tag(out, number, 2);
    write_varint(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

/// Encodes a tree. Consecutive entries of one packed field are written as
/// one packed value, as they were read.
pub fn encode(fields: &Fields, out: &mut Vec<u8>) -> Result<(), TreeError> {
    let entries = &fields.entries;
    let mut index = 0usize;
    while index < entries.len() {
        let entry = &entries[index];
        let packed = entry
            .field
            .is_some_and(|field| field.packed && field.repeated);
        if packed {
            let mut run_end = index;
            while run_end < entries.len() && entries[run_end].number == entry.number {
                run_end += 1;
            }
            let mut packed_bytes = Vec::new();
            for packed_entry in &entries[index..run_end] {
                encode_scalar_payload(packed_entry, &mut packed_bytes)?;
            }
            write_length_delimited(out, entry.number, &packed_bytes);
            index = run_end;
            continue;
        }
        encode_entry(entry, out)?;
        index += 1;
    }
    Ok(())
}

fn encode_entry(entry: &Entry, out: &mut Vec<u8>) -> Result<(), TreeError> {
    let number = entry.number;
    let kind = entry.field.map(|field| field.kind);
    match (&entry.value, kind) {
        (Node::Int(value), Some(Kind::Sint)) => {
            write_tag(out, number, 0);
            write_varint(out, zigzag_encode(*value));
        }
        (Node::Int(value), _) => {
            write_tag(out, number, 0);
            write_varint(out, *value as u64);
        }
        (Node::Uint(value), _) | (Node::RawVarint(value), _) => {
            write_tag(out, number, 0);
            write_varint(out, *value);
        }
        (Node::Bool(value), _) => {
            write_tag(out, number, 0);
            write_varint(out, u64::from(*value));
        }
        (Node::Fixed32(value), _) | (Node::RawFixed32(value), _) => {
            write_tag(out, number, 5);
            out.extend_from_slice(&value.to_le_bytes());
        }
        (Node::Float(value), _) => {
            write_tag(out, number, 5);
            out.extend_from_slice(&value.to_bits().to_le_bytes());
        }
        (Node::Fixed64(value), _) | (Node::RawFixed64(value), _) => {
            write_tag(out, number, 1);
            out.extend_from_slice(&value.to_le_bytes());
        }
        (Node::Double(value), _) => {
            write_tag(out, number, 1);
            out.extend_from_slice(&value.to_bits().to_le_bytes());
        }
        (Node::Str(text), _) => write_length_delimited(out, number, text.as_bytes()),
        (Node::Bytes(bytes), _) | (Node::RawBytes(bytes), _) => {
            write_length_delimited(out, number, bytes)
        }
        (Node::Reference(identifier), _) => {
            let mut nested = Vec::new();
            write_tag(&mut nested, 1, 0);
            write_varint(&mut nested, *identifier);
            write_length_delimited(out, number, &nested);
        }
        (Node::Message(fields), _) => {
            let mut nested = Vec::new();
            encode(fields, &mut nested)?;
            write_length_delimited(out, number, &nested);
        }
    }
    Ok(())
}

/// One element of a packed field, without a tag.
fn encode_scalar_payload(entry: &Entry, out: &mut Vec<u8>) -> Result<(), TreeError> {
    let kind = entry.field.map(|field| field.kind);
    match (&entry.value, kind) {
        (Node::Int(value), Some(Kind::Sint)) => write_varint(out, zigzag_encode(*value)),
        (Node::Int(value), _) => write_varint(out, *value as u64),
        (Node::Uint(value), _) => write_varint(out, *value),
        (Node::Bool(value), _) => write_varint(out, u64::from(*value)),
        (Node::Fixed32(value), _) => out.extend_from_slice(&value.to_le_bytes()),
        (Node::Float(value), _) => out.extend_from_slice(&value.to_bits().to_le_bytes()),
        (Node::Fixed64(value), _) => out.extend_from_slice(&value.to_le_bytes()),
        (Node::Double(value), _) => out.extend_from_slice(&value.to_bits().to_le_bytes()),
        _ => {
            return Err(TreeError::KindMismatch {
                number: entry.number,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    static TABLE: [Message; 2] = [
        Message {
            name: "Inner",
            fields: &[Field {
                number: 1,
                name: "count",
                kind: Kind::Uint,
                repeated: true,
                packed: true,
            }],
        },
        Message {
            name: "Outer",
            fields: &[
                Field {
                    number: 1,
                    name: "name",
                    kind: Kind::String,
                    repeated: false,
                    packed: false,
                },
                Field {
                    number: 2,
                    name: "inner",
                    kind: Kind::Message(0),
                    repeated: false,
                    packed: false,
                },
                Field {
                    number: 3,
                    name: "target",
                    kind: Kind::Reference,
                    repeated: false,
                    packed: false,
                },
                Field {
                    number: 4,
                    name: "delta",
                    kind: Kind::Sint,
                    repeated: false,
                    packed: false,
                },
                Field {
                    number: 5,
                    name: "ratio",
                    kind: Kind::Float,
                    repeated: false,
                    packed: false,
                },
            ],
        },
    ];

    fn sample() -> Vec<u8> {
        let mut bytes = vec![0x0A, 0x02, b'h', b'i'];
        bytes.extend_from_slice(&[0x12, 0x04, 0x0A, 0x02, 0x03, 0x80]); // inner { count packed [3, 128?] } -> 0x03, 0x80 0x01
        bytes[5] = 0x05;
        bytes.truncate(4);
        bytes.extend_from_slice(&[0x12, 0x05, 0x0A, 0x03, 0x03, 0x80, 0x01]);
        bytes.extend_from_slice(&[0x1A, 0x02, 0x08, 0x2A]); // target ref 42
        bytes.extend_from_slice(&[0x20, 0x03]); // delta sint -2
        bytes.extend_from_slice(&[0x2D, 0x00, 0x00, 0x80, 0x3F]); // ratio 1.0
        bytes.extend_from_slice(&[0x30, 0x07]); // unknown field 6 varint 7
        bytes
    }

    #[test]
    fn decodes_named_and_unknown_fields() {
        let tree = decode(&sample(), Some(&TABLE[1]), &TABLE).unwrap();
        let names: Vec<Option<&str>> = tree
            .entries
            .iter()
            .map(|entry| entry.field.map(|field| field.name))
            .collect();
        assert_eq!(
            names,
            vec![
                Some("name"),
                Some("inner"),
                Some("target"),
                Some("delta"),
                Some("ratio"),
                None
            ]
        );
        assert_eq!(tree.entries[0].value, Node::Str("hi".to_string()));
        assert_eq!(tree.entries[2].value, Node::Reference(42));
        assert_eq!(tree.entries[3].value, Node::Int(-2));
        assert_eq!(tree.entries[4].value, Node::Float(1.0));
        assert_eq!(tree.entries[5].value, Node::RawVarint(7));
        match &tree.entries[1].value {
            Node::Message(inner) => {
                let counts: Vec<&Node> = inner.entries.iter().map(|entry| &entry.value).collect();
                assert_eq!(counts, vec![&Node::Uint(3), &Node::Uint(128)]);
            }
            other => panic!("expected a message, got {other:?}"),
        }
    }

    #[test]
    fn re_encodes_to_the_same_bytes() {
        let bytes = sample();
        let tree = decode(&bytes, Some(&TABLE[1]), &TABLE).unwrap();
        let mut out = Vec::new();
        encode(&tree, &mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn schemaless_decode_keeps_everything_raw() {
        let bytes = sample();
        let tree = decode(&bytes, None, &TABLE).unwrap();
        assert!(tree.entries.iter().all(|entry| entry.field.is_none()));
        let mut out = Vec::new();
        encode(&tree, &mut out).unwrap();
        assert_eq!(out, bytes);
    }
}
