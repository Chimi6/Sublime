//! JSON -> TOML. The document reads into the value hub and is written as a
//! TOML document; the root must be an object, and nulls, which TOML cannot
//! express, are dropped and reported.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Fidelity, Input, Location, Tier};
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::json;
use crate::io::toml;
use crate::value::{ChunkedText, Value};

const NAME: &str = "json-to-toml";
const FIDELITY_NOTE: &str =
    "the root must be an object; nulls are dropped; integers beyond 64 bits become floats";

pub struct JsonToToml;

impl Converter for JsonToToml {
    fn name(&self) -> &'static str {
        NAME
    }

    fn from(&self) -> &'static Format {
        &formats::JSON
    }

    fn to(&self) -> &'static Format {
        &formats::TOML
    }

    fn fidelity(&self) -> Fidelity {
        Fidelity::Conditional(FIDELITY_NOTE)
    }

    fn tier(&self) -> Tier {
        Tier::Native
    }

    fn convert(
        &self,
        input: Input<'_>,
        output: &mut dyn Write,
        context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let document = json::parse(input)?;
        let members = match document {
            Value::Table(members) => members,
            Value::Array(_) => return Err(not_a_table("an array")),
            _ => return Err(not_a_table("a scalar")),
        };
        let mut losses = Vec::new();
        let mut text = ChunkedText::new(output);
        toml::write_document(&members, &mut text, &mut losses);
        text.finish()?;
        for path in losses {
            context.loss(
                NAME,
                Location::default(),
                format!("{path}: null has no TOML form, dropped"),
            );
        }
        Ok(())
    }
}

fn not_a_table(what: &str) -> ConvertError {
    ConvertError::Unsupported(format!(
        "a TOML document is a table; the JSON root is {what}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declares_contract() {
        assert_eq!(JsonToToml.name(), "json-to-toml");
        assert_eq!(JsonToToml.from().id, "json");
        assert_eq!(JsonToToml.to().id, "toml");
        assert_eq!(JsonToToml.fidelity(), Fidelity::Conditional(FIDELITY_NOTE));
    }
}
