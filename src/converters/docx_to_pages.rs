//! Word -> Apple Pages. Reads the Word document into the document model and
//! writes a Pages package by rewriting a blank template's body storage.

use std::io::{Read, Write};

use crate::converter::{ConvertError, Converter, Fidelity, Input, Location, Tier};
use crate::converters::pages_to_json::package_error;
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::docx::read_docx;
use crate::io::pages::write_package_to;

const NAME: &str = "docx-to-pages";
const FIDELITY_NOTE: &str = "paragraphs, headings, bulleted and numbered lists, page size and margins, section columns (equal or of their own widths), page, column, and line breaks, and paragraph alignment, spacing, and indents reach Pages with its named styles; a run's bold, italic, colour, size, font (theme fonts included), underline, strikethrough, super- and subscript, caps, highlight, and raised or lowered text carry over (hidden text stays hidden); horizontal rules, page breaks before paragraphs, page numbering restarts, and legacy (VML) pictures and embedded objects' previews come through; any number of tables keep merged cells, shading, borders, cell margins, vertical alignment, and per-run formatting, with their lists as Pages lists (numbered ones keeping their running count across cells); nested tables flatten to tab-separated lines; headers and footers (first-page and odd/even variants, live page numbers) are native; links become clickable hyperlinks; footnotes and endnotes become Pages footnotes; comments and their reply threads keep their author, date, text, and range (overlapping ones too, and those on headers or cells are kept on the table or the body's start); tracked insertions and deletions stay tracked changes, with their authors and dates; inline images are placed with their bytes and size; shapes and text boxes (each one of a group) are drawn in place from Word's common presets with their fill and outline (those in the text line stay in it, those placed from a paragraph move with the text, and each keeps its text wrap, drawings behind the text stay behind it, arrowheads are drawn, and see-through fills are blended over the page); empty paragraphs keep the height of their paragraph mark; a document without headers or footers turns them off; drawings in headers and footers repeat on the pages of their variant; placement by page share, alignment, and relative size is honoured; the page colour fills the pages; framed paragraphs (page numbers beside a footer's text, positioned frames in the body) keep their place; pictures offered as PDF or metafile with a PNG fallback use the PNG; consecutive and trailing page breaks keep their pages; form checkboxes become box characters; charts become a table of their data and line arrowheads are not drawn";

pub struct DocxToPages;

impl Converter for DocxToPages {
    fn name(&self) -> &'static str {
        NAME
    }

    fn from(&self) -> &'static Format {
        &formats::DOCX
    }

    fn to(&self) -> &'static Format {
        &formats::PAGES
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
        _context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let mut bytes = Vec::new();
        input.read_to_end(&mut bytes)?;
        let document = read_docx(&bytes).map_err(|error| ConvertError::Malformed {
            location: Location { line: 0, column: 0 },
            message: error.to_string(),
        })?;
        drop(bytes);
        let mut sink = std::io::BufWriter::with_capacity(64 * 1024, &mut *output);
        write_package_to(&document, &mut sink).map_err(package_error)?;
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
    fn declares_contract() {
        assert_eq!(DocxToPages.name(), "docx-to-pages");
        assert_eq!(DocxToPages.from().id, "docx");
        assert_eq!(DocxToPages.to().id, "pages");
    }
}
