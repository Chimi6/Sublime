//! Field-by-field reader over a serialized message.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtobufError {
    /// The bytes ended in the middle of a field.
    Truncated { offset: usize },
    /// A tag with a wire type that does not exist (6 or 7).
    BadWireType { offset: usize, wire_type: u8 },
    /// A varint longer than ten bytes.
    VarintTooLong { offset: usize },
    /// An end-group tag with no matching start.
    UnexpectedGroupEnd { offset: usize },
}

impl fmt::Display for ProtobufError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProtobufError::Truncated { offset } => {
                write!(formatter, "message truncated at byte {offset}")
            }
            ProtobufError::BadWireType { offset, wire_type } => {
                write!(
                    formatter,
                    "wire type {wire_type} at byte {offset} does not exist"
                )
            }
            ProtobufError::VarintTooLong { offset } => write!(
                formatter,
                "varint at byte {offset} is longer than ten bytes"
            ),
            ProtobufError::UnexpectedGroupEnd { offset } => {
                write!(formatter, "end of group at byte {offset} without a start")
            }
        }
    }
}

impl std::error::Error for ProtobufError {}

/// A field's value, by wire type. Length-delimited values are returned as
/// the raw bytes; they may be a string, a nested message, a packed
/// repeated field, or bytes, and only a schema can say which.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value<'a> {
    Varint(u64),
    Fixed64(u64),
    Bytes(&'a [u8]),
    Fixed32(u32),
    /// A deprecated group: its fields, still encoded, between the start and
    /// end tags.
    Group(&'a [u8]),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field<'a> {
    pub number: u32,
    pub value: Value<'a>,
}

/// Reads `(value, bytes consumed)` of a varint at the start of `bytes`.
pub fn read_varint(bytes: &[u8], offset: usize) -> Result<(u64, usize), ProtobufError> {
    let mut value = 0u64;
    let mut shift = 0u32;
    let mut index = offset;
    loop {
        let byte = match bytes.get(index) {
            Some(byte) => *byte,
            None => return Err(ProtobufError::Truncated { offset }),
        };
        index += 1;
        if shift >= 64 {
            return Err(ProtobufError::VarintTooLong { offset });
        }
        value |= u64::from(byte & 0x7F) << shift;
        if byte & 0x80 == 0 {
            return Ok((value, index - offset));
        }
        shift += 7;
    }
}

/// Iterates the fields of one message.
pub struct FieldReader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> FieldReader<'a> {
    pub fn new(bytes: &'a [u8]) -> FieldReader<'a> {
        FieldReader { bytes, position: 0 }
    }

    pub fn position(&self) -> usize {
        self.position
    }

    fn read_field(&mut self) -> Result<Option<Field<'a>>, ProtobufError> {
        if self.position >= self.bytes.len() {
            return Ok(None);
        }
        let start = self.position;
        let (tag, tag_length) = read_varint(self.bytes, self.position)?;
        self.position += tag_length;
        let number = (tag >> 3) as u32;
        let wire_type = (tag & 7) as u8;
        let value = match wire_type {
            0 => {
                let (value, length) = read_varint(self.bytes, self.position)?;
                self.position += length;
                Value::Varint(value)
            }
            1 => Value::Fixed64(u64::from_le_bytes(self.take::<8>(start)?)),
            2 => {
                let (length, length_length) = read_varint(self.bytes, self.position)?;
                self.position += length_length;
                let end = self.position.saturating_add(length as usize);
                if length > usize::MAX as u64 || end > self.bytes.len() {
                    return Err(ProtobufError::Truncated { offset: start });
                }
                let slice = &self.bytes[self.position..end];
                self.position = end;
                Value::Bytes(slice)
            }
            3 => {
                let group_start = self.position;
                let group_end = self.skip_group(number, start)?;
                Value::Group(&self.bytes[group_start..group_end])
            }
            4 => return Err(ProtobufError::UnexpectedGroupEnd { offset: start }),
            5 => Value::Fixed32(u32::from_le_bytes(self.take::<4>(start)?)),
            other => {
                return Err(ProtobufError::BadWireType {
                    offset: start,
                    wire_type: other,
                });
            }
        };
        Ok(Some(Field { number, value }))
    }

    fn take<const N: usize>(&mut self, field_start: usize) -> Result<[u8; N], ProtobufError> {
        let end = self.position + N;
        if end > self.bytes.len() {
            return Err(ProtobufError::Truncated {
                offset: field_start,
            });
        }
        let mut out = [0u8; N];
        out.copy_from_slice(&self.bytes[self.position..end]);
        self.position = end;
        Ok(out)
    }

    /// Skips to the end-group tag matching `number`, nesting allowed, and
    /// returns the offset where the group's content ends.
    fn skip_group(&mut self, number: u32, field_start: usize) -> Result<usize, ProtobufError> {
        loop {
            if self.position >= self.bytes.len() {
                return Err(ProtobufError::Truncated {
                    offset: field_start,
                });
            }
            let content_end = self.position;
            let (tag, tag_length) = read_varint(self.bytes, self.position)?;
            if tag & 7 == 4 {
                self.position += tag_length;
                if (tag >> 3) as u32 == number {
                    return Ok(content_end);
                }
                continue;
            }
            match self.read_field()? {
                Some(_) => {}
                None => {
                    return Err(ProtobufError::Truncated {
                        offset: field_start,
                    });
                }
            }
        }
    }
}

impl<'a> Iterator for FieldReader<'a> {
    type Item = Result<Field<'a>, ProtobufError>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.read_field() {
            Ok(Some(field)) => Some(Ok(field)),
            Ok(None) => None,
            Err(error) => {
                self.position = self.bytes.len();
                Some(Err(error))
            }
        }
    }
}

