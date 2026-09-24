//! XML: escaping for writers and a small reader for the XML-based formats.

pub mod reader;

pub use reader::{XmlEvent, XmlReader};

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
