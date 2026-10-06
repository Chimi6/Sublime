//! Numbers -> rows: a Numbers document is sheets, each holding tables, read
//! by the same table reader as Pages (`io::pages::read_workbook`). Every
//! table is one part: its own file for formats that hold one table (CSV,
//! TSV, and through them JSON Lines), one sheet of a workbook, one member
//! of a JSON object, one table of a document. A table is named after its
//! sheet when the sheet holds only it, and "Sheet - Table" otherwise, as
//! Numbers names worksheets when it exports to Excel.

use std::io::{Read, Write};

use crate::converter::{ConvertError, Converter, Fidelity, Input, Location, Parts, Tier};
use crate::converters::pages_to_json::package_error;
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::csv::CsvWriter;
use crate::io::pages::{Package, Scope, WorkbookReader};

#[derive(Clone, Copy)]
enum Target {
    Rows(u8),
    Json,
    Markdown,
    Xlsx,
}

pub struct NumbersTo {
    name: &'static str,
    to: &'static Format,
    target: Target,
    note: &'static str,
}

const ROWS_NOTE: &str = "each table its own file (or the one --sheet picks), named after its sheet; cells as Numbers shows them, merged-over cells empty; formatting, formulas, charts, and pictures are dropped";

pub static NUMBERS_TO_CSV: NumbersTo = NumbersTo {
    name: "numbers-to-csv",
    to: &formats::CSV,
    target: Target::Rows(b','),
    note: ROWS_NOTE,
};

pub static NUMBERS_TO_TSV: NumbersTo = NumbersTo {
    name: "numbers-to-tsv",
    to: &formats::TSV,
    target: Target::Rows(b'\t'),
    note: ROWS_NOTE,
};

pub static NUMBERS_TO_JSON: NumbersTo = NumbersTo {
    name: "numbers-to-json",
    to: &formats::JSON,
    target: Target::Json,
    note: "one table as an array of objects keyed by its header row, several as an object of such arrays keyed by table name (or the one --sheet picks); cells as Numbers shows them, as text; formatting, formulas, charts, and pictures are dropped",
};

pub static NUMBERS_TO_MARKDOWN: NumbersTo = NumbersTo {
    name: "numbers-to-markdown",
    to: &formats::MARKDOWN,
    target: Target::Markdown,
    note: "each table under a heading of its name (one table, the table alone), the first row as the header; cells as Numbers shows them, line breaks inside cells as spaces; formatting, formulas, charts, and pictures are dropped",
};

pub static NUMBERS_TO_XLSX: NumbersTo = NumbersTo {
    name: "numbers-to-xlsx",
    to: &formats::XLSX,
    target: Target::Xlsx,
    note: "each table a worksheet (or the one --sheet picks); numbers, dates, and booleans as cells of their type, everything else as text; formatting, formulas, merges, charts, and pictures are dropped",
};

impl Converter for NumbersTo {
    fn name(&self) -> &'static str {
        self.name
    }

    fn from(&self) -> &'static Format {
        &formats::NUMBERS
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
        let bytes = read_all(&mut input)?;
        let package = Package::read_scope(&bytes, Scope::Workbook).map_err(package_error)?;
        let mut workbook = WorkbookReader::new(&package);
        let tables = pick_tables(&workbook, context)?;
        match self.target {
            Target::Rows(delimiter) => {
                // One output holds one table: the first, the rest a loss.
                if tables.len() > 1 {
                    context.loss(
                        self.name,
                        Location::default(),
                        format!(
                            "a {} file holds one table: wrote '{}' and left out {} more; pick one with --sheet",
                            self.to.display_name,
                            tables[0].name,
                            tables.len() - 1
                        ),
                    );
                }
                if let Some(table) = tables.first() {
                    write_rows(&mut workbook, table, delimiter, output)?;
                }
                Ok(())
            }
            Target::Json => {
                let csv = as_csv(&mut workbook, &tables)?;
                crate::converters::csv_to_json::tables_to_json(&csv, output, context)
            }
            Target::Markdown => {
                let csv = as_csv(&mut workbook, &tables)?;
                crate::converters::rows_document::tables_to_markdown(&csv, output, context)
            }
            Target::Xlsx => {
                if tables.is_empty() {
                    context.warning("the document has no table; the workbook has one empty sheet");
                }
                let first = tables.first().map_or("Sheet1", |table| table.name.as_str());
                let mut writer = crate::io::xlsx::XlsxWriter::new(&mut *output, first)?;
                for (index, table) in tables.iter().enumerate() {
                    if index > 0 {
                        writer.next_sheet(&table.name)?;
                    }
                    workbook.rows(table.sheet, table.table, |cells| {
                        writer.write_row(cells.iter().map(String::as_str))
                    })?;
                }
                writer.finish()?;
                output.flush()?;
                Ok(())
            }
        }
    }

    fn splits(&self) -> bool {
        matches!(self.target, Target::Rows(_))
    }

    fn convert_parts(
        &self,
        mut input: Input<'_>,
        parts: &mut dyn Parts,
        context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let Target::Rows(delimiter) = self.target else {
            let output = parts.part("")?;
            return self.convert(input, output, context);
        };
        let bytes = read_all(&mut input)?;
        let package = Package::read_scope(&bytes, Scope::Workbook).map_err(package_error)?;
        let mut workbook = WorkbookReader::new(&package);
        for table in pick_tables(&workbook, context)? {
            write_rows(&mut workbook, &table, delimiter, parts.part(&table.name)?)?;
        }
        Ok(())
    }
}

