//! TOML -> JSON. The document reads into the value hub and is written as
//! one JSON object; integers and floats stay numbers, dates, times,
//! infinities, and NaN become strings and are reported once per key path.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Fidelity, Input, Tier};
use crate::converters::input::read_text_document;
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::json::{JsonWriter, from_tree};
use crate::io::toml;

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
        let tree = toml::parse(&text)?;
        let mut writer = JsonWriter::new(output);
        from_tree::write_tree(&tree, tree.root, &mut writer, NAME, context)?;
        writer.flush()?;
        Ok(())
    }
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
