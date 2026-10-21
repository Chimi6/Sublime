//! TOML -> JSON. The document reads into the value hub and is written as
//! one JSON object; integers and floats stay numbers, dates, times,
//! infinities, and NaN become strings and are reported once per key path.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Entry, Fidelity, Input, Tier};
use crate::converters::input::read_text_document;
use crate::event::Context;
use crate::format::formats;
use crate::io::json::{JsonWriter, from_tree};
use crate::io::toml;

const NAME: &str = "toml-to-json";
const FIDELITY_NOTE: &str = "dates, times, infinities, and NaN become strings";

pub struct TomlToJson;

impl Converter for TomlToJson {
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

impl TomlToJson {
    pub const fn entry(&'static self) -> Entry {
        Entry {
            name: NAME,
            from: &formats::TOML,
            to: &formats::JSON,
            fidelity: Fidelity::Conditional(FIDELITY_NOTE),
            tier: Tier::Native,
            converter: self,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declares_contract() {
        assert_eq!(TomlToJson.entry().name, "toml-to-json");
        assert_eq!(TomlToJson.entry().from.id, "toml");
        assert_eq!(TomlToJson.entry().to.id, "json");
        assert_eq!(
            TomlToJson.entry().fidelity,
            Fidelity::Conditional(FIDELITY_NOTE)
        );
    }
}
