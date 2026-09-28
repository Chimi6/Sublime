//! WebP: a reader into the image hub (lossless and lossy, with alpha)
//! and a lossless writer from it. The RIFF container is parsed here; the
//! bitstreams live in `lossless` (VP8L) and `lossy` (VP8).

pub(crate) mod lossless;
pub(crate) mod lossy;
pub mod reader;
mod tables;
pub mod writer;

pub use reader::{WebpNotes, read_webp, read_webp_from, read_webp_rows};
pub use writer::{Effort, WebpRows, write_webp, write_webp_with};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebpError(pub String);

impl std::fmt::Display for WebpError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl std::error::Error for WebpError {}

pub(crate) fn to_rows(error: WebpError) -> crate::io::png::RowsError {
    crate::io::png::RowsError::Png(crate::io::png::PngError(error.0))
}
