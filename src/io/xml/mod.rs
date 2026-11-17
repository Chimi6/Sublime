//! XML: escaping for writers, a small pull reader for the XML-based
//! formats (Word), and the strict reader and writer that map an XML
//! document to and from the value hub.

pub mod reader;
pub mod tree;
pub mod writer;

pub use reader::{XmlEvent, XmlReader};
pub use tree::XmlError;

use std::cell::Cell;

thread_local! {
    /// Characters the escape helpers dropped because XML 1.0 cannot carry them.
    static DROPPED: Cell<u64> = const { Cell::new(0) };
}

/// The count of characters dropped by the escape helpers on this thread
/// since the last call, and resets it. A converter step that wrote XML
/// reports a nonzero count as a loss.
pub fn take_dropped() -> u64 {
    DROPPED.with(|count| count.replace(0))
}

/// Whether XML 1.0 allows `ch` in a document at all (the `Char` production;
/// even as a character reference, a C0 control is not well-formed).
pub fn is_xml_char(ch: char) -> bool {
    matches!(ch, '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..)
}

fn drop_char() {
    DROPPED.with(|count| count.set(count.get() + 1));
}

/// Appends `text` with `&`, `<`, and `>` escaped, for element content.
/// Characters XML 1.0 cannot carry are dropped.
pub fn escape_text(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other if is_xml_char(other) => out.push(other),
            _ => drop_char(),
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
            other if is_xml_char(other) => out.push(other),
            _ => drop_char(),
        }
    }
}
