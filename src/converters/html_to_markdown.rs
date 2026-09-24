//! HTML -> Markdown. Reads the HTML into the Markdown event stream
//! (`io::html::reader`) and writes them as Markdown.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Fidelity, Input, Tier};
use crate::converters::input::read_text_document;
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::html::reader::parse_into;
use crate::io::markdown::MarkdownWriter;

const NAME: &str = "html-to-markdown";
const FIDELITY_NOTE: &str = "scripts, styles, forms, and layout are dropped; elements without a Markdown form keep their text";

pub struct HtmlToMarkdown;

impl Converter for HtmlToMarkdown {
    fn name(&self) -> &'static str {
        NAME
    }

    fn from(&self) -> &'static Format {
        &formats::HTML
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
        let text = read_text_document(&mut input)?;
        let mut writer = MarkdownWriter::streaming(output);
        parse_into(&text, &mut writer);
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
        assert_eq!(HtmlToMarkdown.name(), "html-to-markdown");
        assert_eq!(HtmlToMarkdown.from().id, "html");
        assert_eq!(HtmlToMarkdown.to().id, "markdown");
        assert_eq!(HtmlToMarkdown.fidelity(), Fidelity::Lossy(FIDELITY_NOTE));
    }
}
