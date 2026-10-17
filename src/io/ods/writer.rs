//! Writes an OpenDocument spreadsheet of one or more sheets, streaming the
//! rows into `content.xml` as they come. Number formats are common styles
//! in `styles.xml`, written at the end, so no row waits for them.
//!
//! A cell carries its value and the text it shows. Text alone is typed as
//! the Excel writer types it, only when reading it back gives the same
//! text: a plain decimal is a number, an ISO 8601 date, date and time, or
//! time is a date or duration shown in that same form, `TRUE` and `FALSE`
//! are booleans, and everything else is a string.

use std::fmt::Write as _;
use std::io::{self, Write};

use super::numfmt::{self, DataStyle, Kind};
use crate::io::deflate::Level;
use crate::io::xlsx::XlsxCell;
use crate::io::xml::{escape_attribute, escape_text};
use crate::io::zip::ZipWriter;

const PART_SIZE: usize = 256 * 1024;
const MIME: &str = "application/vnd.oasis.opendocument.spreadsheet";

const CONTENT_HEAD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\" xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\" xmlns:number=\"urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0\" office:version=\"1.3\"><office:body><office:spreadsheet>";

/// The written forms of text that reads back as a typed value.
const ISO_DATE: &str = "yyyy-mm-dd";
const ISO_DATE_TIME: &str = "yyyy-mm-dd\"T\"hh:mm:ss";
const ISO_TIME: &str = "[hh]:mm:ss";

pub struct OdsWriter<W: Write> {
    zip: ZipWriter<W>,
    part: String,
    /// Every sheet's name, in order; the last is the one being written.
    sheets: Vec<String>,
    /// Number formats cells named, by Excel code: style `ce{i + 1}`.
    formats: Vec<(String, DataStyle)>,
    /// Whether the open sheet's head (its columns) is written.
    head: bool,
    /// Rows written to the open sheet, and empty rows not yet written.
    row: usize,
    empty_rows: usize,
    /// The open sheet's merged ranges as (row, column, rows, columns).
    merges: Vec<(usize, usize, usize, usize)>,
}

