//! Workbook -> delimited rows: one sheet (the first, or `--sheet`), every
//! row as text cells.

use std::io::{Read, Write};

use crate::converter::{ConvertError, Converter, Fidelity, Input, Tier};
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::csv::CsvWriter;
use crate::io::xlsx::{Workbook, XlsxError};

const PROGRESS_INTERVAL: u64 = 4096;
const FIDELITY_NOTE: &str = "one sheet (the first, or --sheet); numbers as stored, dates as ISO 8601, booleans as TRUE and FALSE, formulas as their last value; formatting is dropped";

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
