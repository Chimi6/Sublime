//! Pages -> HTML. Reads the document model out of the package and
//! projects it into the Markdown event stream, which the writer renders.

use std::io::{Read, Write};

use crate::converter::{ConvertError, Converter, Entry, Fidelity, Input, Tier};
use crate::converters::pages_to_json::package_error;
use crate::document::markdown::emit_events;
use crate::event::Context;
use crate::format::formats;
use crate::io::html::HtmlWriter;
use crate::io::pages::{Package, Scope, read_document};

const NAME: &str = "pages-to-html";
const FIDELITY_NOTE: &str = "page layout, headers and footers, fonts, sizes, and colors are dropped; text boxes follow the body";

pub struct PagesToHtml;

impl Converter for PagesToHtml {
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
        let mut writer = HtmlWriter::streaming(output);
        emit_events(&document, &mut writer);
        writer.finish()?;
        output.flush()?;
        Ok(())
    }
}

impl PagesToHtml {
    pub const fn entry(&'static self) -> Entry {
        Entry {
            name: NAME,
            from: &formats::PAGES,
            to: &formats::HTML,
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
        assert_eq!(PagesToHtml.entry().name, "pages-to-html");
        assert_eq!(PagesToHtml.entry().from.id, "pages");
        assert_eq!(PagesToHtml.entry().to.id, "html");
        assert_eq!(PagesToHtml.entry().fidelity, Fidelity::Lossy(FIDELITY_NOTE));
    }
}
