//! Schema-driven decoding of messages into an arena of named values, and
//! encoding back. Fields the schema does not name are kept with their wire
//! type, so a tree always re-encodes to the bytes it came from.
//!
//! A tree holds every entry of a stream in one vector: a message's fields
//! are a chain of entries linked by `next`, and a nested message is an
//! entry whose value points at the first entry of its own chain. Strings
//! and bytes are ranges into one byte buffer. Decoding a stream is a
//! handful of allocations rather than one per field.

use std::fmt;

use super::reader::{FieldReader, ProtobufError, Value, packed_varints, read_varint};
use super::schema::{Field, Kind, MessageRef, Schema};

/// "No entry": the end of a chain, or an empty message.
pub const NONE: u32 = u32::MAX;

/// A range of the tree's byte buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub length: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Node {
    Int(i64),
    Uint(u64),
    Bool(bool),
    Fixed32(u32),
    Fixed64(u64),
    Float(f32),
    Double(f64),
    Str(Span),
    Bytes(Span),
    Reference(u64),
    /// First entry of the nested message's chain, or `NONE` when empty.
    Message(u32),
    /// Values of fields the schema does not describe, by wire type.
    RawVarint(u64),
    RawFixed32(u32),
    RawFixed64(u64),
    RawBytes(Span),
}

const NO_SCHEMA: u16 = u16::MAX;

/// One field occurrence. A repeated field is several entries in a row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Entry {
    pub number: u32,
    /// Schema message index and field slot, or `NO_SCHEMA` when unknown.
    message: u16,
    slot: u16,
    /// Next field of the same message, or `NONE`.
    pub next: u32,
    /// Bit 0: a packed repeated scalar; bit 1: a ZigZag varint. Cached from
    /// the schema so encoding never looks the field up again.
    flags: u8,
    pub value: Node,
}

const FLAG_PACKED: u8 = 1;
const FLAG_SINT: u8 = 2;

impl Entry {
    fn unknown(number: u32, value: Node) -> Entry {
        Entry {
            number,
            message: NO_SCHEMA,
            slot: 0,
            next: NONE,
            flags: 0,
            value,
        }
    }

    fn known(number: u32, message: u16, slot: u16, field: Field, value: Node) -> Entry {
        let mut flags = 0u8;
        if field.packed && field.repeated {
            flags |= FLAG_PACKED;
        }
        if field.kind == Kind::Sint {
            flags |= FLAG_SINT;
        }
        Entry {
            number,
            message,
            slot,
            next: NONE,
            flags,
            value,
        }
    }

    fn is_packed(&self) -> bool {
        self.flags & FLAG_PACKED != 0
    }

    fn is_sint(&self) -> bool {
        self.flags & FLAG_SINT != 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeError {
    Wire(ProtobufError),
    /// A value that the schema's kind cannot represent; only arises when
    /// re-encoding a tree that was built by hand, since decoding keeps
    /// anything it cannot interpret raw.
    KindMismatch {
        number: u32,
    },
    /// More entries or bytes than the 32-bit ranges can address.
    TooLarge,
}

impl fmt::Display for TreeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TreeError::Wire(error) => write!(formatter, "{error}"),
            TreeError::KindMismatch { number } => {
                write!(formatter, "field {number} holds a value of the wrong kind")
            }
            TreeError::TooLarge => write!(formatter, "tree exceeds 4 GiB"),
        }
    }
}

impl std::error::Error for TreeError {}

impl From<ProtobufError> for TreeError {
    fn from(error: ProtobufError) -> Self {
        TreeError::Wire(error)
    }
}

pub struct Tree {
    pub schema: &'static Schema,
    pub entries: Vec<Entry>,
    pub text: Vec<u8>,
}

/// Where a chain being built starts and currently ends.
#[derive(Debug, Clone, Copy)]
pub struct Chain {
    pub first: u32,
    pub last: u32,
}

impl Chain {
    pub fn new() -> Chain {
        Chain {
            first: NONE,
            last: NONE,
        }
    }
}

impl Default for Chain {
    fn default() -> Self {
        Chain::new()
    }
}

impl Tree {
    pub fn new(schema: &'static Schema) -> Tree {
        Tree {
            schema,
            entries: Vec::new(),
            text: Vec::new(),
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.text.clear();
    }

    /// The schema field of an entry, when it has one.
    pub fn field(&self, entry: &Entry) -> Option<Field> {
        if entry.message == NO_SCHEMA {
            return None;
        }
        self.schema.message_at(entry.message)?.field_at(entry.slot)
    }

    pub fn str(&self, span: Span) -> &str {
        // Validated when stored.
        std::str::from_utf8(self.bytes(span)).unwrap_or("")
    }

    pub fn bytes(&self, span: Span) -> &[u8] {
        let start = span.start as usize;
        &self.text[start..start + span.length as usize]
    }

    /// Iterates a chain from its first entry.
    pub fn chain(&self, first: u32) -> ChainIter<'_> {
        ChainIter {
            tree: self,
            next: first,
        }
    }

