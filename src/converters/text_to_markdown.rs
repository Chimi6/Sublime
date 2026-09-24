//! Text -> Markdown. Reads plain text into the Markdown event stream (a
//! paragraph per run of lines) and writes it as Markdown, escaping what
//! would otherwise read as markup.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Fidelity, Input, Tier};
use crate::converters::input::read_text_document;
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::markdown::MarkdownWriter;
use crate::io::text::reader::parse_into;

const NAME: &str = "text-to-markdown";
const FIDELITY_NOTE: &str = "paragraphs are runs of lines separated by blank lines; text that looks like Markdown is escaped so it reads as written";

pub struct TextToMarkdown;

impl Converter for TextToMarkdown {
    fn name(&self) -> &'static str {
        NAME
    }

    fn from(&self) -> &'static Format {
        &formats::TEXT
    }

    fn to(&self) -> &'static Format {
        &formats::MARKDOWN
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
        assert_eq!(TextToMarkdown.name(), "text-to-markdown");
        assert_eq!(TextToMarkdown.from().id, "text");
        assert_eq!(TextToMarkdown.to().id, "markdown");
        assert_eq!(
            TextToMarkdown.fidelity(),
            Fidelity::Conditional(FIDELITY_NOTE)
        );
    }
}
