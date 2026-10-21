//! Markdown -> Markdown events as JSON. Lossless with respect to the event
//! stream; `markdown-json-to-markdown` turns it back into Markdown.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Entry, Fidelity, Input, Tier};
use crate::converters::input::read_text_document;
use crate::event::Context;
use crate::format::formats;
use crate::io::markdown::events_json::JsonEventWriter;
use crate::io::markdown::{Options, parse_into};

const NAME: &str = "markdown-to-json";

pub struct MarkdownToJson;

impl Converter for MarkdownToJson {
    fn convert(
        &self,
        mut input: Input<'_>,
        output: &mut dyn Write,
        _context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let text = read_text_document(&mut input)?;
        let mut writer = JsonEventWriter::new(&mut *output);
        parse_into(&text, Options::default(), &mut writer);
        writer.finish()?;
        output.flush()?;
        Ok(())
    }
}

impl MarkdownToJson {
    pub const fn entry(&'static self) -> Entry {
        Entry {
            name: NAME,
            from: &formats::MARKDOWN,
            to: &formats::MARKDOWN_JSON,
            fidelity: Fidelity::Lossless,
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
        let mut source: &[u8] = b"*a*\n";
        let mut context = Context::new(&mut sink, &options);
        MarkdownToJson
            .convert(Input::Stream(&mut source), &mut output, &mut context)
            .unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "[{\"start\":\"paragraph\"},{\"start\":\"emphasis\"},{\"text\":\"a\"},{\"end\":\"emphasis\"},{\"end\":\"paragraph\"}]"
        );
        assert!(sink.report().is_lossless());
    }

    #[test]
    fn declares_contract() {
        let converter: &'static MarkdownToJson = &MarkdownToJson;
        assert_eq!(converter.entry().name, "markdown-to-json");
        assert_eq!(converter.entry().from.id, "markdown");
        assert_eq!(converter.entry().to.id, "markdown-json");
        assert_eq!(converter.entry().fidelity, Fidelity::Lossless);
    }
}
