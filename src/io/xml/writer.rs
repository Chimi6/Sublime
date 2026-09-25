//! XML from the value hub, the inverse of `tree.rs`: a table is an
//! element, `@name` members its attributes, `#text` its text, other
//! members child elements, arrays repeated elements, scalars text. Pretty
//! printed with two-space indentation, leaf elements on one line.

use crate::io::xml::tree::is_name;
use crate::io::xml::{escape_attribute, escape_text};
use crate::value::{ChunkedText, Data, NONE, Tree, push_float};

/// Writes the subtree at `root` as a document. The root object's one
/// member is the root element; anything else is wrapped in `<root>` with
/// a warning. Keys that are not XML names and attributes that are not
/// scalars are errors.
pub fn write_document(
    tree: &Tree,
    root: u32,
    out: &mut ChunkedText<'_>,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    match tree.data(root) {
        Data::Table(_) if tree.child_count(root) == 1 => {
            let member = tree.children(root).next().unwrap_or(NONE);
            if tree.key(member).starts_with(['@', '#']) {
                warnings.push("the JSON root has no single member; wrapped in <root>".to_string());
                return write_element(tree, "root", root, 0, out);
            }
            write_member(tree, tree.key(member), member, 0, out)
        }
        Data::Table(_) => {
            warnings.push("the JSON root has no single member; wrapped in <root>".to_string());
            write_element(tree, "root", root, 0, out)
        }
        Data::Array(_) => {
            warnings.push(
                "the JSON root is an array; wrapped in <root> as <item> elements".to_string(),
            );
            out.push_str("<root>\n");
            for item in tree.children(root) {
                write_member(tree, "item", item, 2, out)?;
            }
            out.push_str("</root>\n");
            Ok(())
        }
        _ => {
            warnings.push("the JSON root is a scalar; wrapped in <root>".to_string());
            write_element(tree, "root", root, 0, out)
        }
    }
}

/// A member of a table: one element, or one per item of an array.
fn write_member(
    tree: &Tree,
    name: &str,
    node: u32,
    indent: usize,
    out: &mut ChunkedText<'_>,
) -> Result<(), String> {
    if !is_name(name) {
        return Err(format!("'{name}' is not an XML element name"));
    }
    match tree.data(node) {
        Data::Array(_) => {
            for item in tree.children(node) {
                write_member(tree, name, item, indent, out)?;
            }
            Ok(())
        }
        _ => write_element(tree, name, node, indent, out),
    }
}

fn write_element(
    tree: &Tree,
    name: &str,
    node: u32,
    indent: usize,
    out: &mut ChunkedText<'_>,
) -> Result<(), String> {
    push_indent(indent, out);
    out.push('<');
    out.push_str(name);
    if !tree.data(node).is_table() {
        // A scalar: text content, or nothing.
        let mut text = String::new();
        push_scalar_text(tree, node, &mut text);
        if text.is_empty() {
            out.push_str("/>\n");
        } else {
            out.push('>');
            let mut escaped = String::with_capacity(text.len());
            escape_text(&mut escaped, &text);
            out.push_str(&escaped);
            out.push_str("</");
            out.push_str(name);
            out.push_str(">\n");
        }
        return Ok(());
    }
    let mut text: Option<String> = None;
    let mut children: Vec<u32> = Vec::new();
    for member in tree.children(node) {
        let key = tree.key(member);
        if let Some(attribute) = key.strip_prefix('@') {
            if !is_name(attribute) {
                return Err(format!("'{attribute}' is not an XML attribute name"));
            }
            if tree.data(member).is_container() {
                return Err(format!("attribute '{attribute}' must be a scalar"));
            }
            let mut raw = String::new();
            push_scalar_text(tree, member, &mut raw);
            let mut escaped = String::with_capacity(raw.len() + attribute.len() + 4);
            escaped.push(' ');
            escaped.push_str(attribute);
            escaped.push_str("=\"");
            escape_attribute(&mut escaped, &raw);
            escaped.push('"');
            out.push_str(&escaped);
        } else if key == "#text" {
            let mut raw = String::new();
            push_scalar_text(tree, member, &mut raw);
            text = Some(raw);
        } else {
            children.push(member);
        }
    }
    let text = text.filter(|text| !text.is_empty());
    match (text, children.is_empty()) {
        (None, true) => {
            out.push_str("/>\n");
        }
        (Some(text), true) => {
            out.push('>');
            let mut escaped = String::with_capacity(text.len());
            escape_text(&mut escaped, &text);
            out.push_str(&escaped);
            out.push_str("</");
            out.push_str(name);
            out.push_str(">\n");
        }
        (text, false) => {
            out.push_str(">\n");
            if let Some(text) = text {
                push_indent(indent + 2, out);
                let mut escaped = String::with_capacity(text.len());
                escape_text(&mut escaped, &text);
                out.push_str(&escaped);
                out.push('\n');
            }
            for child in children {
                write_member(tree, tree.key(child), child, indent + 2, out)?;
            }
            push_indent(indent, out);
            out.push_str("</");
            out.push_str(name);
            out.push_str(">\n");
        }
    }
    Ok(())
}

fn push_scalar_text(tree: &Tree, node: u32, out: &mut String) {
    use std::fmt::Write;
    match tree.data(node) {
        Data::Null => {}
        Data::Bool(flag) => out.push_str(if flag { "true" } else { "false" }),
        Data::Integer(number) => {
            let _ = write!(out, "{number}");
        }
        Data::Float(number) => {
            if number.is_nan() {
                out.push_str("NaN");
            } else if number.is_infinite() {
                out.push_str(if number > 0.0 { "INF" } else { "-INF" });
            } else {
                push_float(out, number);
            }
        }
        Data::Text(span) | Data::Datetime(span) => out.push_str(tree.str(span)),
        Data::Table(_) | Data::Array(_) => {}
    }
}

fn push_indent(indent: usize, out: &mut ChunkedText<'_>) {
    for _ in 0..indent {
        out.push(' ');
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::json::parse;

    fn render(json: &str) -> (String, Vec<String>) {
        let tree = parse(json.as_bytes()).unwrap();
        let mut bytes = Vec::new();
        let mut warnings = Vec::new();
        {
            let mut out = ChunkedText::new(&mut bytes);
            write_document(&tree, tree.root, &mut out, &mut warnings).unwrap();
            out.finish().unwrap();
        }
        (String::from_utf8(bytes).unwrap(), warnings)
    }

    #[test]
    fn attributes_text_and_repeated_children() {
        let (xml, warnings) = render(r##"{"r":{"@a":1,"k":["x",null],"#text":"t & u"}}"##);
        assert_eq!(
            xml,
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<r a=\"1\">\n  t &amp; u\n  <k>x</k>\n  <k/>\n</r>\n"
        );
        assert!(warnings.is_empty());
    }

    #[test]
    fn bad_names_are_refused() {
        let tree = parse(r#"{"1st":null}"#.as_bytes()).unwrap();
        let mut bytes = Vec::new();
        let mut out = ChunkedText::new(&mut bytes);
        assert!(write_document(&tree, tree.root, &mut out, &mut Vec::new()).is_err());
    }
}
