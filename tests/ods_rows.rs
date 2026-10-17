//! OpenDocument spreadsheets: every fixture sheet reads to LibreOffice's own
//! CSV export of it byte for byte (fixtures built by
//! `scripts/ods-check/fixtures.py`, exports beside them); sheets are parts
//! picked by name or number; written spreadsheets read back as the rows they
//! came from, typed only where that holds; number formats survive ODS ->
//! Excel -> ODS; and the other spreadsheet and document formats reach ODS.

use std::path::PathBuf;

use sublime::converter::{ConvertError, ConvertOptions, Converter, Input, MemoryParts};
use sublime::converters::numbers::NUMBERS_TO_CSV;
use sublime::converters::ods::{
    CSV_TO_ODS, JSON_TO_ODS, MARKDOWN_TO_ODS, NUMBERS_TO_ODS, ODS_TO_CSV, ODS_TO_JSON,
    ODS_TO_MARKDOWN, ODS_TO_NUMBERS, ODS_TO_XLSX, TSV_TO_ODS, XLSX_TO_ODS,
};
use sublime::converters::xlsx_to_csv::XLSX_TO_CSV;
use sublime::event::{Context, NullSink};
use sublime::io::zip::ZipArchive;

fn fixture(folder: &str, name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(folder)
        .join(name);
    std::fs::read(path).expect("fixture readable")
}

fn text(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).expect("utf-8")
}

fn convert_with(
    converter: &dyn Converter,
    bytes: &[u8],
    sheet: Option<&str>,
) -> Result<Vec<u8>, ConvertError> {
    let options = ConvertOptions {
        sheet: sheet.map(str::to_string),
        ..ConvertOptions::default()
    };
    let mut sink = NullSink;
    let mut context = Context::new(&mut sink, &options);
    let mut output = Vec::new();
    let mut source: &[u8] = bytes;
    converter.convert(Input::Stream(&mut source), &mut output, &mut context)?;
    Ok(output)
}

fn convert(converter: &dyn Converter, bytes: &[u8]) -> Vec<u8> {
    convert_with(converter, bytes, None).expect("converts")
}

fn parts(converter: &dyn Converter, bytes: &[u8]) -> Vec<(String, String)> {
    let options = ConvertOptions::default();
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
        .map(|(name, bytes)| (name, text(bytes)))
        .collect()
}

/// A part of a package, as text.
fn part(package: &[u8], name: &str) -> String {
    let archive = ZipArchive::parse(package).expect("a package");
    let entry = archive.find(name).expect("the part");
    let mut bytes = Vec::new();
    archive.read(entry, &mut bytes).expect("readable");
    text(bytes)
}

#[test]
fn cells_read_as_libreoffice_shows_them() {
    let types = fixture("ods", "types.ods");
    assert_eq!(
        text(convert(&ODS_TO_CSV, &types)),
        text(fixture("ods", "types.Types.csv"))
    );
}

