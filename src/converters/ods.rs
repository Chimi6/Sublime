//! OpenDocument spreadsheets <-> rows: an ODS's sheets reach CSV, TSV,
//! JSON, Markdown (and every document format through it), Excel, and
//! Numbers, and each of those reaches ODS.
//!
//! Text outputs show each cell as the spreadsheet shows it (the text the
//! file stores beside the value, as LibreOffice's CSV export writes it);
//! Excel and Numbers get the value with its number format.

use std::io::Write;

use crate::converter::{
    ConvertError, Converter, Fidelity, Input, Location, MemoryParts, Parts, Tier,
};
use crate::converters::numbers::{ExcelCodes, excel_cell, pick_tables, read_all};
use crate::converters::pages_to_json::package_error;
use crate::event::Context;
use crate::format::{Format, Setting, formats};
use crate::io::csv::{CsvReader, CsvWriter, DocumentTables, Record};
use crate::io::ods::{Need, OdsCell, OdsError, OdsWriter, Sheet, Spreadsheet, Value};
use crate::io::pages::{
    NumbersRows, NumbersSheet, NumbersTable, Package, Scope, WorkbookReader, write_numbers_to,
};
use crate::io::xlsx::{Workbook, XlsxCell, XlsxWriter};

impl From<OdsError> for ConvertError {
    fn from(error: OdsError) -> Self {
        match error {
            OdsError::NoSuchSheet { .. } | OdsError::Encrypted => {
                ConvertError::Unsupported(error.to_string())
            }
            other => ConvertError::Malformed {
                location: Location::default(),
                message: other.to_string(),
            },
        }
    }
}

#[derive(Clone, Copy)]
enum Target {
    Rows(u8),
    Json,
    Markdown,
    Xlsx,
    Numbers,
}

pub struct OdsTo {
    name: &'static str,
    to: &'static Format,
    target: Target,
    note: &'static str,
}

const ROWS_NOTE: &str = "each sheet its own file (or the one --sheet picks); cells as the spreadsheet shows them, merged-over cells empty; formatting, formulas (their last value kept), comments, charts, and pictures are dropped";

pub static ODS_TO_CSV: OdsTo = OdsTo {
    name: "ods-to-csv",
    to: &formats::CSV,
    target: Target::Rows(b','),
    note: ROWS_NOTE,
};

pub static ODS_TO_TSV: OdsTo = OdsTo {
    name: "ods-to-tsv",
    to: &formats::TSV,
    target: Target::Rows(b'\t'),
    note: ROWS_NOTE,
};

pub static ODS_TO_JSON: OdsTo = OdsTo {
    name: "ods-to-json",
    to: &formats::JSON,
    target: Target::Json,
    note: "one sheet as an array of objects keyed by its header row, several as an object of such arrays keyed by sheet name (or the one --sheet picks); cells as the spreadsheet shows them, as text; formatting, formulas, comments, charts, and pictures are dropped",
};

pub static ODS_TO_MARKDOWN: OdsTo = OdsTo {
    name: "ods-to-markdown",
    to: &formats::MARKDOWN,
    target: Target::Markdown,
    note: "each sheet a table under a heading of its name (one sheet, the table alone), the first row as the header; cells as the spreadsheet shows them, line breaks inside cells as spaces; formatting, formulas, comments, charts, and pictures are dropped",
};

pub static ODS_TO_XLSX: OdsTo = OdsTo {
    name: "ods-to-xlsx",
    to: &formats::XLSX,
    target: Target::Xlsx,
    note: "each sheet a worksheet (or the one --sheet picks); numbers, dates, times, and booleans as cells of their type with their number formats, everything else as text; merged cells kept; fonts, fills, borders, formulas (their last value kept), comments, charts, and pictures are dropped",
};

pub static ODS_TO_NUMBERS: OdsTo = OdsTo {
    name: "ods-to-numbers",
    to: &formats::NUMBERS,
    target: Target::Numbers,
    note: "each sheet a sheet of one table, its first row the header; numbers, dates, and booleans typed, with the number and date formats Numbers has (decimals, currency, percent, scientific, fraction, date patterns); merged cells kept; formatting, formulas (their last value kept), comments, charts, and pictures are dropped",
};

