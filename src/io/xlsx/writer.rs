//! Writes a workbook of one or more sheets, streaming the rows into each
//! worksheet part as they come; the workbook part, which lists the sheets,
//! is written at the end. A cell is typed only when reading it back gives
//! the same text: a plain decimal of at most fifteen significant digits is
//! a number, an ISO 8601 date, date and time, or time (the forms the reader
//! writes) is a date shown in that same form, and `TRUE` and `FALSE` are
//! booleans. Everything else is an inline string, so no shared string
//! table is held.

use std::io::{self, Write};

use crate::io::deflate::Level;
use crate::io::xml::escape_text;
use crate::io::zip::ZipWriter;

const MAIN_NAMESPACE: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const OFFICE_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PART_SIZE: usize = 256 * 1024;

pub struct XlsxWriter<W: Write> {
    zip: ZipWriter<W>,
    part: String,
    row: u64,
    reference: String,
    /// Every sheet's title, in order; the last is the one being written.
    sheets: Vec<String>,
}

impl<W: Write> XlsxWriter<W> {
    /// Writes the fixed parts and opens the first worksheet for rows.
    pub fn new(sink: W, sheet_name: &str) -> io::Result<XlsxWriter<W>> {
        let mut zip = ZipWriter::new(sink);
        zip.add_deflated("_rels/.rels", PACKAGE_RELS.as_bytes())?;
        zip.add_deflated("xl/styles.xml", STYLES.as_bytes())?;
        let mut writer = XlsxWriter {
            zip,
            part: String::with_capacity(PART_SIZE + 4096),
            row: 0,
            reference: String::new(),
            sheets: Vec::new(),
        };
        writer.open_sheet(sheet_name)?;
        Ok(writer)
    }

    /// Closes the sheet being written and opens the next, named `name`.
    pub fn next_sheet(&mut self, name: &str) -> io::Result<()> {
        self.close_sheet()?;
        self.open_sheet(name)
    }

    fn open_sheet(&mut self, name: &str) -> io::Result<()> {
        let title = unique_title(&sheet_title(name), &self.sheets);
        self.sheets.push(title);
        let path = format!("xl/worksheets/sheet{}.xml", self.sheets.len());
        self.zip.begin_deflated(&path)?;
        self.part.clear();
        self.part.push_str(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<worksheet xmlns=\"",
        );
        self.part.push_str(MAIN_NAMESPACE);
        self.part.push_str("\"><sheetData>");
        self.row = 0;
        Ok(())
    }

    fn close_sheet(&mut self) -> io::Result<()> {
        self.part.push_str("</sheetData></worksheet>");
        self.zip.write_part(self.part.as_bytes(), Level::Fast)?;
        self.part.clear();
        self.zip.end_deflated()
    }

