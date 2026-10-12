//! Reads a workbook: the sheet list from `xl/workbook.xml` and its
//! relationships, the shared string table, the cell styles that mark a
//! number as a date, and then one worksheet's rows, each handed to the
//! caller as text cells. Cells are text as Excel would export them:
//! numbers as written in the file, dates as ISO 8601, booleans as `TRUE`
//! and `FALSE`, errors as their code. Rows are padded to the sheet's
//! dimension so the output is rectangular, and gaps in row numbers are
//! empty rows.

use std::borrow::Cow;

use crate::io::xml::{XmlEvent, XmlReader};
use crate::io::zip::{ZipArchive, ZipError};

#[derive(Debug)]
pub enum XlsxError {
    Zip(ZipError),
    /// A required part is missing or unreadable.
    Part(&'static str),
    NoSuchSheet {
        wanted: String,
        available: Vec<String>,
    },
}

impl std::fmt::Display for XlsxError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            XlsxError::Zip(error) => write!(formatter, "not a workbook package: {error:?}"),
            XlsxError::Part(part) => write!(formatter, "the workbook has no readable {part}"),
            XlsxError::NoSuchSheet { wanted, available } => write!(
                formatter,
                "no sheet '{wanted}' (the sheets are: {})",
                available.join(", ")
            ),
        }
    }
}

impl std::error::Error for XlsxError {}

pub struct Workbook<'a> {
    archive: ZipArchive<'a>,
    /// Sheet names and their part paths, in workbook order.
    sheets: Vec<(String, String)>,
    shared: Vec<String>,
    /// Per cell style index: whether its number format is a date.
    date_styles: Vec<bool>,
    /// Per cell style index: its number format code, `None` for General.
    style_formats: Vec<Option<String>>,
    date1904: bool,
}

