//! Rows -> Numbers (`io::pages::write_numbers_to`): delimited rows as one
//! sheet of one table, a workbook as a sheet per worksheet, JSON as a sheet
//! per table (an object of arrays a sheet per member), and a document's
//! tables as tables of one sheet, each named after the heading before it.
//! Cells are typed by their text: plain decimals are numbers, ISO 8601
//! dates and dates with times are dates, `TRUE` and `FALSE` are booleans.

use std::io::{Read, Write};

use crate::converter::MemoryParts;
use crate::converter::{ConvertError, Converter, Fidelity, Input, Tier};
use crate::converters::pages_to_json::package_error;
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::csv::{CsvReader, DocumentTables, Record};
use crate::io::markdown::{Options, parse_into};
use crate::io::pages::{NumbersSheet, NumbersTable, write_numbers_to};
use crate::io::xlsx::Workbook;

const NOTE: &str = "a sheet per worksheet (delimited rows one sheet, JSON a sheet per table), each one table with its first row the header; plain decimals become numbers, ISO 8601 dates and dates with times dates, TRUE and FALSE booleans, everything else text; Numbers' default table style, no formats or merges";
const DOCUMENT_NOTE: &str = "the document's tables as tables of one sheet, each named after the heading before it, its first row the header; plain decimals become numbers, ISO 8601 dates and dates with times dates, TRUE and FALSE booleans, everything else text; everything but the tables is dropped";

#[derive(Clone, Copy)]
enum Source {
    /// Delimited rows, by their column delimiter.
    Delimited(u8),
    Workbook,
    Json,
    Document,
}

pub struct ToNumbers {
    name: &'static str,
    from: &'static Format,
    source: Source,
}

pub static CSV_TO_NUMBERS: ToNumbers = ToNumbers {
    name: "csv-to-numbers",
    from: &formats::CSV,
    source: Source::Delimited(b','),
};

pub static TSV_TO_NUMBERS: ToNumbers = ToNumbers {
    name: "tsv-to-numbers",
    from: &formats::TSV,
    source: Source::Delimited(b'\t'),
};

pub static XLSX_TO_NUMBERS: ToNumbers = ToNumbers {
    name: "xlsx-to-numbers",
    from: &formats::XLSX,
    source: Source::Workbook,
};

pub static JSON_TO_NUMBERS: ToNumbers = ToNumbers {
    name: "json-to-numbers",
    from: &formats::JSON,
    source: Source::Json,
};

pub static MARKDOWN_TO_NUMBERS: ToNumbers = ToNumbers {
    name: "markdown-to-numbers",
    from: &formats::MARKDOWN,
    source: Source::Document,
};

impl Converter for ToNumbers {
    fn name(&self) -> &'static str {
        self.name
    }

    fn from(&self) -> &'static Format {
        self.from
    }

    fn to(&self) -> &'static Format {
        &formats::NUMBERS
    }

    fn fidelity(&self) -> Fidelity {
        match self.source {
            Source::Document => Fidelity::Lossy(DOCUMENT_NOTE),
            _ => Fidelity::Conditional(NOTE),
        }
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
        let sheets = match self.source {
            Source::Delimited(delimiter) => vec![one_table(
                "Sheet 1",
                "Table 1",
                read_rows(&mut input, delimiter)?,
            )],
            Source::Workbook => {
                let mut bytes = Vec::new();
                input.read_to_end(&mut bytes)?;
                let workbook = Workbook::open(&bytes)?;
                let names: Vec<String> = workbook
                    .sheet_names()
                    .into_iter()
                    .map(str::to_string)
                    .collect();
                let picked: Vec<usize> = match context.options.sheet.as_deref() {
                    Some(selector) => vec![workbook.select(Some(selector))?],
                    None => (0..names.len()).collect(),
                };
                let mut sheets = Vec::with_capacity(picked.len());
                for sheet in picked {
                    let mut rows: Vec<Vec<String>> = Vec::new();
                    workbook.read_rows(sheet, |cells| -> Result<(), ConvertError> {
                        rows.push(cells.to_vec());
                        Ok(())
                    })?;
                    sheets.push(one_table(&names[sheet], "Table 1", rows));
                }
                sheets
            }
            Source::Json => {
                let mut parts = MemoryParts::default();
                crate::converters::json_to_csv::JSON_TO_CSV
                    .convert_parts(input, &mut parts, context)?;
                let mut sheets = Vec::with_capacity(parts.parts.len());
                for (index, (name, csv)) in parts.parts.iter().enumerate() {
                    let name = if name.is_empty() {
                        format!("Sheet {}", index + 1)
                    } else {
                        name.clone()
                    };
                    sheets.push(one_table(
                        &name,
                        "Table 1",
                        read_rows(&mut csv.as_slice(), b',')?,
                    ));
                }
                sheets
            }
            Source::Document => {
                let text = crate::converters::input::read_text_document(&mut input)?;
                let mut found = DocumentTables::new();
                parse_into(&text, Options::default(), &mut found);
                if found.tables.is_empty() {
                    context.warning("the document has no table; the sheet has one empty table");
                }
                vec![NumbersSheet {
                    name: "Sheet 1".to_string(),
                    tables: found
                        .tables
                        .into_iter()
                        .map(|(name, rows)| NumbersTable { name, rows })
                        .collect(),
                }]
            }
        };
        write_numbers_to(sheets, output).map_err(package_error)?;
        output.flush()?;
        Ok(())
    }
}

fn one_table(sheet: &str, table: &str, rows: Vec<Vec<String>>) -> NumbersSheet {
    NumbersSheet {
        name: sheet.to_string(),
        tables: vec![NumbersTable {
            name: table.to_string(),
            rows,
        }],
    }
}

fn read_rows(input: &mut dyn Read, delimiter: u8) -> Result<Vec<Vec<String>>, ConvertError> {
    let mut reader = CsvReader::with_delimiter(input, delimiter);
    let mut record = Record::new();
    let mut rows = Vec::new();
    while reader.read_record(&mut record)? {
        rows.push(record.fields().map(str::to_string).collect());
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declares_contract() {
        assert_eq!(CSV_TO_NUMBERS.name(), "csv-to-numbers");
        assert_eq!(XLSX_TO_NUMBERS.from().id, "xlsx");
        assert_eq!(TSV_TO_NUMBERS.to().id, "numbers");
    }
}
