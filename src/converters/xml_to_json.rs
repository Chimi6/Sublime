//! XML -> JSON under the shared mapping: elements are objects, attributes
//! `@name` members, text `#text`, repeated elements arrays, every value a
//! string. What the mapping drops is reported once per kind.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Entry, Fidelity, Input, Location, Tier};
use crate::event::Context;
use crate::format::formats;
use crate::io::json::{JsonWriter, from_tree};
use crate::io::xml::tree;

const NAME: &str = "xml-to-json";
const FIDELITY_NOTE: &str = "attributes become @-keys, text #text, repeated elements arrays, every value a string; comments, processing instructions, the doctype, and the order of text against elements in mixed content are dropped";

pub struct XmlToJson;

impl Converter for XmlToJson {
    fn convert(
        &self,
        input: Input<'_>,
        output: &mut dyn Write,
        context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        // Japanese XML (`encoding="Shift_JIS"`) is decoded as it is read.
        let mut rest = None;
        let mut input = crate::converters::input::decoded_input(input, &mut rest)?;
        let parsed = tree::parse_reader(&mut input)?;
        for note in parsed.notes {
            context.loss(NAME, Location::default(), note);
        }
        let mut writer = JsonWriter::new(output);
        from_tree::write_tree(&parsed.tree, parsed.tree.root, &mut writer, NAME, context)?;
        writer.flush()?;
        Ok(())
    }
}

impl XmlToJson {
    pub const fn entry(&'static self) -> Entry {
        Entry {
            name: NAME,
            from: &formats::XML,
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
        assert_eq!(XmlToJson.entry().name, "xml-to-json");
        assert_eq!(XmlToJson.entry().from.id, "xml");
        assert_eq!(XmlToJson.entry().to.id, "json");
        assert_eq!(
            XmlToJson.entry().fidelity,
            Fidelity::Conditional(FIDELITY_NOTE)
        );
    }
}
