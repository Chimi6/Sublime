//! Pages -> Text. Reads the document model out of the package and
//! projects it into the Markdown event stream, which the writer renders.

use std::io::{Read, Write};

use crate::converter::{ConvertError, Converter, Entry, Fidelity, Input, Tier};
use crate::converters::pages_to_json::package_error;
use crate::document::markdown::emit_events;
use crate::event::Context;
use crate::format::formats;
use crate::io::pages::{Package, Scope, read_document};
use crate::io::text::TextWriter;

const NAME: &str = "pages-to-text";
const FIDELITY_NOTE: &str =
    "formatting, images, page layout, headers and footers are dropped; text boxes follow the body";

pub struct PagesToText;

impl Converter for PagesToText {
    fn convert(
        &self,
        mut input: Input<'_>,
        output: &mut dyn Write,
        _context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let mut bytes = Vec::new();
        input.read_to_end(&mut bytes)?;
        let package = Package::read_scope(&bytes, Scope::Document).map_err(package_error)?;
        let document = read_document(&package);
        let mut writer = TextWriter::streaming(output);
        emit_events(&document, &mut writer);
        writer.finish()?;
        output.flush()?;
        Ok(())
    }
}

impl PagesToText {
    pub const fn entry(&'static self) -> Entry {
        Entry {
            name: NAME,
            from: &formats::PAGES,
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
        assert_eq!(PagesToText.entry().name, "pages-to-text");
        assert_eq!(PagesToText.entry().from.id, "pages");
        assert_eq!(PagesToText.entry().to.id, "text");
        assert_eq!(PagesToText.entry().fidelity, Fidelity::Lossy(FIDELITY_NOTE));
    }
}
