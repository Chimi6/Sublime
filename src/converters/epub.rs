//! EPUB <-> Word, Pages, RTF, ODT, Markdown, HTML, and text: the EPUB
//! reader's document model rendered by each format's writer, and each
//! format's document model rendered by the EPUB writer.

use std::io::{Read, Write};

use crate::converter::{ConvertError, Converter, Fidelity, Input, Location, Tier};
use crate::converters::input::read_text_document;
use crate::converters::pages_to_json::package_error;
use crate::document::Document;
use crate::document::from_events::DocumentBuilder;
use crate::document::markdown::emit_events;
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::docx::{read_docx, write_docx};
use crate::io::epub::{read_epub, write_epub};
use crate::io::html::HtmlWriter;
use crate::io::markdown::MarkdownWriter;
use crate::io::odt::write_odt;
use crate::io::pages::{Package, Scope, read_document, write_package_to};
use crate::io::rtf::write_rtf;
use crate::io::text::TextWriter;

const TO_DOCUMENT_NOTE: &str = "chapters (each starting a new page), headings, paragraphs, emphasis, lists, tables, links out of the book, footnotes and endnotes (as footnotes), and pictures carry over; the book's style sheets, its metadata, and links between its own pages do not";
const TO_MARKDOWN_NOTE: &str = "chapters, headings, paragraphs, emphasis, lists, tables, links out of the book, footnotes and endnotes, and images by name carry over; the book's style sheets and metadata do not";
const TO_TEXT_NOTE: &str =
    "the text of chapters, lists, tables, and notes; all formatting and pictures are dropped";

/// A document format on the other side of an EPUB conversion.
#[derive(Clone, Copy)]
enum Other {
    Docx,
    Pages,
    Rtf,
    Odt,
    Markdown,
    Html,
    Text,
}

pub struct FromEpub {
    name: &'static str,
    to: &'static Format,
    target: Other,
    note: &'static str,
}

macro_rules! from_epub {
    ($name:ident, $id:literal, $format:ident, $target:ident, $note:ident) => {
        pub static $name: FromEpub = FromEpub {
            name: $id,
            to: &formats::$format,
            target: Other::$target,
            note: $note,
        };
    };
}

from_epub!(EPUB_TO_DOCX, "epub-to-docx", DOCX, Docx, TO_DOCUMENT_NOTE);
from_epub!(
    EPUB_TO_PAGES,
    "epub-to-pages",
    PAGES,
    Pages,
    TO_DOCUMENT_NOTE
);
from_epub!(EPUB_TO_RTF, "epub-to-rtf", RTF, Rtf, TO_DOCUMENT_NOTE);
from_epub!(EPUB_TO_ODT, "epub-to-odt", ODT, Odt, TO_DOCUMENT_NOTE);
from_epub!(
    EPUB_TO_MARKDOWN,
    "epub-to-markdown",
    MARKDOWN,
    Markdown,
    TO_MARKDOWN_NOTE
);
from_epub!(EPUB_TO_HTML, "epub-to-html", HTML, Html, TO_MARKDOWN_NOTE);
from_epub!(EPUB_TO_TEXT, "epub-to-text", TEXT, Text, TO_TEXT_NOTE);

/// Reads the whole input as an EPUB.
pub fn read_input(input: &mut Input<'_>) -> Result<Document, ConvertError> {
    let mut bytes = Vec::new();
    input.read_to_end(&mut bytes)?;
    read_epub(&bytes).map_err(|error| malformed(error.to_string()))
}

fn malformed(message: String) -> ConvertError {
    ConvertError::Malformed {
        location: Location::default(),
        message,
    }
}

impl Converter for FromEpub {
    fn name(&self) -> &'static str {
        self.name
    }

    fn from(&self) -> &'static Format {
        &formats::EPUB
    }

    fn to(&self) -> &'static Format {
        self.to
    }

    fn fidelity(&self) -> Fidelity {
        Fidelity::Lossy(self.note)
    }

    fn tier(&self) -> Tier {
        Tier::Native
    }

    fn convert(
        &self,
        mut input: Input<'_>,
        output: &mut dyn Write,
        context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let document = read_input(&mut input)?;
        write_other(self.target, &document, output, context)
    }
}

/// Renders `document` in `target`'s format.
fn write_other(
    target: Other,
    document: &Document,
    output: &mut dyn Write,
    context: &mut Context<'_>,
) -> Result<(), ConvertError> {
    let effort = context.options.effort.unwrap_or_default();
    match target {
        Other::Docx => {
            write_docx(document, &mut *output, effort)?;
        }
        Other::Odt => {
            write_odt(document, &mut *output, effort)?;
        }
        Other::Pages => {
            let mut sink = std::io::BufWriter::with_capacity(64 * 1024, &mut *output);
            write_package_to(document, &mut sink).map_err(package_error)?;
            sink.flush()?;
        }
        Other::Rtf => {
            let mut sink = std::io::BufWriter::with_capacity(64 * 1024, &mut *output);
            write_rtf(document, &mut sink)?;
            sink.flush()?;
        }
        Other::Markdown => {
            let mut writer = MarkdownWriter::streaming(&mut *output);
            emit_events(document, &mut writer);
            writer.finish()?;
        }
        Other::Html => {
            let mut writer = HtmlWriter::streaming(&mut *output);
            emit_events(document, &mut writer);
            writer.finish()?;
        }
        Other::Text => {
            let mut writer = TextWriter::streaming(&mut *output);
            emit_events(document, &mut writer);
            writer.finish()?;
        }
    }
    output.flush()?;
    Ok(())
}

