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
        .rows(0, 0, &mut |cells| {
            rows.push(cells.iter().map(|cell| cell.text.clone()).collect());
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

/// Rows of CSV text, trailing empty cells and rows dropped.
fn csv_rows(text: &[u8]) -> Vec<Vec<String>> {
    let text = text.strip_prefix(b"\xef\xbb\xbf").unwrap_or(text);
    let mut reader = sublime::io::csv::CsvReader::new(text);
    let mut record = sublime::io::csv::Record::new();
    let mut rows = Vec::new();
    while reader.read_record(&mut record).expect("csv") {
        let mut row: Vec<String> = record.fields().map(str::to_string).collect();
        while row.last().is_some_and(String::is_empty) {
            row.pop();
        }
        rows.push(row);
    }
    while rows.last().is_some_and(Vec::is_empty) {
        rows.pop();
    }
    rows
}

/// Numbers' own CSV export of each document (`reference/`) is what our
/// tables read as: dates, fractions, custom number formats with their
/// padding, currencies, and percentages.
#[test]
fn cells_read_as_numbers_shows_them() {
    let reference =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/numbers/reference");
    let cases = [
        (
            "formats.numbers",
            vec![
                ("Dates", "formats/Dates-Dates.csv"),
                ("Numbers", "formats/Numbers-Table 1.csv"),
            ],
        ),
        ("currencies.numbers", vec![("Income", "currencies.csv")]),
        (
            "pivot.numbers",
            vec![
                ("Sheet 1 - Source", "pivot/Sheet 1-Source.csv"),
                ("Sheet 1 - Pivot", "pivot/Sheet 1-Pivot.csv"),
            ],
        ),
    ];
    for (document, tables) in cases {
        let ours = parts(&NUMBERS_TO_CSV, &fixture(document), None);
        for (part, expected) in tables {
            let (_, text) = ours.iter().find(|(name, _)| name == part).expect(part);
            let expected = std::fs::read(reference.join(expected)).expect("reference");
            assert_eq!(
                csv_rows(text.as_bytes()),
                csv_rows(&expected),
                "{document} {part}"
            );
        }
    }
}

fn write_with(converter: &dyn Converter, bytes: &[u8]) -> Vec<u8> {
    let options = options(None);
    let mut sink = NullSink;
    let mut context = Context::new(&mut sink, &options);
    let mut output = Vec::new();
    let mut source: &[u8] = bytes;
    converter
        .convert(Input::Stream(&mut source), &mut output, &mut context)
        .expect("writes");
    output
}

/// Rows written to Numbers read back as the same text: numbers, dates,
/// dates with times, and booleans typed, the rest text.
#[test]
fn rows_written_to_numbers_read_back_the_same() {
    let csv = "name,amount,when,active,note\nAlpha,12.5,2024-08-08,TRUE,first\nBeta,-3,2024-08-09T14:35:09,FALSE,\nGamma,1e3,,TRUE,0.000123\n";
    let numbers = write_with(
        &sublime::converters::to_numbers::CSV_TO_NUMBERS,
        csv.as_bytes(),
    );
    let tables = parts(&NUMBERS_TO_CSV, &numbers, None);
    assert_eq!(names(&tables), vec!["Sheet 1"]);
    assert_eq!(tables[0].1, csv);
}

#[test]
fn a_workbook_becomes_a_sheet_per_worksheet() {
    let workbook = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/xlsx/types.xlsx"),
    )
    .expect("fixture");
    let numbers = write_with(&sublime::converters::to_numbers::XLSX_TO_NUMBERS, &workbook);
    let ours = parts(&NUMBERS_TO_CSV, &numbers, None);
    let sheets = parts(&XLSX_TO_CSV, &workbook, None);
    assert_eq!(names(&ours), vec!["Data", "Other sheet"]);
    // The cells keep their Excel formats, shown as Numbers shows them: a
    // date as `m/d/yyyy`, a date and time under a date-only format, `0.00`,
    // and a time under a date and time format (on Excel's day zero).
    let mut expected = csv_rows(sheets[0].1.as_bytes());
    expected[1][2] = "1/5/2024".to_string();
    expected[1][3] = "2024-01-05".to_string();
    expected[1][4] = "3.50".to_string();
    expected[4][3] = "12/30/1899 18:00".to_string();
    assert_eq!(csv_rows(ours[0].1.as_bytes()), expected);
    // A Numbers table is a full grid: a short row reads back padded.
    assert_eq!(
        csv_rows(ours[1].1.as_bytes()),
        csv_rows(sheets[1].1.as_bytes())
    );
}

/// The formats' cells as Numbers shows them (checked in Numbers itself).
const EXPECTED_SHOWN: [&str; 10] = [
    "£1,234.50",
    "25.6%",
    "(4,321)",
    "5 Jan 2024",
    "6:00 PM",
    "1.23E+05",
    "2 1/3",
    "€9,876.50",
    "Friday, January 5, 2024 13:30",
    "1/5/2024",
];

/// An Excel cell's number format becomes the Numbers format that shows it
/// the same, where Numbers has one, and merged ranges stay merged.
#[test]
fn a_workbooks_formats_and_merges_carry_over() {
    let workbook = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/xlsx/formats.xlsx"),
    )
    .expect("fixture");
    let numbers = write_with(&sublime::converters::to_numbers::XLSX_TO_NUMBERS, &workbook);
    let ours = parts(&NUMBERS_TO_CSV, &numbers, None);
    let shown: Vec<String> = csv_rows(ours[0].1.as_bytes())
        .iter()
        .skip(1)
        .take(10)
        .map(|row| row[1].clone())
        .collect();
    assert_eq!(shown, EXPECTED_SHOWN);
    let package = Package::read_scope(&numbers, Scope::Everything).expect("package");
    let mut merges = WorkbookReader::new(&package).merges(0, 0);
    merges.sort_unstable();
    assert_eq!(merges, vec![(0, 0, 1, 3), (11, 0, 2, 1), (11, 1, 2, 3)]);
}

