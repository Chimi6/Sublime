//! YAML -> JSON. The stream reads into the value hub; one document is
//! written as its JSON value, several as a JSON array of them, none as
//! `null`. Anchors are expanded, merge keys applied, tags outside the core
//! schema dropped and reported, collection keys written as their JSON
//! text and reported, infinities and NaN written as strings and reported.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Entry, Fidelity, Input, Location, Tier};
use crate::converters::input::read_text_document;
use crate::event::Context;
use crate::format::formats;
use crate::io::json::{JsonWriter, from_tree};
use crate::io::yaml;
use crate::value::Span;

const NAME: &str = "yaml-to-json";
const FIDELITY_NOTE: &str = "anchors are expanded, tags outside the core schema are dropped, keys become strings, infinities and NaN become strings, a multi-document stream becomes an array";

pub struct YamlToJson;

impl Converter for YamlToJson {
    fn convert(
        &self,
        mut input: Input<'_>,
        output: &mut dyn Write,
        context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let text = read_text_document(&mut input)?;
        let parsed = yaml::parse(&text)?;
        for note in parsed.notes {
            context.loss(NAME, Location::default(), note);
        }
        let mut tree = parsed.tree;
        let node = match parsed.documents.as_slice() {
            [] => tree.push(Span::default(), crate::value::Data::Null),
            [single] => *single,
            documents => {
                let array = tree.push_array(Span::default());
                for document in documents {
                    tree.append(array, *document);
                }
                array
            }
        };
        let mut writer = JsonWriter::new(output);
        from_tree::write_tree(&tree, node, &mut writer, NAME, context)?;
        writer.flush()?;
        Ok(())
    }
}

impl YamlToJson {
    pub const fn entry(&'static self) -> Entry {
        Entry {
            name: NAME,
            from: &formats::YAML,
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
        assert_eq!(YamlToJson.entry().name, "yaml-to-json");
        assert_eq!(YamlToJson.entry().from.id, "yaml");
        assert_eq!(YamlToJson.entry().to.id, "json");
        assert_eq!(
            YamlToJson.entry().fidelity,
            Fidelity::Conditional(FIDELITY_NOTE)
        );
    }
}
