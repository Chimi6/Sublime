//! YAML writer from the value hub, block style: `key: value` mappings,
//! `- item` sequences indented two under their key, compact mappings on a
//! dash line, empty collections as `{}` and `[]`, strings plain when the
//! core schema would read them back unchanged and double-quoted otherwise,
//! multi-line strings as literal blocks.

use std::fmt::Write;

use crate::io::yaml::reader::resolve_plain;
use crate::value::{Value, push_double_quoted, push_float};

/// Writes one document (no `---`).
pub fn write_document(value: &Value, out: &mut String) {
    match value {
        Value::Table(members) if !members.is_empty() => write_mapping(members, 0, out),
        Value::Array(items) if !items.is_empty() => write_sequence(items, 0, out),
        Value::String(text) if text.contains('\n') => {
            write_literal_block(text, 2, out);
        }
        scalar => {
            write_scalar(scalar, out);
            out.push('\n');
        }
    }
}

fn write_mapping(members: &[(String, Value)], indent: usize, out: &mut String) {
    for (index, (key, value)) in members.iter().enumerate() {
        if index > 0 {
            push_indent(indent, out);
        }
        write_key(key, out);
        out.push(':');
        write_member_value(value, indent, out);
    }
}

/// After `key:`: a scalar on the line, or a collection below.
fn write_member_value(value: &Value, indent: usize, out: &mut String) {
    match value {
        Value::Table(members) if !members.is_empty() => {
            out.push('\n');
            push_indent(indent + 2, out);
            write_mapping(members, indent + 2, out);
        }
        Value::Array(items) if !items.is_empty() => {
            out.push('\n');
            push_indent(indent + 2, out);
            write_sequence(items, indent + 2, out);
        }
        Value::String(text) if is_literal_block_candidate(text) => {
            out.push(' ');
            write_literal_block(text, indent + 2, out);
        }
        scalar => {
            out.push(' ');
            write_scalar(scalar, out);
            out.push('\n');
        }
    }
}

fn write_sequence(items: &[Value], indent: usize, out: &mut String) {
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            push_indent(indent, out);
        }
        out.push('-');
        match item {
            Value::Table(members) if !members.is_empty() => {
                out.push(' ');
                write_mapping(members, indent + 2, out);
            }
            Value::Array(inner) if !inner.is_empty() => {
                out.push(' ');
                write_sequence(inner, indent + 2, out);
            }
            Value::String(text) if is_literal_block_candidate(text) => {
                out.push(' ');
                write_literal_block(text, indent + 2, out);
            }
            scalar => {
                out.push(' ');
                write_scalar(scalar, out);
                out.push('\n');
            }
        }
    }
}

fn push_indent(indent: usize, out: &mut String) {
    for _ in 0..indent {
        out.push(' ');
    }
}

fn write_key(key: &str, out: &mut String) {
    if is_plain_safe(key) {
        out.push_str(key);
    } else {
        write_double_quoted(key, out);
    }
}

fn write_scalar(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
        Value::Integer(number) => {
            let _ = write!(out, "{number}");
        }
        Value::Float(number) => {
            if number.is_nan() {
                out.push_str(".nan");
            } else if number.is_infinite() {
                out.push_str(if *number > 0.0 { ".inf" } else { "-.inf" });
            } else {
                push_float(out, *number);
            }
        }
        Value::String(text) => {
            if is_plain_safe(text) {
                out.push_str(text);
            } else {
                write_double_quoted(text, out);
            }
        }
        Value::Datetime(text) => out.push_str(text),
        Value::Table(_) => out.push_str("{}"),
        Value::Array(_) => out.push_str("[]"),
    }
}

/// A literal block keeps a multi-line string readable; it needs a first
/// line that does not start with whitespace and no control characters
/// other than the newline.
fn is_literal_block_candidate(text: &str) -> bool {
    text.contains('\n')
        && !text.starts_with([' ', '\t', '\n'])
        && !text
            .chars()
            .any(|character| character.is_control() && character != '\n')
}

/// `|`, `|-`, or `|+` by the trailing newlines, then the lines indented.
fn write_literal_block(text: &str, indent: usize, out: &mut String) {
    let trailing = text.len() - text.trim_end_matches('\n').len();
    let body = &text[..text.len() - trailing];
    out.push('|');
    match trailing {
        0 => out.push('-'),
        1 => {}
        _ => out.push('+'),
    }
    out.push('\n');
    for line in body.split('\n') {
        if !line.is_empty() {
            push_indent(indent, out);
            out.push_str(line);
        }
        out.push('\n');
    }
    for _ in 1..trailing {
        out.push('\n');
    }
}

/// True when the core schema reads the text back as the same string.
fn is_plain_safe(text: &str) -> bool {
    let Some(first) = text.chars().next() else {
        return false;
    };
    if "-?:,[]{}#&*!|>'\"%@`".contains(first) || first.is_whitespace() {
        return false;
    }
    if text.ends_with([' ', '\t']) || text.ends_with(':') {
        return false;
    }
    if text.contains(": ") || text.contains(" #") || text.contains('\t') {
        return false;
    }
    if text.chars().any(|character| character.is_control()) {
        return false;
    }
    if text == "---" || text == "..." {
        return false;
    }
    // YAML 1.1 readers take these as booleans and numbers; quoting them
    // costs nothing and keeps the string a string everywhere.
    let lowered = text.to_ascii_lowercase();
    if matches!(lowered.as_str(), "yes" | "no" | "on" | "off") {
        return false;
    }
    if text.contains('_') && !matches!(resolve_plain(&text.replace('_', "")), Value::String(_)) {
        return false;
    }
    matches!(resolve_plain(text), Value::String(_))
}

fn write_double_quoted(text: &str, out: &mut String) {
    push_double_quoted(out, text);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(value: Value) -> String {
        let mut out = String::new();
        write_document(&value, &mut out);
        out
    }

    #[test]
    fn strings_that_would_resolve_are_quoted() {
        let value = Value::Table(vec![
            ("a".to_string(), Value::String("true".to_string())),
            ("b".to_string(), Value::String("1e3".to_string())),
            ("c".to_string(), Value::String("plain text".to_string())),
            ("d".to_string(), Value::String("".to_string())),
        ]);
        assert_eq!(
            render(value),
            "a: \"true\"\nb: \"1e3\"\nc: plain text\nd: \"\"\n"
        );
    }

    #[test]
    fn nested_collections_indent_under_their_key() {
        let value = Value::Table(vec![(
            "list".to_string(),
            Value::Array(vec![
                Value::Table(vec![
                    ("a".to_string(), Value::Integer(1)),
                    ("b".to_string(), Value::Integer(2)),
                ]),
                Value::Array(vec![Value::Integer(3)]),
            ]),
        )]);
        assert_eq!(render(value), "list:\n  - a: 1\n    b: 2\n  - - 3\n");
    }

    #[test]
    fn multi_line_strings_become_literal_blocks() {
        let value = Value::Table(vec![
            ("keep".to_string(), Value::String("a\nb\n\n".to_string())),
            ("strip".to_string(), Value::String("a\nb".to_string())),
        ]);
        assert_eq!(render(value), "keep: |+\n  a\n  b\n\nstrip: |-\n  a\n  b\n");
    }
}
