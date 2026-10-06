//! Workbook -> delimited rows, every row as text cells: each sheet its own
//! file (a CSV holds one table), or the one `--sheet` picks.

use std::io::{Read, Write};

use crate::converter::{ConvertError, Converter, Fidelity, Input, Parts, Tier};
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::csv::CsvWriter;
use crate::io::xlsx::{Workbook, XlsxError};

const PROGRESS_INTERVAL: u64 = 4096;
const FIDELITY_NOTE: &str = "each sheet its own file (or the one --sheet picks); numbers as stored, dates as ISO 8601, booleans as TRUE and FALSE, formulas as their last value; formatting is dropped";

pub struct XlsxToCsv {
    pub name: &'static str,
    pub to: &'static Format,
    pub delimiter: u8,
}

pub static XLSX_TO_CSV: XlsxToCsv = XlsxToCsv {
    name: "xlsx-to-csv",
    to: &formats::CSV,
    delimiter: b',',
};

pub static XLSX_TO_TSV: XlsxToCsv = XlsxToCsv {
    name: "xlsx-to-tsv",
    to: &formats::TSV,
    delimiter: b'\t',
};

impl Converter for XlsxToCsv {
    fn name(&self) -> &'static str {
        self.name
    }

    fn from(&self) -> &'static Format {
        &formats::XLSX
    }

    fn to(&self) -> &'static Format {
        self.to
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
        let mut bytes = Vec::new();
        input.read_to_end(&mut bytes)?;
        let workbook = Workbook::open(&bytes)?;
        let sheet = workbook.select(context.options.sheet.as_deref())?;
        self.write_sheet(&workbook, sheet, output, context)
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
        let mut bytes = Vec::new();
        input.read_to_end(&mut bytes)?;
        let workbook = Workbook::open(&bytes)?;
        let names: Vec<String> = workbook
            .sheet_names()
            .into_iter()
            .map(str::to_string)
            .collect();
        let sheets: Vec<usize> = match context.options.sheet.as_deref() {
            Some(selector) => vec![workbook.select(Some(selector))?],
            None => (0..names.len()).collect(),
        };
        for sheet in sheets {
            let output = parts.part(&names[sheet])?;
            self.write_sheet(&workbook, sheet, output, context)?;
        }
        Ok(())
    }
}

impl XlsxToCsv {
    fn write_sheet(
        &self,
        workbook: &Workbook<'_>,
        sheet: usize,
        output: &mut dyn Write,
        context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let mut writer = CsvWriter::with_delimiter(output, self.delimiter);
        let mut count: u64 = 0;
        workbook.read_rows(sheet, |cells| -> Result<(), ConvertError> {
            writer.write_record(cells.iter().map(String::as_str))?;
            count += 1;
            if count % PROGRESS_INTERVAL == 0 {
                context.progress(self.name, count);
            }
            Ok(())
        })?;
        writer.flush()?;
        Ok(())
    }
}

const JSON_NOTE: &str = "one sheet as an array of objects keyed by its header row, several as an object of such arrays keyed by sheet name (or the one --sheet picks); every value as text: numbers as stored, dates as ISO 8601, booleans as TRUE and FALSE, formulas as their last value; formatting is dropped";

/// Workbook -> JSON: one sheet as the array `csv-to-json` makes of it,
/// several as an object of those arrays by sheet name, so a workbook is one
/// JSON file however many sheets it has.
pub struct XlsxToJson;

pub static XLSX_TO_JSON: XlsxToJson = XlsxToJson;

impl Converter for XlsxToJson {
    fn name(&self) -> &'static str {
        "xlsx-to-json"
    }

    fn from(&self) -> &'static Format {
        &formats::XLSX
    }

    fn to(&self) -> &'static Format {
        &formats::JSON
    }

    fn fidelity(&self) -> Fidelity {
        Fidelity::Conditional(JSON_NOTE)
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
        let workbook = Workbook::open(&bytes)?;
        let names: Vec<String> = workbook
            .sheet_names()
            .into_iter()
            .map(str::to_string)
            .collect();
        let sheets: Vec<usize> = match context.options.sheet.as_deref() {
            Some(selector) => vec![workbook.select(Some(selector))?],
            None => (0..names.len()).collect(),
        };
        let mut tables: Vec<(String, Vec<u8>)> = Vec::new();
        for sheet in sheets {
            let mut csv = Vec::new();
            XLSX_TO_CSV.write_sheet(&workbook, sheet, &mut csv, context)?;
            tables.push((names[sheet].clone(), csv));
        }
        crate::converters::csv_to_json::tables_to_json(&tables, output, context)
    }
}

const MARKDOWN_NOTE: &str = "each sheet a table under a heading of its name (one sheet, a table alone), the first row as the header; every value as text, rows padded or cut to the header's width, line breaks inside cells as spaces; formatting is dropped";

/// Workbook -> Markdown: a sheet is a table, several sheets each a table
/// under a heading of its name, so a workbook reaches every document
/// format whole.
pub struct XlsxToMarkdown;

pub static XLSX_TO_MARKDOWN: XlsxToMarkdown = XlsxToMarkdown;

impl Converter for XlsxToMarkdown {
    fn name(&self) -> &'static str {
        "xlsx-to-markdown"
    }

    fn from(&self) -> &'static Format {
        &formats::XLSX
    }

    fn to(&self) -> &'static Format {
        &formats::MARKDOWN
    }

    fn fidelity(&self) -> Fidelity {
        Fidelity::Conditional(MARKDOWN_NOTE)
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
        let workbook = Workbook::open(&bytes)?;
        let names: Vec<String> = workbook
            .sheet_names()
            .into_iter()
            .map(str::to_string)
            .collect();
        let sheets: Vec<usize> = match context.options.sheet.as_deref() {
            Some(selector) => vec![workbook.select(Some(selector))?],
            None => (0..names.len()).collect(),
        };
        let mut tables: Vec<(String, Vec<u8>)> = Vec::new();
        for sheet in sheets {
            let mut csv = Vec::new();
            XLSX_TO_CSV.write_sheet(&workbook, sheet, &mut csv, context)?;
            tables.push((names[sheet].clone(), csv));
        }
        crate::converters::rows_document::tables_to_markdown(&tables, output, context)
    }
}

impl From<XlsxError> for ConvertError {
    fn from(error: XlsxError) -> Self {
        match error {
            XlsxError::NoSuchSheet { .. } => ConvertError::Unsupported(error.to_string()),
            other => ConvertError::Malformed {
                location: crate::converter::Location::default(),
                message: other.to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declares_contract() {
        assert_eq!(XLSX_TO_CSV.name(), "xlsx-to-csv");
        assert_eq!(XLSX_TO_CSV.from().id, "xlsx");
        assert_eq!(XLSX_TO_TSV.to().id, "tsv");
        assert_eq!(XLSX_TO_CSV.fidelity(), Fidelity::Conditional(FIDELITY_NOTE));
    }
}
