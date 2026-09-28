//! PDF: a writer that makes a page of each image (the pixels deflated
//! with PNG predictors, or a JPEG embedded unchanged), streaming; and a
//! reader of a page's image through the object model in `document`.

pub mod compose;
pub mod content;
pub mod document;
pub mod filter;
pub mod font;
pub mod lexer;
pub mod markdown;
pub mod object;
pub mod reader;
pub mod tables;
pub mod text;
pub mod writer;

pub use reader::{PdfNotes, page_jpeg, read_pdf, read_pdf_rows};
pub use writer::{PdfDocument, PdfPage, jpeg_info};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfError(pub String);

impl std::fmt::Display for PdfError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl std::error::Error for PdfError {}
