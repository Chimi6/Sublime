//! PDF -> Markdown, HTML, and Word. Reads the PDF's text into the
//! Markdown event stream (headings from size and weight, list items from
//! their markers, paragraphs from blocks) and hands it to each writer.

use std::io::{Read, Write};

use crate::converter::{ConvertError, Converter, Fidelity, Input, Location, Tier};
use crate::converters::pdf_to_text::report_text_notes;
use crate::document::from_events::DocumentBuilder;
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::docx::DocxStream;
use crate::io::html::HtmlWriter;
use crate::io::markdown::{EventSink, MarkdownWriter};
use crate::io::pdf::markdown::pdf_events;

const FIDELITY_NOTE: &str = "headings are read from type size and weight and list items from their markers; fonts, layout, tables, images, and drawings are dropped; reading order is the order the page draws its text in; text in pages that are pictures (scans) is not read";

/// Which writer the events go to.
#[derive(Clone, Copy)]
enum Target {
    Markdown,
    Html,
    Docx,
}

pub struct PdfToDocument {
    name: &'static str,
    to: &'static Format,
    target: Target,
}

pub static PDF_TO_MARKDOWN: PdfToDocument = PdfToDocument {
    name: "pdf-to-markdown",
    to: &formats::MARKDOWN,
    target: Target::Markdown,
};

pub static PDF_TO_HTML: PdfToDocument = PdfToDocument {
    name: "pdf-to-html",
    to: &formats::HTML,
    target: Target::Html,
};

pub static PDF_TO_DOCX: PdfToDocument = PdfToDocument {
    name: "pdf-to-docx",
    to: &formats::DOCX,
    target: Target::Docx,
};

impl Converter for PdfToDocument {
    fn name(&self) -> &'static str {
        self.name
    }

    fn from(&self) -> &'static Format {
        &formats::PDF
    }

    fn to(&self) -> &'static Format {
        self.to
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
        context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let mut bytes = Vec::new();
        input.read_to_end(&mut bytes)?;
        let page = context.options.page;
        let malformed = |failure: crate::io::pdf::PdfError| ConvertError::Malformed {
            location: Location::default(),
            message: failure.0,
        };
        let notes = match self.target {
            Target::Markdown => {
                let mut writer = MarkdownWriter::streaming(&mut *output);
                let notes = pdf_events(&bytes, page, &mut writer as &mut dyn EventSink<'_>)
                    .map_err(malformed)?;
                writer.finish()?;
                notes
            }
            Target::Html => {
                let mut writer = HtmlWriter::streaming(&mut *output);
                let notes = pdf_events(&bytes, page, &mut writer as &mut dyn EventSink<'_>)
                    .map_err(malformed)?;
                writer.finish()?;
                notes
            }
            Target::Docx => {
                let mut stream = DocxStream::new(&mut *output)?;
                let mut builder = DocumentBuilder::streaming(&mut stream);
                let notes = pdf_events(&bytes, page, &mut builder as &mut dyn EventSink<'_>)
                    .map_err(malformed)?;
                let document = builder.finish_streaming()?;
                stream.finish(&document)?;
                notes
            }
        };
        output.flush()?;
        report_text_notes(&notes, context);
        Ok(())
    }
}
