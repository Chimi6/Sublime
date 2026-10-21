//! Markdown -> Apple Pages. Parses Markdown, builds the document model
//! through the events bridge, and writes a Pages package by rewriting a
//! blank template's body storage. The whole input is read before parsing
//! because a link may be defined after its use.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Entry, Fidelity, Input, Tier};
use crate::converters::input::read_text_document;
use crate::converters::pages_to_json::package_error;
use crate::document::from_events::DocumentBuilder;
use crate::event::Context;
use crate::format::formats;
use crate::io::markdown::{Options, parse_into};
use crate::io::pages::write_package_to;

const NAME: &str = "markdown-to-pages";
const FIDELITY_NOTE: &str = "paragraphs, headings, bulleted and numbered lists, and tables reach Pages with its named styles, bold and italic runs reuse its character styles, and links become clickable hyperlinks; images are not yet written";

pub struct MarkdownToPages;

impl Converter for MarkdownToPages {
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
        drop(text);
        let mut sink = std::io::BufWriter::with_capacity(64 * 1024, &mut *output);
        write_package_to(&document, &mut sink).map_err(package_error)?;
        sink.flush()?;
        drop(sink);
        output.flush()?;
        Ok(())
    }
}

impl MarkdownToPages {
    pub const fn entry(&'static self) -> Entry {
        Entry {
            name: NAME,
            from: &formats::MARKDOWN,
            to: &formats::PAGES,
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
        assert_eq!(MarkdownToPages.entry().name, "markdown-to-pages");
        assert_eq!(MarkdownToPages.entry().from.id, "markdown");
        assert_eq!(MarkdownToPages.entry().to.id, "pages");
        assert_eq!(
            MarkdownToPages.entry().fidelity,
            Fidelity::Lossy(FIDELITY_NOTE)
        );
    }
}
