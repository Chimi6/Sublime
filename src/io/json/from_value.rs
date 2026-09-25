//! The value hub written as JSON. Integers and floats stay numbers; dates,
//! times, infinities, and NaN become strings and are reported once per key
//! path with array indices elided (`record[].created`), so a column of
//! dates is one loss, not one per row.

use std::io::Write;

use crate::converter::{ConvertError, Location};
use crate::event::Context;
use crate::io::json::JsonWriter;
use crate::value::{Value, push_float};

/// Writes `value` and reports its losses under `converter`'s name.
pub fn write_value(
    value: &Value,
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
    walk(value, writer, &mut state, Some(context))
}

/// The value as compact JSON text, nothing reported (for keys that are
/// collections, and tagged values that must become text).
pub fn compact_text(value: &Value) -> String {
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
        let _ = walk(value, &mut writer, &mut state, None);
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
    value: &Value,
    writer: &mut JsonWriter<&mut dyn Write>,
    state: &mut State,
    mut context: Option<&mut Context<'_>>,
) -> Result<(), ConvertError> {
    match value {
        Value::Null => writer.null()?,
        Value::Bool(flag) => writer.raw(if *flag { "true" } else { "false" })?,
        Value::Integer(number) => {
            state.scratch.clear();
            use std::fmt::Write as _;
            let _ = write!(state.scratch, "{number}");
            writer.raw(&state.scratch)?;
        }
        Value::Float(number) => {
            if number.is_finite() {
                state.scratch.clear();
                push_float(&mut state.scratch, *number);
                writer.raw(&state.scratch)?;
            } else {
                let text = if number.is_nan() {
                    "nan"
                } else if *number > 0.0 {
                    "inf"
                } else {
                    "-inf"
                };
                writer.string(text)?;
                report(
                    state,
                    context,
                    format!("{text} has no JSON form, written as a string"),
                );
            }
        }
        Value::String(text) => writer.string(text)?,
        Value::Datetime(text) => {
            writer.string(text)?;
            report(
                state,
                context,
                "dates and times written as strings".to_string(),
            );
        }
        Value::Array(items) => {
            writer.begin_array()?;
            let path_length = state.path.len();
            for item in items {
                state.path.push_str("[]");
                walk(item, writer, state, context.as_deref_mut())?;
                state.path.truncate(path_length);
            }
            writer.end_array()?;
        }
        Value::Table(members) => {
            writer.begin_object()?;
            let path_length = state.path.len();
            for (key, member) in members {
                writer.key(key)?;
                if !state.path.is_empty() {
                    state.path.push('.');
                }
                state.path.push_str(key);
                walk(member, writer, state, context.as_deref_mut())?;
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
    let already = state.reported.contains(&state.path);
    if already {
        return;
    }
    state.reported.push(state.path.clone());
    context.loss(
        state.converter,
        Location::default(),
        format!("{}: {what}", state.path),
    );
}
