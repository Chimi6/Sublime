//! Word -> Apple Pages. Reads the Word document into the document model and
//! writes a Pages package by rewriting a blank template's body storage.

use std::io::{Read, Write};

use crate::converter::{ConvertError, Converter, Fidelity, Input, Location, Tier};
use crate::converters::pages_to_json::package_error;
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::docx::read_docx;
use crate::io::pages::write_package;

const NAME: &str = "docx-to-pages";
const FIDELITY_NOTE: &str = "paragraphs, headings, bulleted and numbered lists, and tables reach Pages with its named styles, bold and italic runs reuse its character styles, and links become clickable hyperlinks; images are not yet written";

pub struct DocxToPages;

impl Converter for DocxToPages {
    fn name(&self) -> &'static str {
        NAME
    }

    fn from(&self) -> &'static Format {
        &formats::DOCX
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
        let mut bytes = Vec::new();
        input.read_to_end(&mut bytes)?;
        let document = read_docx(&bytes).map_err(|error| ConvertError::Malformed {
            location: Location { line: 0, column: 0 },
            message: error.to_string(),
        })?;
        let package = write_package(&document).map_err(package_error)?;
        output.write_all(&package)?;
        output.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declares_contract() {
        assert_eq!(DocxToPages.name(), "docx-to-pages");
        assert_eq!(DocxToPages.from().id, "docx");
        assert_eq!(DocxToPages.to().id, "pages");
    }
}
