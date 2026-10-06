//! Numbers documents as workbooks: sheets of tables, each table a part.
//! Fixtures are from numbers-parser's test data (`tests/fixtures/numbers/
//! README.md`), checked against what numbers-parser reads from them.

use std::path::PathBuf;

use sublime::converter::{ConvertOptions, Converter, Input, MemoryParts};
use sublime::converters::numbers::{
    NUMBERS_TO_CSV, NUMBERS_TO_JSON, NUMBERS_TO_MARKDOWN, NUMBERS_TO_XLSX,
};
use sublime::converters::xlsx_to_csv::XLSX_TO_CSV;
use sublime::event::{Context, NullSink};
use sublime::io::pages::{Package, Scope, WorkbookReader};

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/numbers")
        .join(name);
    std::fs::read(path).expect("fixture readable")
}

fn options(sheet: Option<&str>) -> ConvertOptions {
    ConvertOptions {
        sheet: sheet.map(str::to_string),
        ..ConvertOptions::default()
    }
}

fn convert(converter: &dyn Converter, bytes: &[u8]) -> Vec<u8> {
    let options = options(None);
    let mut sink = NullSink;
    let mut context = Context::new(&mut sink, &options);
    let mut output = Vec::new();
    let mut source: &[u8] = bytes;
    converter
        .convert(Input::Stream(&mut source), &mut output, &mut context)
        .expect("converts");
    output
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

/// A table's name, rows, and columns.
type TableShape<'a> = (&'a str, usize, usize);

fn names(parts: &[(String, String)]) -> Vec<&str> {
    parts.iter().map(|(name, _)| name.as_str()).collect()
}

#[test]
fn sheets_hold_their_tables_in_order() {
    let package =
        Package::read_scope(&fixture("sheets.numbers"), Scope::Workbook).expect("package");
    let mut workbook = WorkbookReader::new(&package);
    let layout: Vec<(&str, Vec<TableShape<'_>>)> = workbook
        .sheets()
        .iter()
        .map(|sheet| {
            (
                sheet.name.as_str(),
                sheet
                    .tables
                    .iter()
                    .map(|table| (table.name.as_str(), table.rows, table.columns))
                    .collect(),
            )
        })
        .collect();
    assert_eq!(
        layout,
        vec![
            (
                "ZZZ_Sheet_1",
                vec![("ZZZ_Table_1", 5, 3), ("ZZZ_Table_2", 4, 4)]
            ),
            ("ZZZ_Sheet_2", vec![("XXX_Table_1", 4, 6)]),
        ]
    );
    let mut rows: Vec<Vec<String>> = Vec::new();
    workbook
        .rows(0, 0, |cells| -> Result<(), ()> {
            rows.push(cells.to_vec());
            Ok(())
        })
        .expect("rows");
    assert_eq!(rows.len(), 5);
    assert_eq!(rows[0], vec!["", "YYY_COL_1", "YYY_COL_2"]);
    assert_eq!(rows[4], vec!["YYY_ROW_4", "YYY_4_1", "YYY_4_2"]);
}

#[test]
fn every_table_is_a_part_named_after_its_sheet() {
    let document = fixture("sheets.numbers");
    let tables = parts(&NUMBERS_TO_CSV, &document, None);
    assert_eq!(
        names(&tables),
        vec![
            "ZZZ_Sheet_1 - ZZZ_Table_1",
            "ZZZ_Sheet_1 - ZZZ_Table_2",
            "ZZZ_Sheet_2"
        ]
    );
    assert!(
        tables[2]
            .1
            .starts_with(",XXX_COL_1,XXX_COL_2,XXX_COL_3,XXX_COL_4,XXX_COL_5\nXXX_ROW_1,XXX_1_1,"),
        "{}",
        tables[2].1
    );
    // Empty cells keep their places.
    assert!(
        tables[2]
            .1
            .contains("\nXXX_ROW_2,XXX_2_1,XXX_2_2,,XXX_2_4,XXX_2_5\n")
    );
    // `--sheet` picks a sheet's tables, a table, or a table by number.
    assert_eq!(
        names(&parts(&NUMBERS_TO_CSV, &document, Some("ZZZ_Sheet_1"))),
        vec!["ZZZ_Sheet_1 - ZZZ_Table_1", "ZZZ_Sheet_1 - ZZZ_Table_2"]
    );
    assert_eq!(
        names(&parts(
            &NUMBERS_TO_CSV,
            &document,
            Some("ZZZ_Sheet_1 - ZZZ_Table_2")
        )),
        vec!["ZZZ_Sheet_1 - ZZZ_Table_2"]
    );
    assert_eq!(
        parts(&NUMBERS_TO_CSV, &document, Some("3")),
        vec![tables[2].clone()]
    );
}

#[test]
fn a_workbook_keeps_a_sheet_per_table() {
    let document = fixture("sheets.numbers");
    let workbook = convert(&NUMBERS_TO_XLSX, &document);
    assert_eq!(
        parts(&XLSX_TO_CSV, &workbook, None),
        parts(&NUMBERS_TO_CSV, &document, None)
    );
}

#[test]
fn json_and_markdown_hold_every_table() {
    let document = fixture("sheets.numbers");
    let json = String::from_utf8(convert(&NUMBERS_TO_JSON, &document)).expect("utf-8");
    assert!(
        json.starts_with("{\"ZZZ_Sheet_1 - ZZZ_Table_1\":[{\"\":\"YYY_ROW_1\","),
        "{json}"
    );
    assert!(json.contains(",\"ZZZ_Sheet_2\":[{"), "{json}");
    let markdown = String::from_utf8(convert(&NUMBERS_TO_MARKDOWN, &document)).expect("utf-8");
    assert!(
        markdown.starts_with("# ZZZ\\_Sheet\\_1 - ZZZ\\_Table\\_1\n\n|"),
        "{markdown}"
    );
    assert!(markdown.contains("\n# ZZZ\\_Sheet\\_2\n\n|"), "{markdown}");
}

#[test]
fn a_zipped_package_folder_reads() {
    let tables = parts(&NUMBERS_TO_CSV, &fixture("bundle.numbers"), None);
    assert_eq!(names(&tables), vec!["Sheet 1"]);
    assert!(
        tables[0].1.starts_with(
            "First name,Last Name,Email,Registration Number,Vote Weight,,\nTest,Testakis,abc@gmail.com,1,1,,\n"
        ),
        "{}",
        tables[0].1
    );
}

#[test]
fn an_older_document_without_object_references_reads() {
    let tables = parts(&NUMBERS_TO_CSV, &fixture("old.numbers"), None);
    assert_eq!(tables, vec![("Sheet 1".to_string(), "123\n".to_string())]);
}
