//! JSON -> YAML. The document reads into the value hub and is written in
//! block style. JSON is a subset of YAML, so nothing is lost.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Entry, Fidelity, Input, Tier};
use crate::event::Context;
use crate::format::formats;
use crate::io::json;
use crate::io::yaml;
use crate::value::ChunkedText;

const NAME: &str = "json-to-yaml";

pub struct JsonToYaml;

impl Converter for JsonToYaml {
    fn convert(
        &self,
        input: Input<'_>,
        output: &mut dyn Write,
        _context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let tree = json::parse(input)?;
        let mut text = ChunkedText::new(output);
        yaml::write_document(&tree, tree.root, &mut text);
        text.finish()?;
        Ok(())
    }
}

impl JsonToYaml {
    pub const fn entry(&'static self) -> Entry {
        Entry {
            name: NAME,
            from: &formats::JSON,
            to: &formats::YAML,
            fidelity: Fidelity::Lossless,
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
        assert_eq!(JsonToYaml.entry().name, "json-to-yaml");
        assert_eq!(JsonToYaml.entry().from.id, "json");
        assert_eq!(JsonToYaml.entry().to.id, "yaml");
        assert_eq!(JsonToYaml.entry().fidelity, Fidelity::Lossless);
    }
}
