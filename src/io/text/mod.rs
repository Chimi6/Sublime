//! Plain text writing: rendering of Markdown events as readable text.

pub mod reader;
pub mod writer;

pub use reader::parse_into;
pub use writer::{TextWriter, push_text};
