//! ODS <-> CSV: the csv-json record shape as a spreadsheet LibreOffice
//! writes (the reference writer, run by `bench/pairs/ods-csv.sh`); ours
//! against `calamine` 0.36 with the `csv` crate (read) and LibreOffice
//! (both ways). `spreadsheet-ods`, the Rust ODS writer, cannot be built:
//! every release needs `zip` 6, which is yanked.

use std::fs::File;
use std::io::{BufReader, BufWriter};

use sublime::converters::ods::{CSV_TO_ODS, ODS_TO_CSV};

use crate::common::run_ours;

pub fn run(mode: &str, args: &[String]) -> Result<(), String> {
    match (mode, args) {
        ("ours-ods-csv", [input, output]) => run_ours(&ODS_TO_CSV, input, output),
        ("ours-csv-ods", [input, output]) => run_ours(&CSV_TO_ODS, input, output),
        ("crates-ods-csv", [input, output]) => crates_ods_to_csv(input, output),
        _ => Err(
            "ods-csv modes: ours-ods-csv <in> <out> | ours-csv-ods <in> <out> | crates-ods-csv <in> <out>"
                .to_string(),
        ),
    }
}

fn crates_ods_to_csv(input: &str, output: &str) -> Result<(), String> {
    use calamine36::{Data, Reader};
    let mut workbook: calamine36::Ods<BufReader<File>> = calamine36::open_workbook(input)
        .map_err(|error: calamine36::OdsError| error.to_string())?;
    let range = workbook
        .worksheet_range_at(0)
        .ok_or("no sheet")?
        .map_err(|error| error.to_string())?;
    let out = File::create(output).map_err(|error| error.to_string())?;
    let mut writer = csv::Writer::from_writer(BufWriter::new(out));
    for row in range.rows() {
        let cells: Vec<String> = row
            .iter()
            .map(|cell| match cell {
                Data::Empty => String::new(),
                Data::String(text) => text.clone(),
                Data::Float(number) => number.to_string(),
                Data::Int(number) => number.to_string(),
                Data::Bool(flag) => flag.to_string().to_uppercase(),
                other => other.to_string(),
            })
            .collect();
        writer
            .write_record(&cells)
            .map_err(|error| error.to_string())?;
    }
    writer.flush().map_err(|error| error.to_string())
}
