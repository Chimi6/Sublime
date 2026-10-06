//! Workbooks whole: a workbook's sheets (and a document's tables) become a
//! file each for formats that hold one table (CSV, TSV, JSON Lines), and
//! travel together into formats that hold several (JSON as an object of
//! sheets, a document as a table per sheet, a workbook as a sheet per
//! table).

use std::path::PathBuf;

use sublime::converter::{ConvertOptions, Converter, Input, MemoryParts, part_file_stem};
use sublime::converters::csv_to_json::CSV_TO_JSON;
use sublime::converters::json_to_csv::{JSON_TO_CSV, JSON_TO_XLSX};
use sublime::converters::rows_document::{
    JSON_TO_MARKDOWN, MARKDOWN_TABLES_TO_JSON, MARKDOWN_TO_CSV, MARKDOWN_TO_XLSX,
};
use sublime::converters::xlsx_to_csv::{XLSX_TO_CSV, XLSX_TO_JSON, XLSX_TO_MARKDOWN};
use sublime::event::{Context, NullSink};
use sublime::planner::{self, PlanOptions};
use sublime::registry;

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/xlsx")
        .join(name);
    std::fs::read(path).expect("fixture readable")
}

fn options(sheet: Option<&str>) -> ConvertOptions {
    ConvertOptions {
        sheet: sheet.map(str::to_string),
        ..ConvertOptions::default()
    }
}

fn convert_with(converter: &dyn Converter, bytes: &[u8], sheet: Option<&str>) -> Vec<u8> {
    let options = options(sheet);
    let mut sink = NullSink;
    let mut context = Context::new(&mut sink, &options);
    let mut output = Vec::new();
    let mut source: &[u8] = bytes;
    converter
        .convert(Input::Stream(&mut source), &mut output, &mut context)
        .expect("converts");
    output
}

fn convert(converter: &dyn Converter, bytes: &[u8]) -> Vec<u8> {
    convert_with(converter, bytes, None)
}

fn parts(converter: &dyn Converter, bytes: &[u8], sheet: Option<&str>) -> Vec<(String, String)> {
    let options = options(sheet);
    let mut sink = NullSink;
    let mut context = Context::new(&mut sink, &options);
    let mut parts = MemoryParts::default();
    let mut source: &[u8] = bytes;
    converter
        .convert_parts(Input::Stream(&mut source), &mut parts, &mut context)
        .expect("converts");
    parts
        .parts
        .into_iter()
        .map(|(name, bytes)| (name, String::from_utf8(bytes).expect("utf-8")))
        .collect()
}

const DOCUMENT: &str = "# Sales\n\n| region | total |\n|---|---|\n| North | 10 |\n| South | 20 |\n\n## Costs\n\nSome text.\n\n| item | cost |\n|---|---|\n| Rent | 5.5 |\n\n| loose | table |\n|---|---|\n| a | b |\n";

#[test]
fn every_sheet_is_a_part_with_its_name() {
    let workbook = fixture("types.xlsx");
    let sheets = parts(&XLSX_TO_CSV, &workbook, None);
    let names: Vec<&str> = sheets.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, vec!["Data", "Other sheet"]);
    let first = String::from_utf8(fixture("types.csv")).expect("utf-8");
    let second = String::from_utf8(fixture("types.sheet2.csv")).expect("utf-8");
    assert_eq!(sheets[0].1, first);
    assert_eq!(sheets[1].1, second);
    // `--sheet` picks one part.
    let chosen = parts(&XLSX_TO_CSV, &workbook, Some("2"));
    assert_eq!(chosen, vec![("Other sheet".to_string(), second)]);
}

#[test]
fn a_plan_into_a_one_table_format_splits_and_runs_the_rest_per_sheet() {
    let known = registry::all_formats();
    let from = sublime::format::find_by_id("xlsx", &known).expect("xlsx");
    let to = sublime::format::find_by_id("jsonl", &known).expect("jsonl");
    let plan = planner::plan(
        registry::all_converters(),
        from,
        to,
        &PlanOptions {
            strict: false,
            via: None,
        },
    )
    .expect("a path");
    let options = ConvertOptions::default();
    let mut sink = NullSink;
    let mut context = Context::new(&mut sink, &options);
    let workbook = fixture("types.xlsx");
    let mut source: &[u8] = &workbook;
    let mut parts = MemoryParts::default();
    planner::execute_parts(&plan, Input::Stream(&mut source), &mut parts, &mut context)
        .expect("runs");
    assert_eq!(parts.parts.len(), 2);
    assert_eq!(parts.parts[1].0, "Other sheet");
    assert_eq!(
        String::from_utf8(parts.parts[1].1.clone()).expect("utf-8"),
        "{\"second\":\"3\"}\n"
    );
}

