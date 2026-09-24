//! Pages -> Markdown. Reads the document model out of the package and
//! projects it into the Markdown event stream, which the writer renders.

use std::io::{Read, Write};

use crate::converter::{ConvertError, Converter, Fidelity, Input, Tier};
use crate::converters::pages_to_json::package_error;
use crate::document::markdown::emit_events;
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::markdown::MarkdownWriter;
use crate::io::pages::{Package, Scope, read_document};

const NAME: &str = "pages-to-markdown";
const FIDELITY_NOTE: &str = "page layout, headers and footers, fonts, sizes, and colors are dropped; text boxes follow the body";

pub struct PagesToMarkdown;

impl Converter for PagesToMarkdown {
    fn name(&self) -> &'static str {
        NAME
    }

    fn from(&self) -> &'static Format {
        &formats::PAGES
    }

    fn to(&self) -> &'static Format {
        &formats::MARKDOWN
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
        let package = Package::read_scope(&bytes, Scope::Document).map_err(package_error)?;
        let document = read_document(&package);
        let mut writer = MarkdownWriter::streaming(output);
        emit_events(&document, &mut writer);
        writer.finish()?;
        output.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declares_contract() {
        assert_eq!(PagesToMarkdown.name(), "pages-to-markdown");
        assert_eq!(PagesToMarkdown.from().id, "pages");
        assert_eq!(PagesToMarkdown.to().id, "markdown");
        assert_eq!(PagesToMarkdown.fidelity(), Fidelity::Lossy(FIDELITY_NOTE));
    }
}