    /// Appends an entry to a chain and returns its index.
    pub fn push(&mut self, chain: &mut Chain, entry: Entry) -> Result<u32, TreeError> {
        let index = u32::try_from(self.entries.len()).map_err(|_| TreeError::TooLarge)?;
        if index == NONE {
            return Err(TreeError::TooLarge);
        }
        self.entries.push(entry);
        if chain.first == NONE {
            chain.first = index;
        } else {
            self.entries[chain.last as usize].next = index;
        }
        chain.last = index;
        Ok(index)
    }

    pub fn push_bytes(&mut self, bytes: &[u8]) -> Result<Span, TreeError> {
        let start = u32::try_from(self.text.len()).map_err(|_| TreeError::TooLarge)?;
        let length = u32::try_from(bytes.len()).map_err(|_| TreeError::TooLarge)?;
        self.text.extend_from_slice(bytes);
        Ok(Span { start, length })
    }

    /// Builds an unknown-field entry for a chain being read from JSON.
    pub fn push_unknown(
        &mut self,
        chain: &mut Chain,
        number: u32,
        value: Node,
    ) -> Result<u32, TreeError> {
        self.push(chain, Entry::unknown(number, value))
    }

    /// Builds a known-field entry for a chain being read from JSON.
    pub fn push_known(
        &mut self,
        chain: &mut Chain,
        message: MessageRef,
        slot: u16,
        field: Field,
        number: u32,
        value: Node,
    ) -> Result<u32, TreeError> {
        self.push(
            chain,
            Entry::known(number, message.index(), slot, field, value),
        )
    }

    // ----- decoding -----

    /// Decodes `bytes` as `message` and returns the first entry of the
    /// resulting chain, or `NONE` for an empty message.
    pub fn decode(&mut self, bytes: &[u8], message: Option<MessageRef>) -> Result<u32, TreeError> {
        let mut chain = Chain::new();
        for field in FieldReader::new(bytes) {
            let field = field?;
            let slot = message.and_then(|message| message.slot(field.number));
            match (message, slot) {
                (Some(message), Some(slot)) => {
                    let known = message.field_at(slot).ok_or(TreeError::KindMismatch {
                        number: field.number,
                    })?;
                    self.decode_known(&mut chain, message, slot, known, field.number, field.value)?;
                }
                _ => {
                    let value = self.raw(field.value)?;
                    self.push(&mut chain, Entry::unknown(field.number, value))?;
                }
            }
        }
        Ok(chain.first)
    }

    fn raw(&mut self, value: Value<'_>) -> Result<Node, TreeError> {
        Ok(match value {
            Value::Varint(value) => Node::RawVarint(value),
            Value::Fixed32(value) => Node::RawFixed32(value),
            Value::Fixed64(value) => Node::RawFixed64(value),
            Value::Bytes(bytes) | Value::Group(bytes) => Node::RawBytes(self.push_bytes(bytes)?),
        })
    }

