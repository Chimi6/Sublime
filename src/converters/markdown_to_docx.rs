//! Markdown -> Word. Parses with `io::markdown`, builds the document model
//! through the events bridge, and writes it with the Word writer. Markdown
//! allows a link to be defined after its use, so the whole input is read
//! before parsing.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Entry, Fidelity, Input, Tier};
use crate::converters::input::read_text_document;
use crate::document::from_events::DocumentBuilder;
use crate::event::Context;
use crate::format::formats;
use crate::io::docx::DocxStream;
use crate::io::markdown::{Options, parse_into};

const NAME: &str = "markdown-to-docx";
const FIDELITY_NOTE: &str = "lossless for headings, paragraphs, lists, quotes, code, tables, links, footnotes, and images given as data URIs; raw HTML is dropped, other images become links, and loose lists come out tight";

pub struct MarkdownToDocx;

impl Converter for MarkdownToDocx {
    fn convert(
        &self,
        mut input: Input<'_>,
        output: &mut dyn Write,
        context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let text = read_text_document(&mut input)?;
        let mut stream = DocxStream::new(&mut *output, context.options.effort.unwrap_or_default())?;
        let mut builder = DocumentBuilder::streaming(&mut stream);
        builder.reserve_text(text.len());
        parse_into(&text, Options::default(), &mut builder);
        let document = builder.finish_streaming()?;
        stream.finish(&document)?;
        output.flush()?;
        Ok(())
    }
}

impl MarkdownToDocx {
    pub const fn entry(&'static self) -> Entry {
        Entry {
            name: NAME,
            from: &formats::MARKDOWN,
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
        assert_eq!(MarkdownToDocx.entry().name, "markdown-to-docx");
        assert_eq!(MarkdownToDocx.entry().from.id, "markdown");
        assert_eq!(MarkdownToDocx.entry().to.id, "docx");
        assert_eq!(
            MarkdownToDocx.entry().fidelity,
            Fidelity::Conditional(FIDELITY_NOTE)
        );
    }
}
