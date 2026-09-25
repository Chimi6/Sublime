//! TOML writer from the value tree. A table's plain members come first,
//! then its sub-tables under `[a.b]` headers and its arrays of tables under
//! `[[a.b]]`, in document order. Arrays that are not all tables, and tables
//! inside them, are written inline. Nulls have no TOML form and are dropped;
//! every dropped path is reported in `losses`.

use std::fmt::Write;

use crate::value::{ChunkedText, Data, Tree, push_double_quoted, push_float};

/// Writes the table at `root` as a TOML document.
pub fn write_document(tree: &Tree, root: u32, out: &mut ChunkedText<'_>, losses: &mut Vec<String>) {
    let mut path: Vec<&str> = Vec::new();
    write_table(tree, root, &mut path, out, losses);
}

fn write_table<'t>(
    tree: &'t Tree,
    table: u32,
    path: &mut Vec<&'t str>,
    out: &mut ChunkedText<'_>,
    losses: &mut Vec<String>,
) {
    for member in tree.children(table) {
        if is_deferred(tree, member) {
            continue;
        }
        let key = tree.key(member);
        if let Data::Null = tree.data(member) {
            path.push(key);
            losses.push(path.join("."));
            path.pop();
            continue;
        }
        write_key(out, key);
        out.push_str(" = ");
        path.push(key);
        write_inline(tree, member, path, out, losses);
        path.pop();
        out.push('\n');
    }
    for member in tree.children(table) {
        if !is_deferred(tree, member) {
            continue;
        }
        path.push(tree.key(member));
        match tree.data(member) {
            Data::Table(_) => {
                separate(out);
                write_header(out, path, "[", "]");
                write_table(tree, member, path, out, losses);
            }
            Data::Array(_) => {
                for item in tree.children(member) {
                    separate(out);
                    write_header(out, path, "[[", "]]");
                    write_table(tree, item, path, out, losses);
                }
            }
            _ => {}
        }
        path.pop();
    }
}

/// Sub-tables and arrays of tables are written after the plain members.
fn is_deferred(tree: &Tree, node: u32) -> bool {
    match tree.data(node) {
        Data::Table(_) => true,
        Data::Array(children) => {
            children.first != crate::value::NONE
                && tree.children(node).all(|item| tree.data(item).is_table())
        }
        _ => false,
    }
}

fn separate(out: &mut ChunkedText<'_>) {
    if !out.is_empty() {
        out.push('\n');
    }
}

#[inline(never)]
fn write_header(out: &mut ChunkedText<'_>, path: &[&str], open: &str, close: &str) {
    out.push_str(open);
    for (index, key) in path.iter().enumerate() {
        if index > 0 {
            out.push('.');
        }
        write_key(out, key);
    }
    out.push_str(close);
    out.push('\n');
}

fn write_key(out: &mut ChunkedText<'_>, key: &str) {
    let is_bare = !key.is_empty()
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-');
    if is_bare {
        out.push_str(key);
    } else {
        write_string(out, key);
    }
}

#[inline(never)]
fn write_inline<'t>(
    tree: &'t Tree,
    node: u32,
    path: &mut Vec<&'t str>,
    out: &mut ChunkedText<'_>,
    losses: &mut Vec<String>,
) {
    match tree.data(node) {
        Data::Null => losses.push(path.join(".")),
        Data::Bool(flag) => out.push_str(if flag { "true" } else { "false" }),
        Data::Integer(number) => {
            let _ = write!(out, "{number}");
        }
        Data::Float(number) => write_float(out, number),
        Data::Text(span) => write_string(out, tree.str(span)),
        Data::Datetime(span) => out.push_str(tree.str(span)),
        Data::Array(_) => {
            out.push('[');
            let mut written = 0;
            let mut index_text = String::new();
            for (index, item) in tree.children(node).enumerate() {
                if let Data::Null = tree.data(item) {
                    index_text.clear();
                    let _ = write!(index_text, "[{index}]");
                    let mut dropped = path.join(".");
                    dropped.push_str(&index_text);
                    losses.push(dropped);
                    continue;
                }
                if written > 0 {
                    out.push_str(", ");
                }
                write_inline(tree, item, path, out, losses);
                written += 1;
            }
            out.push(']');
        }
        Data::Table(_) => {
            let nothing_to_write = tree
                .children(node)
                .all(|member| matches!(tree.data(member), Data::Null));
            if nothing_to_write {
                for member in tree.children(node) {
                    path.push(tree.key(member));
                    losses.push(path.join("."));
                    path.pop();
                }
                out.push_str("{}");
                return;
            }
            out.push_str("{ ");
            let mut written = 0;
            for member in tree.children(node) {
                let key = tree.key(member);
                path.push(key);
                if let Data::Null = tree.data(member) {
                    losses.push(path.join("."));
                    path.pop();
                    continue;
                }
                if written > 0 {
                    out.push_str(", ");
                }
                write_key(out, key);
                out.push_str(" = ");
                write_inline(tree, member, path, out, losses);
                path.pop();
                written += 1;
            }
            out.push_str(" }");
        }
    }
}

fn write_float(out: &mut ChunkedText<'_>, number: f64) {
    if number.is_nan() {
        out.push_str("nan");
    } else if number.is_infinite() {
        out.push_str(if number > 0.0 { "inf" } else { "-inf" });
    } else {
        let mut text = String::new();
        push_float(&mut text, number);
        out.push_str(&text);
    }
}

fn write_string(out: &mut ChunkedText<'_>, text: &str) {
    let mut quoted = String::with_capacity(text.len() + 2);
    push_double_quoted(&mut quoted, text);
    out.push_str(&quoted);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::json::parse;

    fn render(json: &str) -> (String, Vec<String>) {
        let tree = parse(json.as_bytes()).unwrap();
        let mut bytes = Vec::new();
        let mut losses = Vec::new();
        {
            let mut out = ChunkedText::new(&mut bytes);
            write_document(&tree, tree.root, &mut out, &mut losses);
            out.finish().unwrap();
        }
        (String::from_utf8(bytes).unwrap(), losses)
    }

    #[test]
    fn plain_members_come_before_sub_tables() {
        let (out, losses) = render(r#"{"sub":{"x":1},"a":"s"}"#);
        assert_eq!(out, "a = \"s\"\n\n[sub]\nx = 1\n");
        assert!(losses.is_empty());
    }

    #[test]
    fn nulls_are_dropped_and_reported_by_path() {
        let (out, losses) = render(r#"{"gone":null,"list":[1,null],"inline":[{"n":null},2]}"#);
        assert_eq!(out, "list = [1]\ninline = [{}, 2]\n");
        assert_eq!(losses, vec!["gone", "list[1]", "inline.n"]);
    }
}
