//! TOML writer from the value hub. A table's plain members come first,
//! then its sub-tables under `[a.b]` headers and its arrays of tables under
//! `[[a.b]]`, in document order. Arrays that are not all tables, and tables
//! inside them, are written inline. Nulls have no TOML form and are dropped;
//! every dropped path is reported in `losses`.

use std::fmt::Write;

use crate::value::{Value, push_double_quoted, push_float};

/// Writes `members` as a TOML document.
pub fn write_document(members: &[(String, Value)], out: &mut String, losses: &mut Vec<String>) {
    let mut path: Vec<&str> = Vec::new();
    write_table(members, &mut path, out, losses);
}

fn write_table<'v>(
    members: &'v [(String, Value)],
    path: &mut Vec<&'v str>,
    out: &mut String,
    losses: &mut Vec<String>,
) {
    for (key, value) in members {
        if is_deferred(value) {
            continue;
        }
        if let Value::Null = value {
            path.push(key);
            losses.push(path.join("."));
            path.pop();
            continue;
        }
        write_key(out, key);
        out.push_str(" = ");
        path.push(key);
        write_inline(value, path, out, losses);
        path.pop();
        out.push('\n');
    }
    for (key, value) in members {
        if !is_deferred(value) {
            continue;
        }
        path.push(key);
        match value {
            Value::Table(children) => {
                separate(out);
                write_header(out, path, "[", "]");
                write_table(children, path, out, losses);
            }
            Value::Array(items) => {
                for item in items {
                    if let Value::Table(children) = item {
                        separate(out);
                        write_header(out, path, "[[", "]]");
                        write_table(children, path, out, losses);
                    }
                }
            }
            _ => {}
        }
        path.pop();
    }
}

/// Sub-tables and arrays of tables are written after the plain members.
fn is_deferred(value: &Value) -> bool {
    match value {
        Value::Table(_) => true,
        Value::Array(items) => !items.is_empty() && items.iter().all(Value::is_table),
        _ => false,
    }
}

fn separate(out: &mut String) {
    if !out.is_empty() {
        out.push('\n');
    }
}

#[inline(never)]
fn write_header(out: &mut String, path: &[&str], open: &str, close: &str) {
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

fn write_key(out: &mut String, key: &str) {
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
fn write_inline<'v>(
    value: &'v Value,
    path: &mut Vec<&'v str>,
    out: &mut String,
    losses: &mut Vec<String>,
) {
    match value {
        Value::Null => losses.push(path.join(".")),
        Value::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
        Value::Integer(number) => {
            let _ = write!(out, "{number}");
        }
        Value::Float(number) => write_float(out, *number),
        Value::String(text) => write_string(out, text),
        Value::Datetime(text) => out.push_str(text),
        Value::Array(items) => {
            out.push('[');
            let mut written = 0;
            let mut index_text = String::new();
            for (index, item) in items.iter().enumerate() {
                if let Value::Null = item {
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
                write_inline(item, path, out, losses);
                written += 1;
            }
            out.push(']');
        }
        Value::Table(members) => {
            if members.is_empty() {
                out.push_str("{}");
                return;
            }
            let open_at = out.len();
            out.push_str("{ ");
            let mut written = 0;
            for (key, member) in members {
                path.push(key);
                if let Value::Null = member {
                    losses.push(path.join("."));
                    path.pop();
                    continue;
                }
                if written > 0 {
                    out.push_str(", ");
                }
                write_key(out, key);
                out.push_str(" = ");
                write_inline(member, path, out, losses);
                path.pop();
                written += 1;
            }
            if written == 0 {
                out.truncate(open_at);
                out.push_str("{}");
            } else {
                out.push_str(" }");
            }
        }
    }
}

fn write_float(out: &mut String, number: f64) {
    if number.is_nan() {
        out.push_str("nan");
    } else if number.is_infinite() {
        out.push_str(if number > 0.0 { "inf" } else { "-inf" });
    } else {
        push_float(out, number);
    }
}

fn write_string(out: &mut String, text: &str) {
    push_double_quoted(out, text);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(members: Vec<(&str, Value)>) -> (String, Vec<String>) {
        let owned: Vec<(String, Value)> = members
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect();
        let mut out = String::new();
        let mut losses = Vec::new();
        write_document(&owned, &mut out, &mut losses);
        (out, losses)
    }

    #[test]
    fn plain_members_come_before_sub_tables() {
        let (out, losses) = render(vec![
            (
                "sub",
                Value::Table(vec![("x".to_string(), Value::Integer(1))]),
            ),
            ("a", Value::String("s".to_string())),
        ]);
        assert_eq!(out, "a = \"s\"\n\n[sub]\nx = 1\n");
        assert!(losses.is_empty());
    }

    #[test]
    fn nulls_are_dropped_and_reported_by_path() {
        let (out, losses) = render(vec![
            ("gone", Value::Null),
            ("list", Value::Array(vec![Value::Integer(1), Value::Null])),
            (
                "inline",
                Value::Array(vec![
                    Value::Table(vec![("n".to_string(), Value::Null)]),
                    Value::Integer(2),
                ]),
            ),
        ]);
        assert_eq!(out, "list = [1]\ninline = [{}, 2]\n");
        assert_eq!(losses, vec!["gone", "list[1]", "inline.n"]);
    }
}
