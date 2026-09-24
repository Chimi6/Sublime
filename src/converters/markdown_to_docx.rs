//! Markdown -> Word. Parses with `io::markdown`, builds the document model
//! through the events bridge, and writes it with the Word writer. Markdown
//! allows a link to be defined after its use, so the whole input is read
//! before parsing.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Fidelity, Input, Tier};
use crate::converters::input::read_text_document;
use crate::document::from_events::DocumentBuilder;
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::docx::write_docx;
use crate::io::markdown::{Options, parse_into};

const NAME: &str = "markdown-to-docx";
const FIDELITY_NOTE: &str = "lossless for headings, paragraphs, lists, quotes, code, tables, links, footnotes, and images given as data URIs; raw HTML is dropped, other images become links, and loose lists come out tight";

pub struct MarkdownToDocx;

impl Converter for MarkdownToDocx {
    fn name(&self) -> &'static str {
        NAME
    }

    fn from(&self) -> &'static Format {
        &formats::MARKDOWN
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
        let text = read_text_document(&mut input)?;
        let mut builder = DocumentBuilder::new();
        parse_into(&text, Options::default(), &mut builder);
        let document = builder.finish();
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
        assert_eq!(MarkdownToDocx.name(), "markdown-to-docx");
        assert_eq!(MarkdownToDocx.from().id, "markdown");
        assert_eq!(MarkdownToDocx.to().id, "docx");
        assert_eq!(
            MarkdownToDocx.fidelity(),
            Fidelity::Conditional(FIDELITY_NOTE)
        );
    }
}