#[test]
fn every_sheet_is_a_part_with_its_name() {
    let book = fixture("ods", "sheets.ods");
    let sheets = parts(&ODS_TO_CSV, &book);
    let names: Vec<&str> = sheets.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, vec!["Sales", "Costs & more", "Empty"]);
    assert_eq!(sheets[0].1, text(fixture("ods", "sheets.Sales.csv")));
    assert_eq!(sheets[1].1, text(fixture("ods", "sheets.Costs & more.csv")));
    // LibreOffice writes an empty sheet as one empty line; a sheet without
    // content is no rows here, as for a workbook.
    assert_eq!(text(fixture("ods", "sheets.Empty.csv")), "\n");
    assert_eq!(sheets[2].1, "");
    // `--sheet` picks one by name or number; a missing one lists them.
    assert_eq!(
        text(convert_with(&ODS_TO_CSV, &book, Some("2")).unwrap()),
        sheets[1].1
    );
    assert_eq!(
        text(convert_with(&ODS_TO_CSV, &book, Some("Sales")).unwrap()),
        sheets[0].1
    );
    match convert_with(&ODS_TO_CSV, &book, Some("Nope")) {
        Err(ConvertError::Unsupported(message)) => {
            assert!(
                message.contains("Sales") && message.contains("Empty"),
                "{message}"
            )
        }
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

#[test]
fn a_workbook_whole_reaches_json_and_markdown() {
    let book = fixture("ods", "sheets.ods");
    let json = text(convert(&ODS_TO_JSON, &book));
    assert!(json.contains("\"Costs & more\""), "{json}");
    assert!(json.contains("\"Cost\":\"12.50\""), "{json}");
    let markdown = text(convert(&ODS_TO_MARKDOWN, &book));
    assert!(
        markdown.contains("# Sales") && markdown.contains("| Paper | 12.50 |"),
        "{markdown}"
    );
}

#[test]
fn not_a_spreadsheet_is_refused() {
    assert!(matches!(
        convert_with(&ODS_TO_CSV, b"not a zip", None),
        Err(ConvertError::Malformed { .. })
    ));
}

/// Rows as our CSV writer writes them, every row padded to the widest (a
/// sheet is rectangular).
fn padded(csv: &[u8]) -> String {
    use sublime::io::csv::{CsvReader, CsvWriter, Record};
    let mut reader = CsvReader::with_delimiter(csv, b',');
    let mut record = Record::new();
    let mut rows: Vec<Vec<String>> = Vec::new();
    while reader.read_record(&mut record).expect("csv") {
        rows.push(record.fields().map(str::to_string).collect());
    }
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    let mut out = Vec::new();
    let mut writer = CsvWriter::with_delimiter(&mut out, b',');
    for row in &mut rows {
        row.resize(width, String::new());
        writer
            .write_record(row.iter().map(String::as_str))
            .expect("csv");
    }
    writer.flush().expect("csv");
    text(out)
}

#[test]
fn csv_survives_the_trip_through_a_spreadsheet() {
    let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/csv");
    for entry in std::fs::read_dir(folder).expect("fixtures") {
        let path = entry.expect("entry").path();
        let csv = std::fs::read(&path).expect("readable");
        let ods = convert(&CSV_TO_ODS, &csv);
        let back = text(convert(&ODS_TO_CSV, &ods));
        assert_eq!(back, padded(&csv), "{}", path.display());
    }
    let tsv = b"a\tb\n1\t007\n";
    let ods = convert(&TSV_TO_ODS, tsv);
    assert_eq!(text(convert(&ODS_TO_CSV, &ods)), "a,b\n1,007\n");
}

#[test]
fn cells_are_typed_only_when_they_read_back_the_same() {
    let rows = "number,1234.5\nleading zeros,007\ntrailing zero,1.50\nexponent,2.59E+005\nsmall,1E-05\ndate,2024-01-05\ndate and time,2024-01-05T13:45:00\ntime,13:45:00\nboolean,TRUE\nlower,true\nspaces,\"  two  spaces \"\nlines,\"a\nb\"\n";
    let ods = convert(&CSV_TO_ODS, rows.as_bytes());
    assert_eq!(text(convert(&ODS_TO_CSV, &ods)), padded(rows.as_bytes()));
    let content = part(&ods, "content.xml");
    for typed in [
        "office:value-type=\"float\" office:value=\"1234.5\"",
        "office:value-type=\"float\" office:value=\"2.59E+005\"",
        "office:date-value=\"2024-01-05\"",
        "office:date-value=\"2024-01-05T13:45:00\"",
        "office:time-value=\"PT13H45M00S\"",
        "office:boolean-value=\"true\"",
    ] {
        assert!(content.contains(typed), "{typed} in {content}");
    }
    // Text that would show differently as a value stays text.
    for kept in ["007", "1.50", "true"] {
        assert!(
            content.contains(&format!("<text:p>{kept}</text:p>")),
            "{kept}"
        );
    }
    assert!(
        content.contains("<text:p><text:s text:c=\"2\"/>two <text:s/>spaces<text:s/></text:p>")
    );
    let styles = part(&ods, "styles.xml");
    assert!(styles.contains("number:scientific-number"), "{styles}");
    // The package: the media type first and stored.
    assert_eq!(
        &ods[30..38],
        b"mimetype",
        "the first entry is the media type"
    );
}

#[test]
fn the_written_spreadsheet_names_its_sheet() {
    let options = Some("Results");
    let ods = convert_with(&CSV_TO_ODS, b"a,b\n1,2\n", options).unwrap();
    assert_eq!(
        text(convert_with(&ODS_TO_CSV, &ods, Some("Results")).unwrap()),
        "a,b\n1,2\n"
    );
}

#[test]
fn formats_and_merges_survive_excel() {
    let types = fixture("ods", "types.ods");
    let xlsx = convert(&ODS_TO_XLSX, &types);
    let styles = part(&xlsx, "xl/styles.xml");
    for code in [
        "#,##0.00",
        "0.00%",
        "0.00E+00",
        "yyyy\\-mm\\-dd",
        "[hh]:mm:ss",
    ] {
        let escaped = code.replace('\\', "");
        assert!(
            styles.contains(&format!("formatCode=\"{code}\""))
                || styles.contains(&format!("formatCode=\"{escaped}\"")),
            "{code} in {styles}"
        );
    }
    // Back to ODS, every cell shows as it did; the German-language format
    // is shown in English, the language this writer shows values in.
    let ods = convert(&XLSX_TO_ODS, &xlsx);
    // The Excel reader gives times to the second (06:04:59.99 is 06:05).
    let expected = text(fixture("ods", "types.Types.csv"))
        .replace("\"1.234,50\"", "\"1,234.50\"")
        .replace("06:04 PM", "06:05 PM");
    assert_eq!(text(convert(&ODS_TO_CSV, &ods)), expected);

    let sheets = convert(&ODS_TO_XLSX, &fixture("ods", "sheets.ods"));
    let first = part(&sheets, "xl/worksheets/sheet1.xml");
    assert!(first.contains("<mergeCell ref=\"A2:B2\"/>"), "{first}");
    // Excel's reader shows values as stored: the date column's dates.
    assert!(text(convert(&XLSX_TO_CSV, &sheets)).contains("North,hidden,2024-03-01"));
}

#[test]
fn documents_and_json_become_a_sheet_per_table() {
    let markdown = "# Sales\n\n| region | total |\n|---|---|\n| North | 10 |\n\n# Costs\n\n| item | cost |\n|---|---|\n| Rent | 5.5 |\n";
    let ods = convert(&MARKDOWN_TO_ODS, markdown.as_bytes());
    let sheets = parts(&ODS_TO_CSV, &ods);
    assert_eq!(
        sheets,
        vec![
            ("Sales".to_string(), "region,total\nNorth,10\n".to_string()),
            ("Costs".to_string(), "item,cost\nRent,5.5\n".to_string()),
        ]
    );
    let json = br#"{"People": [{"name": "Ann", "age": 41}], "Pets": [{"kind": "cat"}]}"#;
    let ods = convert(&JSON_TO_ODS, json);
    let sheets = parts(&ODS_TO_CSV, &ods);
    assert_eq!(
        sheets[0],
        ("People".to_string(), "name,age\nAnn,41\n".to_string())
    );
    assert_eq!(sheets[1], ("Pets".to_string(), "kind\ncat\n".to_string()));
}

#[test]
fn numbers_documents_and_ods_reach_each_other() {
    let numbers = fixture("numbers", "formats.numbers");
    let ods = convert(&NUMBERS_TO_ODS, &numbers);
    // Every table, named as Numbers' parts are; cells in formats Excel and
    // ODS share show as Numbers shows them (the first column, dates with
    // a weekday and seconds), and the rest by their value.
    let ours = parts(&ODS_TO_CSV, &ods);
    let theirs = parts(&NUMBERS_TO_CSV, &numbers);
    assert_eq!(ours.len(), theirs.len());
    for ((name, csv), (their_name, their_csv)) in ours.iter().zip(&theirs) {
        assert_eq!(name, their_name);
        assert_eq!(csv.lines().count(), their_csv.lines().count(), "{name}");
    }
    let dates = ours
        .iter()
        .position(|(name, _)| name == "Dates")
        .expect("Dates");
    let first = |csv: &str| -> Vec<String> {
        csv.lines()
            .map(|line| line.split("\",").next().unwrap_or("").to_string())
            .collect()
    };
    assert_eq!(first(&ours[dates].1), first(&theirs[dates].1));
    let back = convert(&ODS_TO_NUMBERS, &fixture("ods", "sheets.ods"));
    let tables = parts(&NUMBERS_TO_CSV, &back);
    assert_eq!(tables[0].0, "Sales");
    assert!(
        tables[0].1.starts_with("Region,Units,Day\n"),
        "{}",
        tables[0].1
    );
}