impl Converter for OdsTo {
    fn name(&self) -> &'static str {
        self.name
    }

    fn from(&self) -> &'static Format {
        &formats::ODS
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
        let book = Spreadsheet::open(&bytes)?;
        let selector = context.options.sheet.clone();
        match self.target {
            Target::Rows(delimiter) => {
                // One output holds one sheet: the first, or the one picked.
                let delimiter = crate::converters::input::written_delimiter(delimiter, context);
                let mut found = false;
                let names = book.read(
                    Need::Text,
                    &mut |index, name| picks(selector.as_deref(), index, name),
                    &mut |sheet| -> Result<bool, ConvertError> {
                        found = true;
                        write_rows(&sheet, delimiter, &mut *output)?;
                        Ok(false)
                    },
                )?;
                missing(found, selector.as_deref(), names)
            }
            Target::Json | Target::Markdown => {
                let mut tables = Vec::new();
                let names = book.read(
                    Need::Text,
                    &mut |index, name| picks(selector.as_deref(), index, name),
                    &mut |sheet| -> Result<bool, ConvertError> {
                        let mut csv = Vec::new();
                        write_rows(&sheet, b',', &mut csv)?;
                        tables.push((sheet.name, csv));
                        Ok(true)
                    },
                )?;
                missing(!tables.is_empty(), selector.as_deref(), names)?;
                if matches!(self.target, Target::Json) {
                    crate::converters::csv_to_json::tables_to_json(&tables, output, context)
                } else {
                    crate::converters::rows_document::tables_to_markdown(&tables, output, context)
                }
            }
            Target::Xlsx => {
                let effort = context.options.effort.unwrap_or_default();
                // The workbook opens with its first sheet's name.
                let mut sink = Some(output);
                let mut writer: Option<XlsxWriter<&mut dyn Write>> = None;
                let names = book.read(
                    Need::Typed,
                    &mut |index, name| picks(selector.as_deref(), index, name),
                    &mut |sheet| -> Result<bool, ConvertError> {
                        let writer = match &mut writer {
                            Some(writer) => {
                                writer.next_sheet(&sheet.name)?;
                                writer
                            }
                            None => {
                                let output = sink.take().ok_or(OdsError::Part("sheet"))?;
                                writer.insert(XlsxWriter::new(output, &sheet.name, effort)?)
                            }
                        };
                        for &(row, column, rows, columns) in sheet.merges() {
                            writer.merge(row, column, rows, columns);
                        }
                        sheet.rows(&mut |cells: &[OdsCell<'_>]| {
                            let row: Vec<(XlsxCell<'_>, Option<&str>)> =
                                cells.iter().map(|cell| xlsx_cell(&book, cell)).collect();
                            writer.write_cells(&row)
                        })?;
                        Ok(true)
                    },
                )?;
                missing(writer.is_some(), selector.as_deref(), names)?;
                if let Some(writer) = writer {
                    writer.finish()?.flush()?;
                }
                Ok(())
            }
            Target::Numbers => {
                let mut sheets = Vec::new();
                let names = book.read(
                    Need::Typed,
                    &mut |index, name| picks(selector.as_deref(), index, name),
                    &mut |sheet| -> Result<bool, ConvertError> {
                        sheets.push(NumbersSheet {
                            name: sheet.name.clone(),
                            tables: vec![numbers_table(&book, &sheet)?],
                        });
                        Ok(true)
                    },
                )?;
                missing(!sheets.is_empty(), selector.as_deref(), names)?;
                write_numbers_to(sheets, output).map_err(package_error)?;
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
        let book = Spreadsheet::open(&bytes)?;
        let selector = context.options.sheet.clone();
        let delimiter = crate::converters::input::written_delimiter(delimiter, context);
        let mut found = false;
        let names = book.read(
            Need::Text,
            &mut |index, name| picks(selector.as_deref(), index, name),
            &mut |sheet| -> Result<bool, ConvertError> {
                found = true;
                write_rows(&sheet, delimiter, parts.part(&sheet.name)?)?;
                Ok(true)
            },
        )?;
        missing(found, selector.as_deref(), names)
    }
}

/// Whether `--sheet` (a name, or a number from 1) picks a sheet; every
/// sheet without it.
fn picks(selector: Option<&str>, index: usize, name: &str) -> bool {
    selector.is_none_or(|selector| selector == name || selector == (index + 1).to_string())
}

/// The error for a `--sheet` no sheet answered, listing the sheets; an
/// error too for a spreadsheet without a sheet.
fn missing(found: bool, selector: Option<&str>, names: Vec<String>) -> Result<(), ConvertError> {
    if found {
        return Ok(());
    }
    Err(match selector {
        Some(wanted) => OdsError::NoSuchSheet {
            wanted: wanted.to_string(),
            available: names,
        },
        None => OdsError::Part("sheet"),
    }
    .into())
}

fn write_rows(sheet: &Sheet, delimiter: u8, output: &mut dyn Write) -> Result<(), ConvertError> {
    let mut writer = CsvWriter::with_delimiter(output, delimiter);
    sheet.rows(&mut |cells: &[OdsCell<'_>]| {
        writer.write_record(cells.iter().map(|cell| cell.text))
    })?;
    writer.flush()?;
    Ok(())
}

/// A cell as Excel stores it: its value and the code of its format.
fn xlsx_cell<'a>(book: &'a Spreadsheet<'_>, cell: &OdsCell<'a>) -> (XlsxCell<'a>, Option<&'a str>) {
    let code = cell.format.map(|index| book.format(index).1);
    let dated = cell.format.is_some_and(|index| {
        matches!(
            book.format(index).0.kind(),
            crate::io::ods::numfmt::Kind::Date | crate::io::ods::numfmt::Kind::Time
        )
    });
    match cell.value {
        Value::Empty => (XlsxCell::Empty, None),
        Value::Text => (XlsxCell::Text(cell.text), None),
        Value::Boolean(value) => (XlsxCell::Boolean(value), None),
        Value::Number(value) => (XlsxCell::Number(value), code),
        // A date or time without a format of its kind shows as ISO 8601.
        Value::Date(serial) => (
            XlsxCell::Number(serial),
            code.filter(|_| dated).or(Some(if serial.fract() == 0.0 {
                "yyyy-mm-dd"
            } else {
                "yyyy-mm-dd hh:mm:ss"
            })),
        ),
        Value::Time(days) => (
            XlsxCell::Number(days),
            code.filter(|_| dated).or(Some("[hh]:mm:ss")),
        ),
    }
}