    fn decode_known(
        &mut self,
        chain: &mut Chain,
        message: MessageRef,
        slot: u16,
        known: Field,
        number: u32,
        value: Value<'_>,
    ) -> Result<(), TreeError> {
        let index = message.index();
        let node = match (known.kind, value) {
            (Kind::Int | Kind::Enum, Value::Varint(value)) => Node::Int(value as i64),
            (Kind::Uint, Value::Varint(value)) => Node::Uint(value),
            (Kind::Sint, Value::Varint(value)) => Node::Int(zigzag_decode(value)),
            (Kind::Bool, Value::Varint(value)) => Node::Bool(value != 0),
            (Kind::Fixed32, Value::Fixed32(value)) => Node::Fixed32(value),
            (Kind::Fixed64, Value::Fixed64(value)) => Node::Fixed64(value),
            (Kind::Float, Value::Fixed32(value)) => Node::Float(f32::from_bits(value)),
            (Kind::Double, Value::Fixed64(value)) => Node::Double(f64::from_bits(value)),
            (Kind::String, Value::Bytes(bytes)) => match std::str::from_utf8(bytes) {
                Ok(_) => Node::Str(self.push_bytes(bytes)?),
                Err(_) => Node::RawBytes(self.push_bytes(bytes)?),
            },
            (Kind::Bytes, Value::Bytes(bytes)) => Node::Bytes(self.push_bytes(bytes)?),
            (Kind::Reference, Value::Bytes(bytes)) => match reference_identifier(bytes) {
                Some(identifier) => Node::Reference(identifier),
                None => Node::RawBytes(self.push_bytes(bytes)?),
            },
            (Kind::Message(nested_index), Value::Bytes(bytes)) => {
                // Push the entry first so the nested chain follows it; on a
                // failed nested decode, drop what was added and keep raw.
                let entry_index = self.push(
                    chain,
                    Entry::known(number, index, slot, known, Node::Message(NONE)),
                )?;
                let entries_mark = self.entries.len();
                let text_mark = self.text.len();
                let nested = self.schema.message_at(nested_index);
                match self.decode(bytes, nested) {
                    Ok(first) => self.entries[entry_index as usize].value = Node::Message(first),
                    Err(_) => {
                        self.entries.truncate(entries_mark);
                        self.text.truncate(text_mark);
                        let span = self.push_bytes(bytes)?;
                        self.entries[entry_index as usize].value = Node::RawBytes(span);
                    }
                }
                return Ok(());
            }
            (
                Kind::Int | Kind::Uint | Kind::Sint | Kind::Bool | Kind::Enum,
                Value::Bytes(bytes),
            ) if known.repeated => match packed_varints(bytes) {
                Ok(values) => {
                    for value in values {
                        let node = match known.kind {
                            Kind::Uint => Node::Uint(value),
                            Kind::Sint => Node::Int(zigzag_decode(value)),
                            Kind::Bool => Node::Bool(value != 0),
                            _ => Node::Int(value as i64),
                        };
                        self.push(chain, Entry::known(number, index, slot, known, node))?;
                    }
                    return Ok(());
                }
                Err(_) => Node::RawBytes(self.push_bytes(bytes)?),
            },
            (Kind::Float | Kind::Fixed32, Value::Bytes(bytes))
                if known.repeated && bytes.len() % 4 == 0 =>
            {
                for chunk in bytes.chunks(4) {
                    let bits = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                    let node = if known.kind == Kind::Float {
                        Node::Float(f32::from_bits(bits))
                    } else {
                        Node::Fixed32(bits)
                    };
                    self.push(chain, Entry::known(number, index, slot, known, node))?;
                }
                return Ok(());
            }
            (Kind::Double | Kind::Fixed64, Value::Bytes(bytes))
                if known.repeated && bytes.len() % 8 == 0 =>
            {
                for chunk in bytes.chunks(8) {
                    let mut array = [0u8; 8];
                    array.copy_from_slice(chunk);
                    let bits = u64::from_le_bytes(array);
                    let node = if known.kind == Kind::Double {
                        Node::Double(f64::from_bits(bits))
                    } else {
                        Node::Fixed64(bits)
                    };
                    self.push(chain, Entry::known(number, index, slot, known, node))?;
                }
                return Ok(());
            }
            (_, value) => {
                let node = self.raw(value)?;
                self.push(chain, Entry::unknown(number, node))?;
                return Ok(());
            }
        };
        self.push(chain, Entry::known(number, index, slot, known, node))?;
        Ok(())
    }

    // ----- encoding -----

    /// Encoded size of a chain, without any enclosing tag.
    pub fn size(&self, first: u32) -> usize {
        let mut total = 0usize;
        let mut cursor = first;
        while cursor != NONE {
            let entry = &self.entries[cursor as usize];
            if entry.is_packed() {
                let (payload, run_end) = self.packed_size(cursor);
                total += tag_size(entry.number) + varint_size(payload as u64) + payload;
                cursor = run_end;
                continue;
            }
            total += tag_size(entry.number) + self.value_size(entry);
            cursor = entry.next;
        }
        total
    }

    /// Size of the packed payload of the run starting at `first`, and the
    /// entry after the run.
    fn packed_size(&self, first: u32) -> (usize, u32) {
        let number = self.entries[first as usize].number;
        let mut payload = 0usize;
        let mut cursor = first;
        while cursor != NONE && self.entries[cursor as usize].number == number {
            payload += self.scalar_size(&self.entries[cursor as usize]);
            cursor = self.entries[cursor as usize].next;
        }
        (payload, cursor)
    }

