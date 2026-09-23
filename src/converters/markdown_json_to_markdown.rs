//! Markdown events as JSON -> Markdown, through the Markdown writer. The
//! events stream from the JSON one object at a time.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Fidelity, Input, Tier};
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::markdown::MarkdownWriter;
use crate::io::markdown::events_json::read_events_json;

const NAME: &str = "markdown-json-to-markdown";

pub struct MarkdownJsonToMarkdown;

impl Converter for MarkdownJsonToMarkdown {
    fn name(&self) -> &'static str {
        NAME
    }

    fn from(&self) -> &'static Format {
        &formats::MARKDOWN_JSON
    }

    fn to(&self) -> &'static Format {
        &formats::MARKDOWN
    }

    fn fidelity(&self) -> Fidelity {
        Fidelity::Lossless
    }

    fn tier(&self) -> Tier {
        Tier::Native
    }

    fn convert(
        &self,
        input: Input<'_>,
        output: &mut dyn Write,
        _context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let mut writer = MarkdownWriter::streaming(output);
        read_events_json(input, &mut writer)?;
        writer.finish()?;
        output.flush()?;
        Ok(())
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
        let mut source: &[u8] = b"[{\"start\":\"heading\",\"level\":2},{\"text\":\"Hi\"},{\"end\":\"heading\",\"level\":2}]";
        let mut context = Context::new(&mut sink, &options);
        MarkdownJsonToMarkdown
            .convert(Input::Stream(&mut source), &mut output, &mut context)
            .unwrap();
        assert_eq!(String::from_utf8(output).unwrap(), "## Hi\n");
    }

    #[test]
    fn rejects_unknown_events() {
        let options = ConvertOptions::default();
        let mut sink = CollectingSink::new();
        let mut output = Vec::new();
        let mut source: &[u8] = b"[{\"dance\":true}]";
        let mut context = Context::new(&mut sink, &options);
        let error = MarkdownJsonToMarkdown
            .convert(Input::Stream(&mut source), &mut output, &mut context)
            .unwrap_err();
        assert!(matches!(error, ConvertError::Malformed { .. }));
    }
}