    pub fn write_row<'a>(&mut self, cells: impl IntoIterator<Item = &'a str>) -> io::Result<()> {
        self.row += 1;
        let row = self.row;
        self.part.push_str("<row r=\"");
        push_number(&mut self.part, row);
        self.part.push_str("\">");
        for (index, cell) in cells.into_iter().enumerate() {
            self.reference.clear();
            push_column(&mut self.reference, index);
            push_number(&mut self.reference, row);
            self.part.push_str("<c r=\"");
            self.part.push_str(&self.reference);
            if cell.is_empty() {
                // Kept as a cell without a value, so a row read back has
                // its width (trailing empty cells, an empty row).
                self.part.push_str("\"/>");
            } else if is_number_literal(cell) {
                self.part.push_str("\"><v>");
                self.part.push_str(cell);
                self.part.push_str("</v></c>");
            } else if cell == "TRUE" || cell == "FALSE" {
                self.part.push_str("\" t=\"b\"><v>");
                self.part.push(if cell == "TRUE" { '1' } else { '0' });
                self.part.push_str("</v></c>");
            } else if let Some((serial, style)) = iso_serial(cell) {
                self.part.push_str("\" s=\"");
                self.part.push(char::from(b'0' + style));
                self.part.push_str("\"><v>");
                if serial.fract() == 0.0 {
                    push_number(&mut self.part, serial as u64);
                } else {
                    use std::fmt::Write as _;
                    let _ = write!(self.part, "{serial}");
                }
                self.part.push_str("</v></c>");
            } else {
                self.part
                    .push_str("\" t=\"inlineStr\"><is><t xml:space=\"preserve\">");
                escape_text(&mut self.part, cell);
                self.part.push_str("</t></is></c>");
            }
        }
        self.part.push_str("</row>");
        if self.part.len() >= PART_SIZE {
            self.zip.write_part(self.part.as_bytes(), Level::Fast)?;
            self.part.clear();
        }
        Ok(())
    }

    /// Closes the last worksheet, writes the parts that list the sheets,
    /// and closes the package; returns the sink.
    pub fn finish(mut self) -> io::Result<W> {
        self.close_sheet()?;
        let count = self.sheets.len();
        let mut types = String::from(CONTENT_TYPES_HEAD);
        for index in 1..=count {
            types.push_str("<Override PartName=\"/xl/worksheets/sheet");
            push_number(&mut types, index as u64);
            types.push_str(".xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>");
        }
        types.push_str("</Types>");
        self.zip
            .add_deflated("[Content_Types].xml", types.as_bytes())?;
        let mut workbook = String::new();
        workbook.push_str(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<workbook xmlns=\"",
        );
        workbook.push_str(MAIN_NAMESPACE);
        workbook.push_str("\" xmlns:r=\"");
        workbook.push_str(OFFICE_REL);
        workbook.push_str("\"><sheets>");
        let mut rels = String::from(WORKBOOK_RELS_HEAD);
        for (index, title) in self.sheets.iter().enumerate() {
            let number = index as u64 + 1;
            workbook.push_str("<sheet name=\"");
            crate::io::xml::escape_attribute(&mut workbook, title);
            workbook.push_str("\" sheetId=\"");
            push_number(&mut workbook, number);
            workbook.push_str("\" r:id=\"rId");
            push_number(&mut workbook, number);
            workbook.push_str("\"/>");
            rels.push_str("<Relationship Id=\"rId");
            push_number(&mut rels, number);
            rels.push_str("\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet");
            push_number(&mut rels, number);
            rels.push_str(".xml\"/>");
        }
        workbook.push_str("</sheets></workbook>");
        rels.push_str("<Relationship Id=\"rIdStyles\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/></Relationships>");
        self.zip
            .add_deflated("xl/workbook.xml", workbook.as_bytes())?;
        self.zip
            .add_deflated("xl/_rels/workbook.xml.rels", rels.as_bytes())?;
        self.zip.finish()
    }
}

/// A sheet title Excel accepts: at most 31 characters, none of `\ / ? * [ ] :`.
fn sheet_title(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|character| {
            if "\\/?*[]:".contains(character) {
                '_'
            } else {
                character
            }
        })
        .take(31)
        .collect();
    if cleaned.trim().is_empty() {
        "Sheet1".to_string()
    } else {
        cleaned
    }
}

/// `title`, or `title (2)` and on when an earlier sheet has it (Excel
/// compares sheet names without case), kept within 31 characters.
fn unique_title(title: &str, taken: &[String]) -> String {
    let mut candidate = title.to_string();
    let mut number = 2;
    while taken
        .iter()
        .any(|taken| taken.to_lowercase() == candidate.to_lowercase())
    {
        let suffix = format!(" ({number})");
        let room = 31usize.saturating_sub(suffix.chars().count());
        let base: String = title.chars().take(room).collect();
        candidate = format!("{base}{suffix}");
        number += 1;
    }
    candidate
}

fn push_number(out: &mut String, number: u64) {
    use std::fmt::Write;
    let _ = write!(out, "{number}");
}

/// 0 -> `A`, 26 -> `AA`.
fn push_column(out: &mut String, index: usize) {
    let mut letters = [0u8; 8];
    let mut count = 0;
    let mut remaining = index + 1;
    while remaining > 0 && count < letters.len() {
        remaining -= 1;
        letters[count] = b'A' + (remaining % 26) as u8;
        remaining /= 26;
        count += 1;
    }
    for letter in letters[..count].iter().rev() {
        out.push(*letter as char);
    }
}

