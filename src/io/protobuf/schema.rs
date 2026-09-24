//! Schema tables for schema-driven decoding: a message is a list of fields,
//! each with a number, a name, and a kind. Tables are generated per format
//! (see `io::pages::schema`); this module only defines their shape.

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Field {
    pub number: u32,
    pub name: &'static str,
    pub kind: Kind,
    pub repeated: bool,
    /// Repeated scalars are written packed into one length-delimited value.
    pub packed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Message {
    pub name: &'static str,
    /// Sorted by number.
    pub fields: &'static [Field],
}

impl Message {
    pub fn field(&self, number: u32) -> Option<&'static Field> {
        let index = self
            .fields
            .binary_search_by_key(&number, |field| field.number)
            .ok()?;
        Some(&self.fields[index])
    }
}

/// Finds a message by name in a table sorted by name.
pub fn find_message(table: &'static [Message], name: &str) -> Option<&'static Message> {
    let index = table
        .binary_search_by_key(&name, |message| message.name)
        .ok()?;
    Some(&table[index])
}
