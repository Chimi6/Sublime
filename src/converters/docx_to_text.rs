//! Word -> Text. Reads the document model out of the package and
//! projects it into the Markdown event stream, which the text writer renders.

use std::io::{Read, Write};

use crate::converter::{ConvertError, Converter, Fidelity, Input, Location, Tier};
use crate::document::markdown::emit_events;
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::docx::read_docx;
use crate::io::text::TextWriter;

const NAME: &str = "docx-to-text";
const FIDELITY_NOTE: &str =
    "formatting, images, page layout, headers and footers are dropped; text boxes follow the body";

pub struct DocxToText;

impl Converter for DocxToText {
    fn name(&self) -> &'static str {
        NAME
    }

    fn from(&self) -> &'static Format {
        &formats::DOCX
    }

    fn to(&self) -> &'static Format {
        &formats::TEXT
    }

    fn fidelity(&self) -> Fidelity {
        Fidelity::Lossy(FIDELITY_NOTE)
    }

    fn tier(&self) -> Tier {
        Tier::Native
    }

    fn convert(
        &self,
        mut input: Input<'_>,
        output: &mut dyn Write,
        _context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let mut bytes = Vec::new();
        input.read_to_end(&mut bytes)?;
        let document = read_docx(&bytes).map_err(|error| ConvertError::Malformed {
            location: Location { line: 0, column: 0 },
            message: error.to_string(),
        })?;
        let mut writer = TextWriter::streaming(output);
        emit_events(&document, &mut writer);
        writer.finish()?;
        output.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declares_contract() {
        assert_eq!(DocxToText.name(), "docx-to-text");
        assert_eq!(DocxToText.from().id, "docx");
        assert_eq!(DocxToText.to().id, "text");
        assert_eq!(DocxToText.fidelity(), Fidelity::Lossy(FIDELITY_NOTE));
    }
}