/// A sheet as a Numbers table: each cell's value as text (Numbers types
/// it), the Numbers format of its Excel code, and its merges.
fn numbers_table(book: &Spreadsheet<'_>, sheet: &Sheet) -> Result<NumbersTable, ConvertError> {
    let mut table = NumbersTable {
        name: "Table 1".to_string(),
        ..NumbersTable::default()
    };
    // Per data style, its Numbers format's key (0 for none).
    let mut keys: Vec<Option<u16>> = Vec::new();
    let mut formatted = false;
    let mut rows = NumbersRows::new();
    let mut cell_formats = Vec::new();
    sheet.rows(&mut |cells: &[OdsCell<'_>]| -> Result<(), ConvertError> {
        rows.push_row(cells.iter().map(|cell| cell.raw));
        let mut row_keys = Vec::with_capacity(cells.len());
        for cell in cells {
            let key = match cell.format {
                None => 0,
                Some(index) => {
                    if keys.len() <= index {
                        keys.resize(index + 1, None);
                    }
                    *keys[index].get_or_insert_with(|| {
                        match crate::io::pages::format::from_excel(book.format(index).1) {
                            Some(format) if table.formats.len() < usize::from(u16::MAX) => {
                                table.formats.push(format);
                                table.formats.len() as u16
                            }
                            _ => 0,
                        }
                    })
                }
            };
            formatted |= key > 0;
            row_keys.push(key);
        }
        cell_formats.push(row_keys);
        Ok(())
    })?;
    table.rows = rows;
    if formatted {
        table.cell_formats = cell_formats;
    }
    table.merges = sheet.merges().to_vec();
    Ok(table)
}

// ---------------------------------------------------------------------
// To ODS
// ---------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Source {
    Delimited(u8),
    Xlsx,
    Json,
    Document,
    Numbers,
}

pub struct ToOds {
    name: &'static str,
    from: &'static Format,
    source: Source,
    note: &'static str,
}