impl<'a> Workbook<'a> {
    pub fn open(bytes: &'a [u8]) -> Result<Workbook<'a>, XlsxError> {
        let archive = ZipArchive::parse(bytes).map_err(XlsxError::Zip)?;
        let workbook_xml =
            read_text(&archive, "xl/workbook.xml").ok_or(XlsxError::Part("workbook part"))?;
        let rels = read_text(&archive, "xl/_rels/workbook.xml.rels").unwrap_or_default();
        let sheets = sheet_list(&workbook_xml, &rels);
        if sheets.is_empty() {
            return Err(XlsxError::Part("sheet"));
        }
        let date1904 =
            workbook_xml.contains("date1904=\"1\"") || workbook_xml.contains("date1904=\"true\"");
        let shared = match read_text(&archive, "xl/sharedStrings.xml") {
            Some(text) => shared_strings(&text),
            None => Vec::new(),
        };
        let (date_styles, style_formats) = match read_text(&archive, "xl/styles.xml") {
            Some(text) => cell_styles(&text),
            None => (Vec::new(), Vec::new()),
        };
        Ok(Workbook {
            archive,
            sheets,
            shared,
            date_styles,
            style_formats,
            date1904,
        })
    }

    pub fn sheet_names(&self) -> Vec<&str> {
        self.sheets.iter().map(|(name, _)| name.as_str()).collect()
    }

    /// The sheet index for a selector: none for the first sheet, a name,
    /// or a 1-based number.
    pub fn select(&self, selector: Option<&str>) -> Result<usize, XlsxError> {
        let Some(selector) = selector else {
            return Ok(0);
        };
        if let Some(index) = self.sheets.iter().position(|(name, _)| name == selector) {
            return Ok(index);
        }
        if let Ok(number) = selector.parse::<usize>() {
            if number >= 1 && number <= self.sheets.len() {
                return Ok(number - 1);
            }
        }
        Err(XlsxError::NoSuchSheet {
            wanted: selector.to_string(),
            available: self.sheets.iter().map(|(name, _)| name.clone()).collect(),
        })
    }

    /// Hands every row of the sheet to `on_row`, as text cells.
    pub fn read_rows<E>(
        &self,
        sheet: usize,
        mut on_row: impl FnMut(&[String]) -> Result<(), E>,
    ) -> Result<(), E>
    where
        E: From<XlsxError>,
    {
        self.read_sheet(sheet, &mut |cells, _| on_row(cells))
            .map(|_| ())
    }

    /// The number format code of a cell style (`#,##0.00`, `d mmm yyyy`),
    /// `None` for General.
    pub fn style_format(&self, style: usize) -> Option<&str> {
        self.style_formats.get(style)?.as_deref()
    }

    /// Hands every row of the sheet to `on_row`, as text cells with each
    /// cell's style (its `cellXfs` index), and returns the sheet's merged
    /// ranges as (row, column, rows, columns) from 0.
    pub fn read_sheet<E>(
        &self,
        sheet: usize,
        on_row: &mut StyledRow<'_, E>,
    ) -> Result<Vec<(usize, usize, usize, usize)>, E>
    where
        E: From<XlsxError>,
    {
        let mut merges = Vec::new();
        let mut styles: Vec<Option<usize>> = Vec::new();
        let mut no_styles: Vec<Option<usize>> = Vec::new();
        let part = &self.sheets[sheet].1;
        let xml = read_text(&self.archive, part).ok_or(XlsxError::Part("worksheet part"))?;
        let reader = XmlReader::new(&xml);
        let mut width = 0usize;
        let mut cells: Vec<String> = Vec::new();
        let mut next_row: u64 = 1;
        let mut in_sheet_data = false;
        let mut current = Cell::default();
        let mut text_target = TextTarget::None;
        // A row the file skips is an empty row of the sheet's width.
        let mut gap: Vec<String> = Vec::new();
        for event in reader {
            match event {
                XmlEvent::Start {
                    name,
                    attributes,
                    self_closing,
                } => match name {
                    "dimension" => {
                        if let Some(reference) = attribute(&attributes, "ref") {
                            width = dimension_width(reference);
                            pad(&mut gap, width);
                        }
                    }
                    "sheetData" => in_sheet_data = !self_closing,
                    "row" if in_sheet_data => {
                        let number = attribute(&attributes, "r")
                            .and_then(|text| text.parse::<u64>().ok())
                            .unwrap_or(next_row);
                        while next_row < number {
                            no_styles.resize(gap.len(), None);
                            on_row(&gap, &no_styles)?;
                            next_row += 1;
                        }
                        cells.clear();
                        styles.clear();
                        if self_closing {
                            pad(&mut cells, width);
                            styles.resize(cells.len(), None);
                            on_row(&cells, &styles)?;
                            next_row = number + 1;
                        }
                    }
                    "c" if in_sheet_data => {
                        current = Cell {
                            column: attribute(&attributes, "r")
                                .map(column_index)
                                .unwrap_or(cells.len()),
                            kind: attribute(&attributes, "t").unwrap_or("n").to_string(),
                            style: attribute(&attributes, "s").and_then(|text| text.parse().ok()),
                            value: String::new(),
                        };
                        if self_closing {
                            self.place(&mut cells, &current);
                            place_style(&mut styles, &current);
                        }
                    }
                    "mergeCell" => {
                        if let Some(range) = attribute(&attributes, "ref").and_then(merge_range) {
                            merges.push(range);
                        }
                    }
                    "v" | "t" if in_sheet_data => {
                        text_target = TextTarget::Value;
                        if self_closing {
                            text_target = TextTarget::None;
                        }
                    }
                    _ => {}
                },
                XmlEvent::Text(text) => {
                    if text_target == TextTarget::Value {
                        current.value.push_str(&text);
                    }
                }
                XmlEvent::End { name } => match name {
                    "v" | "t" => text_target = TextTarget::None,
                    "c" if in_sheet_data => {
                        self.place(&mut cells, &current);
                        place_style(&mut styles, &current);
                    }
                    "row" if in_sheet_data => {
                        pad(&mut cells, width);
                        styles.resize(cells.len(), None);
                        on_row(&cells, &styles)?;
                        next_row += 1;
                    }
                    "sheetData" => in_sheet_data = false,
                    _ => {}
                },
            }
        }
        Ok(merges)
    }

    /// Puts the cell's text at its column, filling the gap with empties.
    fn place(&self, cells: &mut Vec<String>, cell: &Cell) {
        while cells.len() < cell.column {
            cells.push(String::new());
        }
        let text = self.cell_text(cell);
        if cells.len() == cell.column {
            cells.push(text);
        } else {
            cells[cell.column] = text;
        }
    }

    fn cell_text(&self, cell: &Cell) -> String {
        match cell.kind.as_str() {
            "s" => {
                let index: usize = cell.value.trim().parse().unwrap_or(usize::MAX);
                self.shared.get(index).cloned().unwrap_or_default()
            }
            "b" => {
                if cell.value.trim() == "1" {
                    "TRUE".to_string()
                } else {
                    "FALSE".to_string()
                }
            }
            "n" => {
                let is_date = cell
                    .style
                    .and_then(|style| self.date_styles.get(style).copied())
                    .unwrap_or(false);
                if is_date {
                    if let Ok(serial) = cell.value.trim().parse::<f64>() {
                        return serial_to_iso(serial, self.date1904);
                    }
                }
                cell.value.trim().to_string()
            }
            _ => cell.value.clone(),
        }
    }
}

#[derive(Default)]
struct Cell {
    column: usize,
    kind: String,
    style: Option<usize>,
    value: String,
}

#[derive(PartialEq, Eq)]
enum TextTarget {
    None,
    Value,
}

/// A handler of a row's cells and their styles (`Workbook::read_sheet`).
pub type StyledRow<'a, E> = dyn FnMut(&[String], &[Option<usize>]) -> Result<(), E> + 'a;

/// Puts the cell's style at its column, as `place` puts its text.
fn place_style(styles: &mut Vec<Option<usize>>, cell: &Cell) {
    if styles.len() <= cell.column {
        styles.resize(cell.column + 1, None);
    }
    styles[cell.column] = cell.style;
}

/// `B2:C4` as (row, column, rows, columns) from 0.
fn merge_range(reference: &str) -> Option<(usize, usize, usize, usize)> {
    let (start, end) = reference.split_once(':')?;
    let point = |cell: &str| -> Option<(usize, usize)> {
        let split = cell.find(|c: char| c.is_ascii_digit())?;
        let row: usize = cell[split..].parse().ok()?;
        Some((row.checked_sub(1)?, column_index(&cell[..split])))
    };
    let (top, left) = point(start)?;
    let (bottom, right) = point(end)?;
    (bottom >= top && right >= left).then(|| (top, left, bottom - top + 1, right - left + 1))
}

fn pad(cells: &mut Vec<String>, width: usize) {
    while cells.len() < width {
        cells.push(String::new());
    }
}

fn read_text(archive: &ZipArchive<'_>, name: &str) -> Option<String> {
    let entry = archive.find(name)?;
    let mut bytes = Vec::with_capacity(entry.uncompressed_size as usize);
    archive.read(entry, &mut bytes).ok()?;
    String::from_utf8(bytes).ok()
}

fn attribute<'e>(attributes: &'e [(&str, Cow<'_, str>)], name: &str) -> Option<&'e str> {
    attributes
        .iter()
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.as_ref())
}

/// `<sheet name= r:id=>` entries joined to the relationship targets.
fn sheet_list(workbook_xml: &str, rels: &str) -> Vec<(String, String)> {
    let mut targets: Vec<(String, String)> = Vec::new();
    for event in XmlReader::new(rels) {
        if let XmlEvent::Start {
            name: "Relationship",
            attributes,
            ..
        } = &event
        {
            if let (Some(id), Some(target)) =
                (attribute(attributes, "Id"), attribute(attributes, "Target"))
            {
                let path = match target.strip_prefix('/') {
                    Some(absolute) => absolute.to_string(),
                    None => format!("xl/{target}"),
                };
                targets.push((id.to_string(), path));
            }
        }
    }
    let mut sheets = Vec::new();
    let mut fallback = 1;
    for event in XmlReader::new(workbook_xml) {
        if let XmlEvent::Start {
            name: "sheet",
            attributes,
            ..
        } = &event
        {
            let name = attribute(attributes, "name").unwrap_or("Sheet").to_string();
            let id = attribute(attributes, "r:id").or_else(|| attribute(attributes, "id"));
            let part = id
                .and_then(|id| targets.iter().find(|(known, _)| known == id))
                .map(|(_, path)| path.clone())
                .unwrap_or_else(|| format!("xl/worksheets/sheet{fallback}.xml"));
            fallback += 1;
            sheets.push((name, part));
        }
    }
    sheets
}

/// The shared string table: each `si` is its `t` runs joined.
fn shared_strings(xml: &str) -> Vec<String> {
    let mut strings = Vec::new();
    let mut current = String::new();
    let mut in_item = false;
    let mut in_text = false;
    let mut in_phonetic = false;
    for event in XmlReader::new(xml) {
        match event {
            XmlEvent::Start {
                name, self_closing, ..
            } => match name {
                "si" => {
                    in_item = true;
                    current.clear();
                    if self_closing {
                        strings.push(String::new());
                        in_item = false;
                    }
                }
                "rPh" => in_phonetic = !self_closing,
                "t" if in_item && !in_phonetic => in_text = !self_closing,
                _ => {}
            },
            XmlEvent::Text(text) => {
                if in_text {
                    current.push_str(&text);
                }
            }
            XmlEvent::End { name } => match name {
                "t" => in_text = false,
                "rPh" => in_phonetic = false,
                "si" => {
                    strings.push(std::mem::take(&mut current));
                    in_item = false;
                }
                _ => {}
            },
        }
    }
    strings
}

/// Per `cellXfs` entry, whether its number format shows a date or time, and
/// its format code (`None` for General).
fn cell_styles(xml: &str) -> (Vec<bool>, Vec<Option<String>>) {
    let mut custom_dates: Vec<u32> = Vec::new();
    let mut custom_codes: Vec<(u32, String)> = Vec::new();
    let mut styles = Vec::new();
    let mut formats = Vec::new();
    let mut in_cell_xfs = false;
    for event in XmlReader::new(xml) {
        match event {
            XmlEvent::Start {
                name,
                attributes,
                self_closing,
            } => match name {
                "numFmt" => {
                    let id = attribute(&attributes, "numFmtId")
                        .and_then(|text| text.parse::<u32>().ok());
                    let code = attribute(&attributes, "formatCode").unwrap_or("");
                    if let Some(id) = id {
                        if format_code_is_date(code) {
                            custom_dates.push(id);
                        }
                        custom_codes.push((id, code.to_string()));
                    }
                }
                "cellXfs" => in_cell_xfs = !self_closing,
                "xf" if in_cell_xfs => {
                    let id = attribute(&attributes, "numFmtId")
                        .and_then(|text| text.parse::<u32>().ok())
                        .unwrap_or(0);
                    styles.push(builtin_is_date(id) || custom_dates.contains(&id));
                    formats.push(
                        custom_codes
                            .iter()
                            .find(|(known, _)| *known == id)
                            .map(|(_, code)| code.clone())
                            .or_else(|| builtin_code(id).map(str::to_string))
                            .filter(|code| !code.eq_ignore_ascii_case("general")),
                    );
                }
                _ => {}
            },
            XmlEvent::End { name: "cellXfs" } => in_cell_xfs = false,
            _ => {}
        }
    }
    (styles, formats)
}

/// The code of a built-in number format (ECMA-376 18.8.30), as Excel shows
/// it in the en-US locale.
fn builtin_code(id: u32) -> Option<&'static str> {
    Some(match id {
        1 => "0",
        2 => "0.00",
        3 => "#,##0",
        4 => "#,##0.00",
        9 => "0%",
        10 => "0.00%",
        11 => "0.00E+00",
        12 => "# ?/?",
        13 => "# ??/??",
        14 => "m/d/yyyy",
        15 => "d-mmm-yy",
        16 => "d-mmm",
        17 => "mmm-yy",
        18 => "h:mm AM/PM",
        19 => "h:mm:ss AM/PM",
        20 => "h:mm",
        21 => "h:mm:ss",
        22 => "m/d/yyyy h:mm",
        37 => "#,##0 ;(#,##0)",
        38 => "#,##0 ;[Red](#,##0)",
        39 => "#,##0.00;(#,##0.00)",
        40 => "#,##0.00;[Red](#,##0.00)",
        45 => "mm:ss",
        46 => "[h]:mm:ss",
        47 => "mmss.0",
        48 => "##0.0E+0",
        49 => "@",
        _ => return None,
    })
}

fn builtin_is_date(id: u32) -> bool {
    matches!(id, 14..=22 | 27..=36 | 45..=47 | 50..=58)
}

/// A format code shows a date or time when, outside quotes and brackets,
/// it uses day, month, year, hour, or second tokens.
fn format_code_is_date(code: &str) -> bool {
    let mut in_quotes = false;
    let mut in_brackets = false;
    let mut escaped = false;
    for character in code.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match character {
            '\\' => escaped = true,
            '"' => in_quotes = !in_quotes,
            '[' if !in_quotes => in_brackets = true,
            ']' if !in_quotes => in_brackets = false,
            'd' | 'D' | 'm' | 'M' | 'y' | 'Y' | 'h' | 'H' | 's' | 'S'
                if !in_quotes && !in_brackets =>
            {
                return true;
            }
            _ => {}
        }
    }
    false
}

