//! Schema tables for schema-driven decoding: messages and their fields,
//! each field with a number, a name, and a kind. Tables are generated per
//! format (see `io::pages::schema`) in a packed form: fixed-size records
//! that index one shared name blob, so a few thousand fields cost tens of
//! kilobytes rather than hundreds.

/// How a field's wire value is interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Two's-complement varint (`int32`, `int64`).
    Int,
    /// Unsigned varint.
    Uint,
    /// ZigZag varint (`sint32`, `sint64`).
    Sint,
    Bool,
    /// Varint holding an enum value.
    Enum,
    Fixed32,
    Fixed64,
    Float,
    Double,
    String,
    Bytes,
    /// A `TSP.Reference` message: an object identifier.
    Reference,
    /// A nested message, by index into the schema's message table.
    Message(u16),
}

impl Kind {
    /// Decodes the generated record encoding: kinds 0 to 11 as listed, 12
    /// for a message whose index is carried separately.
    fn from_record(code: u8, message_index: u16) -> Kind {
        match code {
            0 => Kind::Int,
            1 => Kind::Uint,
            2 => Kind::Sint,
            3 => Kind::Bool,
            4 => Kind::Enum,
            5 => Kind::Fixed32,
            6 => Kind::Fixed64,
            7 => Kind::Float,
            8 => Kind::Double,
            9 => Kind::String,
            10 => Kind::Bytes,
            11 => Kind::Reference,
            _ => Kind::Message(message_index),
        }
    }
}

/// A field as a caller sees it, built from a record on demand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Field {
    pub number: u32,
    pub name: &'static str,
    pub kind: Kind,
    pub repeated: bool,
    /// Repeated scalars are written packed into one length-delimited value.
    pub packed: bool,
}

/// Generated per field: `number`, name as an offset and length into the
/// blob, the kind code, flags (bit 0 repeated, bit 1 packed), and the
/// nested message index for message kinds.
#[derive(Debug, Clone, Copy)]
pub struct FieldRecord {
    pub name_offset: u32,
    pub number: u16,
    pub message_index: u16,
    pub name_length: u8,
    pub kind: u8,
    pub flags: u8,
}

/// Generated per message: name in the blob, and its fields as a range of
/// the field records, sorted by number.
#[derive(Debug, Clone, Copy)]
pub struct MessageRecord {
    pub name_offset: u32,
    pub name_length: u8,
    pub fields_start: u32,
    pub fields_length: u16,
}

pub struct Schema {
    pub names: &'static str,
    /// Sorted by name.
    pub messages: &'static [MessageRecord],
    pub fields: &'static [FieldRecord],
}

/// A message of a schema.
#[derive(Clone, Copy)]
pub struct MessageRef {
    schema: &'static Schema,
    index: u16,
}

impl std::fmt::Debug for MessageRef {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "MessageRef({})", self.name())
    }
}

impl PartialEq for MessageRef {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.schema, other.schema) && self.index == other.index
    }
}

impl Eq for MessageRef {}

impl Schema {
    fn name_at(&'static self, offset: u32, length: u8) -> &'static str {
        let start = offset as usize;
        &self.names[start..start + usize::from(length)]
    }

    pub fn message_at(&'static self, index: u16) -> Option<MessageRef> {
        if usize::from(index) >= self.messages.len() {
            return None;
        }
        Some(MessageRef {
            schema: self,
            index,
        })
    }

    /// Finds a message by name.
    pub fn message(&'static self, name: &str) -> Option<MessageRef> {
        let index = self
            .messages
            .binary_search_by(|record| {
                self.name_at(record.name_offset, record.name_length)
                    .cmp(name)
            })
            .ok()?;
        self.message_at(index as u16)
    }
}

impl MessageRef {
    pub fn name(&self) -> &'static str {
        let record = &self.schema.messages[usize::from(self.index)];
        self.schema.name_at(record.name_offset, record.name_length)
    }

    pub fn index(&self) -> u16 {
        self.index
    }

    fn records(&self) -> &'static [FieldRecord] {
        let record = &self.schema.messages[usize::from(self.index)];
        let start = record.fields_start as usize;
        &self.schema.fields[start..start + usize::from(record.fields_length)]
    }

    fn field_from(&self, record: &FieldRecord) -> Field {
        Field {
            number: u32::from(record.number),
            name: self.schema.name_at(record.name_offset, record.name_length),
            kind: Kind::from_record(record.kind, record.message_index),
            repeated: record.flags & 1 != 0,
            packed: record.flags & 2 != 0,
        }
    }

    /// Position of the field with `number` among this message's fields.
    /// Most messages have a handful of fields, where a scan beats a search.
    pub fn slot(&self, number: u32) -> Option<u16> {
        let records = self.records();
        if records.len() <= 8 {
            let index = records
                .iter()
                .position(|record| u32::from(record.number) == number)?;
            return Some(index as u16);
        }
        let index = records
            .binary_search_by_key(&number, |record| u32::from(record.number))
            .ok()?;
        Some(index as u16)
    }

    pub fn field_at(&self, slot: u16) -> Option<Field> {
        let record = self.records().get(usize::from(slot))?;
        Some(self.field_from(record))
    }

    pub fn field(&self, number: u32) -> Option<Field> {
        let records = self.records();
        let index = records
            .binary_search_by_key(&number, |record| u32::from(record.number))
            .ok()?;
        Some(self.field_from(&records[index]))
    }

    /// Slot and field of the field called `name`.
    pub fn slot_named(&self, name: &str) -> Option<(u16, Field)> {
        self.records()
            .iter()
            .enumerate()
            .map(|(slot, record)| (slot as u16, self.field_from(record)))
            .find(|(_, field)| field.name == name)
    }

    pub fn field_named(&self, name: &str) -> Option<Field> {
        self.records()
            .iter()
            .map(|record| self.field_from(record))
            .find(|field| field.name == name)
    }

    pub fn fields(&self) -> impl Iterator<Item = Field> + '_ {
        self.records().iter().map(|record| self.field_from(record))
    }
}
