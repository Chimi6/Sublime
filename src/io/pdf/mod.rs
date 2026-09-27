//! PDF: a writer that makes a page of each image (the pixels deflated
//! with PNG predictors, or a JPEG embedded unchanged), streaming.

pub mod writer;

pub use writer::{PdfDocument, PdfPage, jpeg_info};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfError(pub String);

impl std::fmt::Display for PdfError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl std::error::Error for PdfError {}
