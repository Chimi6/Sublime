//! RTF <-> Word, Pages, Markdown, HTML, and text: the RTF reader's document
//! model rendered by each format's writer, and each format's document model
//! rendered by the RTF writer.

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
use crate::io::html::HtmlWriter;
use crate::io::markdown::MarkdownWriter;
use crate::io::pages::{Package, Scope, read_document, write_package_to};
use crate::io::rtf::{read_rtf, write_rtf};
use crate::io::text::TextWriter;

const TO_DOCUMENT_NOTE: &str = "paragraphs, named styles, run and paragraph formatting, lists, tables (merged cells, borders, shading, nesting), sections with page setup, columns, headers, and footers, links, page-number fields, footnotes, comments, tracked changes, pictures (PNG, JPEG, EMF, WMF, bitmaps), text boxes, and the page colour carry over; other fields keep their shown text, drawn shapes without text, OLE objects (their picture is kept), and text in double-byte code pages without Unicode escapes are not carried";
const TO_MARKDOWN_NOTE: &str = "headings, paragraphs, emphasis, lists, tables, links, footnotes, and images by name carry over; fonts, sizes, colours, alignment, page setup, headers and footers, comments, and tracked changes (insertions kept, deletions dropped) do not";
const TO_HTML_NOTE: &str = "headings, paragraphs, emphasis, lists, tables, links, footnotes, and images by name carry over; fonts, sizes, colours, alignment, page setup, headers and footers, comments, and tracked changes (insertions kept, deletions dropped) do not";
const TO_TEXT_NOTE: &str = "the text of paragraphs, lists, tables, and footnotes; all formatting, pictures, and comments are dropped";

#[derive(Clone, Copy)]
enum Target {
    Docx,
    Pages,
    Markdown,
    Html,
    Text,
}

pub struct FromRtf {
    name: &'static str,
    to: &'static Format,
    target: Target,
    note: &'static str,
}

pub static RTF_TO_DOCX: FromRtf = FromRtf {
    name: "rtf-to-docx",
    to: &formats::DOCX,
    target: Target::Docx,
    note: TO_DOCUMENT_NOTE,
};

pub static RTF_TO_PAGES: FromRtf = FromRtf {
    name: "rtf-to-pages",
    to: &formats::PAGES,
    target: Target::Pages,
    note: TO_DOCUMENT_NOTE,
};

pub static RTF_TO_MARKDOWN: FromRtf = FromRtf {
    name: "rtf-to-markdown",
    to: &formats::MARKDOWN,
    target: Target::Markdown,
    note: TO_MARKDOWN_NOTE,
};

pub static RTF_TO_HTML: FromRtf = FromRtf {
    name: "rtf-to-html",
    to: &formats::HTML,
    target: Target::Html,
    note: TO_HTML_NOTE,
};

pub static RTF_TO_TEXT: FromRtf = FromRtf {
    name: "rtf-to-text",
    to: &formats::TEXT,
    target: Target::Text,
    note: TO_TEXT_NOTE,
};

/// Reads the whole input as an RTF document.
pub fn read_input(input: &mut Input<'_>) -> Result<Document, ConvertError> {
    let mut bytes = Vec::new();
    input.read_to_end(&mut bytes)?;
    read_rtf(&bytes).map_err(|error| ConvertError::Malformed {
        location: Location::default(),
        message: error.to_string(),
    })
}

impl Converter for FromRtf {
    fn name(&self) -> &'static str {
        self.name
    }

    fn from(&self) -> &'static Format {
        &formats::RTF
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
        _context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let document = read_input(&mut input)?;
        match self.target {
            Target::Docx => {
                write_docx(&document, &mut *output)?;
            }
            Target::Pages => {
                let mut sink = std::io::BufWriter::with_capacity(64 * 1024, &mut *output);
                write_package_to(&document, &mut sink).map_err(package_error)?;
                sink.flush()?;
            }
            Target::Markdown => {
                let mut writer = MarkdownWriter::streaming(&mut *output);
                emit_events(&document, &mut writer);
                writer.finish()?;
            }
            Target::Html => {
                let mut writer = HtmlWriter::streaming(&mut *output);
                emit_events(&document, &mut writer);
                writer.finish()?;
            }
            Target::Text => {
                let mut writer = TextWriter::streaming(&mut *output);
                emit_events(&document, &mut writer);
                writer.finish()?;
            }
        }
        output.flush()?;
        Ok(())
    }
}