/// `A1` -> 0, `AB12` -> 27.
fn column_index(reference: &str) -> usize {
    let mut index = 0usize;
    for byte in reference.bytes() {
        if !byte.is_ascii_alphabetic() {
            break;
        }
        index = index * 26 + (byte.to_ascii_uppercase() - b'A') as usize + 1;
    }
    index.saturating_sub(1)
}

/// The column count of `A1:D10`.
fn dimension_width(reference: &str) -> usize {
    let last = reference.rsplit(':').next().unwrap_or(reference);
    column_index(last) + 1
}

/// An Excel serial as ISO 8601: a date when it has no time part, a time
/// when it has no date part, otherwise both.
pub fn serial_to_iso(serial: f64, date1904: bool) -> String {
    let days = serial.floor();
    let fraction = serial - days;
    let seconds_total = (fraction * 86_400.0).round() as u64;
    let (hour, minute, second) = (
        seconds_total / 3600,
        (seconds_total / 60) % 60,
        seconds_total % 60,
    );
    let has_time = seconds_total > 0 && seconds_total < 86_400;
    if days < 1.0 && !date1904 {
        return format!("{hour:02}:{minute:02}:{second:02}");
    }
    let unix_days = if date1904 {
        days as i64 - 24_107
    } else if days < 61.0 {
        days as i64 - 25_568
    } else {
        days as i64 - 25_569
    };
    let (year, month, day) = civil_from_days(unix_days);
    if has_time {
        format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}")
    } else {
        format!("{year:04}-{month:02}-{day:02}")
    }
}

