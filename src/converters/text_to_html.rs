//! Text -> Html. Reads plain text into the Markdown event stream (a
//! paragraph per run of lines) and writes it as HTML paragraphs.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Entry, Fidelity, Input, Tier};
use crate::converters::input::read_text_document;
use crate::event::Context;
use crate::format::formats;
use crate::io::html::HtmlWriter;
use crate::io::text::reader::parse_into;

const NAME: &str = "text-to-html";
const FIDELITY_NOTE: &str =
    "paragraphs are runs of lines separated by blank lines; the text itself is kept exactly";

pub struct TextToHtml;

impl Converter for TextToHtml {
    fn convert(
        &self,
        mut input: Input<'_>,
        output: &mut dyn Write,
        _context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let text = read_text_document(&mut input)?;
        let mut writer = HtmlWriter::streaming(output);
        parse_into(&text, &mut writer);
        writer.finish()?;
        output.flush()?;
        Ok(())
    }
}

impl TextToHtml {
    pub const fn entry(&'static self) -> Entry {
        Entry {
            name: NAME,
            from: &formats::TEXT,
            to: &formats::HTML,
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
        assert_eq!(TextToHtml.entry().name, "text-to-html");
        assert_eq!(TextToHtml.entry().from.id, "text");
        assert_eq!(TextToHtml.entry().to.id, "html");
        assert_eq!(
            TextToHtml.entry().fidelity,
            Fidelity::Conditional(FIDELITY_NOTE)
        );
    }
}
