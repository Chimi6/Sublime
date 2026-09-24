//! Pages -> Word. Reads the document model out of the package and renders
//! it as `.docx`. Conditional: everything the fixtures hold survives;
//! what Word cannot hold is reported as it is dropped.

use std::io::{Read, Write};

use crate::converter::{ConvertError, Converter, Fidelity, Input, Tier};
use crate::converters::pages_to_json::package_error;
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::docx::write_docx;
use crate::io::pages::{Package, Scope, read_document};

const NAME: &str = "pages-to-docx";
const FIDELITY_NOTE: &str =
    "lossless for text, styles, lists, links, footnotes, and tables; shapes and charts are dropped";

pub struct PagesToDocx;

impl Converter for PagesToDocx {
    fn name(&self) -> &'static str {
        NAME
    }

    fn from(&self) -> &'static Format {
        &formats::PAGES
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
        let mut bytes = Vec::new();
        input.read_to_end(&mut bytes)?;
        let package = Package::read_scope(&bytes, Scope::Document).map_err(package_error)?;
        let document = read_document(&package);
        write_docx(&document, &mut *output)?;
        output.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declares_contract() {
        assert_eq!(PagesToDocx.name(), "pages-to-docx");
        assert_eq!(PagesToDocx.from().id, "pages");
        assert_eq!(PagesToDocx.to().id, "docx");
        assert_eq!(PagesToDocx.fidelity(), Fidelity::Conditional(FIDELITY_NOTE));
    }
}