/// A table to write: where it is and the name its part takes.
struct Picked {
    sheet: usize,
    table: usize,
    name: String,
}

fn read_all(input: &mut Input<'_>) -> Result<Vec<u8>, ConvertError> {
    let mut bytes = Vec::new();
    input.read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// The document's tables, named, or the ones `--sheet` picks: a sheet's
/// name (its every table), a table's part name, or a table's number from 1.
fn pick_tables(
    workbook: &WorkbookReader<'_>,
    context: &mut Context<'_>,
) -> Result<Vec<Picked>, ConvertError> {
    let mut tables: Vec<(&str, Picked)> = Vec::new();
    for (sheet_index, sheet) in workbook.sheets().iter().enumerate() {
        let alone = sheet.tables.len() == 1;
        for (table_index, table) in sheet.tables.iter().enumerate() {
            let name = if alone || table.name.is_empty() {
                sheet.name.clone()
            } else {
                format!("{} - {}", sheet.name, table.name)
            };
            tables.push((
                sheet.name.as_str(),
                Picked {
                    sheet: sheet_index,
                    table: table_index,
                    name,
                },
            ));
        }
    }
    if tables.is_empty() {
        context.warning("the document has no table");
    }
    let Some(selector) = context.options.sheet.as_deref() else {
        return Ok(tables.into_iter().map(|(_, table)| table).collect());
    };
    let picked: Vec<Picked> = if let Some(index) = selector
        .parse::<usize>()
        .ok()
        .filter(|number| (1..=tables.len()).contains(number))
    {
        vec![tables.swap_remove(index - 1).1]
    } else {
        let by_part = tables.iter().any(|(_, table)| table.name == selector);
        tables
            .into_iter()
            .filter(|(sheet, table)| {
                if by_part {
                    table.name == selector
                } else {
                    *sheet == selector
                }
            })
            .map(|(_, table)| table)
            .collect()
    };
    if picked.is_empty() {
        return Err(ConvertError::Unsupported(format!(
            "no sheet or table '{selector}'"
        )));
    }
    Ok(picked)
}

fn write_rows(
    workbook: &mut WorkbookReader<'_>,
    table: &Picked,
    delimiter: u8,
    output: &mut dyn Write,
) -> Result<(), ConvertError> {
    let mut writer = CsvWriter::with_delimiter(output, delimiter);
    workbook.rows(table.sheet, table.table, |cells| {
        writer.write_record(cells.iter().map(String::as_str))
    })?;
    writer.flush()?;
    Ok(())
}

/// Each table as CSV, for the targets that gather every table into one
/// document.
fn as_csv(
    workbook: &mut WorkbookReader<'_>,
    tables: &[Picked],
) -> Result<Vec<(String, Vec<u8>)>, ConvertError> {
    let mut csv = Vec::with_capacity(tables.len());
    for table in tables {
        let mut bytes = Vec::new();
        write_rows(workbook, table, b',', &mut bytes)?;
        csv.push((table.name.clone(), bytes));
    }
    Ok(csv)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declares_contract() {
        assert_eq!(NUMBERS_TO_CSV.name(), "numbers-to-csv");
        assert_eq!(NUMBERS_TO_CSV.from().id, "numbers");
        assert_eq!(NUMBERS_TO_XLSX.to().id, "xlsx");
        assert!(NUMBERS_TO_TSV.splits());
        assert!(!NUMBERS_TO_JSON.splits());
    }
}
