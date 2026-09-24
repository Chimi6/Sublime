//! Text -> Docx. Reads plain text into the Markdown event stream (a
//! paragraph per run of lines) and builds the document model through the
//! events bridge, which the Word writer renders.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Fidelity, Input, Tier};
use crate::converters::input::read_text_document;
use crate::document::from_events::DocumentBuilder;
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::docx::DocxStream;
use crate::io::text::reader::parse_into;

const NAME: &str = "text-to-docx";
const FIDELITY_NOTE: &str = "paragraphs are runs of lines separated by blank lines; line breaks inside a paragraph become spaces";

pub struct TextToDocx;

impl Converter for TextToDocx {
    fn name(&self) -> &'static str {
        NAME
    }

    fn from(&self) -> &'static Format {
        &formats::TEXT
    }

    fn to(&self) -> &'static Format {
        &formats::DOCX
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
        _context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let text = read_text_document(&mut input)?;
        let mut stream = DocxStream::new(&mut *output)?;
        let mut builder = DocumentBuilder::streaming(&mut stream);
        builder.reserve_text(text.len());
        parse_into(&text, &mut builder);
        let document = builder.finish_streaming()?;
        stream.finish(&document)?;
        output.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declares_contract() {
        assert_eq!(TextToDocx.name(), "text-to-docx");
        assert_eq!(TextToDocx.from().id, "text");
        assert_eq!(TextToDocx.to().id, "docx");
        assert_eq!(TextToDocx.fidelity(), Fidelity::Conditional(FIDELITY_NOTE));
    }
}