/// Reads `input` as `source`'s format into the document model.
fn read_other(source: Other, input: &mut Input<'_>) -> Result<Document, ConvertError> {
    Ok(match source {
        Other::Docx => {
            let mut bytes = Vec::new();
            input.read_to_end(&mut bytes)?;
            read_docx(&bytes).map_err(|error| malformed(error.to_string()))?
        }
        Other::Pages => {
            let mut bytes = Vec::new();
            input.read_to_end(&mut bytes)?;
            let package = Package::read_scope(&bytes, Scope::Document).map_err(package_error)?;
            read_document(&package)
        }
        Other::Rtf => crate::converters::rtf::read_input(input)?,
        Other::Odt => crate::converters::odt::read_input(input)?,
        Other::Markdown | Other::Html | Other::Text => {
            let text = read_text_document(input)?;
            let mut builder = DocumentBuilder::new();
            builder.reserve_text(text.len());
            match source {
                Other::Markdown => crate::io::markdown::parse_into(
                    &text,
                    crate::io::markdown::Options::default(),
                    &mut builder,
                ),
                Other::Html => crate::io::html::reader::parse_into(&text, &mut builder),
                _ => crate::io::text::reader::parse_into(&text, &mut builder),
            }
            builder.finish()
        }
    })
}

const FROM_DOCUMENT_NOTE: &str = "chapters split at page breaks and top headings, with a table of contents; named styles (as CSS), run and paragraph formatting, lists, tables (merged cells, shading), links, footnotes (as EPUB pop-up notes), pictures, equations (as MathML), text boxes, and embedded fonts carry over; pages, headers and footers, comments, and deleted text do not";
const FROM_TEXT_NOTE: &str = "chapters split at top headings, with a table of contents; headings, paragraphs, emphasis, lists, tables, links, footnotes, and code carry over; images are kept only when the source holds their bytes";

pub struct ToEpub {
    name: &'static str,
    from: &'static Format,
    source: Other,
    note: &'static str,
}

macro_rules! to_epub {
    ($name:ident, $id:literal, $format:ident, $source:ident, $note:ident) => {
        pub static $name: ToEpub = ToEpub {
            name: $id,
            from: &formats::$format,
            source: Other::$source,
            note: $note,
        };
    };
}

to_epub!(DOCX_TO_EPUB, "docx-to-epub", DOCX, Docx, FROM_DOCUMENT_NOTE);
to_epub!(
    PAGES_TO_EPUB,
    "pages-to-epub",
    PAGES,
    Pages,
    FROM_DOCUMENT_NOTE
);
to_epub!(RTF_TO_EPUB, "rtf-to-epub", RTF, Rtf, FROM_DOCUMENT_NOTE);
to_epub!(ODT_TO_EPUB, "odt-to-epub", ODT, Odt, FROM_DOCUMENT_NOTE);
to_epub!(
    MARKDOWN_TO_EPUB,
    "markdown-to-epub",
    MARKDOWN,
    Markdown,
    FROM_TEXT_NOTE
);
to_epub!(HTML_TO_EPUB, "html-to-epub", HTML, Html, FROM_TEXT_NOTE);
to_epub!(TEXT_TO_EPUB, "text-to-epub", TEXT, Text, FROM_TEXT_NOTE);

impl Converter for ToEpub {
    fn name(&self) -> &'static str {
        self.name
    }

    fn from(&self) -> &'static Format {
        self.from
    }

    fn to(&self) -> &'static Format {
        &formats::EPUB
    }

    fn fidelity(&self) -> Fidelity {
        Fidelity::Lossy(self.note)
    }

    fn tier(&self) -> Tier {
        Tier::Native
    }

    fn convert(
        &self,
        mut input: Input<'_>,
        output: &mut dyn Write,
        context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let document = read_other(self.source, &mut input)?;
        write_epub(
            &document,
            &mut *output,
            context.options.effort.unwrap_or_default(),
        )?;
        output.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declares_contracts() {
        for (converter, to) in [
            (&EPUB_TO_DOCX, "docx"),
            (&EPUB_TO_PAGES, "pages"),
            (&EPUB_TO_RTF, "rtf"),
            (&EPUB_TO_ODT, "odt"),
            (&EPUB_TO_MARKDOWN, "markdown"),
            (&EPUB_TO_HTML, "html"),
            (&EPUB_TO_TEXT, "text"),
        ] {
            assert_eq!(converter.from().id, "epub");
            assert_eq!(converter.to().id, to);
            assert_eq!(converter.name(), format!("epub-to-{to}"));
        }
        for (converter, from) in [
            (&DOCX_TO_EPUB, "docx"),
            (&PAGES_TO_EPUB, "pages"),
            (&RTF_TO_EPUB, "rtf"),
            (&ODT_TO_EPUB, "odt"),
            (&MARKDOWN_TO_EPUB, "markdown"),
            (&HTML_TO_EPUB, "html"),
            (&TEXT_TO_EPUB, "text"),
        ] {
            assert_eq!(converter.from().id, from);
            assert_eq!(converter.to().id, "epub");
            assert_eq!(converter.name(), format!("{from}-to-epub"));
        }
    }
}