/// Days since 1970-01-01 to a calendar date (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_index + 2) / 5 + 1) as u32;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    } as u32;
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serials_become_iso_dates_and_times() {
        assert_eq!(serial_to_iso(45_296.0, false), "2024-01-05");
        assert_eq!(serial_to_iso(1.0, false), "1900-01-01");
        assert_eq!(serial_to_iso(59.0, false), "1900-02-28");
        assert_eq!(serial_to_iso(61.0, false), "1900-03-01");
        assert_eq!(serial_to_iso(45_296.5, false), "2024-01-05T12:00:00");
        assert_eq!(serial_to_iso(0.75, false), "18:00:00");
        assert_eq!(serial_to_iso(43_834.0, true), "2024-01-05");
    }

    #[test]
    fn column_references_and_dimensions() {
        assert_eq!(column_index("A1"), 0);
        assert_eq!(column_index("Z9"), 25);
        assert_eq!(column_index("AA1"), 26);
        assert_eq!(dimension_width("A1:D10"), 4);
    }

    #[test]
    fn format_codes_that_are_dates() {
        assert!(format_code_is_date("yyyy-mm-dd"));
        assert!(format_code_is_date("[$-409]d-mmm-yy;@"));
        assert!(format_code_is_date("h:mm AM/PM"));
        assert!(!format_code_is_date("0.00"));
        assert!(!format_code_is_date("#,##0 \"dollars\""));
        assert!(!format_code_is_date("[Red]0"));
        assert!(!format_code_is_date("General"));
    }

    #[test]
    fn merged_ranges_and_style_formats() {
        assert_eq!(merge_range("B2:C4"), Some((1, 1, 3, 2)));
        assert_eq!(merge_range("A1:A1"), Some((0, 0, 1, 1)));
        assert_eq!(merge_range("C1:A1"), None);
        let styles = r#"<styleSheet><numFmts count="1"><numFmt numFmtId="164" formatCode="&quot;£&quot;#,##0"/></numFmts><cellXfs count="4"><xf numFmtId="0"/><xf numFmtId="164"/><xf numFmtId="10"/><xf numFmtId="14"/></cellXfs></styleSheet>"#;
        let (dates, formats) = cell_styles(styles);
        assert_eq!(dates, [false, false, false, true]);
        assert_eq!(
            formats,
            [
                None,
                Some("\"£\"#,##0".to_string()),
                Some("0.00%".to_string()),
                Some("m/d/yyyy".to_string())
            ]
        );
    }
}
