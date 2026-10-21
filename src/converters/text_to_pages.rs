//! Plain text -> Apple Pages. Reads paragraphs into the document model and
//! writes a Pages package by rewriting a blank template's body storage.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Entry, Fidelity, Input, Tier};
use crate::converters::input::read_text_document;
use crate::converters::pages_to_json::package_error;
use crate::document::from_events::DocumentBuilder;
use crate::event::Context;
use crate::format::formats;
use crate::io::pages::write_package_to;
use crate::io::text::reader::parse_into;

const NAME: &str = "text-to-pages";
const FIDELITY_NOTE: &str = "paragraphs reach Pages as body text; text carries no other structure";

pub struct TextToPages;

impl Converter for TextToPages {
    fn convert(
        &self,
        mut input: Input<'_>,
        output: &mut dyn Write,
        _context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let text = read_text_document(&mut input)?;
        let mut builder = DocumentBuilder::new();
        builder.reserve_text(text.len());
        parse_into(&text, &mut builder);
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

impl TextToPages {
    pub const fn entry(&'static self) -> Entry {
        Entry {
            name: NAME,
            from: &formats::TEXT,
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
        assert_eq!(TextToPages.entry().name, "text-to-pages");
        assert_eq!(TextToPages.entry().from.id, "text");
        assert_eq!(TextToPages.entry().to.id, "pages");
    }
}
