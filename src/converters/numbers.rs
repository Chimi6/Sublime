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
use crate::io::pages::{Package, Scope, read_workbook};

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
    note: "each table a worksheet (or the one --sheet picks); cells as Numbers shows them, as text; formatting, formulas, merges, charts, and pictures are dropped",
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
        input: Input<'_>,
        output: &mut dyn Write,
        context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let tables = read_tables(input, context)?;
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
                            tables[0].0,
                            tables.len() - 1
                        ),
                    );
                }
                if let Some((_, rows)) = tables.first() {
                    write_rows(rows, delimiter, output)?;
                }
                Ok(())
            }
            Target::Json => {
                let csv = as_csv(&tables)?;
                crate::converters::csv_to_json::tables_to_json(&csv, output, context)
            }
            Target::Markdown => {
                let csv = as_csv(&tables)?;
                crate::converters::rows_document::tables_to_markdown(&csv, output, context)
            }
            Target::Xlsx => {
                if tables.is_empty() {
                    context.warning("the document has no table; the workbook has one empty sheet");
                }
                let first = tables.first().map_or("Sheet1", |(name, _)| name.as_str());
                let mut writer = crate::io::xlsx::XlsxWriter::new(&mut *output, first)?;
                for (index, (name, rows)) in tables.iter().enumerate() {
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
    }

    fn splits(&self) -> bool {
        matches!(self.target, Target::Rows(_))
    }

    fn convert_parts(
        &self,
        input: Input<'_>,
        parts: &mut dyn Parts,
        context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let Target::Rows(delimiter) = self.target else {
            let output = parts.part("")?;
            return self.convert(input, output, context);
        };
        for (name, rows) in read_tables(input, context)? {
            write_rows(&rows, delimiter, parts.part(&name)?)?;
        }
        Ok(())
    }
}

type NamedTable = (String, Vec<Vec<String>>);

/// The document's tables, named, or the ones `--sheet` picks: a sheet's
/// name (its every table), a table's part name, or a table's number from 1.
fn read_tables(
    mut input: Input<'_>,
    context: &mut Context<'_>,
) -> Result<Vec<NamedTable>, ConvertError> {
    let mut bytes = Vec::new();
    input.read_to_end(&mut bytes)?;
    let package = Package::read_scope(&bytes, Scope::Workbook).map_err(package_error)?;
    let sheets = read_workbook(&package);
    let mut tables: Vec<(String, String, Vec<Vec<String>>)> = Vec::new();
    for sheet in sheets {
        let alone = sheet.tables.len() == 1;
        for table in sheet.tables {
            let name = if alone || table.name.is_empty() {
                sheet.name.clone()
            } else {
                format!("{} - {}", sheet.name, table.name)
            };
            tables.push((sheet.name.clone(), name, table.rows));
        }
    }
    if tables.is_empty() {
        context.warning("the document has no table");
    }
    let Some(selector) = context.options.sheet.as_deref() else {
        return Ok(tables
            .into_iter()
            .map(|(_, name, rows)| (name, rows))
            .collect());
    };
    let picked: Vec<NamedTable> = if let Some(index) = selector
        .parse::<usize>()
        .ok()
        .filter(|number| (1..=tables.len()).contains(number))
    {
        let (_, name, rows) = tables.swap_remove(index - 1);
        vec![(name, rows)]
    } else {
        let by_part: Vec<usize> = tables
            .iter()
            .enumerate()
            .filter(|(_, (_, name, _))| name == selector)
            .map(|(index, _)| index)
            .collect();
        tables
            .into_iter()
            .enumerate()
            .filter(|(index, (sheet, _, _))| {
                if by_part.is_empty() {
                    sheet == selector
                } else {
                    by_part.contains(index)
                }
            })
            .map(|(_, (_, name, rows))| (name, rows))
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
    rows: &[Vec<String>],
    delimiter: u8,
    output: &mut dyn Write,
) -> Result<(), ConvertError> {
    let mut writer = CsvWriter::with_delimiter(output, delimiter);
    for row in rows {
        writer.write_record(row.iter().map(String::as_str))?;
    }
    writer.flush()?;
    Ok(())
}

fn as_csv(tables: &[NamedTable]) -> Result<Vec<(String, Vec<u8>)>, ConvertError> {
    let mut csv = Vec::with_capacity(tables.len());
    for (name, rows) in tables {
        let mut bytes = Vec::new();
        write_rows(rows, b',', &mut bytes)?;
        csv.push((name.clone(), bytes));
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
