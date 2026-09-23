//! HTML writing: escaping and rendering of Markdown events.

pub mod writer;

pub use writer::{escape_href, escape_html, push_html, write_html};
