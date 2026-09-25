//! Excel workbooks (Office Open XML SpreadsheetML): a reader that streams
//! one sheet's rows out of the package, and a writer that streams rows into
//! a one-sheet workbook.

pub mod reader;
pub mod writer;

pub use reader::{Workbook, XlsxError};
pub use writer::XlsxWriter;
