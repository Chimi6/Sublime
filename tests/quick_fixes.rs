//! Regression tests for small silent-wrong-output bugs found by the
//! November 2026 audit. Each runs the built binary on a tiny input.

use std::io::Write;
use std::process::{Command, Output, Stdio};

fn convert(from: &str, to: &str, input: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_sublime"))
        .args(["convert", "-", "--from", from, "--to", to])
        .env_remove("SUBLIME_LOG")
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("binary spawns");
    {
        let mut stdin = child.stdin.take().expect("stdin");
        let _ = stdin.write_all(input);
    }
    child.wait_with_output().expect("wait")
}

fn text(from: &str, to: &str, input: &str) -> String {
    let out = convert(from, to, input.as_bytes());
    assert!(
        out.status.code().is_some(),
        "{from}->{to} crashed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn bare_email_after_non_ascii_does_not_panic() {
    let out = text(
        "markdown",
        "html",
        "ééé a@b.co\n\n日本語a@b.co\n\néééa@b.co\n",
    );
    assert!(out.contains("mailto:"), "{out}");
}

#[test]
fn html_comment_does_not_leak() {
    let out = text("html", "markdown", "<p>a</p><!-- hidden --><p>b</p>");
    assert!(!out.contains("-->") && !out.contains("hidden"), "{out}");
    let out = text("html", "markdown", "<p>a<!--x-->tail</p>");
    assert!(out.contains("atail") && !out.contains("-->"), "{out}");
}

fn fixture(name: &str) -> Vec<u8> {
    let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests/fixtures/audit");
    path.push(name);
    std::fs::read(path).expect("fixture readable")
}

#[test]
fn nested_table_text_flows_into_the_outer_cell() {
    let docx = fixture("nested-table.docx");
    for to in ["text", "markdown", "html"] {
        let out = String::from_utf8_lossy(&convert("docx", to, &docx).stdout).into_owned();
        for word in ["Inner 1", "Inner 2", "Inner 3", "Inner 4", "Outer B end"] {
            assert!(out.contains(word), "{to}: missing {word}: {out}");
        }
    }
}

#[test]
fn word_symbol_characters_follow_their_font() {
    let out = text_of("docx", "text", &fixture("sym.docx"));
    assert!(out.contains("alpha=α"), "{out}");
    assert!(out.contains("check=✓"), "{out}");
    assert!(out.contains("lessequal=≤"), "{out}");
    assert!(out.contains("smile=☺"), "{out}");
}

fn text_of(from: &str, to: &str, input: &[u8]) -> String {
    String::from_utf8_lossy(&convert(from, to, input).stdout).into_owned()
}

#[test]
fn ods_number_in_text_format_cell_keeps_its_value() {
    let out = text_of("ods", "csv", &fixture("textfmt.ods"));
    assert!(out.contains("123,num with @ format"), "{out}");
    assert!(out.contains("45.5,x"), "{out}");
}

fn zip_part(bytes: &[u8], name: &str) -> String {
    let zip = sublime::io::zip::ZipArchive::parse(bytes).expect("a ZIP");
    let entry = zip.find(name).expect("part present");
    let mut out = Vec::new();
    zip.read(entry, &mut out).expect("part reads");
    String::from_utf8(out).expect("UTF-8 part")
}

#[test]
fn xlsx_text_cells_stay_text_in_ods() {
    let ods = convert("xlsx", "ods", &fixture("txt.xlsx")).stdout;
    let content = zip_part(&ods, "content.xml");
    assert!(
        content.contains("office:value-type=\"string\""),
        "{content}"
    );
    assert!(
        !content.contains("office:value-type=\"float\""),
        "{content}"
    );
}

#[test]
fn xlsx_prefixed_worksheet_reads() {
    let out = text_of("xlsx", "csv", &fixture("prefixed.xlsx"));
    assert!(out.contains("inline,2,TRUE"), "{out}");
}

#[test]
fn xlsx_midnight_rounding_durations_and_escapes() {
    let out = text_of("xlsx", "csv", &fixture("date_round.xlsx"));
    assert!(out.contains("2024-08-09"), "{out}");
    assert!(out.contains("30:05:00"), "{out}");
    assert!(
        out.contains("23:59:59") && !out.contains("24:00:00"),
        "{out}"
    );
    let out = text_of("xlsx", "csv", &fixture("hand.xlsx"));
    assert!(out.contains("2400:00:01"), "{out}");
    assert!(
        out.contains("\"\rescA_x\"") || out.contains("escA_x"),
        "{out}"
    );
    assert!(!out.contains("_x000D_"), "{out}");
}

#[test]
fn xlsx_durations_survive_to_ods() {
    let ods = convert("xlsx", "ods", &fixture("date_round.xlsx")).stdout;
    let content = zip_part(&ods, "content.xml");
    assert!(
        content.contains("office:time-value=\"PT30H05M00S\""),
        "{content}"
    );
    assert!(
        content.contains("office:date-value=\"2024-08-09\""),
        "{content}"
    );
}

#[test]
fn control_characters_round_trip_through_xlsx_as_escapes() {
    let csv = "a,b\nx,a\u{1}b\ny,vt\u{b}z _x0041_ end\n";
    let xlsx = convert("csv", "xlsx", csv.as_bytes());
    let sheet = zip_part(&xlsx.stdout, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains("a_x0001_b"), "{sheet}");
    assert!(
        !sheet.contains('\u{1}') && !sheet.contains('\u{b}'),
        "{sheet}"
    );
    let back = text_of("xlsx", "csv", &xlsx.stdout);
    assert_eq!(back, csv);
}

#[test]
fn control_characters_are_dropped_with_a_loss_from_ods_and_xml() {
    let out = convert("csv", "ods", "a\nx\u{1}y\n".as_bytes());
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    let content = zip_part(&out.stdout, "content.xml");
    assert!(!content.contains('\u{1}'), "{content}");
    assert!(content.contains(">xy<"), "{content}");
    let out = convert("json", "xml", br#"{"r":{"a":"x\u0001y"}}"#);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    let xml = String::from_utf8_lossy(&out.stdout);
    assert!(xml.contains("<a>xy</a>"), "{xml}");
}

#[test]
fn xml_root_member_that_is_an_array_is_wrapped_in_one_root() {
    for json in [
        r#"{"r":[1,2]}"#,
        r#"{"r":[]}"#,
        r#"{"rows":[{"a":1},{"a":2}]}"#,
    ] {
        let out = text("json", "xml", json);
        assert_eq!(out.matches("<root>").count(), 1, "{out}");
        assert!(out.trim_end().ends_with("</root>"), "{out}");
    }
}
