//! Streaming CSV reading and writing.

pub mod reader;
pub mod table;
pub mod writer;

pub use reader::{CsvError, CsvReader, Record};
pub use table::{TableNotes, TableRows, emit_table};
pub use writer::CsvWriter;
