//! PDF -> Text. Runs each page's content for its text and writes it as
//! pdftotext does without `-layout`: a line per line of the page, a blank
//! line between blocks, a form feed after each page.

use std::io::{Read, Write};

use crate::converter::{ConvertError, Converter, Fidelity, Input, Location, Tier};
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::pdf::text::{TextNotes, write_pdf_text};

const NAME: &str = "pdf-to-text";
const FIDELITY_NOTE: &str = "fonts, layout, images, and drawings are dropped; reading order is the order the page draws its text in; text in pages that are pictures (scans) is not read";

pub struct PdfToText;

impl Converter for PdfToText {
    fn name(&self) -> &'static str {
        NAME
    }

    fn from(&self) -> &'static Format {
        &formats::PDF
    }

    fn to(&self) -> &'static Format {
        &formats::TEXT
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
        let notes = write_pdf_text(&bytes, context.options.page, output).map_err(|failure| {
            ConvertError::Malformed {
                location: Location::default(),
                message: failure.0,
            }
        })?;
        output.flush()?;
        report_text_notes(&notes, context);
        Ok(())
    }
}

/// Warnings for what the text left out.
pub(crate) fn report_text_notes(notes: &TextNotes, context: &mut Context<'_>) {
    if notes.pages_without_text > 0 {
        context.warning(format!(
            "{} of {} pages drew no text (scanned pages are pictures; their text is not read)",
            notes.pages_without_text, notes.pages
        ));
    }
    if notes.rotated_glyphs > 0 {
        context.warning(format!(
            "{} glyphs are set rotated or vertically and read in drawing order",
            notes.rotated_glyphs
        ));
    }
}
