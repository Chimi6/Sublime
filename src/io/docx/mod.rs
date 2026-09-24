//! Word documents (Office Open XML WordprocessingML).

pub mod reader;
pub mod writer;

pub use reader::{DocxError, read_docx};
pub use writer::{DocxStream, write_docx};
