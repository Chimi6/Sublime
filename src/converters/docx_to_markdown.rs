//! Word -> Markdown. Reads the document model out of the package and
//! projects it into the Markdown event stream, which the Markdown writer renders.

use std::io::{Read, Write};

use crate::converter::{ConvertError, Converter, Entry, Fidelity, Input, Location, Tier};
use crate::document::markdown::emit_events;
use crate::event::Context;
use crate::format::formats;
use crate::io::docx::read_docx;
use crate::io::markdown::MarkdownWriter;

const NAME: &str = "docx-to-markdown";
const FIDELITY_NOTE: &str = "page layout, headers and footers, fonts, sizes, and colors are dropped; text boxes follow the body";

pub struct DocxToMarkdown;

impl Converter for DocxToMarkdown {
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
        let mut writer = MarkdownWriter::streaming(output);
        emit_events(&document, &mut writer);
        writer.finish()?;
        output.flush()?;
        Ok(())
    }
}

impl DocxToMarkdown {
    pub const fn entry(&'static self) -> Entry {
        Entry {
            name: NAME,
            from: &formats::DOCX,
            to: &formats::MARKDOWN,
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
        assert_eq!(DocxToMarkdown.entry().name, "docx-to-markdown");
        assert_eq!(DocxToMarkdown.entry().from.id, "docx");
        assert_eq!(DocxToMarkdown.entry().to.id, "markdown");
        assert_eq!(
            DocxToMarkdown.entry().fidelity,
            Fidelity::Lossy(FIDELITY_NOTE)
        );
    }
}
