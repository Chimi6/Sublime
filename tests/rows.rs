//! TSV and JSON Lines: every TSV fixture reads to the JSON beside it and
//! comes back through the TSV writer to the same JSON; CSV and TSV swap
//! byte for byte; every JSON Lines fixture reads to the JSON beside it and
//! back; the row converters give JSON Lines the same rows as JSON; every
//! invalid JSON Lines file is refused at its line.

use std::fs;
use std::path::PathBuf;

use sublime::converter::{ConvertError, ConvertOptions, Converter, Input};
use sublime::converters::csv_to_json::{CSV_TO_JSON, CSV_TO_JSONL, TSV_TO_JSON, TSV_TO_JSONL};
use sublime::converters::json_to_csv::{JSON_TO_CSV, JSON_TO_TSV, JSONL_TO_CSV, JSONL_TO_TSV};
use sublime::converters::rows::{CSV_TO_TSV, JsonToJsonl, JsonlToJson, TSV_TO_CSV};
use sublime::event::{Context, NullSink};

fn convert(converter: &dyn Converter, bytes: &[u8]) -> Result<String, ConvertError> {
    let options = ConvertOptions::default();
    let mut sink = NullSink;
    let mut context = Context::new(&mut sink, &options);
    let mut output = Vec::new();
    let mut cursor = std::io::Cursor::new(bytes);
    converter.convert(Input::Rewindable(&mut cursor), &mut output, &mut context)?;
    Ok(String::from_utf8(output).expect("utf-8 output"))
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

fn beside(sub: &str, name: &str, extension: &str) -> Vec<u8> {
    fs::read(fixture_dir(sub).join(format!("{name}.{extension}"))).expect("fixture beside")
}

#[test]
fn tsv_reads_to_the_json_beside_it_and_back() {
    for (name, tsv) in fixtures("tsv", "tsv") {
        let expected = String::from_utf8(beside("tsv", &name, "json")).unwrap();
        let json = convert(&TSV_TO_JSON, &tsv).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(json, expected, "{name}");
        let back = convert(&JSON_TO_TSV, json.as_bytes()).unwrap();
        let again = convert(&TSV_TO_JSON, back.as_bytes()).unwrap();
        assert_eq!(again, expected, "{name}\n{back}");
    }
}

#[test]
fn csv_and_tsv_swap_without_loss() {
    for (name, csv) in fixtures("csv", "csv") {
        let tsv = convert(&CSV_TO_TSV, &csv).unwrap_or_else(|error| panic!("{name}: {error}"));
        let csv_again = convert(&TSV_TO_CSV, tsv.as_bytes()).unwrap();
        let direct = convert(&CSV_TO_JSON, &csv).unwrap();
        let via_tsv = convert(&TSV_TO_JSON, tsv.as_bytes()).unwrap();
        let round = convert(&CSV_TO_JSON, csv_again.as_bytes()).unwrap();
        assert_eq!(via_tsv, direct, "{name}");
        assert_eq!(round, direct, "{name}");
    }
}

#[test]
fn jsonl_reads_to_the_json_beside_it_and_back() {
    for (name, jsonl) in fixtures("jsonl", "jsonl") {
        let expected = String::from_utf8(beside("jsonl", &name, "json")).unwrap();
        let json = convert(&JsonlToJson, &jsonl).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(json, expected, "{name}");
        let lines = convert(&JsonToJsonl, json.as_bytes()).unwrap();
        let again = convert(&JsonlToJson, lines.as_bytes()).unwrap();
        assert_eq!(again, expected, "{name}\n{lines}");
    }
}

#[test]
fn json_lines_and_json_give_the_same_rows() {
    let jsonl = beside("jsonl", "rows", "jsonl");
    let json = beside("jsonl", "rows", "json");
    let expected_csv = String::from_utf8(beside("jsonl", "rows", "csv")).unwrap();
    assert_eq!(convert(&JSONL_TO_CSV, &jsonl).unwrap(), expected_csv);
    assert_eq!(convert(&JSON_TO_CSV, &json).unwrap(), expected_csv);
    let expected_tsv = expected_csv.replace(',', "\t");
    assert_eq!(convert(&JSONL_TO_TSV, &jsonl).unwrap(), expected_tsv);
    let csv = expected_csv.as_bytes();
    let from_csv_lines = convert(&CSV_TO_JSONL, csv).unwrap();
    let from_csv_array = convert(&CSV_TO_JSON, csv).unwrap();
    assert_eq!(
        convert(&JsonlToJson, from_csv_lines.as_bytes()).unwrap(),
        from_csv_array
    );
    let tsv = expected_tsv.as_bytes();
    assert_eq!(convert(&TSV_TO_JSONL, tsv).unwrap(), from_csv_lines);
}

#[test]
fn invalid_json_lines_are_refused_at_their_line() {
    for (name, jsonl) in fixtures("jsonl/invalid", "jsonl") {
        match convert(&JsonlToJson, &jsonl) {
            Err(ConvertError::Malformed { location, .. }) => {
                assert!(location.line >= 2, "{name}: line {}", location.line)
            }
            Err(other) => panic!("{name}: expected Malformed, got {other}"),
            Ok(json) => panic!("{name}: accepted: {json}"),
        }
    }
}