#[test]
fn a_workbook_is_one_json_object_of_sheets() {
    let workbook = fixture("types.xlsx");
    let json = String::from_utf8(convert(&XLSX_TO_JSON, &workbook)).expect("utf-8");
    assert!(json.starts_with("{\"Data\":[{\"name\":"), "{json}");
    assert!(
        json.ends_with(",\"Other sheet\":[{\"second\":\"3\"}]}"),
        "{json}"
    );
    // One sheet keeps the plain array `csv-to-json` makes.
    let chosen = convert_with(&XLSX_TO_JSON, &workbook, Some("Other sheet"));
    assert_eq!(chosen, b"[{\"second\":\"3\"}]");
    let single = fixture("date1904.xlsx");
    let csv = convert(&XLSX_TO_CSV, &single);
    assert_eq!(convert(&XLSX_TO_JSON, &single), convert(&CSV_TO_JSON, &csv));
}

#[test]
fn a_workbook_survives_json_and_back() {
    let workbook = fixture("types.xlsx");
    let json = convert(&XLSX_TO_JSON, &workbook);
    let rebuilt = convert(&JSON_TO_XLSX, &json);
    assert_eq!(convert(&XLSX_TO_JSON, &rebuilt), json);
    // Its sheets split back into a CSV each.
    let sheets = parts(&JSON_TO_CSV, &json, None);
    assert_eq!(sheets.len(), 2);
    assert_eq!(sheets[0].0, "Data");
}

#[test]
fn a_document_is_a_sheet_per_table() {
    let workbook = convert(&MARKDOWN_TO_XLSX, DOCUMENT.as_bytes());
    let sheets = parts(&XLSX_TO_CSV, &workbook, None);
    assert_eq!(
        sheets,
        vec![
            (
                "Sales".to_string(),
                "region,total\nNorth,10\nSouth,20\n".to_string()
            ),
            ("Costs".to_string(), "item,cost\nRent,5.5\n".to_string()),
            ("Table 3".to_string(), "loose,table\na,b\n".to_string()),
        ]
    );
    // The same tables come out as a CSV each.
    assert_eq!(parts(&MARKDOWN_TO_CSV, DOCUMENT.as_bytes(), None), sheets);
}

#[test]
fn tables_travel_between_documents_and_json() {
    let json =
        String::from_utf8(convert(&MARKDOWN_TABLES_TO_JSON, DOCUMENT.as_bytes())).expect("utf-8");
    assert_eq!(
        json,
        "{\"Sales\":[{\"region\":\"North\",\"total\":\"10\"},{\"region\":\"South\",\"total\":\"20\"}],\"Costs\":[{\"item\":\"Rent\",\"cost\":\"5.5\"}],\"Table 3\":[{\"loose\":\"a\",\"table\":\"b\"}]}"
    );
    let markdown = String::from_utf8(convert(&JSON_TO_MARKDOWN, json.as_bytes())).expect("utf-8");
    assert!(
        markdown.starts_with("# Sales\n\n| region | total |"),
        "{markdown}"
    );
    assert!(
        markdown.contains("\n# Costs\n\n| item | cost |"),
        "{markdown}"
    );
}

#[test]
fn a_workbook_is_a_document_with_a_table_per_sheet() {
    let markdown =
        String::from_utf8(convert(&XLSX_TO_MARKDOWN, &fixture("types.xlsx"))).expect("utf-8");
    assert!(markdown.starts_with("# Data\n\n| name |"), "{markdown}");
    assert!(
        markdown.contains("\n# Other sheet\n\n| second |"),
        "{markdown}"
    );
    // One sheet is the table alone.
    let single =
        String::from_utf8(convert(&XLSX_TO_MARKDOWN, &fixture("date1904.xlsx"))).expect("utf-8");
    assert!(single.starts_with("| when |"), "{single}");
}

#[test]
fn part_file_names_are_safe_and_unique() {
    assert_eq!(part_file_stem("Q1/Q2: sales", 0, &[]), "Q1_Q2_ sales");
    assert_eq!(part_file_stem("", 2, &[]), "Part 3");
    let taken = vec!["Sales".to_string()];
    assert_eq!(part_file_stem("sales", 1, &taken), "sales (2)");
}
