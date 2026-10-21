//! Pages -> Word. Reads the document model out of the package and renders
//! it as `.docx`. Conditional: everything the fixtures hold survives;
//! what Word cannot hold is reported as it is dropped.

use std::io::{Read, Write};

use crate::converter::{ConvertError, Converter, Entry, Fidelity, Input, Tier};
use crate::converters::pages_to_json::package_error;
use crate::event::Context;
use crate::format::formats;
use crate::io::docx::write_docx;
use crate::io::pages::{Package, Scope, read_document};

const NAME: &str = "pages-to-docx";
const FIDELITY_NOTE: &str = "text, named styles, lists (with their indents and numbering), links, footnotes, comments and reply threads (author, date, text, and range), tracked insertions and deletions, headers and footers, section columns and column breaks, tab stops, and paragraph borders carry over; tables keep merges, fills, borders, and cell alignment, including tables placed on the page; shapes, lines, and text boxes keep their outline, fill, stroke, and text wrap, moving with their paragraph when anchored in the text or sitting in its line, behind the text where Pages draws them there, with their arrowheads; empty paragraphs keep the size of their mark; charts become native Word charts with their data; drawings on page templates return to their header or footer; the page colour becomes the Word page colour; images keep their place";

pub struct PagesToDocx;

impl Converter for PagesToDocx {
    fn convert(
        &self,
        mut input: Input<'_>,
        output: &mut dyn Write,
        context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let mut bytes = Vec::new();
        input.read_to_end(&mut bytes)?;
        let package = Package::read_scope(&bytes, Scope::Document).map_err(package_error)?;
        let document = read_document(&package);
        write_docx(
            &document,
            &mut *output,
            context.options.effort.unwrap_or_default(),
        )?;
        output.flush()?;
        Ok(())
    }
}

impl PagesToDocx {
    pub const fn entry(&'static self) -> Entry {
        Entry {
            name: NAME,
            from: &formats::PAGES,
            to: &formats::DOCX,
            fidelity: Fidelity::Conditional(FIDELITY_NOTE),
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
        assert_eq!(PagesToDocx.entry().name, "pages-to-docx");
        assert_eq!(PagesToDocx.entry().from.id, "pages");
        assert_eq!(PagesToDocx.entry().to.id, "docx");
        assert_eq!(
            PagesToDocx.entry().fidelity,
            Fidelity::Conditional(FIDELITY_NOTE)
        );
    }
}
