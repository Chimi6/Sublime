//! YAML -> JSON. The stream reads into the value hub; one document is
//! written as its JSON value, several as a JSON array of them, none as
//! `null`. Anchors are expanded, merge keys applied, tags outside the core
//! schema dropped and reported, collection keys written as their JSON
//! text and reported, infinities and NaN written as strings and reported.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Fidelity, Input, Location, Tier};
use crate::converters::input::read_text_document;
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::json::{JsonWriter, from_tree};
use crate::io::yaml;
use crate::value::Span;

const NAME: &str = "yaml-to-json";
const FIDELITY_NOTE: &str = "anchors are expanded, tags outside the core schema are dropped, keys become strings, infinities and NaN become strings, a multi-document stream becomes an array";

pub struct YamlToJson;

impl Converter for YamlToJson {
    fn name(&self) -> &'static str {
        NAME
    }

    fn from(&self) -> &'static Format {
        &formats::YAML
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declares_contract() {
        assert_eq!(YamlToJson.name(), "yaml-to-json");
        assert_eq!(YamlToJson.from().id, "yaml");
        assert_eq!(YamlToJson.to().id, "json");
        assert_eq!(YamlToJson.fidelity(), Fidelity::Conditional(FIDELITY_NOTE));
    }
}