/// Reads packed varints from a length-delimited value.
pub fn packed_varints(bytes: &[u8]) -> Result<Vec<u64>, ProtobufError> {
    let mut values = Vec::new();
    let mut offset = 0usize;
    while offset < bytes.len() {
        let (value, length) = read_varint(bytes, offset)?;
        values.push(value);
        offset += length;
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(bytes: &[u8]) -> Vec<Field<'_>> {
        FieldReader::new(bytes)
            .map(|field| field.unwrap())
            .collect()
    }

    #[test]
    fn reads_every_wire_type() {
        let bytes = [
            0x08, 0x96, 0x01, // field 1 varint 150
            0x11, 1, 0, 0, 0, 0, 0, 0, 0, // field 2 fixed64 1
            0x1A, 0x03, b'a', b'b', b'c', // field 3 bytes "abc"
            0x25, 7, 0, 0, 0, // field 4 fixed32 7
        ];
        assert_eq!(
            fields(&bytes),
            vec![
                Field {
                    number: 1,
                    value: Value::Varint(150)
                },
                Field {
                    number: 2,
                    value: Value::Fixed64(1)
                },
                Field {
                    number: 3,
                    value: Value::Bytes(b"abc")
                },
                Field {
                    number: 4,
                    value: Value::Fixed32(7)
                },
            ]
        );
    }

    #[test]
    fn groups_are_skipped_as_one_value() {
        // field 5 group { field 1 varint 3 } end
        let bytes = [0x2B, 0x08, 0x03, 0x2C, 0x08, 0x01];
        let parsed = fields(&bytes);
        assert_eq!(
            parsed[0],
            Field {
                number: 5,
                value: Value::Group(&[0x08, 0x03])
            }
        );
        assert_eq!(
            parsed[1],
            Field {
                number: 1,
                value: Value::Varint(1)
            }
        );
    }

    #[test]
    fn truncation_is_an_error_not_a_panic() {
        let mut reader = FieldReader::new(&[0x1A, 0x05, b'a']);
        assert_eq!(
            reader.next(),
            Some(Err(ProtobufError::Truncated { offset: 0 }))
        );
        assert_eq!(reader.next(), None);
    }

    #[test]
    fn packed_values_decode() {
        assert_eq!(
            packed_varints(&[0x01, 0x96, 0x01, 0x02]).unwrap(),
            vec![1, 150, 2]
        );
    }
}
