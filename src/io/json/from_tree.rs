//! The value tree written as JSON. Integers and floats stay numbers; dates,
//! times, infinities, and NaN become strings and are reported once per key
//! path with array indices elided (`record[].created`), so a column of
//! dates is one loss, not one per row.

use std::io::Write;

use crate::converter::{ConvertError, Location};
use crate::event::Context;
use crate::io::json::JsonWriter;
use crate::value::{Data, Tree, push_float};

/// Writes the subtree at `node` and reports its losses under `converter`'s name.
pub fn write_tree(
    tree: &Tree,
    node: u32,
    writer: &mut JsonWriter<&mut dyn Write>,
    converter: &'static str,
    context: &mut Context<'_>,
) -> Result<(), ConvertError> {
    let mut state = State {
        converter,
        path: String::new(),
        scratch: String::new(),
        reported: Vec::new(),
    };
    walk(tree, node, writer, &mut state, Some(context))
}

/// The subtree as compact JSON text, nothing reported (for keys that are
/// collections, tagged values that must become text, and tests).
pub fn compact_text(tree: &Tree, node: u32) -> String {
    let mut bytes: Vec<u8> = Vec::new();
    {
        let sink: &mut dyn Write = &mut bytes;
        let mut writer = JsonWriter::new(sink);
        let mut state = State {
            converter: "",
            path: String::new(),
            scratch: String::new(),
            reported: Vec::new(),
        };
        let _ = walk(tree, node, &mut writer, &mut state, None);
        let _ = writer.flush();
    }
    String::from_utf8(bytes).unwrap_or_default()
}

struct State {
    converter: &'static str,
    path: String,
    scratch: String,
    reported: Vec<String>,
}

fn walk(
    tree: &Tree,
    node: u32,
    writer: &mut JsonWriter<&mut dyn Write>,
    state: &mut State,
    mut context: Option<&mut Context<'_>>,
) -> Result<(), ConvertError> {
    match tree.data(node) {
        Data::Null => writer.null()?,
        Data::Bool(flag) => writer.raw(if flag { "true" } else { "false" })?,
        Data::Integer(number) => {
            state.scratch.clear();
            use std::fmt::Write as _;
            let _ = write!(state.scratch, "{number}");
            writer.raw(&state.scratch)?;
        }
        Data::Float(number) => {
            if number.is_finite() {
                state.scratch.clear();
                push_float(&mut state.scratch, number);
                writer.raw(&state.scratch)?;
            } else {
                let text = if number.is_nan() {
                    "nan"
                } else if number > 0.0 {
                    "inf"
                } else {
                    "-inf"
                };
                writer.string(text)?;
                report(
                    state,
                    context.as_deref_mut(),
                    format!("{text} has no JSON form, written as a string"),
                );
            }
        }
        Data::Text(span) => writer.string(tree.str(span))?,
        Data::Datetime(span) => {
            writer.string(tree.str(span))?;
            report(
                state,
                context.as_deref_mut(),
                "dates and times written as strings".to_string(),
            );
        }
        Data::Array(_) => {
            writer.begin_array()?;
            let path_length = state.path.len();
            for item in tree.children(node) {
                state.path.push_str("[]");
                walk(tree, item, writer, state, context.as_deref_mut())?;
                state.path.truncate(path_length);
            }
            writer.end_array()?;
        }
        Data::Table(_) => {
            writer.begin_object()?;
            let path_length = state.path.len();
            for member in tree.children(node) {
                let key = tree.key(member);
                writer.key(key)?;
                if !state.path.is_empty() {
                    state.path.push('.');
                }
                state.path.push_str(key);
                walk(tree, member, writer, state, context.as_deref_mut())?;
                state.path.truncate(path_length);
            }
            writer.end_object()?;
        }
    }
    Ok(())
}

/// One loss per key path: the first date in a column of dates reports, the
/// rest are the same fact.
fn report(state: &mut State, context: Option<&mut Context<'_>>, what: String) {
    let Some(context) = context else {
        return;
    };
    if state.reported.contains(&state.path) {
        return;
    }
    state.reported.push(state.path.clone());
    context.loss(
        state.converter,
        Location::default(),
        format!("{}: {what}", state.path),
    );
}
