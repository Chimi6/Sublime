//! Workbooks: every fixture's first sheet reads to the CSV beside it, a
//! sheet chosen by name or number reads to its own CSV, a missing sheet is
//! refused, and every CSV fixture survives CSV to workbook to CSV.

use std::fs;
use std::path::PathBuf;

use sublime::converter::{ConvertError, ConvertOptions, Converter, Input};
use sublime::converters::csv_to_xlsx::{CSV_TO_XLSX, TSV_TO_XLSX};
use sublime::converters::rows::CSV_TO_TSV;
use sublime::converters::xlsx_to_csv::{XLSX_TO_CSV, XLSX_TO_TSV};
use sublime::event::{Context, NullSink};

fn convert_with(
    converter: &dyn Converter,
    bytes: &[u8],
    sheet: Option<&str>,
) -> Result<Vec<u8>, ConvertError> {
    let options = ConvertOptions {
        strict: false,
        sheet: sheet.map(str::to_string),
    };
    let mut sink = NullSink;
    let mut context = Context::new(&mut sink, &options);
    let mut output = Vec::new();
    let mut cursor = std::io::Cursor::new(bytes);
    converter.convert(Input::Rewindable(&mut cursor), &mut output, &mut context)?;
    Ok(output)
}

fn convert(converter: &dyn Converter, bytes: &[u8]) -> Result<Vec<u8>, ConvertError> {
    convert_with(converter, bytes, None)
}

fn fixture_dir(sub: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(sub)
}

fn fixtures(sub: &str, extension: &str) -> Vec<(String, Vec<u8>)> {
    let mut paths: Vec<PathBuf> = fs::read_dir(fixture_dir(sub))
        .expect("fixture dir")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == extension))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no {extension} fixtures in {sub}");
    paths
        .into_iter()
        .map(|path| {
            let name = path
                .file_stem()
                .expect("stem")
                .to_string_lossy()
                .to_string();
            (name, fs::read(&path).expect("read fixture"))
        })
        .collect()
}

fn text(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).expect("utf-8")
}

#[test]
fn every_workbook_reads_its_first_sheet_to_the_csv_beside_it() {
    for (name, xlsx) in fixtures("xlsx", "xlsx") {
        let expected =
            fs::read_to_string(fixture_dir("xlsx").join(format!("{name}.csv"))).expect("csv");
        let csv =
            text(convert(&XLSX_TO_CSV, &xlsx).unwrap_or_else(|error| panic!("{name}: {error}")));
        assert_eq!(csv, expected, "{name}");
        let tsv = text(convert(&XLSX_TO_TSV, &xlsx).unwrap());
        let expected_tsv = text(convert(&CSV_TO_TSV, expected.as_bytes()).unwrap());
        assert_eq!(tsv, expected_tsv, "{name}");
    }
}

#[test]
fn sheets_are_chosen_by_name_or_number() {
    let xlsx = fs::read(fixture_dir("xlsx").join("types.xlsx")).unwrap();
    let expected = fs::read_to_string(fixture_dir("xlsx").join("types.sheet2.csv")).unwrap();
    assert_eq!(
        text(convert_with(&XLSX_TO_CSV, &xlsx, Some("Other sheet")).unwrap()),
        expected
    );
    assert_eq!(
        text(convert_with(&XLSX_TO_CSV, &xlsx, Some("2")).unwrap()),
        expected
    );
    let first = text(convert_with(&XLSX_TO_CSV, &xlsx, Some("1")).unwrap());
    assert!(first.starts_with("name,count"));
    match convert_with(&XLSX_TO_CSV, &xlsx, Some("Nope")) {
        Err(ConvertError::Unsupported(message)) => {
            assert!(
                message.contains("Data") && message.contains("Other sheet"),
                "{message}"
            )
        }
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

#[test]
fn csv_survives_the_trip_through_a_workbook() {
    for (name, csv) in fixtures("csv", "csv") {
        let xlsx = convert(&CSV_TO_XLSX, &csv).unwrap_or_else(|error| panic!("{name}: {error}"));
        let back =
            text(convert(&XLSX_TO_CSV, &xlsx).unwrap_or_else(|error| panic!("{name}: {error}")));
        // Our own CSV writer's form of the same rows.
        let normalized = text(convert(&CSV_TO_TSV, &csv).unwrap());
        let normalized = text(
            convert(
                &sublime::converters::rows::TSV_TO_CSV,
                normalized.as_bytes(),
            )
            .unwrap(),
        );
        assert_eq!(back, normalized, "{name}");
    }
    let tsv = b"a\tb\n1\t007\n";
    let xlsx = convert(&TSV_TO_XLSX, tsv).unwrap();
    assert_eq!(
        text(convert(&XLSX_TO_TSV, &xlsx).unwrap()),
        "a\tb\n1\t007\n"
    );
}

#[test]
fn the_written_workbook_names_its_sheet() {
    let xlsx = convert_with(&CSV_TO_XLSX, b"a,b\n1,2\n", Some("Results")).unwrap();
    assert_eq!(
        text(convert_with(&XLSX_TO_CSV, &xlsx, Some("Results")).unwrap()),
        "a,b\n1,2\n"
    );
    assert!(convert_with(&XLSX_TO_CSV, &xlsx, Some("Sheet1")).is_err());
}
