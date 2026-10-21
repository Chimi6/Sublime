//! Markdown -> HTML. Parses with `io::markdown` and renders with `io::html`.
//! Markdown allows a link to be defined after its use, so the whole input
//! is read before parsing; memory is proportional to the document.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Entry, Fidelity, Input, Tier};
use crate::converters::input::read_text_document;
use crate::event::Context;
use crate::format::formats;
use crate::io::html::HtmlWriter;
use crate::io::markdown::{Options, parse_into};

const NAME: &str = "markdown-to-html";

pub struct MarkdownToHtml;

impl Converter for MarkdownToHtml {
    fn convert(
        &self,
        mut input: Input<'_>,
        output: &mut dyn Write,
        _context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let text = read_text_document(&mut input)?;
        let mut writer = HtmlWriter::streaming(output);
        parse_into(&text, Options::default(), &mut writer);
        writer.finish()?;
        output.flush()?;
        Ok(())
    }
}

impl MarkdownToHtml {
    pub const fn entry(&'static self) -> Entry {
        Entry {
            name: NAME,
            from: &formats::MARKDOWN,
            to: &formats::HTML,
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
        let mut source: &[u8] = b"# Title\n\nSome *text* with a [link](/x).\n";
        let mut context = Context::new(&mut sink, &options);
        MarkdownToHtml
            .convert(Input::Stream(&mut source), &mut output, &mut context)
            .unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "<h1>Title</h1>\n<p>Some <em>text</em> with a <a href=\"/x\">link</a>.</p>\n"
        );
        assert!(sink.report().is_lossless());
    }

    #[test]
    fn declares_contract() {
        let converter: &'static MarkdownToHtml = &MarkdownToHtml;
        assert_eq!(converter.entry().name, "markdown-to-html");
        assert_eq!(converter.entry().from.id, "markdown");
        assert_eq!(converter.entry().to.id, "html");
        assert_eq!(converter.entry().fidelity, Fidelity::Lossless);
        assert_eq!(converter.entry().tier, Tier::Native);
    }
}
