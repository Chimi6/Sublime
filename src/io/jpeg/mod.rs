//! JPEG: a reader into the image hub and a writer from it.

pub mod reader;
pub mod writer;

pub use reader::{JpegError, JpegNotes, read_jpeg, read_jpeg_from, read_jpeg_rows};
pub use writer::{DEFAULT_QUALITY, JpegRows, write_jpeg};
