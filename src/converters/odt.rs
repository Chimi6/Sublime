//! OpenDocument Text <-> Word, Pages, RTF, Markdown, HTML, and text: the
//! ODT reader's document model rendered by each format's writer, and each
//! format's document model rendered by the ODT writer.

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
use crate::io::odt::{read_odt, write_odt};
use crate::io::pages::{Package, Scope, read_document, write_package_to};
use crate::io::rtf::write_rtf;
use crate::io::text::TextWriter;

const TO_DOCUMENT_NOTE: &str = "paragraphs and headings, named styles, run and paragraph formatting, lists (restarts and start numbers kept), tables (merged cells, borders, shading, nesting), sections with page setup, columns, headers, and footers, links, page-number fields, footnotes (endnotes become footnotes), comments, tracked changes, pictures, and formulas (as MathML) carry over; other fields keep their shown text, text boxes and shapes' text follow their paragraph, drawn shapes, charts, and other embedded objects are not carried";
const TO_MARKDOWN_NOTE: &str = "headings, paragraphs, emphasis, lists, tables, links, footnotes, and images by name carry over; fonts, sizes, colours, alignment, page setup, headers and footers, comments, and tracked changes (insertions kept, deletions dropped) do not";
const TO_TEXT_NOTE: &str = "the text of paragraphs, lists, tables, and footnotes; all formatting, pictures, and comments are dropped";

#[derive(Clone, Copy)]
enum Target {
    Docx,
    Pages,
    Rtf,
    Markdown,
    Html,
    Text,
}

pub struct FromOdt {
    name: &'static str,
    to: &'static Format,
    target: Target,
    note: &'static str,
}

pub static ODT_TO_DOCX: FromOdt = FromOdt {
    name: "odt-to-docx",
    to: &formats::DOCX,
    target: Target::Docx,
    note: TO_DOCUMENT_NOTE,
};

pub static ODT_TO_PAGES: FromOdt = FromOdt {
    name: "odt-to-pages",
    to: &formats::PAGES,
    target: Target::Pages,
    note: TO_DOCUMENT_NOTE,
};

pub static ODT_TO_RTF: FromOdt = FromOdt {
    name: "odt-to-rtf",
    to: &formats::RTF,
    target: Target::Rtf,
    note: TO_DOCUMENT_NOTE,
};

pub static ODT_TO_MARKDOWN: FromOdt = FromOdt {
    name: "odt-to-markdown",
    to: &formats::MARKDOWN,
    target: Target::Markdown,
    note: TO_MARKDOWN_NOTE,
};

pub static ODT_TO_HTML: FromOdt = FromOdt {
    name: "odt-to-html",
    to: &formats::HTML,
    target: Target::Html,
    note: TO_MARKDOWN_NOTE,
};

pub static ODT_TO_TEXT: FromOdt = FromOdt {
    name: "odt-to-text",
    to: &formats::TEXT,
    target: Target::Text,
    note: TO_TEXT_NOTE,
};

/// Reads the whole input as an OpenDocument text.
pub fn read_input(input: &mut Input<'_>) -> Result<Document, ConvertError> {
    let mut bytes = Vec::new();
    input.read_to_end(&mut bytes)?;
    read_odt(&bytes).map_err(|error| ConvertError::Malformed {
        location: Location::default(),
        message: error.to_string(),
    })
}

