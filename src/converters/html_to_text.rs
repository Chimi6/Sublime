//! HTML -> Text. Reads the HTML into the Markdown event stream
//! (`io::html::reader`) and writes their text.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Fidelity, Input, Tier};
use crate::converters::input::read_text_document;
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::html::reader::parse_into;
use crate::io::text::TextWriter;

const NAME: &str = "html-to-text";
const FIDELITY_NOTE: &str = "markup, scripts, styles, and image sources are dropped";

pub struct HtmlToText;

impl Converter for HtmlToText {
    fn name(&self) -> &'static str {
        NAME
    }

    fn from(&self) -> &'static Format {
        &formats::HTML
    }

    fn to(&self) -> &'static Format {
        &formats::TEXT
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
        let mut writer = TextWriter::streaming(output);
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
        assert_eq!(HtmlToText.name(), "html-to-text");
        assert_eq!(HtmlToText.from().id, "html");
        assert_eq!(HtmlToText.to().id, "text");
        assert_eq!(HtmlToText.fidelity(), Fidelity::Lossy(FIDELITY_NOTE));
    }
}
