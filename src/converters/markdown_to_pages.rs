//! Markdown -> Apple Pages. Parses Markdown, builds the document model
//! through the events bridge, and writes a Pages package by rewriting a
//! blank template's body storage. The whole input is read before parsing
//! because a link may be defined after its use.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Fidelity, Input, Tier};
use crate::converters::input::read_text_document;
use crate::converters::pages_to_json::package_error;
use crate::document::from_events::DocumentBuilder;
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::markdown::{Options, parse_into};
use crate::io::pages::write_package;

const NAME: &str = "markdown-to-pages";
const FIDELITY_NOTE: &str = "paragraphs, headings, bulleted and numbered lists, and tables reach Pages with its named styles, bold and italic runs reuse its character styles, and links become clickable hyperlinks; images are not yet written";

pub struct MarkdownToPages;

impl Converter for MarkdownToPages {
    fn name(&self) -> &'static str {
        NAME
    }

    fn from(&self) -> &'static Format {
        &formats::MARKDOWN
    }

    fn to(&self) -> &'static Format {
        &formats::PAGES
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
        let text = read_text_document(&mut input)?;
        let mut builder = DocumentBuilder::new();
        builder.reserve_text(text.len());
        parse_into(&text, Options::default(), &mut builder);
        let document = builder.finish();
        let bytes = write_package(&document).map_err(package_error)?;
        output.write_all(&bytes)?;
        output.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declares_contract() {
        assert_eq!(MarkdownToPages.name(), "markdown-to-pages");
        assert_eq!(MarkdownToPages.from().id, "markdown");
        assert_eq!(MarkdownToPages.to().id, "pages");
        assert_eq!(MarkdownToPages.fidelity(), Fidelity::Lossy(FIDELITY_NOTE));
    }
}
