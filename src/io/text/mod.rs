//! Plain text writing: rendering of Markdown events as readable text.

pub mod writer;

pub use writer::{TextWriter, push_text};
