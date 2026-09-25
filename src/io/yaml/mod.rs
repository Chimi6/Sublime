//! YAML 1.2 (core schema): a reader into the value hub and a writer from it.

pub mod reader;
pub mod writer;

pub use reader::{Parsed, Plain, YamlError, parse, resolve_plain};
pub use writer::write_document;