const DELIMITED_NOTE: &str = "plain decimals become numbers, ISO 8601 dates, dates and times, and times become dates and durations shown in the same form, TRUE and FALSE become booleans, everything else stays text; one sheet, named by --sheet or Sheet1";

pub static CSV_TO_ODS: ToOds = ToOds {
    name: "csv-to-ods",
    from: &formats::CSV,
    source: Source::Delimited(b','),
    note: DELIMITED_NOTE,
};

pub static TSV_TO_ODS: ToOds = ToOds {
    name: "tsv-to-ods",
    from: &formats::TSV,
    source: Source::Delimited(b'\t'),
    note: DELIMITED_NOTE,
};

pub static XLSX_TO_ODS: ToOds = ToOds {
    name: "xlsx-to-ods",
    from: &formats::XLSX,
    source: Source::Xlsx,
    note: "each worksheet a sheet (or the one --sheet picks); numbers, dates, times, and booleans typed with their number formats, everything else text; merged cells kept; fonts, fills, borders, formulas (their last value kept), comments, charts, and pictures are dropped",
};

pub static JSON_TO_ODS: ToOds = ToOds {
    name: "json-to-ods",
    from: &formats::JSON,
    source: Source::Json,
    note: "an array of objects as one sheet, an object of such arrays as a sheet per member; keys become the header row, nested values JSON text, decimals numbers, ISO date strings dates, and objects with differing key sets get empty cells for missing keys",
};

pub static MARKDOWN_TO_ODS: ToOds = ToOds {
    name: "markdown-to-ods",
    from: &formats::MARKDOWN,
    source: Source::Document,
    note: "each of the document's tables a sheet, named after the heading before it, inline formatting flattened to text, decimals as numbers, ISO dates as dates, and TRUE and FALSE as booleans; everything else in the document is dropped",
};

pub static NUMBERS_TO_ODS: ToOds = ToOds {
    name: "numbers-to-ods",
    from: &formats::NUMBERS,
    source: Source::Numbers,
    note: "each table a sheet (or the one --sheet picks), named after its sheet; numbers, dates, durations, and booleans typed with the number formats that show them as Numbers does, shown as Numbers shows them; merged cells kept; styles, formulas, charts, and pictures are dropped",
};