    fn scalar_size(&self, entry: &Entry) -> usize {
        match entry.value {
            Node::Int(value) if entry.is_sint() => varint_size(zigzag_encode(value)),
            Node::Int(value) => varint_size(value as u64),
            Node::Uint(value) | Node::RawVarint(value) => varint_size(value),
            Node::Bool(_) => 1,
            Node::Fixed32(_) | Node::Float(_) | Node::RawFixed32(_) => 4,
            Node::Fixed64(_) | Node::Double(_) | Node::RawFixed64(_) => 8,
            _ => 0,
        }
    }

    fn value_size(&self, entry: &Entry) -> usize {
        match entry.value {
            Node::Str(span) | Node::Bytes(span) | Node::RawBytes(span) => {
                varint_size(u64::from(span.length)) + span.length as usize
            }
            Node::Reference(identifier) => {
                let nested = 1 + varint_size(identifier);
                varint_size(nested as u64) + nested
            }
            Node::Message(first) => {
                let nested = self.size(first);
                varint_size(nested as u64) + nested
            }
            _ => self.scalar_size(entry),
        }
    }

    /// Encodes a chain. Consecutive entries of a packed field are written
    /// as one packed value, as they were read.
    pub fn encode(&self, first: u32, out: &mut Vec<u8>) -> Result<(), TreeError> {
        let mut cursor = first;
        while cursor != NONE {
            let entry = &self.entries[cursor as usize];
            if entry.is_packed() {
                let (payload, run_end) = self.packed_size(cursor);
                write_tag(out, entry.number, 2);
                write_varint(out, payload as u64);
                let mut element = cursor;
                while element != run_end {
                    self.encode_scalar(&self.entries[element as usize], out)?;
                    element = self.entries[element as usize].next;
                }
                cursor = run_end;
                continue;
            }
            self.encode_entry(entry, out)?;
            cursor = entry.next;
        }
        Ok(())
    }

    fn encode_entry(&self, entry: &Entry, out: &mut Vec<u8>) -> Result<(), TreeError> {
        let number = entry.number;
        match entry.value {
            Node::Str(span) | Node::Bytes(span) | Node::RawBytes(span) => {
                write_tag(out, number, 2);
                write_varint(out, u64::from(span.length));
                out.extend_from_slice(self.bytes(span));
            }
            Node::Reference(identifier) => {
                write_tag(out, number, 2);
                write_varint(out, (1 + varint_size(identifier)) as u64);
                write_tag(out, 1, 0);
                write_varint(out, identifier);
            }
            Node::Message(first) => {
                write_tag(out, number, 2);
                write_varint(out, self.size(first) as u64);
                self.encode(first, out)?;
            }
            Node::Fixed32(_) | Node::Float(_) | Node::RawFixed32(_) => {
                write_tag(out, number, 5);
                self.encode_scalar(entry, out)?;
            }
            Node::Fixed64(_) | Node::Double(_) | Node::RawFixed64(_) => {
                write_tag(out, number, 1);
                self.encode_scalar(entry, out)?;
            }
            _ => {
                write_tag(out, number, 0);
                self.encode_scalar(entry, out)?;
            }
        }
        Ok(())
    }

    /// One scalar without a tag (also an element of a packed field).
    fn encode_scalar(&self, entry: &Entry, out: &mut Vec<u8>) -> Result<(), TreeError> {
        match entry.value {
            Node::Int(value) if entry.is_sint() => write_varint(out, zigzag_encode(value)),
            Node::Int(value) => write_varint(out, value as u64),
            Node::Uint(value) | Node::RawVarint(value) => write_varint(out, value),
            Node::Bool(value) => write_varint(out, u64::from(value)),
            Node::Fixed32(value) | Node::RawFixed32(value) => {
                out.extend_from_slice(&value.to_le_bytes())
            }
            Node::Float(value) => out.extend_from_slice(&value.to_bits().to_le_bytes()),
            Node::Fixed64(value) | Node::RawFixed64(value) => {
                out.extend_from_slice(&value.to_le_bytes())
            }
            Node::Double(value) => out.extend_from_slice(&value.to_bits().to_le_bytes()),
            _ => {
                return Err(TreeError::KindMismatch {
                    number: entry.number,
                });
            }
        }
        Ok(())
    }
}

pub struct ChainIter<'t> {
    tree: &'t Tree,
    next: u32,
}