/// A plain decimal Excel would store exactly: an optional sign, digits
/// without a leading zero, an optional fraction, an optional exponent,
/// at most fifteen significant digits.
pub fn is_number_literal(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut at = 0;
    if bytes.first() == Some(&b'-') {
        at += 1;
    }
    let integer_start = at;
    while at < bytes.len() && bytes[at].is_ascii_digit() {
        at += 1;
    }
    let integer_digits = at - integer_start;
    if integer_digits == 0 || (integer_digits > 1 && bytes[integer_start] == b'0') {
        return false;
    }
    let mut significant = integer_digits;
    if at < bytes.len() && bytes[at] == b'.' {
        at += 1;
        let fraction_start = at;
        while at < bytes.len() && bytes[at].is_ascii_digit() {
            at += 1;
        }
        let fraction_digits = at - fraction_start;
        if fraction_digits == 0 || bytes[at - 1] == b'0' {
            return false;
        }
        significant += fraction_digits;
    }
    if at < bytes.len() && (bytes[at] == b'e' || bytes[at] == b'E') {
        at += 1;
        if at < bytes.len() && (bytes[at] == b'+' || bytes[at] == b'-') {
            at += 1;
        }
        let exponent_start = at;
        while at < bytes.len() && bytes[at].is_ascii_digit() {
            at += 1;
        }
        if at == exponent_start {
            return false;
        }
    }
    at == bytes.len() && significant <= 15 && text.parse::<f64>().is_ok_and(f64::is_finite)
}

/// Cell styles (`cellXfs` entries) for the dates `iso_serial` reads, each
/// shown in the form it was written in: `yyyy-mm-dd`, the same with a
/// quoted `T` and `hh:mm:ss`, and `[hh]:mm:ss` (LibreOffice shows a plain
/// `hh:mm:ss` with AM and PM).
const STYLE_DATE: u8 = 1;
const STYLE_DATE_TIME: u8 = 2;
const STYLE_TIME: u8 = 3;

/// The Excel serial (days since 1899-12-30, the 1900 system) and cell style
/// of an ISO 8601 date (`2024-08-08`), date and time (`2024-08-08T14:35:00`),
/// or time (`14:35:00`), in the forms the reader writes and only when the
/// reader would write this text back: a real calendar date from 1900-03-01
/// (before it, Excel's serials count a day that did not exist) to 9999-12-31,
/// a date and time whose time is not midnight (that reads back as a date).
pub fn iso_serial(text: &str) -> Option<(f64, u8)> {
    let bytes = text.as_bytes();
    let digits = |range: std::ops::Range<usize>| -> Option<u32> {
        let slice = bytes.get(range)?;
        if !slice.iter().all(u8::is_ascii_digit) {
            return None;
        }
        Some(
            slice
                .iter()
                .fold(0, |value, digit| value * 10 + u32::from(digit - b'0')),
        )
    };
    let time = |at: usize| -> Option<u32> {
        if bytes.get(at + 2) != Some(&b':') || bytes.get(at + 5) != Some(&b':') {
            return None;
        }
        let (hour, minute, second) = (
            digits(at..at + 2)?,
            digits(at + 3..at + 5)?,
            digits(at + 6..at + 8)?,
        );
        (hour < 24 && minute < 60 && second < 60).then_some(hour * 3600 + minute * 60 + second)
    };
    let date = || -> Option<f64> {
        if bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') {
            return None;
        }
        let (year, month, day) = (digits(0..4)?, digits(5..7)?, digits(8..10)?);
        let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
        let length = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if leap => 29,
            2 => 28,
            _ => return None,
        };
        if year < 1900 || day == 0 || day > length || (year == 1900 && month < 3) {
            return None;
        }
        Some((days_from_civil(year, month, day) + 25_569) as f64)
    };
    match bytes.len() {
        10 => Some((date()?, STYLE_DATE)),
        19 if bytes[10] == b'T' => {
            let seconds = time(11)?;
            (seconds > 0).then_some(())?;
            Some((date()? + f64::from(seconds) / 86_400.0, STYLE_DATE_TIME))
        }
        8 => Some((f64::from(time(0)?) / 86_400.0, STYLE_TIME)),
        _ => None,
    }
}