impl Converter for FromOdt {
    fn name(&self) -> &'static str {
        self.name
    }

    fn from(&self) -> &'static Format {
        &formats::ODT
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
        match self.target {
            Target::Docx => {
                write_docx(
                    &document,
                    &mut *output,
                    context.options.effort.unwrap_or_default(),
                )?;
            }
            Target::Pages => {
                let mut sink = std::io::BufWriter::with_capacity(64 * 1024, &mut *output);
                write_package_to(&document, &mut sink).map_err(package_error)?;
                sink.flush()?;
            }
            Target::Rtf => {
                let mut sink = std::io::BufWriter::with_capacity(64 * 1024, &mut *output);
                write_rtf(&document, &mut sink)?;
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

const FROM_DOCUMENT_NOTE: &str = "paragraphs and headings, named styles, run and paragraph formatting, lists (restarts and start numbers kept), tables (merged cells, borders, shading, nesting), sections with page setup, columns, headers, and footers, links, page-number fields, footnotes, comments with their replies, tracked changes, pictures, text boxes, and equations (as formula objects) carry over; charts and drawn shapes do not";
const FROM_TEXT_NOTE: &str = "headings, paragraphs, emphasis, lists, tables, links, footnotes, and code carry over as OpenDocument formatting; images are kept only when the source holds their bytes";

#[derive(Clone, Copy)]
enum Source {
    Docx,
    Pages,
    Rtf,
    Markdown,
    Html,
    Text,
}

pub struct ToOdt {
    name: &'static str,
    from: &'static Format,
    source: Source,
    note: &'static str,
}

pub static DOCX_TO_ODT: ToOdt = ToOdt {
    name: "docx-to-odt",
    from: &formats::DOCX,
    source: Source::Docx,
    note: FROM_DOCUMENT_NOTE,
};

pub static PAGES_TO_ODT: ToOdt = ToOdt {
    name: "pages-to-odt",
    from: &formats::PAGES,
    source: Source::Pages,
    note: FROM_DOCUMENT_NOTE,
};

pub static RTF_TO_ODT: ToOdt = ToOdt {
    name: "rtf-to-odt",
    from: &formats::RTF,
    source: Source::Rtf,
    note: FROM_DOCUMENT_NOTE,
};

pub static MARKDOWN_TO_ODT: ToOdt = ToOdt {
    name: "markdown-to-odt",
    from: &formats::MARKDOWN,
    source: Source::Markdown,
    note: FROM_TEXT_NOTE,
};

pub static HTML_TO_ODT: ToOdt = ToOdt {
    name: "html-to-odt",
    from: &formats::HTML,
    source: Source::Html,
    note: FROM_TEXT_NOTE,
};

pub static TEXT_TO_ODT: ToOdt = ToOdt {
    name: "text-to-odt",
    from: &formats::TEXT,
    source: Source::Text,
    note: FROM_TEXT_NOTE,
};

impl Converter for ToOdt {
    fn name(&self) -> &'static str {
        self.name
    }

    fn from(&self) -> &'static Format {
        self.from
    }

    fn to(&self) -> &'static Format {
        &formats::ODT
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
        let malformed = |message: String| ConvertError::Malformed {
            location: Location::default(),
            message,
        };
        let document = match self.source {
            Source::Docx => {
                let mut bytes = Vec::new();
                input.read_to_end(&mut bytes)?;
                read_docx(&bytes).map_err(|error| malformed(error.to_string()))?
            }
            Source::Pages => {
                let mut bytes = Vec::new();
                input.read_to_end(&mut bytes)?;
                let package =
                    Package::read_scope(&bytes, Scope::Document).map_err(package_error)?;
                read_document(&package)
            }
            Source::Rtf => crate::converters::rtf::read_input(&mut input)?,
            Source::Markdown | Source::Html | Source::Text => {
                let text = read_text_document(&mut input)?;
                let mut builder = DocumentBuilder::new();
                builder.reserve_text(text.len());
                match self.source {
                    Source::Markdown => crate::io::markdown::parse_into(
                        &text,
                        crate::io::markdown::Options::default(),
                        &mut builder,
                    ),
                    Source::Html => crate::io::html::reader::parse_into(&text, &mut builder),
                    _ => crate::io::text::reader::parse_into(&text, &mut builder),
                }
                builder.finish()
            }
        };
        write_odt(
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
            (&ODT_TO_DOCX, "docx"),
            (&ODT_TO_PAGES, "pages"),
            (&ODT_TO_RTF, "rtf"),
            (&ODT_TO_MARKDOWN, "markdown"),
            (&ODT_TO_HTML, "html"),
            (&ODT_TO_TEXT, "text"),
        ] {
            assert_eq!(converter.from().id, "odt");
            assert_eq!(converter.to().id, to);
            assert_eq!(converter.name(), format!("odt-to-{to}"));
        }
        for (converter, from) in [
            (&DOCX_TO_ODT, "docx"),
            (&PAGES_TO_ODT, "pages"),
            (&RTF_TO_ODT, "rtf"),
            (&MARKDOWN_TO_ODT, "markdown"),
            (&HTML_TO_ODT, "html"),
            (&TEXT_TO_ODT, "text"),
        ] {
            assert_eq!(converter.from().id, from);
            assert_eq!(converter.to().id, "odt");
            assert_eq!(converter.name(), format!("{from}-to-odt"));
        }
    }
}
