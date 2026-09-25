//! XML -> JSON under the shared mapping: elements are objects, attributes
//! `@name` members, text `#text`, repeated elements arrays, every value a
//! string. What the mapping drops is reported once per kind.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Fidelity, Input, Location, Tier};
use crate::converters::input::read_text_document;
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::json::{JsonWriter, from_value};
use crate::io::xml::tree;

const NAME: &str = "xml-to-json";
const FIDELITY_NOTE: &str = "attributes become @-keys, text #text, repeated elements arrays, every value a string; comments, processing instructions, the doctype, and the order of text against elements in mixed content are dropped";

pub struct XmlToJson;

impl Converter for XmlToJson {
    fn name(&self) -> &'static str {
        NAME
    }

    fn from(&self) -> &'static Format {
        &formats::XML
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
        let parsed = tree::parse(&text)?;
        for note in parsed.notes {
            context.loss(NAME, Location::default(), note);
        }
        let mut writer = JsonWriter::new(output);
        from_value::write_value(&parsed.document, &mut writer, NAME, context)?;
        writer.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declares_contract() {
        assert_eq!(XmlToJson.name(), "xml-to-json");
        assert_eq!(XmlToJson.from().id, "xml");
        assert_eq!(XmlToJson.to().id, "json");
        assert_eq!(XmlToJson.fidelity(), Fidelity::Conditional(FIDELITY_NOTE));
    }
}
