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
        let delimiter = crate::converters::input::written_delimiter(self.delimiter, context);
        self.write_sheet(&workbook, sheet, delimiter, output, context)
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
            let delimiter = crate::converters::input::written_delimiter(self.delimiter, context);
            self.write_sheet(&workbook, sheet, delimiter, output, context)?;
        }
        Ok(())
    }
}

impl XlsxToCsv {
    fn write_sheet(
        &self,
        workbook: &Workbook<'_>,
        sheet: usize,
        delimiter: u8,
        output: &mut dyn Write,
        context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let mut writer = CsvWriter::with_delimiter(output, delimiter);
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
const MARKDOWN_NOTE: &str = "each sheet a table under a heading of its name (one sheet, a table alone), the first row as the header; every value as text, rows padded or cut to the header's width, line breaks inside cells as spaces; formatting is dropped";

/// Writes tables (each as CSV, by name) as one document.
type Gather =
    fn(&[(String, Vec<u8>)], &mut dyn Write, &mut Context<'_>) -> Result<(), ConvertError>;

/// Workbook -> one document of every sheet: JSON (one sheet as the array
/// `csv-to-json` makes of it, several as an object of those arrays by
/// sheet name) or Markdown (a table under a heading per sheet, so a
/// workbook reaches every document format whole).
pub struct XlsxWhole {
    name: &'static str,
    to: &'static Format,
    note: &'static str,
    /// Writes the gathered sheets, each as CSV, as the target.
    gather: Gather,
}

pub static XLSX_TO_JSON: XlsxWhole = XlsxWhole {
    name: "xlsx-to-json",
    to: &formats::JSON,
    note: JSON_NOTE,
    gather: crate::converters::csv_to_json::tables_to_json,
};

pub static XLSX_TO_MARKDOWN: XlsxWhole = XlsxWhole {
    name: "xlsx-to-markdown",
    to: &formats::MARKDOWN,
    note: MARKDOWN_NOTE,
    gather: crate::converters::rows_document::tables_to_markdown,
};

impl Converter for XlsxWhole {
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
        Fidelity::Conditional(self.note)
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
            XLSX_TO_CSV.write_sheet(&workbook, sheet, b',', &mut csv, context)?;
            tables.push((names[sheet].clone(), csv));
        }
        (self.gather)(&tables, output, context)
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