impl Converter for ToOds {
    fn name(&self) -> &'static str {
        self.name
    }

    fn from(&self) -> &'static Format {
        self.from
    }

    fn to(&self) -> &'static Format {
        &formats::ODS
    }

    fn options(&self) -> Vec<Setting> {
        match self.source {
            // Sheets are named after the source's own names.
            Source::Json | Source::Document => vec![Setting::Effort],
            Source::Numbers | Source::Xlsx => vec![Setting::Sheet, Setting::Effort],
            Source::Delimited(_) => {
                let mut options = self.from.read_options.to_vec();
                options.extend_from_slice(formats::ODS.write_options);
                options
            }
        }
    }

    fn fidelity(&self) -> Fidelity {
        match self.source {
            Source::Document => Fidelity::Lossy(self.note),
            _ => Fidelity::Conditional(self.note),
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
        let effort = context.options.effort.unwrap_or_default();
        match self.source {
            Source::Delimited(delimiter) => {
                let mut rest = None;
                let (delimiter, input) = crate::converters::input::delimited_input(
                    input, delimiter, context, &mut rest,
                )?;
                let mut reader = CsvReader::with_delimiter(input, delimiter);
                let name = context.options.sheet.as_deref().unwrap_or("Sheet1");
                let mut writer = OdsWriter::new(&mut *output, name, effort)?;
                let mut record = Record::new();
                let mut count: u64 = 0;
                while reader.read_record(&mut record)? {
                    writer.write_row(record.fields())?;
                    count += 1;
                    if count % 4096 == 0 {
                        context.progress(self.name, reader.bytes_consumed());
                    }
                }
                writer.finish()?;
            }
            Source::Xlsx => {
                let bytes = read_all(&mut input)?;
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
                let mut writer = OdsWriter::new(&mut *output, &names[picked[0]], effort)?;
                for (index, &sheet) in picked.iter().enumerate() {
                    if index > 0 {
                        writer.next_sheet(&names[sheet])?;
                    }
                    for (row, column, rows, columns) in workbook.merges(sheet) {
                        writer.merge(row, column, rows, columns);
                    }
                    workbook.read_sheet(
                        sheet,
                        &mut |cells: &[String],
                              styles: &[Option<usize>]|
                         -> Result<(), ConvertError> {
                            let typed: Vec<(XlsxCell<'_>, Option<&str>, Option<&str>)> = cells
                                .iter()
                                .zip(styles)
                                .map(|(text, style)| {
                                    typed_xlsx_cell(
                                        text,
                                        style.and_then(|style| workbook.style_format(style)),
                                    )
                                })
                                .collect();
                            writer.write_cells(&typed)?;
                            Ok(())
                        },
                    )?;
                }
                writer.finish()?;
            }
            Source::Json => {
                let mut parts = MemoryParts::default();
                crate::converters::json_to_csv::JSON_TO_CSV
                    .convert_parts(input, &mut parts, context)?;
                let first = parts
                    .parts
                    .first()
                    .map(|(name, _)| name.as_str())
                    .filter(|name| !name.is_empty())
                    .unwrap_or("Sheet1");
                let mut writer = OdsWriter::new(&mut *output, first, effort)?;
                // As `JSON_TO_CSV` wrote it, `--delimiter` and all.
                let delimiter = crate::converters::input::written_delimiter(b',', context);
                for (index, (name, csv)) in parts.parts.iter().enumerate() {
                    if index > 0 {
                        writer.next_sheet(name)?;
                    }
                    let mut reader = CsvReader::with_delimiter(csv.as_slice(), delimiter);
                    let mut record = Record::new();
                    while reader.read_record(&mut record)? {
                        writer.write_row(record.fields())?;
                    }
                }
                writer.finish()?;
            }
            Source::Document => {
                let text = crate::converters::input::read_text_document(&mut input)?;
                let mut found = DocumentTables::new();
                crate::io::markdown::parse_into(
                    &text,
                    crate::io::markdown::Options::default(),
                    &mut found,
                );
                if found.tables.is_empty() {
                    context
                        .warning("the document has no table; the spreadsheet has one empty sheet");
                }
                let first = found
                    .tables
                    .first()
                    .map_or("Sheet1", |(name, _)| name.as_str());
                let mut writer = OdsWriter::new(&mut *output, first, effort)?;
                for (index, (name, rows)) in found.tables.iter().enumerate() {
                    if index > 0 {
                        writer.next_sheet(name)?;
                    }
                    for row in rows {
                        writer.write_row(row.iter().map(String::as_str))?;
                    }
                }
                writer.finish()?;
            }
            Source::Numbers => {
                let bytes = read_all(&mut input)?;
                let package =
                    Package::read_scope(&bytes, Scope::Workbook).map_err(package_error)?;
                let mut workbook = WorkbookReader::new(&package);
                let tables = pick_tables(&workbook, context)?;
                let first = tables.first().map_or("Sheet1", |table| table.name.as_str());
                let mut writer = OdsWriter::new(&mut *output, first, effort)?;
                for (index, table) in tables.iter().enumerate() {
                    if index > 0 {
                        writer.next_sheet(&table.name)?;
                    }
                    let mut codes = ExcelCodes::new();
                    for (row, column, rows, columns) in workbook.merges(table.sheet, table.table) {
                        writer.merge(row, column, rows, columns);
                    }
                    workbook.rows(table.sheet, table.table, &mut |cells| {
                        let typed: Vec<(XlsxCell<'_>, Option<std::rc::Rc<str>>)> = cells
                            .iter()
                            .map(|cell| excel_cell(cell, &mut codes))
                            .collect();
                        let row: Vec<(XlsxCell<'_>, Option<&str>, Option<&str>)> = typed
                            .iter()
                            .zip(cells)
                            .map(|((cell, code), source)| {
                                (*cell, code.as_deref(), Some(source.text.as_str()))
                            })
                            .collect();
                        writer.write_cells(&row)
                    })?;
                }
                writer.finish()?;
            }
        }
        output.flush()?;
        Ok(())
    }
}

/// An Excel cell (as the workbook reader gives it: numbers as stored,
/// dates in ISO 8601, booleans as `TRUE` and `FALSE`) typed by its text,
/// with its number format.
fn typed_xlsx_cell<'a>(
    text: &'a str,
    code: Option<&'a str>,
) -> (XlsxCell<'a>, Option<&'a str>, Option<&'a str>) {
    use crate::io::xlsx::writer::{STYLE_DATE, STYLE_DATE_TIME, iso_serial};
    if text.is_empty() {
        return (XlsxCell::Empty, None, None);
    }
    if text == "TRUE" || text == "FALSE" {
        return (XlsxCell::Boolean(text == "TRUE"), None, Some(text));
    }
    let dated = code
        .and_then(crate::io::ods::numfmt::from_excel)
        .is_some_and(|style| {
            matches!(
                style.kind(),
                crate::io::ods::numfmt::Kind::Date | crate::io::ods::numfmt::Kind::Time
            )
        });
    // A duration past a day reads as a date in Excel's first days of 1900
    // (`1900-01-01T12:00:00` is 1.5 days): its serial again, with Excel's
    // day for 1900-02-29 that never was.
    if dated && iso_serial(text).is_none() {
        if let Some(days) = crate::io::ods::reader::date_serial(text) {
            if days < 61.0 && text.starts_with("1900-") {
                return (XlsxCell::Number(days - 1.0), code, None);
            }
        }
    }
    if let Some((serial, kind)) = iso_serial(text) {
        let code = if dated {
            code
        } else {
            Some(match kind {
                STYLE_DATE => "yyyy-mm-dd",
                STYLE_DATE_TIME => "yyyy-mm-dd\"T\"hh:mm:ss",
                _ => "[hh]:mm:ss",
            })
        };
        // Shown as the format shows it.
        return (XlsxCell::Number(serial), code, None);
    }
    if let Ok(value) = text.parse::<f64>() {
        // Numbers as Excel stores them (a double's shortest decimal, up to
        // seventeen digits, `1E-05`); text with leading zeros stays text.
        let stored = is_stored_number(text);
        if value.is_finite() && stored {
            let shown = match code {
                Some(_) => None,
                None => Some(text),
            };
            return (XlsxCell::Number(value), code, shown);
        }
    }
    (XlsxCell::Text(text), None, Some(text))
}

/// A decimal as a workbook stores a number: an optional minus, digits
/// without a leading zero (but `0.5`), an optional fraction, an optional
/// exponent.
fn is_stored_number(text: &str) -> bool {
    let body = text.strip_prefix('-').unwrap_or(text);
    let (mantissa, exponent) = match body.find(['E', 'e']) {
        Some(at) => (&body[..at], Some(&body[at + 1..])),
        None => (body, None),
    };
    let (integer, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    let integer_ok = digits(integer) && (integer == "0" || !integer.starts_with('0'));
    let fraction_ok = !mantissa.contains('.') || digits(fraction);
    let exponent_ok = exponent
        .is_none_or(|exponent| digits(exponent.strip_prefix(['+', '-']).unwrap_or(exponent)));
    integer_ok && fraction_ok && exponent_ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declares_contracts() {
        assert_eq!(ODS_TO_CSV.from().id, "ods");
        assert_eq!(ODS_TO_XLSX.to().id, "xlsx");
        assert!(ODS_TO_TSV.splits());
        assert!(!ODS_TO_JSON.splits());
        assert_eq!(CSV_TO_ODS.to().id, "ods");
        assert_eq!(NUMBERS_TO_ODS.from().id, "numbers");
    }

    #[test]
    fn excel_cells_keep_their_types() {
        assert_eq!(
            typed_xlsx_cell("1234.5", Some("#,##0.00")),
            (XlsxCell::Number(1234.5), Some("#,##0.00"), None)
        );
        assert_eq!(
            typed_xlsx_cell("1E-05", None),
            (XlsxCell::Number(1e-5), None, Some("1E-05"))
        );
        assert_eq!(
            typed_xlsx_cell("2024-01-05", Some("d mmm yyyy")),
            (XlsxCell::Number(45_296.0), Some("d mmm yyyy"), None)
        );
        assert_eq!(
            typed_xlsx_cell("TRUE", None),
            (XlsxCell::Boolean(true), None, Some("TRUE"))
        );
        assert_eq!(typed_xlsx_cell("00123", None).0, XlsxCell::Text("00123"));
        assert_eq!(
            typed_xlsx_cell("-0.000359453630481621", Some("0.00%")).0,
            XlsxCell::Number(-0.000359453630481621)
        );
    }
}
