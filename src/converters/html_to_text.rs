//! HTML -> Text. Reads the HTML into the Markdown event stream
//! (`io::html::reader`) and writes their text.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Entry, Fidelity, Input, Tier};
use crate::converters::input::read_text_document;
use crate::event::Context;
use crate::format::formats;
use crate::io::html::reader::parse_into;
use crate::io::text::TextWriter;

const NAME: &str = "html-to-text";
const FIDELITY_NOTE: &str = "markup, scripts, styles, and image sources are dropped";

pub struct HtmlToText;

impl Converter for HtmlToText {
    fn convert(
        &self,
        mut input: Input<'_>,
        output: &mut dyn Write,
        _context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let text = read_text_document(&mut input)?;
        let mut writer = TextWriter::streaming(output);
        parse_into(&text, &mut writer);
        writer.finish()?;
        output.flush()?;
        Ok(())
    }
}

impl HtmlToText {
    pub const fn entry(&'static self) -> Entry {
        Entry {
            name: NAME,
            from: &formats::HTML,
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
        assert_eq!(HtmlToText.entry().name, "html-to-text");
        assert_eq!(HtmlToText.entry().from.id, "html");
        assert_eq!(HtmlToText.entry().to.id, "text");
        assert_eq!(HtmlToText.entry().fidelity, Fidelity::Lossy(FIDELITY_NOTE));
    }
}
