//! Markdown -> HTML. Parses with `io::markdown` and renders with `io::html`.
//! Markdown allows a link to be defined after its use, so the whole input
//! is read before parsing; memory is proportional to the document.

use std::io::{Read, Write};

use crate::converter::{ConvertError, Converter, Fidelity, Input, Location, Tier};
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::html::write_html;
use crate::io::markdown::Parser;

const NAME: &str = "markdown-to-html";

pub struct MarkdownToHtml;

impl Converter for MarkdownToHtml {
    fn name(&self) -> &'static str {
        NAME
    }

    fn from(&self) -> &'static Format {
        &formats::MARKDOWN
    }

    fn to(&self) -> &'static Format {
        &formats::HTML
    }

    fn fidelity(&self) -> Fidelity {
        Fidelity::Lossless
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
        let mut bytes = Vec::new();
        input.read_to_end(&mut bytes)?;
        let text = match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(error) => {
                let offset = error.utf8_error().valid_up_to();
                let newlines = error.as_bytes()[..offset]
                    .iter()
                    .filter(|byte| **byte == b'\n')
                    .count();
                return Err(ConvertError::Malformed {
                    location: Location {
                        line: newlines as u64 + 1,
                        column: 0,
                    },
                    message: "invalid UTF-8".to_string(),
                });
            }
        };
        // The spec replaces NUL with U+FFFD for security.
        let text = if text.contains('\0') {
            text.replace('\0', "\u{FFFD}")
        } else {
            text
        };
        let parser = Parser::new(&text);
        write_html(output, parser)?;
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
        let converter = MarkdownToHtml;
        assert_eq!(converter.name(), "markdown-to-html");
        assert_eq!(converter.from().id, "markdown");
        assert_eq!(converter.to().id, "html");
        assert_eq!(converter.fidelity(), Fidelity::Lossless);
        assert_eq!(converter.tier(), Tier::Native);
    }
}
