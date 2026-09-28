//! Markdown, HTML, text, Word, and Pages -> PDF. Each reader's Markdown
//! event stream is set as pages by the composer (`io::pdf::compose`):
//! Letter pages, one-inch margins, the base-14 Helvetica and Courier.

use std::io::{Read, Write};

use crate::converter::{ConvertError, Converter, Fidelity, Input, Location, Tier};
use crate::converters::input::read_text_document;
use crate::converters::pages_to_json::package_error;
use crate::document::markdown::emit_events;
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::docx::read_docx;
use crate::io::markdown::{EventSink, Options};
use crate::io::pages::{Package, Scope, read_document};
use crate::io::pdf::PdfDocument;
use crate::io::pdf::compose::{ComposeNotes, Composer, PageSetup};

const FIDELITY_NOTE: &str = "set on Letter pages in Helvetica and Courier; images are shown by their alt text, raw HTML is dropped, and characters outside Western European text become ?";

#[derive(Clone, Copy)]
enum Source {
    Markdown,
    Html,
    Text,
    Docx,
    Pages,
}

pub struct ToPdf {
    name: &'static str,
    from: &'static Format,
    source: Source,
}

pub static MARKDOWN_TO_PDF: ToPdf = ToPdf {
    name: "markdown-to-pdf",
    from: &formats::MARKDOWN,
    source: Source::Markdown,
};

pub static HTML_TO_PDF: ToPdf = ToPdf {
    name: "html-to-pdf",
    from: &formats::HTML,
    source: Source::Html,
};

pub static TEXT_TO_PDF: ToPdf = ToPdf {
    name: "text-to-pdf",
    from: &formats::TEXT,
    source: Source::Text,
};

pub static DOCX_TO_PDF: ToPdf = ToPdf {
    name: "docx-to-pdf",
    from: &formats::DOCX,
    source: Source::Docx,
};

pub static PAGES_TO_PDF: ToPdf = ToPdf {
    name: "pages-to-pdf",
    from: &formats::PAGES,
    source: Source::Pages,
};

impl Converter for ToPdf {
    fn name(&self) -> &'static str {
        self.name
    }

    fn from(&self) -> &'static Format {
        self.from
    }

    fn to(&self) -> &'static Format {
        &formats::PDF
    }

    fn fidelity(&self) -> Fidelity {
        Fidelity::Conditional(FIDELITY_NOTE)
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
        let mut document = PdfDocument::new(&mut *output)?;
        let mut composer = Composer::new(&mut document, PageSetup::default());
        match self.source {
            Source::Markdown => {
                let text = read_text_document(&mut input)?;
                crate::io::markdown::parse_into(
                    &text,
                    Options::default(),
                    &mut composer as &mut dyn EventSink<'_>,
                );
                let notes = composer.finish()?;
                report(&notes, self.name, context);
            }
            Source::Html => {
                let text = read_text_document(&mut input)?;
                crate::io::html::reader::parse_into(&text, &mut composer as &mut dyn EventSink<'_>);
                let notes = composer.finish()?;
                report(&notes, self.name, context);
            }
            Source::Text => {
                let text = read_text_document(&mut input)?;
                crate::io::text::reader::parse_into(&text, &mut composer as &mut dyn EventSink<'_>);
                let notes = composer.finish()?;
                report(&notes, self.name, context);
            }
            Source::Docx => {
                let mut bytes = Vec::new();
                input.read_to_end(&mut bytes)?;
                let model = read_docx(&bytes).map_err(|error| ConvertError::Malformed {
                    location: Location::default(),
                    message: error.to_string(),
                })?;
                emit_events(&model, &mut composer as &mut dyn EventSink<'_>);
                let notes = composer.finish()?;
                report(&notes, self.name, context);
            }
            Source::Pages => {
                let mut bytes = Vec::new();
                input.read_to_end(&mut bytes)?;
                let package =
                    Package::read_scope(&bytes, Scope::Document).map_err(package_error)?;
                let model = read_document(&package);
                emit_events(&model, &mut composer as &mut dyn EventSink<'_>);
                let notes = composer.finish()?;
                report(&notes, self.name, context);
            }
        }
        document.finish()?;
        output.flush()?;
        Ok(())
    }
}

fn report(notes: &ComposeNotes, name: &'static str, context: &mut Context<'_>) {
    if notes.unset_characters > 0 {
        context.loss(
            name,
            Location::default(),
            format!(
                "{} characters outside Western European text set as ? (the standard PDF fonts cover WinAnsi only)",
                notes.unset_characters
            ),
        );
    }
    if notes.images > 0 {
        context.warning(format!("{} images shown by their alt text", notes.images));
    }
    if notes.html > 0 {
        context.warning(format!("{} pieces of raw HTML dropped", notes.html));
    }
}
