//! ZIP archives: reading entries stored or deflated, with checksums.

pub mod crc32;
pub mod reader;

pub use crc32::crc32;
pub use reader::{Entry, ZipArchive, ZipError};