/// A cell as written.
enum Written<'a> {
    Empty,
    Text(&'a str),
    /// A number with the text its value is written as.
    Number(f64, Option<&'a str>),
    Date(f64),
    Time(f64),
    Boolean(bool),
}

impl<W: Write> OdsWriter<W> {
    /// Writes the media type and opens the first sheet for rows.
    pub fn new(
        sink: W,
        sheet_name: &str,
        effort: crate::format::Effort,
    ) -> io::Result<OdsWriter<W>> {
        let mut zip = ZipWriter::new(sink).with_effort(effort);
        zip.add("mimetype", MIME.as_bytes())?;
        zip.begin_deflated("content.xml")?;
        let mut writer = OdsWriter {
            zip,
            part: String::with_capacity(PART_SIZE + 4096),
            sheets: Vec::new(),
            formats: Vec::new(),
            head: false,
            row: 0,
            empty_rows: 0,
            merges: Vec::new(),
        };
        writer.part.push_str(CONTENT_HEAD);
        writer.open_sheet(sheet_name);
        Ok(writer)
    }

    /// Closes the sheet being written and opens the next, named `name`.
    pub fn next_sheet(&mut self, name: &str) -> io::Result<()> {
        self.close_sheet()?;
        self.open_sheet(name);
        Ok(())
    }

    fn open_sheet(&mut self, name: &str) {
        let title = unique_name(&sheet_name(name), &self.sheets);
        self.sheets.push(title);
        self.head = false;
        self.row = 0;
        self.empty_rows = 0;
        self.merges.clear();
    }

    /// Merges a range of the open sheet: `rows` by `columns` cells from
    /// (`row`, `column`), from 0. Call before the range's rows.
    pub fn merge(&mut self, row: usize, column: usize, rows: usize, columns: usize) {
        if rows > 0 && columns > 0 && (rows > 1 || columns > 1) {
            self.merges.push((row, column, rows, columns));
        }
    }

    /// Writes the sheet's start and its columns, as wide as its first row.
    fn write_head(&mut self, width: usize) {
        self.part.push_str("<table:table table:name=\"");
        let name = self.sheets.last().cloned().unwrap_or_default();
        escape_attribute(&mut self.part, &name);
        self.part.push_str("\"><table:table-column");
        let width = width
            .max(
                self.merges
                    .iter()
                    .map(|(_, column, _, columns)| column + columns)
                    .max()
                    .unwrap_or(0),
            )
            .max(1);
        if width > 1 {
            let _ = write!(self.part, " table:number-columns-repeated=\"{width}\"");
        }
        self.part.push_str("/>");
        self.head = true;
    }

    fn close_sheet(&mut self) -> io::Result<()> {
        if !self.head {
            self.write_head(1);
        }
        self.flush_empty_rows();
        if self.row == 0 {
            // A table needs a row.
            self.part
                .push_str("<table:table-row><table:table-cell/></table:table-row>");
        }
        self.part.push_str("</table:table>");
        self.spill()
    }

    /// Writes one row, typing each cell by its text.
    pub fn write_row<'a>(&mut self, cells: impl IntoIterator<Item = &'a str>) -> io::Result<()> {
        let cells: Vec<&str> = cells.into_iter().collect();
        let typed: Vec<(Written<'_>, Option<usize>, Option<&str>)> =
            cells.iter().map(|cell| self.typed_by_text(cell)).collect();
        self.write_written(&typed)
    }

    /// A cell's type from its text, as the Excel writer reads it.
    fn typed_by_text<'a>(
        &mut self,
        cell: &'a str,
    ) -> (Written<'a>, Option<usize>, Option<&'a str>) {
        if cell.is_empty() {
            return (Written::Empty, None, None);
        }
        if crate::io::xlsx::writer::is_number_literal(cell) {
            if let Ok(value) = cell.parse::<f64>() {
                if !cell.contains(['E', 'e']) && general_shows(value, cell) {
                    return (Written::Number(value, Some(cell)), None, Some(cell));
                }
                // An exponent (`2.59E+005`) needs a scientific format that
                // shows it so; General would show `259000`.
                if let Some(code) = scientific_code(cell) {
                    if numfmt::from_excel(&code).is_some_and(|style| style.render(value) == cell) {
                        let style = self.style_for(&code);
                        return (Written::Number(value, Some(cell)), style, Some(cell));
                    }
                }
            }
        }
        if cell == "TRUE" || cell == "FALSE" {
            return (Written::Boolean(cell == "TRUE"), None, Some(cell));
        }
        if let Some((serial, kind)) = crate::io::xlsx::writer::iso_serial(cell) {
            let code = match kind {
                crate::io::xlsx::writer::STYLE_DATE => ISO_DATE,
                crate::io::xlsx::writer::STYLE_DATE_TIME => ISO_DATE_TIME,
                _ => ISO_TIME,
            };
            let style = self.style_for(code);
            let written = if code == ISO_TIME {
                Written::Time(serial)
            } else {
                Written::Date(serial)
            };
            return (written, style, Some(cell));
        }
        (Written::Text(cell), None, Some(cell))
    }

    /// Writes one row of typed cells, each with the Excel number format
    /// it is shown in, if any, and the text it shows (worked out from the
    /// format when not given). A number with a date or time format is
    /// written as a date or duration.
    pub fn write_cells(
        &mut self,
        cells: &[(XlsxCell<'_>, Option<&str>, Option<&str>)],
    ) -> io::Result<()> {
        let mut typed = Vec::with_capacity(cells.len());
        for (cell, code, shown) in cells {
            let style = match cell {
                XlsxCell::Number(_) => code.and_then(|code| self.style_for(code)),
                _ => None,
            };
            let written = match cell {
                XlsxCell::Empty => Written::Empty,
                XlsxCell::Text(text) => Written::Text(text),
                XlsxCell::Boolean(value) => Written::Boolean(*value),
                XlsxCell::Number(value) if !value.is_finite() => Written::Empty,
                XlsxCell::Number(value) => match style.map(|index| self.formats[index].1.kind()) {
                    Some(Kind::Date) => Written::Date(*value),
                    Some(Kind::Time) => Written::Time(*value),
                    _ => Written::Number(*value, None),
                },
            };
            typed.push((written, style, *shown));
        }
        self.write_written(&typed)
    }

    /// The cell style for an Excel code, registered when first seen;
    /// `None` for General.
    fn style_for(&mut self, code: &str) -> Option<usize> {
        if let Some(index) = self.formats.iter().position(|(known, _)| known == code) {
            return Some(index);
        }
        let style = numfmt::from_excel(code)?;
        self.formats.push((code.to_string(), style));
        Some(self.formats.len() - 1)
    }

    fn write_written(
        &mut self,
        cells: &[(Written<'_>, Option<usize>, Option<&str>)],
    ) -> io::Result<()> {
        if !self.head {
            self.write_head(cells.len());
        }
        let row = self.row + self.empty_rows;
        let merged_here = self
            .merges
            .iter()
            .any(|(top, _, rows, _)| (*top..top + rows).contains(&row));
        if !merged_here
            && cells
                .iter()
                .all(|(cell, _, _)| matches!(cell, Written::Empty))
        {
            self.empty_rows += 1;
            return Ok(());
        }
        self.flush_empty_rows();
        self.part.push_str("<table:table-row>");
        let mut empty_run = 0usize;
        for (column, (cell, style, shown)) in cells.iter().enumerate() {
            let span = self
                .merges
                .iter()
                .find(|(top, left, _, _)| *top == row && *left == column)
                .map(|(_, _, rows, columns)| (*rows, *columns));
            let covered = span.is_none()
                && self.merges.iter().any(|(top, left, rows, columns)| {
                    (*top..top + rows).contains(&row) && (*left..left + columns).contains(&column)
                });
            if matches!(cell, Written::Empty) && span.is_none() && !covered {
                empty_run += 1;
                continue;
            }
            self.push_empty(empty_run);
            empty_run = 0;
            let element = if covered {
                "table:covered-table-cell"
            } else {
                "table:table-cell"
            };
            self.part.push('<');
            self.part.push_str(element);
            if let Some(style) = style {
                let _ = write!(self.part, " table:style-name=\"ce{}\"", style + 1);
            }
            if let Some((rows, columns)) = span {
                let _ = write!(
                    self.part,
                    " table:number-columns-spanned=\"{columns}\" table:number-rows-spanned=\"{rows}\""
                );
            }
            let shown: std::borrow::Cow<'_, str> = match (cell, shown) {
                (Written::Empty, _) => "".into(),
                (_, Some(text)) => (*text).into(),
                (Written::Text(text), None) => (*text).into(),
                (Written::Number(value, _) | Written::Date(value) | Written::Time(value), None) => {
                    match style {
                        Some(index) => self.formats[*index].1.render(*value).into(),
                        None => numfmt::plain(*value).into(),
                    }
                }
                (Written::Boolean(value), None) => if *value { "TRUE" } else { "FALSE" }.into(),
            };
            match cell {
                Written::Empty => {}
                Written::Text(_) => self.part.push_str(" office:value-type=\"string\""),
                Written::Number(value, text) => {
                    let percent =
                        style.is_some_and(|index| self.formats[index].1.kind() == Kind::Percentage);
                    self.part.push_str(if percent {
                        " office:value-type=\"percentage\" office:value=\""
                    } else {
                        " office:value-type=\"float\" office:value=\""
                    });
                    match text {
                        Some(text) => self.part.push_str(text),
                        None => {
                            let _ = write!(self.part, "{value}");
                        }
                    }
                    self.part.push('"');
                }
                Written::Date(serial) => {
                    self.part
                        .push_str(" office:value-type=\"date\" office:date-value=\"");
                    push_date(&mut self.part, *serial);
                    self.part.push('"');
                }
                Written::Time(days) => {
                    self.part
                        .push_str(" office:value-type=\"time\" office:time-value=\"");
                    push_duration(&mut self.part, *days);
                    self.part.push('"');
                }
                Written::Boolean(value) => {
                    let _ = write!(
                        self.part,
                        " office:value-type=\"boolean\" office:boolean-value=\"{value}\""
                    );
                }
            }
            if shown.is_empty() {
                self.part.push_str("/>");
            } else {
                self.part.push('>');
                push_paragraphs(&mut self.part, &shown);
                self.part.push_str("</");
                self.part.push_str(element);
                self.part.push('>');
            }
        }
        self.push_empty(empty_run);
        if cells.is_empty() {
            self.part.push_str("<table:table-cell/>");
        }
        self.part.push_str("</table:table-row>");
        self.row = row + 1;
        if self.part.len() >= PART_SIZE {
            self.spill()?;
        }
        Ok(())
    }

    fn push_empty(&mut self, count: usize) {
        match count {
            0 => {}
            1 => self.part.push_str("<table:table-cell/>"),
            _ => {
                let _ = write!(
                    self.part,
                    "<table:table-cell table:number-columns-repeated=\"{count}\"/>"
                );
            }
        }
    }

    fn flush_empty_rows(&mut self) {
        match self.empty_rows {
            0 => return,
            1 => self
                .part
                .push_str("<table:table-row><table:table-cell/></table:table-row>"),
            count => {
                let _ = write!(
                    self.part,
                    "<table:table-row table:number-rows-repeated=\"{count}\"><table:table-cell/></table:table-row>"
                );
            }
        }
        self.row += self.empty_rows;
        self.empty_rows = 0;
    }

    fn spill(&mut self) -> io::Result<()> {
        self.zip.write_part(self.part.as_bytes(), Level::Fast)?;
        self.part.clear();
        Ok(())
    }

    /// Closes the last sheet and the package; returns the sink.
    pub fn finish(mut self) -> io::Result<W> {
        self.close_sheet()?;
        self.part
            .push_str("</office:spreadsheet></office:body></office:document-content>");
        self.spill()?;
        self.zip.end_deflated()?;
        let mut styles = String::from(STYLES_HEAD);
        for (index, (_, style)) in self.formats.iter().enumerate() {
            numfmt::to_ods(&mut styles, &format!("N{}", index + 1), style);
        }
        for index in 1..=self.formats.len() {
            let _ = write!(
                styles,
                "<style:style style:name=\"ce{index}\" style:family=\"table-cell\" style:parent-style-name=\"Default\" style:data-style-name=\"N{index}\"/>"
            );
        }
        styles.push_str("</office:styles></office:document-styles>");
        self.zip.add_deflated("styles.xml", styles.as_bytes())?;
        self.zip.add_deflated("meta.xml", META.as_bytes())?;
        self.zip
            .add_deflated("META-INF/manifest.xml", MANIFEST.as_bytes())?;
        self.zip.finish()
    }
}

/// Whether LibreOffice's General format shows `value` as `text`: plain
/// decimals, unless so small or large it switches to an exponent.
fn general_shows(value: f64, text: &str) -> bool {
    let magnitude = value.abs();
    magnitude == 0.0 || ((1e-5..1e15).contains(&magnitude) && !text.starts_with('+'))
}

/// The Excel code that shows a number in the form of `text` (`2.59E+005`
/// is `0.00E+000`, `258.9E3` engineering `##0.0E0`).
fn scientific_code(text: &str) -> Option<String> {
    let body = text.trim_start_matches('-');
    let at = body.find(['E', 'e'])?;
    let (mantissa, exponent) = (&body[..at], &body[at + 1..]);
    let (integer, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let sign = if exponent.starts_with('+') {
        "E+"
    } else {
        "E-"
    };
    let digits = exponent.trim_start_matches(['+', '-']).len();
    let mut code = if integer.len() > 1 {
        format!("{}0", "#".repeat(integer.len().max(3) - 1))
    } else {
        "0".to_string()
    };
    if !fraction.is_empty() {
        code.push('.');
        code.push_str(&"0".repeat(fraction.len()));
    }
    code.push_str(sign);
    code.push_str(&"0".repeat(digits.max(1)));
    Some(code)
}

/// A sheet name LibreOffice accepts: none of `[ ] * ? : / \`, not
/// starting or ending with an apostrophe.
fn sheet_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|character| {
            if "[]*?:/\\".contains(character) {
                '_'
            } else {
                character
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches('\'');
    if cleaned.trim().is_empty() {
        "Sheet1".to_string()
    } else {
        cleaned.to_string()
    }
}

/// `name`, or `name (2)` and on when an earlier sheet has it (names are
/// compared without case).
fn unique_name(name: &str, taken: &[String]) -> String {
    let mut candidate = name.to_string();
    let mut number = 2;
    while taken
        .iter()
        .any(|taken| taken.to_lowercase() == candidate.to_lowercase())
    {
        candidate = format!("{name} ({number})");
        number += 1;
    }
    candidate
}

/// Text as paragraphs: a line break starts one, runs of spaces past the
/// first are `text:s`, tabs `text:tab`.
fn push_paragraphs(out: &mut String, text: &str) {
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        out.push_str("<text:p>");
        let mut spaces = 0usize;
        let mut at_start = true;
        let flush = |out: &mut String, spaces: &mut usize, at_start: bool| {
            if *spaces == 0 {
                return;
            }
            // The first space of a run is text, unless the run starts the
            // paragraph (ODF drops it there).
            let mut count = *spaces;
            if !at_start {
                out.push(' ');
                count -= 1;
            }
            match count {
                0 => {}
                1 => out.push_str("<text:s/>"),
                _ => {
                    let _ = write!(out, "<text:s text:c=\"{count}\"/>");
                }
            }
            *spaces = 0;
        };
        for character in line.chars() {
            match character {
                ' ' => spaces += 1,
                '\t' => {
                    flush(out, &mut spaces, at_start);
                    out.push_str("<text:tab/>");
                    at_start = false;
                }
                _ => {
                    flush(out, &mut spaces, at_start);
                    at_start = false;
                    let mut buffer = [0u8; 4];
                    escape_text(out, character.encode_utf8(&mut buffer));
                }
            }
        }
        // Trailing spaces are kept as `text:s`, which no reader collapses.
        if spaces > 0 {
            if spaces == 1 {
                out.push_str("<text:s/>");
            } else {
                let _ = write!(out, "<text:s text:c=\"{spaces}\"/>");
            }
        }
        out.push_str("</text:p>");
    }
}

/// Days since 1899-12-30 as an ISO date, with the time when not midnight
/// (to the millisecond).
fn push_date(out: &mut String, serial: f64) {
    let millis_total = (serial * 86_400_000.0).round() as i64;
    let days = millis_total.div_euclid(86_400_000);
    let millis = millis_total.rem_euclid(86_400_000);
    let (year, month, day) = crate::io::xlsx::reader::civil_from_days(days - 25_569);
    let _ = write!(out, "{year:04}-{month:02}-{day:02}");
    if millis > 0 {
        let seconds = millis / 1000;
        let _ = write!(
            out,
            "T{:02}:{:02}:{:02}",
            seconds / 3600,
            (seconds / 60) % 60,
            seconds % 60
        );
        let fraction = millis % 1000;
        if fraction > 0 {
            let digits = format!("{fraction:03}");
            let _ = write!(out, ".{}", digits.trim_end_matches('0'));
        }
    }
}

/// Days as an ISO duration (`PT36H00M00S`, `PT00H00M01.5S`).
fn push_duration(out: &mut String, days: f64) {
    let millis = (days.abs() * 86_400_000.0).round() as u64;
    if days < 0.0 && millis > 0 {
        out.push('-');
    }
    let seconds = millis / 1000;
    let _ = write!(
        out,
        "PT{:02}H{:02}M{:02}",
        seconds / 3600,
        (seconds / 60) % 60,
        seconds % 60
    );
    let fraction = millis % 1000;
    if fraction > 0 {
        let digits = format!("{fraction:03}");
        let _ = write!(out, ".{}", digits.trim_end_matches('0'));
    }
    out.push('S');
}

const STYLES_HEAD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-styles xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\" xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\" xmlns:number=\"urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0\" xmlns:loext=\"urn:org:documentfoundation:names:experimental:office:xmlns:loext:1.0\" office:version=\"1.3\"><office:styles><style:default-style style:family=\"table-cell\"/><style:style style:name=\"Default\" style:family=\"table-cell\"/>";

const META: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-meta xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:meta=\"urn:oasis:names:tc:opendocument:xmlns:meta:1.0\" office:version=\"1.3\"><office:meta><meta:generator>Sublime</meta:generator></office:meta></office:document-meta>";

const MANIFEST: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" manifest:version=\"1.3\"><manifest:file-entry manifest:full-path=\"/\" manifest:version=\"1.3\" manifest:media-type=\"application/vnd.oasis.opendocument.spreadsheet\"/><manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/><manifest:file-entry manifest:full-path=\"styles.xml\" manifest:media-type=\"text/xml\"/><manifest:file-entry manifest:full-path=\"meta.xml\" manifest:media-type=\"text/xml\"/></manifest:manifest>";
