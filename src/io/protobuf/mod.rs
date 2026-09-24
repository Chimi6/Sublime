//! Protocol Buffers wire format, without schemas. A message is a sequence of
//! fields, each a number plus a value of one of the wire types. This reader
//! yields them as they are; what a field means is the caller's business.

pub mod reader;

pub use reader::{Field, FieldReader, ProtobufError, Value, read_varint};
