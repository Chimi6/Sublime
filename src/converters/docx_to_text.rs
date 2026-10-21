//! Word -> Text. Reads the document model out of the package and
//! projects it into the Markdown event stream, which the text writer renders.

use std::io::{Read, Write};

use crate::converter::{ConvertError, Converter, Entry, Fidelity, Input, Location, Tier};
use crate::document::markdown::emit_events;
use crate::event::Context;
use crate::format::formats;
use crate::io::docx::read_docx;
use crate::io::text::TextWriter;

const NAME: &str = "docx-to-text";
const FIDELITY_NOTE: &str =
    "formatting, images, page layout, headers and footers are dropped; text boxes follow the body";

pub struct DocxToText;

impl Converter for DocxToText {
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

impl DocxToText {
    pub const fn entry(&'static self) -> Entry {
        Entry {
            name: NAME,
            from: &formats::DOCX,
            to: &formats::TEXT,
            fidelity: Fidelity::Lossy(FIDELITY_NOTE),
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
        assert_eq!(DocxToText.entry().name, "docx-to-text");
        assert_eq!(DocxToText.entry().from.id, "docx");
        assert_eq!(DocxToText.entry().to.id, "text");
        assert_eq!(DocxToText.entry().fidelity, Fidelity::Lossy(FIDELITY_NOTE));
    }
}
