//! OpenDocument Text (`.odt`, ISO/IEC 26300, ODF 1.3): a ZIP of XML parts,
//! read into the document model and written from it.

pub mod reader;
mod styles;
pub mod writer;

pub use reader::{OdtError, read_odt};
pub use writer::write_odt;
