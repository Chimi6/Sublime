//! OpenDocument spreadsheets (`.ods`, ISO/IEC 26300, ODF 1.3): a reader
//! that hands out each sheet's rows with every cell's shown text, value,
//! and number format, and a writer that streams rows into sheets.

mod builtin;
pub mod numfmt;
pub mod reader;
pub mod writer;

pub use reader::{Need, OdsCell, OdsError, Sheet, Spreadsheet, Value};
pub use writer::OdsWriter;