#[test]
fn a_documents_tables_become_tables_of_one_sheet() {
    let markdown = "# Sales\n\n| region | total |\n|---|---|\n| North | 10 |\n\n## Costs\n\n| item | cost |\n|---|---|\n| Rent | 5.5 |\n";
    let numbers = write_with(
        &sublime::converters::to_numbers::MARKDOWN_TO_NUMBERS,
        markdown.as_bytes(),
    );
    let package = Package::read_scope(&numbers, Scope::Workbook).expect("package");
    let workbook = WorkbookReader::new(&package);
    let layout: Vec<(&str, Vec<&str>)> = workbook
        .sheets()
        .iter()
        .map(|sheet| {
            (
                sheet.name.as_str(),
                sheet
                    .tables
                    .iter()
                    .map(|table| table.name.as_str())
                    .collect(),
            )
        })
        .collect();
    assert_eq!(layout, vec![("Sheet 1", vec!["Sales", "Costs"])]);
}

/// A table past 256 rows is written in tiles of 256, as Numbers stores
/// one, and reads back whole.
#[test]
fn a_long_table_reads_back_whole() {
    let mut csv = String::from("id,label\n");
    for row in 0..600 {
        csv.push_str(&format!("{row},row {row}\n"));
    }
    let numbers = write_with(
        &sublime::converters::to_numbers::CSV_TO_NUMBERS,
        csv.as_bytes(),
    );
    let tables = parts(&NUMBERS_TO_CSV, &numbers, None);
    assert_eq!(tables[0].1, csv);
}

/// A Numbers table's merged cells (its merge owner's ranges) stay merged
/// in Excel.
#[test]
fn merged_cells_carry_into_excel() {
    let workbook = convert(&NUMBERS_TO_XLSX, &fixture("formats.numbers"));
    let workbook = sublime::io::xlsx::Workbook::open(&workbook).expect("workbook");
    let sheet = workbook.select(Some("Numbers")).expect("sheet");
    let mut merges = workbook
        .read_sheet(
            sheet,
            &mut |_, _| -> Result<(), sublime::io::xlsx::XlsxError> { Ok(()) },
        )
        .expect("reads");
    merges.sort_unstable();
    // A2:A37, A38:A45, A50:A69, A70:A85, A86:A89, A90:A102, A103:A105,
    // A106:A113 (numbers-parser's ranges).
    let starts: Vec<(usize, usize)> = merges
        .iter()
        .map(|&(row, _, rows, _)| (row, rows))
        .collect();
    assert_eq!(
        starts,
        vec![
            (1, 36),
            (37, 8),
            (49, 20),
            (69, 16),
            (85, 4),
            (89, 13),
            (102, 3),
            (105, 8)
        ]
    );
}
