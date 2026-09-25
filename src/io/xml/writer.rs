//! XML from the value hub, the inverse of `tree.rs`: a table is an
//! element, `@name` members its attributes, `#text` its text, other
//! members child elements, arrays repeated elements, scalars text. Pretty
//! printed with two-space indentation, leaf elements on one line.

use crate::io::xml::tree::is_name;
use crate::io::xml::{escape_attribute, escape_text};
use crate::value::{ChunkedText, Value, push_float};

/// Writes `value` as a document. The root object's one member is the root
/// element; anything else is wrapped in `<root>` with a warning. Keys that
/// are not XML names and attributes that are not scalars are errors.
pub fn write_document(
    value: &Value,
    out: &mut ChunkedText<'_>,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    match value {
        Value::Table(members) if members.len() == 1 && !members[0].0.starts_with(['@', '#']) => {
            let (name, node) = &members[0];
            write_member(name, node, 0, out)?;
        }
        Value::Table(_) => {
            warnings.push("the JSON root has no single member; wrapped in <root>".to_string());
            write_element("root", value, 0, out)?;
        }
        Value::Array(items) => {
            warnings.push(
                "the JSON root is an array; wrapped in <root> as <item> elements".to_string(),
            );
            let wrapped = Value::Table(vec![("item".to_string(), Value::Array(items.clone()))]);
            write_element("root", &wrapped, 0, out)?;
        }
        scalar => {
            warnings.push("the JSON root is a scalar; wrapped in <root>".to_string());
            write_element("root", scalar, 0, out)?;
        }
    }
    Ok(())
}

/// A member of a table: one element, or one per item of an array.
fn write_member(
    name: &str,
    value: &Value,
    indent: usize,
    out: &mut ChunkedText<'_>,
) -> Result<(), String> {
    if !is_name(name) {
        return Err(format!("'{name}' is not an XML element name"));
    }
    match value {
        Value::Array(items) => {
            for item in items {
                write_member(name, item, indent, out)?;
            }
            Ok(())
        }
        other => write_element(name, other, indent, out),
    }
}

fn write_element(
    name: &str,
    value: &Value,
    indent: usize,
    out: &mut ChunkedText<'_>,
) -> Result<(), String> {
    push_indent(indent, out);
    out.push('<');
    out.push_str(name);
    let Value::Table(members) = value else {
        // A scalar: text content, or nothing.
        let mut text = String::new();
        push_scalar_text(value, &mut text);
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
    };
    let mut text: Option<String> = None;
    let mut children: Vec<(&str, &Value)> = Vec::new();
    for (key, member) in members {
        if let Some(attribute) = key.strip_prefix('@') {
            if !is_name(attribute) {
                return Err(format!("'{attribute}' is not an XML attribute name"));
            }
            if matches!(member, Value::Table(_) | Value::Array(_)) {
                return Err(format!("attribute '{attribute}' must be a scalar"));
            }
            let mut raw = String::new();
            push_scalar_text(member, &mut raw);
            let mut escaped = String::with_capacity(raw.len() + attribute.len() + 4);
            escaped.push(' ');
            escaped.push_str(attribute);
            escaped.push_str("=\"");
            escape_attribute(&mut escaped, &raw);
            escaped.push('"');
            out.push_str(&escaped);
        } else if key == "#text" {
            let mut raw = String::new();
            push_scalar_text(member, &mut raw);
            text = Some(raw);
        } else {
            children.push((key, member));
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
            for (key, child) in children {
                write_member(key, child, indent + 2, out)?;
            }
            push_indent(indent, out);
            out.push_str("</");
            out.push_str(name);
            out.push_str(">\n");
        }
    }
    Ok(())
}

fn push_scalar_text(value: &Value, out: &mut String) {
    use std::fmt::Write;
    match value {
        Value::Null => {}
        Value::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
        Value::Integer(number) => {
            let _ = write!(out, "{number}");
        }
        Value::Float(number) => {
            if number.is_nan() {
                out.push_str("NaN");
            } else if number.is_infinite() {
                out.push_str(if *number > 0.0 { "INF" } else { "-INF" });
            } else {
                push_float(out, *number);
            }
        }
        Value::String(text) | Value::Datetime(text) => out.push_str(text),
        Value::Table(_) | Value::Array(_) => {}
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

    fn render(value: Value) -> (String, Vec<String>) {
        let mut bytes = Vec::new();
        let mut warnings = Vec::new();
        {
            let mut out = ChunkedText::new(&mut bytes);
            write_document(&value, &mut out, &mut warnings).unwrap();
            out.finish().unwrap();
        }
        (String::from_utf8(bytes).unwrap(), warnings)
    }

    #[test]
    fn attributes_text_and_repeated_children() {
        let value = Value::Table(vec![(
            "r".to_string(),
            Value::Table(vec![
                ("@a".to_string(), Value::Integer(1)),
                (
                    "k".to_string(),
                    Value::Array(vec![Value::String("x".to_string()), Value::Null]),
                ),
                ("#text".to_string(), Value::String("t & u".to_string())),
            ]),
        )]);
        let (xml, warnings) = render(value);
        assert_eq!(
            xml,
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<r a=\"1\">\n  t &amp; u\n  <k>x</k>\n  <k/>\n</r>\n"
        );
        assert!(warnings.is_empty());
    }

    #[test]
    fn bad_names_are_refused() {
        let value = Value::Table(vec![("1st".to_string(), Value::Null)]);
        let mut bytes = Vec::new();
        let mut out = ChunkedText::new(&mut bytes);
        assert!(write_document(&value, &mut out, &mut Vec::new()).is_err());
    }
}
