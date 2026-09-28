//! The bridge between rows and documents: every CSV fixture becomes a
//! Markdown table and comes back as the same rows (line breaks in cells
//! aside), a document's first table comes out as rows, pipes survive both
//! ways, and a workbook reaches HTML through the command line.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use sublime::converter::{ConvertError, ConvertOptions, Converter, Input};
use sublime::converters::csv_to_json::CSV_TO_JSON;
use sublime::converters::json_to_csv::JSON_TO_CSV;
use sublime::converters::rows::{CSV_TO_TSV, TSV_TO_CSV};
use sublime::converters::rows_document::{
    CSV_TO_MARKDOWN, MARKDOWN_TO_CSV, MARKDOWN_TO_TSV, TSV_TO_MARKDOWN,
};
use sublime::event::{Context, NullSink};

fn convert(converter: &dyn Converter, bytes: &[u8]) -> Result<String, ConvertError> {
    let options = ConvertOptions::default();
    let mut sink = NullSink;
    let mut context = Context::new(&mut sink, &options);
    let mut output = Vec::new();
    let mut cursor = std::io::Cursor::new(bytes);
    converter.convert(Input::Rewindable(&mut cursor), &mut output, &mut context)?;
    Ok(String::from_utf8(output).expect("utf-8"))
}

fn fixture_dir(sub: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(sub)
}

#[test]
fn every_csv_fixture_survives_the_trip_through_markdown() {
    let mut paths: Vec<PathBuf> = fs::read_dir(fixture_dir("csv"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "csv"))
        .collect();
    paths.sort();
    for path in paths {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let csv = fs::read(&path).unwrap();
        let markdown =
            convert(&CSV_TO_MARKDOWN, &csv).unwrap_or_else(|error| panic!("{name}: {error}"));
        let back = convert(&MARKDOWN_TO_CSV, markdown.as_bytes()).unwrap();
        // The same rows in our writer's form, with line breaks inside cells
        // folded to spaces the way the table had to.
        let json = convert(&CSV_TO_JSON, &csv).unwrap().replace("\\n", " ");
        let expected = if json == "[]" {
            // A header alone has no JSON rows; the table keeps it.
            let tsv = convert(&CSV_TO_TSV, &csv).unwrap();
            convert(&TSV_TO_CSV, tsv.as_bytes()).unwrap()
        } else {
            convert(&JSON_TO_CSV, json.as_bytes()).unwrap()
        };
        assert_eq!(back, expected, "{name}\n{markdown}");
    }
}

#[test]
fn a_documents_first_table_comes_out_as_rows() {
    let markdown = fs::read(fixture_dir("markdown").join("report.md")).unwrap();
    let expected = fs::read_to_string(fixture_dir("markdown").join("report.csv")).unwrap();
    assert_eq!(convert(&MARKDOWN_TO_CSV, &markdown).unwrap(), expected);
    assert_eq!(
        convert(&MARKDOWN_TO_TSV, &markdown).unwrap(),
        expected.replace(',', "\t")
    );
}

#[test]
fn pipes_and_tabs_survive_both_ways() {
    let csv = b"k,v\na|b,c\td\n";
    let markdown = convert(&CSV_TO_MARKDOWN, csv).unwrap();
    assert!(markdown.contains("\\|"), "{markdown}");
    assert_eq!(
        convert(&MARKDOWN_TO_CSV, markdown.as_bytes()).unwrap(),
        "k,v\na|b,c\td\n"
    );
    let tsv = b"k\tv\na|b\tc,d\n";
    let markdown = convert(&TSV_TO_MARKDOWN, tsv).unwrap();
    assert_eq!(
        convert(&MARKDOWN_TO_TSV, markdown.as_bytes()).unwrap(),
        "k\tv\na|b\tc,d\n"
    );
}

#[test]
fn a_document_without_a_table_gives_empty_rows() {
    assert_eq!(
        convert(&MARKDOWN_TO_CSV, b"# Just a heading\n\ntext\n").unwrap(),
        ""
    );
}

#[test]
fn a_workbook_reaches_html_through_the_planner() {
    let output = Command::new(env!("CARGO_BIN_EXE_sublime"))
        .args(["-q", "convert", "--to", "html"])
        .arg(fixture_dir("xlsx").join("types.xlsx"))
        .output()
        .expect("run sublime");
    let html = String::from_utf8_lossy(&output.stdout);
    assert!(html.contains("<table>"), "{html}");
    assert!(html.contains("<th>name</th>"), "{html}");
    assert!(html.contains("2024-01-05"), "{html}");
}
