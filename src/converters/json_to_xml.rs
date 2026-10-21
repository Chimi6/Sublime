//! JSON -> XML, the inverse mapping: the root object's one member is the
//! root element, `@name` members are attributes, `#text` the text, other
//! members child elements, arrays repeated elements, scalars text.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Entry, Fidelity, Input, Tier};
use crate::event::Context;
use crate::format::formats;
use crate::io::json;
use crate::io::xml::writer;
use crate::value::ChunkedText;

const NAME: &str = "json-to-xml";
const FIDELITY_NOTE: &str = "@-keys become attributes, #text the text, arrays repeated elements; the root object must have one member, otherwise it is wrapped in <root>; numbers, booleans, and nulls become text";

pub struct JsonToXml;

impl Converter for JsonToXml {
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

impl JsonToXml {
    pub const fn entry(&'static self) -> Entry {
        Entry {
            name: NAME,
            from: &formats::JSON,
            to: &formats::XML,
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
        assert_eq!(JsonToXml.entry().name, "json-to-xml");
        assert_eq!(JsonToXml.entry().from.id, "json");
        assert_eq!(JsonToXml.entry().to.id, "xml");
        assert_eq!(
            JsonToXml.entry().fidelity,
            Fidelity::Conditional(FIDELITY_NOTE)
        );
    }
}
