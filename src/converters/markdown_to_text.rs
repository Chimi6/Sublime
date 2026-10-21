//! Markdown -> plain text. Parses with `io::markdown` and renders with
//! `io::text`. Lossy: markup, raw HTML, and image sources are dropped.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Entry, Fidelity, Input, Tier};
use crate::converters::input::read_text_document;
use crate::event::Context;
use crate::format::formats;
use crate::io::markdown::{Options, parse_into};
use crate::io::text::TextWriter;

const NAME: &str = "markdown-to-text";
const FIDELITY_NOTE: &str = "markup, raw HTML, and image sources are dropped";

pub struct MarkdownToText;

impl Converter for MarkdownToText {
    fn convert(
        &self,
        mut input: Input<'_>,
        output: &mut dyn Write,
        _context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let text = read_text_document(&mut input)?;
        let mut writer = TextWriter::streaming(output);
        parse_into(&text, Options::default(), &mut writer);
        writer.finish()?;
        output.flush()?;
        Ok(())
    }
}

impl MarkdownToText {
    pub const fn entry(&'static self) -> Entry {
        Entry {
            name: NAME,
            from: &formats::MARKDOWN,
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
    use crate::converter::ConvertOptions;
    use crate::event::CollectingSink;

    #[test]
    fn converts_a_document() {
        let options = ConvertOptions::default();
        let mut sink = CollectingSink::new();
        let mut output = Vec::new();
        let mut source: &[u8] = b"# Title\n\nSome *text* with a [link](/x).\n";
        let mut context = Context::new(&mut sink, &options);
        MarkdownToText
            .convert(Input::Stream(&mut source), &mut output, &mut context)
            .unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "Title\n\nSome text with a link (/x).\n"
        );
        let _ = sink;
    }

    #[test]
    fn declares_contract() {
        let converter: &'static MarkdownToText = &MarkdownToText;
        assert_eq!(converter.entry().name, "markdown-to-text");
        assert_eq!(converter.entry().from.id, "markdown");
        assert_eq!(converter.entry().to.id, "text");
        assert_eq!(converter.entry().fidelity, Fidelity::Lossy(FIDELITY_NOTE));
        assert_eq!(converter.entry().tier, Tier::Native);
    }
}