/// Days from 1970-01-01 to a calendar date (Howard Hinnant's algorithm,
/// the inverse of the reader's `civil_from_days`).
fn days_from_civil(year: u32, month: u32, day: u32) -> i64 {
    let year = i64::from(year) - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month = i64::from(month);
    let day_of_year =
        (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

const CONTENT_TYPES_HEAD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/><Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/>";

const PACKAGE_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>";

const WORKBOOK_RELS_HEAD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">";

const STYLES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><numFmts count=\"3\"><numFmt numFmtId=\"164\" formatCode=\"yyyy\\-mm\\-dd\"/><numFmt numFmtId=\"165\" formatCode=\"yyyy\\-mm\\-dd&quot;T&quot;hh:mm:ss\"/><numFmt numFmtId=\"166\" formatCode=\"[hh]:mm:ss\"/></numFmts><fonts count=\"1\"><font><sz val=\"11\"/><name val=\"Calibri\"/></font></fonts><fills count=\"2\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill patternType=\"gray125\"/></fill></fills><borders count=\"1\"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs><cellXfs count=\"4\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/><xf numFmtId=\"164\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/><xf numFmtId=\"165\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/><xf numFmtId=\"166\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/></cellXfs><cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles></styleSheet>";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_dates_times_and_booleans_are_typed_only_when_they_read_back() {
        assert_eq!(iso_serial("1900-03-01"), Some((61.0, STYLE_DATE)));
        assert_eq!(iso_serial("2024-08-08"), Some((45_512.0, STYLE_DATE)));
        assert_eq!(
            iso_serial("2024-08-08T12:00:00"),
            Some((45_512.5, STYLE_DATE_TIME))
        );
        assert_eq!(iso_serial("06:00:00"), Some((0.25, STYLE_TIME)));
        for text in [
            "2024-02-30",
            "2023-02-29",
            "1900-02-28",
            "1899-12-31",
            "2024-8-8",
            "2024-08-08T00:00:00",
            "2024-08-08 14:35:00",
            "2024-08-08T14:35",
            "24:00:00",
            "12:60:00",
            "SEPT2",
            "1-2",
            "3/4",
        ] {
            assert_eq!(iso_serial(text), None, "{text}");
        }
        // Every text comes back as it went in, typed or not.
        let texts = [
            "2024-08-08",
            "2024-02-29",
            "1900-03-01",
            "9999-12-31",
            "2024-08-08T14:35:09",
            "2024-08-08T23:59:59",
            "2024-08-08T00:00:00",
            "00:00:00",
            "23:59:59",
            "TRUE",
            "FALSE",
            "true",
            "False",
            "2023-02-29",
            "1900-02-28",
            "2024-8-8",
            "SEPT2",
            "1-2",
            "3/4",
            "007",
            "1.50",
            "1.5",
            "42",
        ];
        let mut writer = XlsxWriter::new(Vec::new(), "Sheet1").expect("opens");
        writer.write_row(texts).expect("writes");
        let bytes = writer.finish().expect("finishes");
        let workbook = crate::io::xlsx::Workbook::open(&bytes).expect("reads");
        let mut rows: Vec<Vec<String>> = Vec::new();
        workbook
            .read_rows(0, |cells| -> Result<(), crate::io::xlsx::XlsxError> {
                rows.push(cells.to_vec());
                Ok(())
            })
            .expect("rows");
        assert_eq!(rows, vec![texts.map(str::to_string).to_vec()]);
        // The typed ones are cells of their type, not strings.
        let archive = crate::io::zip::ZipArchive::parse(&bytes).expect("zip");
        let entry = archive
            .entries()
            .iter()
            .find(|entry| entry.name == "xl/worksheets/sheet1.xml")
            .expect("sheet");
        let mut sheet = Vec::new();
        archive.read(entry, &mut sheet).expect("inflates");
        let sheet = String::from_utf8(sheet).expect("utf-8");
        assert_eq!(sheet.matches(" t=\"b\"").count(), 2);
        assert_eq!(sheet.matches(" s=\"1\"").count(), 4);
        assert_eq!(sheet.matches(" s=\"2\"").count(), 2);
        assert_eq!(sheet.matches(" s=\"3\"").count(), 2);
    }

    #[test]
    fn number_literals_are_what_excel_stores_exactly() {
        for text in [
            "0",
            "7",
            "-12",
            "1.5",
            "0.25",
            "1e10",
            "-2.5E-3",
            "123456789012345",
        ] {
            assert!(is_number_literal(text), "{text}");
        }
        for text in [
            "",
            "007",
            "1.",
            ".5",
            "1.50",
            "+1",
            "1e",
            "1234567890123456",
            "1,000",
            " 1",
            "NaN",
            "0x10",
        ] {
            assert!(!is_number_literal(text), "{text}");
        }
    }

    #[test]
    fn columns_count_in_letters() {
        let mut out = String::new();
        push_column(&mut out, 0);
        push_column(&mut out, 25);
        push_column(&mut out, 26);
        push_column(&mut out, 701);
        assert_eq!(out, "AZAAZZ");
    }

    #[test]
    fn sheet_titles_are_cleaned() {
        assert_eq!(sheet_title("a/b:c"), "a_b_c");
        assert_eq!(sheet_title("   "), "Sheet1");
        assert_eq!(sheet_title(&"x".repeat(40)).len(), 31);
    }
}
