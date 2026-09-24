//! HTML -> Docx. Reads the HTML into the Markdown event stream
//! (`io::html::reader`) and builds the document model through the events
//! bridge, which the Word writer renders.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Fidelity, Input, Tier};
use crate::converters::input::read_text_document;
use crate::document::from_events::DocumentBuilder;
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::docx::DocxStream;
use crate::io::html::reader::parse_into;

const NAME: &str = "html-to-docx";
const FIDELITY_NOTE: &str = "lossless for headings, paragraphs, lists, quotes, code, tables, links, footnotes, and images given as data URIs; scripts, styles, forms, and layout are dropped, and other images become links";

pub struct HtmlToDocx;

impl Converter for HtmlToDocx {
    fn name(&self) -> &'static str {
        NAME
    }

    fn from(&self) -> &'static Format {
        &formats::HTML
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
        assert_eq!(HtmlToDocx.name(), "html-to-docx");
        assert_eq!(HtmlToDocx.from().id, "html");
        assert_eq!(HtmlToDocx.to().id, "docx");
        assert_eq!(HtmlToDocx.fidelity(), Fidelity::Conditional(FIDELITY_NOTE));
    }
}
