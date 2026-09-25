//! XML: escaping for writers, a small pull reader for the XML-based
//! formats (Word), and the strict reader and writer that map an XML
//! document to and from the value hub.

pub mod reader;
pub mod tree;
pub mod writer;

pub use reader::{XmlEvent, XmlReader};
pub use tree::XmlError;

/// Appends `text` with `&`, `<`, and `>` escaped, for element content.
pub fn escape_text(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
}

/// Appends `text` escaped for a double-quoted attribute value.
pub fn escape_attribute(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("&#10;"),
            '\t' => out.push_str("&#9;"),
            other => out.push(other),
        }
    }
}
