//! ZIP archives: reading entries stored or deflated, with checksums.

pub mod crc32;
pub mod reader;
pub mod writer;

pub use crc32::crc32;
pub use reader::{Entry, EntryStream, ZipArchive, ZipError};
pub use writer::ZipWriter;