impl<'t> Iterator for ChainIter<'t> {
    type Item = (u32, &'t Entry);

    fn next(&mut self) -> Option<Self::Item> {
        if self.next == NONE {
            return None;
        }
        let index = self.next;
        let entry = &self.tree.entries[index as usize];
        self.next = entry.next;
        Some((index, entry))
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

pub fn varint_size(value: u64) -> usize {
    (64 - (value | 1).leading_zeros() as usize).div_ceil(7)
}

fn tag_size(number: u32) -> usize {
    varint_size(u64::from(number) << 3)
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::protobuf::schema::{FieldRecord, MessageRecord};

    // Inner { 1 count: repeated packed uint }, Outer { 1 name: string,
    // 2 inner: Inner, 3 target: reference, 4 delta: sint, 5 ratio: float }.
    static SCHEMA: Schema = Schema {
        names: "countnameinnertargetdeltaratioInnerOuter",
        messages: &[
            MessageRecord {
                name_offset: 30,
                name_length: 5,
                fields_start: 0,
                fields_length: 1,
            },
            MessageRecord {
                name_offset: 35,
                name_length: 5,
                fields_start: 1,
                fields_length: 5,
            },
        ],
        fields: &[
            FieldRecord {
                name_offset: 0,
                number: 1,
                message_index: 0,
                name_length: 5,
                kind: 1,
                flags: 3,
            },
            FieldRecord {
                name_offset: 5,
                number: 1,
                message_index: 0,
                name_length: 4,
                kind: 9,
                flags: 0,
            },
            FieldRecord {
                name_offset: 9,
                number: 2,
                message_index: 0,
                name_length: 5,
                kind: 12,
                flags: 0,
            },
            FieldRecord {
                name_offset: 14,
                number: 3,
                message_index: 0,
                name_length: 6,
                kind: 11,
                flags: 0,
            },
            FieldRecord {
                name_offset: 20,
                number: 4,
                message_index: 0,
                name_length: 5,
                kind: 2,
                flags: 0,
            },
            FieldRecord {
                name_offset: 25,
                number: 5,
                message_index: 0,
                name_length: 5,
                kind: 7,
                flags: 0,
            },
        ],
    };

    fn sample() -> Vec<u8> {
        let mut bytes = vec![0x0A, 0x02, b'h', b'i']; // name "hi"
        bytes.extend_from_slice(&[0x12, 0x05, 0x0A, 0x03, 0x03, 0x80, 0x01]); // inner { count packed [3, 128] }
        bytes.extend_from_slice(&[0x1A, 0x02, 0x08, 0x2A]); // target ref 42
        bytes.extend_from_slice(&[0x20, 0x03]); // delta sint -2
        bytes.extend_from_slice(&[0x2D, 0x00, 0x00, 0x80, 0x3F]); // ratio 1.0
        bytes.extend_from_slice(&[0x30, 0x07]); // unknown field 6 varint 7
        bytes
    }

    #[test]
    fn decodes_named_and_unknown_fields() {
        let mut tree = Tree::new(&SCHEMA);
        let first = tree.decode(&sample(), SCHEMA.message("Outer")).unwrap();
        let entries: Vec<&Entry> = tree.chain(first).map(|(_, entry)| entry).collect();
        let names: Vec<Option<&str>> = entries
            .iter()
            .map(|entry| tree.field(entry).map(|field| field.name))
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
        match entries[0].value {
            Node::Str(span) => assert_eq!(tree.str(span), "hi"),
            other => panic!("{other:?}"),
        }
        assert_eq!(entries[2].value, Node::Reference(42));
        assert_eq!(entries[3].value, Node::Int(-2));
        assert_eq!(entries[4].value, Node::Float(1.0));
        assert_eq!(entries[5].value, Node::RawVarint(7));
        match entries[1].value {
            Node::Message(inner) => {
                let counts: Vec<Node> = tree.chain(inner).map(|(_, entry)| entry.value).collect();
                assert_eq!(counts, vec![Node::Uint(3), Node::Uint(128)]);
            }
            other => panic!("expected a message, got {other:?}"),
        }
    }

    #[test]
    fn re_encodes_to_the_same_bytes() {
        let bytes = sample();
        let mut tree = Tree::new(&SCHEMA);
        let first = tree.decode(&bytes, SCHEMA.message("Outer")).unwrap();
        assert_eq!(tree.size(first), bytes.len());
        let mut out = Vec::new();
        tree.encode(first, &mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn schemaless_decode_keeps_everything_raw() {
        let bytes = sample();
        let mut tree = Tree::new(&SCHEMA);
        let first = tree.decode(&bytes, None).unwrap();
        assert!(
            tree.chain(first)
                .all(|(_, entry)| tree.field(entry).is_none())
        );
        let mut out = Vec::new();
        tree.encode(first, &mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn varint_sizes() {
        assert_eq!(varint_size(0), 1);
        assert_eq!(varint_size(127), 1);
        assert_eq!(varint_size(128), 2);
        assert_eq!(varint_size(u64::MAX), 10);
    }
}
