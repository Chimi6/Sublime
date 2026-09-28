//! The bridge between rows and documents: CSV and TSV into the document
//! hub as a Markdown table (so a spreadsheet reaches HTML, Word, and every
//! document format), and a document's first table out as rows.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Fidelity, Input, Location, Tier};
use crate::converters::input::read_text_document;
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::csv::{CsvReader, TableRows, emit_table};
use crate::io::markdown::{MarkdownWriter, Options, parse_into};

/// Rows -> Markdown: one table, the first row as its header.
pub struct RowsToMarkdown {
    pub name: &'static str,
    pub from: &'static Format,
    pub delimiter: u8,
}

const ROWS_TO_MARKDOWN_NOTE: &str = "one table with the first row as the header; rows are padded or cut to the header's width and line breaks inside cells become spaces";

pub static CSV_TO_MARKDOWN: RowsToMarkdown = RowsToMarkdown {
    name: "csv-to-markdown",
    from: &formats::CSV,
    delimiter: b',',
};

pub static TSV_TO_MARKDOWN: RowsToMarkdown = RowsToMarkdown {
    name: "tsv-to-markdown",
    from: &formats::TSV,
    delimiter: b'\t',
};

impl Converter for RowsToMarkdown {
    fn name(&self) -> &'static str {
        self.name
    }

    fn from(&self) -> &'static Format {
        self.from
    }

    fn to(&self) -> &'static Format {
        &formats::MARKDOWN
    }

    fn fidelity(&self) -> Fidelity {
        Fidelity::Conditional(ROWS_TO_MARKDOWN_NOTE)
    }

    fn tier(&self) -> Tier {
        Tier::Native
    }

    fn convert(
        &self,
        input: Input<'_>,
        output: &mut dyn Write,
        context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let mut reader = CsvReader::with_delimiter(input, self.delimiter);
        let mut writer = MarkdownWriter::streaming(output);
        let notes = emit_table(&mut reader, &mut writer)?;
        writer.finish()?;
        output.flush()?;
        if notes.trimmed_rows > 0 {
            context.loss(
                self.name,
                Location::default(),
                format!(
                    "{} row(s) had more cells than the header; the extra cells were dropped",
                    notes.trimmed_rows
                ),
            );
        }
        if notes.folded_cells > 0 {
            context.loss(
                self.name,
                Location::default(),
                format!(
                    "{} cell(s) held line breaks, folded to spaces",
                    notes.folded_cells
                ),
            );
        }
        Ok(())
    }
}

/// Markdown -> rows: the document's first table.
pub struct MarkdownToRows {
    pub name: &'static str,
    pub to: &'static Format,
    pub delimiter: u8,
}

const MARKDOWN_TO_ROWS_NOTE: &str = "only the document's first table, its inline formatting flattened to text; everything else in the document is dropped";

pub static MARKDOWN_TO_CSV: MarkdownToRows = MarkdownToRows {
    name: "markdown-to-csv",
    to: &formats::CSV,
    delimiter: b',',
};

pub static MARKDOWN_TO_TSV: MarkdownToRows = MarkdownToRows {
    name: "markdown-to-tsv",
    to: &formats::TSV,
    delimiter: b'\t',
};

impl Converter for MarkdownToRows {
    fn name(&self) -> &'static str {
        self.name
    }

    fn from(&self) -> &'static Format {
        &formats::MARKDOWN
    }

    fn to(&self) -> &'static Format {
        self.to
    }

    fn fidelity(&self) -> Fidelity {
        Fidelity::Lossy(MARKDOWN_TO_ROWS_NOTE)
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
        let text = read_text_document(&mut input)?;
        let mut rows = TableRows::new(output, self.delimiter);
        parse_into(&text, Options::default(), &mut rows);
        let found = rows.found_table();
        rows.finish()?;
        if !found {
            context.warning("the document has no table; the output is empty");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declares_contracts() {
        assert_eq!(CSV_TO_MARKDOWN.name(), "csv-to-markdown");
        assert_eq!(TSV_TO_MARKDOWN.from().id, "tsv");
        assert_eq!(CSV_TO_MARKDOWN.to().id, "markdown");
        assert_eq!(MARKDOWN_TO_CSV.from().id, "markdown");
        assert_eq!(MARKDOWN_TO_TSV.to().id, "tsv");
        assert!(matches!(MARKDOWN_TO_CSV.fidelity(), Fidelity::Lossy(_)));
    }
}
