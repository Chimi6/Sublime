//! Writes a one-sheet workbook, streaming the rows into the worksheet
//! part as they come. Cells whose text is a plain decimal of at most
//! fifteen significant digits become numbers (what Excel makes of them);
//! everything else is an inline string, so no shared string table is held.

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
}

impl<W: Write> XlsxWriter<W> {
    /// Writes the fixed parts and opens the worksheet for rows.
    pub fn new(sink: W, sheet_name: &str) -> io::Result<XlsxWriter<W>> {
        let mut zip = ZipWriter::new(sink);
        zip.add_deflated("[Content_Types].xml", CONTENT_TYPES.as_bytes())?;
        zip.add_deflated("_rels/.rels", PACKAGE_RELS.as_bytes())?;
        let mut workbook = String::new();
        workbook.push_str(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<workbook xmlns=\"",
        );
        workbook.push_str(MAIN_NAMESPACE);
        workbook.push_str("\" xmlns:r=\"");
        workbook.push_str(OFFICE_REL);
        workbook.push_str("\"><sheets><sheet name=\"");
        crate::io::xml::escape_attribute(&mut workbook, &sheet_title(sheet_name));
        workbook.push_str("\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>");
        zip.add_deflated("xl/workbook.xml", workbook.as_bytes())?;
        zip.add_deflated("xl/_rels/workbook.xml.rels", WORKBOOK_RELS.as_bytes())?;
        zip.add_deflated("xl/styles.xml", STYLES.as_bytes())?;
        zip.begin_deflated("xl/worksheets/sheet1.xml")?;
        let mut part = String::with_capacity(PART_SIZE + 4096);
        part.push_str(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<worksheet xmlns=\"",
        );
        part.push_str(MAIN_NAMESPACE);
        part.push_str("\"><sheetData>");
        Ok(XlsxWriter {
            zip,
            part,
            row: 0,
            reference: String::new(),
        })
    }

    pub fn write_row<'a>(&mut self, cells: impl IntoIterator<Item = &'a str>) -> io::Result<()> {
        self.row += 1;
        let row = self.row;
        self.part.push_str("<row r=\"");
        push_number(&mut self.part, row);
        self.part.push_str("\">");
        for (index, cell) in cells.into_iter().enumerate() {
            if cell.is_empty() {
                continue;
            }
            self.reference.clear();
            push_column(&mut self.reference, index);
            push_number(&mut self.reference, row);
            self.part.push_str("<c r=\"");
            self.part.push_str(&self.reference);
            if is_number_literal(cell) {
                self.part.push_str("\"><v>");
                self.part.push_str(cell);
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

    /// Closes the worksheet and the package; returns the sink.
    pub fn finish(mut self) -> io::Result<W> {
        self.part.push_str("</sheetData></worksheet>");
        self.zip.write_part(self.part.as_bytes(), Level::Fast)?;
        self.zip.end_deflated()?;
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

const CONTENT_TYPES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/><Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/><Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/></Types>";

const PACKAGE_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>";

const WORKBOOK_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/></Relationships>";

const STYLES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><fonts count=\"1\"><font><sz val=\"11\"/><name val=\"Calibri\"/></font></fonts><fills count=\"2\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill patternType=\"gray125\"/></fill></fills><borders count=\"1\"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs><cellXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/></cellXfs><cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles></styleSheet>";

#[cfg(test)]
mod tests {
    use super::*;

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
