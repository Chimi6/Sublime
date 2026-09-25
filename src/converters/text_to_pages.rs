//! Plain text -> Apple Pages. Reads paragraphs into the document model and
//! writes a Pages package by rewriting a blank template's body storage.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Fidelity, Input, Tier};
use crate::converters::input::read_text_document;
use crate::converters::pages_to_json::package_error;
use crate::document::from_events::DocumentBuilder;
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::pages::write_package;
use crate::io::text::reader::parse_into;

const NAME: &str = "text-to-pages";
const FIDELITY_NOTE: &str = "paragraphs reach Pages as body text; text carries no other structure";

pub struct TextToPages;

impl Converter for TextToPages {
    fn name(&self) -> &'static str {
        NAME
    }

    fn from(&self) -> &'static Format {
        &formats::TEXT
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
        parse_into(&text, &mut builder);
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
        assert_eq!(TextToPages.name(), "text-to-pages");
        assert_eq!(TextToPages.from().id, "text");
        assert_eq!(TextToPages.to().id, "pages");
    }
}
