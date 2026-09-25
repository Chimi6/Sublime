//! JSON -> XML, the inverse mapping: the root object's one member is the
//! root element, `@name` members are attributes, `#text` the text, other
//! members child elements, arrays repeated elements, scalars text.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Fidelity, Input, Tier};
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::json;
use crate::io::xml::writer;
use crate::value::ChunkedText;

const NAME: &str = "json-to-xml";
const FIDELITY_NOTE: &str = "@-keys become attributes, #text the text, arrays repeated elements; the root object must have one member, otherwise it is wrapped in <root>; numbers, booleans, and nulls become text";

pub struct JsonToXml;

impl Converter for JsonToXml {
    fn name(&self) -> &'static str {
        NAME
    }

    fn from(&self) -> &'static Format {
        &formats::JSON
    }

    fn to(&self) -> &'static Format {
        &formats::XML
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
        let tree = json::parse(input)?;
        let mut warnings = Vec::new();
        let mut text = ChunkedText::new(output);
        let written = writer::write_document(&tree, tree.root, &mut text, &mut warnings);
        if let Err(message) = written {
            return Err(ConvertError::Unsupported(message));
        }
        text.finish()?;
        for warning in warnings {
            context.warning(warning);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declares_contract() {
        assert_eq!(JsonToXml.name(), "json-to-xml");
        assert_eq!(JsonToXml.from().id, "json");
        assert_eq!(JsonToXml.to().id, "xml");
        assert_eq!(JsonToXml.fidelity(), Fidelity::Conditional(FIDELITY_NOTE));
    }
}
