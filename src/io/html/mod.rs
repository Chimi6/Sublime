//! HTML: escaping and rendering of Markdown events, and reading HTML into
//! them.

pub mod reader;
pub mod writer;

pub use reader::parse_into;
pub use writer::{HtmlWriter, escape_href, escape_html, push_html, write_html};
