//! TOML -> JSON. The document reads into the value hub and is written as
//! one JSON object; integers and floats stay numbers, dates, times,
//! infinities, and NaN become strings and are reported once per key path
//! (array indices elided, so a column of dates is one loss, not one per
//! row).

use std::io::Write;

use crate::converter::{ConvertError, Converter, Fidelity, Input, Location, Tier};
use crate::converters::input::read_text_document;
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::json::JsonWriter;
use crate::io::toml;
use crate::value::{Value, push_float};

const NAME: &str = "toml-to-json";
const FIDELITY_NOTE: &str = "dates, times, infinities, and NaN become strings";

pub struct TomlToJson;

impl Converter for TomlToJson {
    fn name(&self) -> &'static str {
        NAME
    }

    fn from(&self) -> &'static Format {
        &formats::TOML
    }

    fn to(&self) -> &'static Format {
        &formats::JSON
    }

    fn fidelity(&self) -> Fidelity {
        Fidelity::Conditional(FIDELITY_NOTE)
    }

    fn tier(&self) -> Tier {
        Tier::Native
    }

    fn convert(
        &self,
        mut input: Input<'_>,
        output: &mut dyn Write,
        context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let text = read_text_document(&mut input)?;
        let document = toml::parse(&text)?;
        let mut writer = JsonWriter::new(output);
        let mut path = String::new();
        let mut scratch = String::new();
        let mut reported: Vec<String> = Vec::new();
        write_value(
            &document,
            &mut writer,
            &mut path,
            &mut scratch,
            &mut reported,
            context,
        )?;
        writer.flush()?;
        Ok(())
    }
}

fn write_value(
    value: &Value,
    writer: &mut JsonWriter<&mut dyn Write>,
    path: &mut String,
    scratch: &mut String,
    reported: &mut Vec<String>,
    context: &mut Context<'_>,
) -> Result<(), ConvertError> {
    match value {
        Value::Null => writer.null()?,
        Value::Bool(flag) => writer.raw(if *flag { "true" } else { "false" })?,
        Value::Integer(number) => {
            scratch.clear();
            use std::fmt::Write as _;
            let _ = write!(scratch, "{number}");
            writer.raw(scratch)?;
        }
        Value::Float(number) => {
            if number.is_finite() {
                scratch.clear();
                push_float(scratch, *number);
                writer.raw(scratch)?;
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
                    context,
                    reported,
                    path,
                    format!("{text} has no JSON form, written as a string"),
                );
            }
        }
        Value::String(text) => writer.string(text)?,
        Value::Datetime(text) => {
            writer.string(text)?;
            report(
                context,
                reported,
                path,
                "dates and times written as strings".to_string(),
            );
        }
        Value::Array(items) => {
            writer.begin_array()?;
            let path_length = path.len();
            for item in items {
                path.push_str("[]");
                write_value(item, writer, path, scratch, reported, context)?;
                path.truncate(path_length);
            }
            writer.end_array()?;
        }
        Value::Table(members) => {
            writer.begin_object()?;
            let path_length = path.len();
            for (key, member) in members {
                writer.key(key)?;
                if !path.is_empty() {
                    path.push('.');
                }
                path.push_str(key);
                write_value(member, writer, path, scratch, reported, context)?;
                path.truncate(path_length);
            }
            writer.end_object()?;
        }
    }
    Ok(())
}

/// One loss per key path: the first date in a column of dates reports, the
/// rest are the same fact.
fn report(context: &mut Context<'_>, reported: &mut Vec<String>, path: &str, what: String) {
    let already = reported.iter().any(|seen| seen == path);
    if already {
        return;
    }
    reported.push(path.to_string());
    context.loss(NAME, Location::default(), format!("{path}: {what}"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declares_contract() {
        assert_eq!(TomlToJson.name(), "toml-to-json");
        assert_eq!(TomlToJson.from().id, "toml");
        assert_eq!(TomlToJson.to().id, "json");
        assert_eq!(TomlToJson.fidelity(), Fidelity::Conditional(FIDELITY_NOTE));
    }
}