const FROM_WORD_NOTE: &str = "paragraphs, named styles, run and paragraph formatting, lists (restarts and start numbers kept), tables (merged cells, borders, shading, nesting), sections with page setup, columns, headers, and footers, links, page-number fields, footnotes, comments with their replies, tracked changes, pictures (PNG, JPEG, EMF, WMF, BMP), text boxes, and the page colour carry over; charts, equations (their text is kept), and pictures in other formats do not";
const FROM_PAGES_NOTE: &str = "paragraphs, named styles, run and paragraph formatting, lists, tables, sections, headers and footers, links, footnotes, comments, tracked changes, pictures, text boxes, and the page colour carry over; charts and pictures in formats RTF cannot hold do not";
const FROM_TEXT_NOTE: &str = "headings, paragraphs, emphasis, lists, tables, links, footnotes, and code carry over as RTF formatting; images are kept only when the source holds their bytes";

#[derive(Clone, Copy)]
enum Source {
    Docx,
    Pages,
    Markdown,
    Html,
    Text,
}

pub struct ToRtf {
    name: &'static str,
    from: &'static Format,
    source: Source,
    note: &'static str,
}

pub static DOCX_TO_RTF: ToRtf = ToRtf {
    name: "docx-to-rtf",
    from: &formats::DOCX,
    source: Source::Docx,
    note: FROM_WORD_NOTE,
};

pub static PAGES_TO_RTF: ToRtf = ToRtf {
    name: "pages-to-rtf",
    from: &formats::PAGES,
    source: Source::Pages,
    note: FROM_PAGES_NOTE,
};

pub static MARKDOWN_TO_RTF: ToRtf = ToRtf {
    name: "markdown-to-rtf",
    from: &formats::MARKDOWN,
    source: Source::Markdown,
    note: FROM_TEXT_NOTE,
};

pub static HTML_TO_RTF: ToRtf = ToRtf {
    name: "html-to-rtf",
    from: &formats::HTML,
    source: Source::Html,
    note: FROM_TEXT_NOTE,
};

pub static TEXT_TO_RTF: ToRtf = ToRtf {
    name: "text-to-rtf",
    from: &formats::TEXT,
    source: Source::Text,
    note: FROM_TEXT_NOTE,
};

impl Converter for ToRtf {
    fn name(&self) -> &'static str {
        self.name
    }

    fn from(&self) -> &'static Format {
        self.from
    }

    fn to(&self) -> &'static Format {
        &formats::RTF
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
        _context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let document = match self.source {
            Source::Docx => {
                let mut bytes = Vec::new();
                input.read_to_end(&mut bytes)?;
                read_docx(&bytes).map_err(|error| ConvertError::Malformed {
                    location: Location::default(),
                    message: error.to_string(),
                })?
            }
            Source::Pages => {
                let mut bytes = Vec::new();
                input.read_to_end(&mut bytes)?;
                let package =
                    Package::read_scope(&bytes, Scope::Document).map_err(package_error)?;
                read_document(&package)
            }
            Source::Markdown => {
                let text = read_text_document(&mut input)?;
                let mut builder = DocumentBuilder::new();
                builder.reserve_text(text.len());
                crate::io::markdown::parse_into(
                    &text,
                    crate::io::markdown::Options::default(),
                    &mut builder,
                );
                builder.finish()
            }
            Source::Html => {
                let text = read_text_document(&mut input)?;
                let mut builder = DocumentBuilder::new();
                builder.reserve_text(text.len());
                crate::io::html::reader::parse_into(&text, &mut builder);
                builder.finish()
            }
            Source::Text => {
                let text = read_text_document(&mut input)?;
                let mut builder = DocumentBuilder::new();
                builder.reserve_text(text.len());
                crate::io::text::reader::parse_into(&text, &mut builder);
                builder.finish()
            }
        };
        let mut sink = std::io::BufWriter::with_capacity(64 * 1024, &mut *output);
        write_rtf(&document, &mut sink)?;
        sink.flush()?;
        drop(sink);
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
            (&RTF_TO_DOCX, "docx"),
            (&RTF_TO_PAGES, "pages"),
            (&RTF_TO_MARKDOWN, "markdown"),
            (&RTF_TO_HTML, "html"),
            (&RTF_TO_TEXT, "text"),
        ] {
            assert_eq!(converter.from().id, "rtf");
            assert_eq!(converter.to().id, to);
            assert_eq!(converter.name(), format!("rtf-to-{to}"));
        }
        for (converter, from) in [
            (&DOCX_TO_RTF, "docx"),
            (&PAGES_TO_RTF, "pages"),
            (&MARKDOWN_TO_RTF, "markdown"),
            (&HTML_TO_RTF, "html"),
            (&TEXT_TO_RTF, "text"),
        ] {
            assert_eq!(converter.from().id, from);
            assert_eq!(converter.to().id, "rtf");
            assert_eq!(converter.name(), format!("{from}-to-rtf"));
        }
    }
}
