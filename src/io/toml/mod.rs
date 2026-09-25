//! TOML 1.0: a reader into the value hub and a writer from it.

pub mod reader;
pub mod writer;

pub use reader::{TomlError, parse};
pub use writer::write_document;
