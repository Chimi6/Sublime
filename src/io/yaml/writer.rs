//! YAML writer from the value tree, block style: `key: value` mappings,
//! `- item` sequences indented two under their key, compact mappings on a
//! dash line, empty collections as `{}` and `[]`, strings plain when the
//! core schema would read them back unchanged and double-quoted otherwise,
//! multi-line strings as literal blocks.

use std::fmt::Write;

use crate::io::yaml::reader::{Plain, resolve_plain};
use crate::value::{ChunkedText, Data, NONE, Tree, push_double_quoted, push_float};

/// Writes the subtree at `root` as one document (no `---`).
pub fn write_document(tree: &Tree, root: u32, out: &mut ChunkedText<'_>) {
    match tree.data(root) {
        Data::Table(children) if children.first != NONE => write_mapping(tree, root, 0, out),
        Data::Array(children) if children.first != NONE => write_sequence(tree, root, 0, out),
        Data::Text(span) if tree.str(span).contains('\n') => {
            write_literal_block(tree.str(span), 2, out);
        }
        _ => {
            write_scalar(tree, root, out);
            out.push('\n');
        }
    }
}

fn write_mapping(tree: &Tree, table: u32, indent: usize, out: &mut ChunkedText<'_>) {
    for (index, member) in tree.children(table).enumerate() {
        if index > 0 {
            push_indent(indent, out);
        }
        write_key(tree.key(member), out);
        out.push(':');
        write_member_value(tree, member, indent, out);
    }
}

/// After `key:`: a scalar on the line, or a collection below.
fn write_member_value(tree: &Tree, node: u32, indent: usize, out: &mut ChunkedText<'_>) {
    match tree.data(node) {
        Data::Table(children) if children.first != NONE => {
            out.push('\n');
            push_indent(indent + 2, out);
            write_mapping(tree, node, indent + 2, out);
        }
        Data::Array(children) if children.first != NONE => {
            out.push('\n');
            push_indent(indent + 2, out);
            write_sequence(tree, node, indent + 2, out);
        }
        Data::Text(span) if is_literal_block_candidate(tree.str(span)) => {
            out.push(' ');
            write_literal_block(tree.str(span), indent + 2, out);
        }
        _ => {
            out.push(' ');
            write_scalar(tree, node, out);
            out.push('\n');
        }
    }
}

fn write_sequence(tree: &Tree, array: u32, indent: usize, out: &mut ChunkedText<'_>) {
    for (index, item) in tree.children(array).enumerate() {
        if index > 0 {
            push_indent(indent, out);
        }
        out.push_str("- ");
        match tree.data(item) {
            Data::Table(children) if children.first != NONE => {
                write_mapping(tree, item, indent + 2, out);
            }
            Data::Array(children) if children.first != NONE => {
                write_sequence(tree, item, indent + 2, out);
            }
            Data::Text(span) if is_literal_block_candidate(tree.str(span)) => {
                write_literal_block(tree.str(span), indent + 2, out);
            }
            _ => {
                write_scalar(tree, item, out);
                out.push('\n');
            }
        }
    }
}

fn push_indent(indent: usize, out: &mut ChunkedText<'_>) {
    for _ in 0..indent {
        out.push(' ');
    }
}

fn write_key(key: &str, out: &mut ChunkedText<'_>) {
    if is_plain_safe(key) {
        out.push_str(key);
    } else {
        write_double_quoted(key, out);
    }
}

fn write_scalar(tree: &Tree, node: u32, out: &mut ChunkedText<'_>) {
    match tree.data(node) {
        Data::Null => out.push_str("null"),
        Data::Bool(flag) => out.push_str(if flag { "true" } else { "false" }),
        Data::Integer(number) => {
            let _ = write!(out, "{number}");
        }
        Data::Float(number) => {
            if number.is_nan() {
                out.push_str(".nan");
            } else if number.is_infinite() {
                out.push_str(if number > 0.0 { ".inf" } else { "-.inf" });
            } else {
                let mut text = String::new();
                push_float(&mut text, number);
                out.push_str(&text);
            }
        }
        Data::Text(span) => {
            let text = tree.str(span);
            if is_plain_safe(text) {
                out.push_str(text);
            } else {
                write_double_quoted(text, out);
            }
        }
        Data::Datetime(span) => out.push_str(tree.str(span)),
        Data::Table(_) => out.push_str("{}"),
        Data::Array(_) => out.push_str("[]"),
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
fn write_literal_block(text: &str, indent: usize, out: &mut ChunkedText<'_>) {
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
    if text.contains('_') && resolve_plain(&text.replace('_', "")) != Plain::Str {
        return false;
    }
    resolve_plain(text) == Plain::Str
}

fn write_double_quoted(text: &str, out: &mut ChunkedText<'_>) {
    let mut quoted = String::with_capacity(text.len() + 2);
    push_double_quoted(&mut quoted, text);
    out.push_str(&quoted);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::json::parse;

    fn render(json: &str) -> String {
        let tree = parse(json.as_bytes()).unwrap();
        let mut bytes = Vec::new();
        {
            let mut out = ChunkedText::new(&mut bytes);
            write_document(&tree, tree.root, &mut out);
            out.finish().unwrap();
        }
        String::from_utf8(bytes).unwrap()
    }

    #[test]
    fn strings_that_would_resolve_are_quoted() {
        assert_eq!(
            render(r#"{"a":"true","b":"1e3","c":"plain text","d":""}"#),
            "a: \"true\"\nb: \"1e3\"\nc: plain text\nd: \"\"\n"
        );
    }

    #[test]
    fn nested_collections_indent_under_their_key() {
        assert_eq!(
            render(r#"{"list":[{"a":1,"b":2},[3]]}"#),
            "list:\n  - a: 1\n    b: 2\n  - - 3\n"
        );
    }

    #[test]
    fn multi_line_strings_become_literal_blocks() {
        assert_eq!(
            render(r#"{"keep":"a\nb\n\n","strip":"a\nb"}"#),
            "keep: |+\n  a\n  b\n\nstrip: |-\n  a\n  b\n"
        );
    }
}
