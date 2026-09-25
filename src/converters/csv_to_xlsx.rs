//! Delimited rows -> workbook: one sheet, streamed row by row.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Fidelity, Input, Tier};
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::csv::{CsvReader, Record};
use crate::io::xlsx::XlsxWriter;

const PROGRESS_INTERVAL: u64 = 4096;
const FIDELITY_NOTE: &str = "cells that are plain decimals of up to fifteen digits become numbers, everything else stays text; one sheet named Sheet1";

pub struct CsvToXlsx {
    pub name: &'static str,
    pub from: &'static Format,
    pub delimiter: u8,
}

pub static CSV_TO_XLSX: CsvToXlsx = CsvToXlsx {
    name: "csv-to-xlsx",
    from: &formats::CSV,
    delimiter: b',',
};

pub static TSV_TO_XLSX: CsvToXlsx = CsvToXlsx {
    name: "tsv-to-xlsx",
    from: &formats::TSV,
    delimiter: b'\t',
};

impl Converter for CsvToXlsx {
    fn name(&self) -> &'static str {
        self.name
    }

    fn from(&self) -> &'static Format {
        self.from
    }

    fn to(&self) -> &'static Format {
        &formats::XLSX
    }

    fn fidelity(&self) -> Fidelity {
        Fidelity::Conditional(FIDELITY_NOTE)
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
        let mut reader = CsvReader::with_delimiter(input, self.delimiter);
        let sheet_name = context.options.sheet.as_deref().unwrap_or("Sheet1");
        let mut writer = XlsxWriter::new(output, sheet_name)?;
        let mut record = Record::new();
        let mut count: u64 = 0;
        while reader.read_record(&mut record)? {
            writer.write_row(record.fields())?;
            count += 1;
            if count % PROGRESS_INTERVAL == 0 {
                context.progress(self.name, reader.bytes_consumed());
            }
        }
        let sink = writer.finish()?;
        sink.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declares_contract() {
        assert_eq!(CSV_TO_XLSX.name(), "csv-to-xlsx");
        assert_eq!(TSV_TO_XLSX.from().id, "tsv");
        assert_eq!(CSV_TO_XLSX.to().id, "xlsx");
        assert_eq!(CSV_TO_XLSX.fidelity(), Fidelity::Conditional(FIDELITY_NOTE));
    }
}
