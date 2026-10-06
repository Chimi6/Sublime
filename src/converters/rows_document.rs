//! The bridge between rows and documents: CSV and TSV into the document
//! hub as a Markdown table (so a spreadsheet reaches HTML, Word, and every
//! document format), and a document's tables out as rows: each its own CSV,
//! or each a sheet of one workbook.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Fidelity, Input, Location, Parts, Tier};
use crate::converters::input::read_text_document;
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::csv::{CsvReader, CsvWriter, DocumentTables, TableRows, emit_table};
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

const MARKDOWN_TO_ROWS_NOTE: &str = "each of the document's tables its own file, named after the heading before it, inline formatting flattened to text; everything else in the document is dropped";

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

    fn splits(&self) -> bool {
        true
    }

    fn convert_parts(
        &self,
        mut input: Input<'_>,
        parts: &mut dyn Parts,
        context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let text = read_text_document(&mut input)?;
        let mut tables = DocumentTables::new();
        parse_into(&text, Options::default(), &mut tables);
        if tables.tables.is_empty() {
            context.warning("the document has no table; the output is empty");
            parts.part("")?;
            return Ok(());
        }
        for (name, rows) in &tables.tables {
            let output = parts.part(name)?;
            let mut writer = CsvWriter::with_delimiter(output, self.delimiter);
            for row in rows {
                writer.write_record(row.iter().map(String::as_str))?;
            }
            writer.flush()?;
        }
        Ok(())
    }
}

/// Markdown -> workbook: every table a sheet, named after the heading
/// before it.
pub struct MarkdownToXlsx;

pub static MARKDOWN_TO_XLSX: MarkdownToXlsx = MarkdownToXlsx;

const MARKDOWN_TO_XLSX_NOTE: &str = "each of the document's tables a sheet, named after the heading before it, inline formatting flattened to text, decimals as numbers, ISO dates as dates, and TRUE and FALSE as booleans; everything else in the document is dropped";

impl Converter for MarkdownToXlsx {
    fn name(&self) -> &'static str {
        "markdown-to-xlsx"
    }

    fn from(&self) -> &'static Format {
        &formats::MARKDOWN
    }

    fn to(&self) -> &'static Format {
        &formats::XLSX
    }

    fn fidelity(&self) -> Fidelity {
        Fidelity::Lossy(MARKDOWN_TO_XLSX_NOTE)
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
        let mut tables = DocumentTables::new();
        parse_into(&text, Options::default(), &mut tables);
        if tables.tables.is_empty() {
            context.warning("the document has no table; the workbook has one empty sheet");
        }
        let first = tables
            .tables
            .first()
            .map_or("Sheet1", |(name, _)| name.as_str());
        let mut writer = crate::io::xlsx::XlsxWriter::new(&mut *output, first)?;
        for (index, (name, rows)) in tables.tables.iter().enumerate() {
            if index > 0 {
                writer.next_sheet(name)?;
            }
            for row in rows {
                writer.write_row(row.iter().map(String::as_str))?;
            }
        }
        writer.finish()?;
        output.flush()?;
        Ok(())
    }
}

/// Tables (each as CSV) as Markdown: one table alone, several each under a
/// heading of its name.
pub fn tables_to_markdown(
    tables: &[(String, Vec<u8>)],
    output: &mut dyn Write,
    context: &mut Context<'_>,
) -> Result<(), ConvertError> {
    let headed = tables.len() > 1;
    for (position, (name, csv)) in tables.iter().enumerate() {
        if position > 0 {
            output.write_all(b"\n")?;
        }
        if headed {
            let mut heading = String::from("# ");
            for character in name.chars() {
                // Characters Markdown would read as markup.
                if "\\`*_[]#<>|".contains(character) {
                    heading.push('\\');
                }
                heading.push(character);
            }
            heading.push_str("\n\n");
            output.write_all(heading.as_bytes())?;
        }
        let mut source: &[u8] = csv;
        CSV_TO_MARKDOWN.convert(Input::Stream(&mut source), output, context)?;
    }
    output.flush()?;
    Ok(())
}

/// JSON -> Markdown: an array of objects as one table, an object of such
/// arrays (a workbook's sheets) as a table under a heading per member.
pub struct JsonToMarkdown;

pub static JSON_TO_MARKDOWN: JsonToMarkdown = JsonToMarkdown;

const JSON_TO_MARKDOWN_NOTE: &str = "an array of objects as one table, an object of such arrays as a table under a heading per member; keys become the header row, nested values JSON text, and line breaks inside cells spaces";

impl Converter for JsonToMarkdown {
    fn name(&self) -> &'static str {
        "json-to-markdown"
    }

    fn from(&self) -> &'static Format {
        &formats::JSON
    }

    fn to(&self) -> &'static Format {
        &formats::MARKDOWN
    }

    fn fidelity(&self) -> Fidelity {
        Fidelity::Conditional(JSON_TO_MARKDOWN_NOTE)
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
        let mut tables = crate::converter::MemoryParts::default();
        crate::converters::json_to_csv::JSON_TO_CSV.convert_parts(input, &mut tables, context)?;
        tables_to_markdown(&tables.parts, output, context)
    }
}

/// Markdown -> JSON of its tables: one table as an array of objects keyed
/// by its header, several as an object of such arrays keyed by the heading
/// before each.
pub struct MarkdownTablesToJson;

pub static MARKDOWN_TABLES_TO_JSON: MarkdownTablesToJson = MarkdownTablesToJson;

const TABLES_TO_JSON_NOTE: &str = "the document's tables only: one as an array of objects keyed by its header, several as an object of such arrays keyed by the heading before each; inline formatting is flattened to text and everything else in the document is dropped";

impl Converter for MarkdownTablesToJson {
    fn name(&self) -> &'static str {
        "markdown-tables-to-json"
    }

    fn from(&self) -> &'static Format {
        &formats::MARKDOWN
    }

    fn to(&self) -> &'static Format {
        &formats::JSON
    }

    fn fidelity(&self) -> Fidelity {
        Fidelity::Lossy(TABLES_TO_JSON_NOTE)
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
        let mut collected = DocumentTables::new();
        parse_into(&text, Options::default(), &mut collected);
        if collected.tables.is_empty() {
            context.warning("the document has no table; the output is an empty array");
            output.write_all(b"[]")?;
            return Ok(());
        }
        let mut tables: Vec<(String, Vec<u8>)> = Vec::new();
        for (name, rows) in &collected.tables {
            let mut csv = Vec::new();
            {
                let mut writer = CsvWriter::new(&mut csv);
                for row in rows {
                    writer.write_record(row.iter().map(String::as_str))?;
                }
                writer.flush()?;
            }
            tables.push((name.clone(), csv));
        }
        crate::converters::csv_to_json::tables_to_json(&tables, output, context)
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
