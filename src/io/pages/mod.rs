//! Apple Pages documents: a ZIP package of IWA streams (`io::iwa`) whose
//! objects are the Pages, shared text (TSWP), drawing (TSD), and table
//! (TST) archives. This module holds what is specific to Pages: the type
//! registry and, as they are mapped, the readers for its archives.

pub mod json;
pub mod package;
pub mod schema;
pub mod types;

pub use package::{Entry, Object, ObjectMessage, Package, PackageError};
pub use types::type_name;

/// Schema of a message type, when the registry and the schema both know it.
pub fn message_schema(message_type: u32) -> Option<crate::io::protobuf::schema::MessageRef> {
    let name = type_name(message_type)?;
    schema::SCHEMA.message(name)
}
