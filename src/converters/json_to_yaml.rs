//! JSON -> YAML. The document reads into the value hub and is written in
//! block style. JSON is a subset of YAML, so nothing is lost.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Fidelity, Input, Tier};
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::json;
use crate::io::yaml;

const NAME: &str = "json-to-yaml";

pub struct JsonToYaml;

impl Converter for JsonToYaml {
    fn name(&self) -> &'static str {
        NAME
    }

    fn from(&self) -> &'static Format {
        &formats::JSON
    }

    fn to(&self) -> &'static Format {
        &formats::YAML
    }

    fn fidelity(&self) -> Fidelity {
        Fidelity::Lossless
    }

    fn tier(&self) -> Tier {
        Tier::Native
    }

    fn convert(
        &self,
        input: Input<'_>,
        output: &mut dyn Write,
        _context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let document = json::parse(input)?;
        let mut text = String::new();
        yaml::write_document(&document, &mut text);
        output.write_all(text.as_bytes())?;
        output.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declares_contract() {
        assert_eq!(JsonToYaml.name(), "json-to-yaml");
        assert_eq!(JsonToYaml.from().id, "json");
        assert_eq!(JsonToYaml.to().id, "yaml");
        assert_eq!(JsonToYaml.fidelity(), Fidelity::Lossless);
    }
}
