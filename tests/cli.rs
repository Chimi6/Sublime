//! Runs the built binary as a subprocess and checks stdout, stderr, and exit codes.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_sublime"))
}

fn fixture(relative: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests/fixtures");
    path.push(relative);
    path
}

fn run(args: &[&str]) -> Output {
    Command::new(binary())
        .args(args)
        .env_remove("SUBLIME_LOG")
        .env("NO_COLOR", "1")
        .output()
        .expect("binary runs")
}

fn run_with_stdin(args: &[&str], stdin_bytes: &[u8]) -> Output {
    let mut child = Command::new(binary())
        .args(args)
        .env_remove("SUBLIME_LOG")
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("binary spawns");
    {
        let mut stdin = child.stdin.take().expect("stdin");
        let written = stdin.write_all(stdin_bytes);
        if let Err(error) = written {
            let is_broken_pipe = error.kind() == std::io::ErrorKind::BrokenPipe;
            assert!(is_broken_pipe, "write stdin: {error}");
        }
    }
    child.wait_with_output().expect("wait")
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("utf8 stdout")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("utf8 stderr")
}

fn code(output: &Output) -> i32 {
    output.status.code().expect("exit code")
}

fn temp_path(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let unique = format!("sublime-test-{}-{name}", std::process::id());
    path.push(unique);
    path
}

#[test]
fn convert_csv_to_json_via_extension() {
    let input = fixture("csv/simple.csv");
    let output = run(&["convert", input.to_str().unwrap(), "--to", "json"]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "[{\"name\":\"Ada\",\"age\":\"36\",\"city\":\"London\"},{\"name\":\"Lin\",\"age\":\"29\",\"city\":\"Taipei\"}]"
    );
    assert_eq!(stderr(&output), "");
}

#[test]
fn convert_writes_output_file_and_infers_format_from_it() {
    let input = fixture("csv/simple.csv");
    let target = temp_path("out.json");
    let output = run(&["convert", input.to_str().unwrap(), target.to_str().unwrap()]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let written = std::fs::read_to_string(&target).expect("output file");
    assert!(written.starts_with("[{\"name\":\"Ada\""));
    let _ = std::fs::remove_file(&target);
}

#[test]
fn convert_from_stdin_requires_from_flag() {
    let output = run_with_stdin(&["convert", "-", "--to", "json"], b"a\n1\n");
    assert_eq!(code(&output), 4);
    assert!(stderr(&output).contains("--from"));
    let ok = run_with_stdin(
        &["convert", "-", "--from", "csv", "--to", "json"],
        b"a\n1\n",
    );
    assert_eq!(code(&ok), 0);
    assert_eq!(stdout(&ok), "[{\"a\":\"1\"}]");
}

#[test]
fn ragged_rows_exit_with_loss_and_report_it() {
    let input = fixture("csv/ragged.csv");
    let output = run(&["convert", input.to_str().unwrap(), "--to", "json"]);
    assert_eq!(code(&output), 2);
    assert_eq!(
        stdout(&output),
        "[{\"a\":\"1\",\"b\":\"2\"},{\"a\":\"3\",\"b\":\"4\",\"c\":\"5\"}]"
    );
    let diagnostics = stderr(&output);
    assert!(diagnostics.contains("warning:"), "{diagnostics}");
    assert!(diagnostics.contains("loss: csv-to-json"), "{diagnostics}");
}

#[test]
fn quiet_silences_diagnostics() {
    let input = fixture("csv/ragged.csv");
    let output = run(&["-q", "convert", input.to_str().unwrap(), "--to", "json"]);
    assert_eq!(code(&output), 2);
    assert_eq!(stderr(&output), "");
}

#[test]
fn json_log_format_emits_events_as_lines() {
    let input = fixture("csv/ragged.csv");
    let output = run(&[
        "--log-format",
        "json",
        "convert",
        input.to_str().unwrap(),
        "--to",
        "json",
    ]);
    assert_eq!(code(&output), 2);
    let diagnostics = stderr(&output);
    let mut saw_path = false;
    let mut saw_loss = false;
    for line in diagnostics.lines() {
        assert!(
            line.starts_with('{') && line.ends_with('}'),
            "not a JSON line: {line}"
        );
        if line.contains("\"event\":\"path_chosen\"") {
            saw_path = true;
        }
        if line.contains("\"event\":\"loss\"") {
            saw_loss = true;
        }
    }
    assert!(saw_path && saw_loss, "{diagnostics}");
}

#[test]
fn verbose_shows_steps_and_timing() {
    let input = fixture("csv/simple.csv");
    let output = run(&["-v", "convert", input.to_str().unwrap(), "--to", "json"]);
    assert_eq!(code(&output), 0);
    let diagnostics = stderr(&output);
    assert!(
        diagnostics.contains("path: csv -> json via csv-to-json (lossless)"),
        "{diagnostics}"
    );
    assert!(
        diagnostics.contains("step: csv-to-json finished in"),
        "{diagnostics}"
    );
}

#[test]
fn env_var_sets_verbosity() {
    let input = fixture("csv/simple.csv");
    let output = Command::new(binary())
        .args(["convert", input.to_str().unwrap(), "--to", "json"])
        .env("SUBLIME_LOG", "verbose")
        .env("NO_COLOR", "1")
        .output()
        .expect("runs");
    assert!(stderr(&output).contains("step:"));
}

#[test]
fn json_to_csv_round_trip_of_flat_fixture() {
    let input = fixture("json/flat.json");
    let output = run(&["convert", input.to_str().unwrap(), "--to", "csv"]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "name,age\nAda,36\nLin,29\n");
}

#[test]
fn json_to_csv_reports_nested_and_scalar_losses() {
    let nested = run(&[
        "convert",
        fixture("json/nested.json").to_str().unwrap(),
        "--to",
        "csv",
    ]);
    assert_eq!(code(&nested), 2);
    assert!(stderr(&nested).contains("nested value"));
    let scalars = run(&[
        "convert",
        fixture("json/scalars.json").to_str().unwrap(),
        "--to",
        "csv",
    ]);
    assert_eq!(code(&scalars), 2);
    assert_eq!(stdout(&scalars), "n,t,z\n1.5,true,\n-2e3,false,\n");
}

#[test]
fn json_not_array_is_an_error() {
    let output = run(&[
        "convert",
        fixture("json/not_array.json").to_str().unwrap(),
        "--to",
        "csv",
    ]);
    assert_eq!(code(&output), 1);
    assert!(stderr(&output).contains("sublime: error:"));
}

#[test]
fn failed_conversion_removes_partial_output_file() {
    let target = temp_path("partial.csv");
    let output = run(&[
        "convert",
        fixture("json/not_array.json").to_str().unwrap(),
        target.to_str().unwrap(),
    ]);
    assert_eq!(code(&output), 1);
    assert!(!target.exists());
}

#[test]
fn csv_round_trips_through_json() {
    let first = run(&[
        "convert",
        fixture("csv/quoted.csv").to_str().unwrap(),
        "--to",
        "json",
    ]);
    assert_eq!(code(&first), 0);
    let back = run_with_stdin(
        &["convert", "-", "--from", "json", "--to", "csv"],
        &first.stdout,
    );
    assert_eq!(code(&back), 0, "stderr: {}", stderr(&back));
    let original = std::fs::read_to_string(fixture("csv/quoted.csv")).unwrap();
    assert_eq!(stdout(&back), original);
}

#[test]
fn bom_and_crlf_are_handled() {
    let output = run(&[
        "convert",
        fixture("csv/bom_crlf.csv").to_str().unwrap(),
        "--to",
        "json",
    ]);
    assert_eq!(code(&output), 0);
    assert_eq!(
        stdout(&output),
        "[{\"id\":\"1\",\"value\":\"x\"},{\"id\":\"2\",\"value\":\"y\"}]"
    );
}

#[test]
fn unicode_passes_through() {
    let output = run(&[
        "convert",
        fixture("csv/unicode.csv").to_str().unwrap(),
        "--to",
        "json",
    ]);
    assert_eq!(code(&output), 0);
    assert_eq!(
        stdout(&output),
        "[{\"word\":\"héllo\",\"mark\":\"✓\"},{\"word\":\"日本\",\"mark\":\"語\"}]"
    );
}

#[test]
fn empty_and_header_only_csv_produce_empty_arrays() {
    for name in ["csv/empty.csv", "csv/header_only.csv"] {
        let output = run(&["convert", fixture(name).to_str().unwrap(), "--to", "json"]);
        assert_eq!(code(&output), 0, "{name}");
        assert_eq!(stdout(&output), "[]", "{name}");
    }
}

#[test]
fn check_reports_fidelity_and_exit_codes() {
    let lossless = run(&["check", "csv", "json"]);
    assert_eq!(code(&lossless), 0);
    assert_eq!(
        stdout(&lossless),
        "csv -> json\n  1. csv-to-json (native, lossless)\nfidelity: lossless\n"
    );

    let conditional = run(&["check", "json", "csv"]);
    assert_eq!(code(&conditional), 2);
    assert!(stdout(&conditional).contains("fidelity: conditional"));

    let strict = run(&["check", "json", "csv", "--strict"]);
    assert_eq!(code(&strict), 3);
    assert!(stderr(&strict).contains("json-to-csv"));

    let unknown = run(&["check", "csv", "pdoc"]);
    assert_eq!(code(&unknown), 4);

    let same = run(&["check", "csv", "csv"]);
    assert_eq!(code(&same), 4);
}

#[test]
fn check_json_output() {
    let output = run(&["--log-format", "json", "check", "csv", "json"]);
    assert_eq!(code(&output), 0);
    assert_eq!(
        stdout(&output),
        "{\"from\":\"csv\",\"to\":\"json\",\"fidelity\":\"lossless\",\"hops\":[{\"converter\":\"csv-to-json\",\"from\":\"csv\",\"to\":\"json\",\"tier\":\"native\",\"fidelity\":\"lossless\",\"description\":null}]}\n"
    );
}

#[test]
fn formats_and_paths_list_the_registry() {
    let formats = run(&["formats"]);
    assert_eq!(code(&formats), 0);
    assert!(stdout(&formats).contains("csv"));
    assert!(stdout(&formats).contains("json"));

    let paths = run(&["paths"]);
    assert_eq!(code(&paths), 0);
    assert_eq!(
        stdout(&paths),
        "bmp -> csv: lossy via bmp-to-pdf -> pdf-to-markdown -> markdown-to-csv\nbmp -> docx: lossy via bmp-to-pdf -> pdf-to-docx\nbmp -> heic: lossy via bmp-to-heic\nbmp -> html: lossy via bmp-to-pdf -> pdf-to-html\nbmp -> ico: conditional via bmp-to-ico\nbmp -> jpeg: lossy via bmp-to-jpeg\nbmp -> json: lossy via bmp-to-pdf -> pdf-to-markdown -> markdown-tables-to-json\nbmp -> jsonl: lossy via bmp-to-pdf -> pdf-to-markdown -> markdown-to-csv -> csv-to-jsonl\nbmp -> markdown: lossy via bmp-to-pdf -> pdf-to-markdown\nbmp -> markdown-json: lossy via bmp-to-pdf -> pdf-to-markdown -> markdown-to-json\nbmp -> numbers: lossy via bmp-to-pdf -> pdf-to-markdown -> markdown-to-numbers\nbmp -> pages: lossy via bmp-to-pdf -> pdf-to-docx -> docx-to-pages\nbmp -> pages-json: lossy via bmp-to-pdf -> pdf-to-docx -> docx-to-pages -> pages-to-json\nbmp -> pam: lossless via bmp-to-pam\nbmp -> pbm: lossy via bmp-to-pbm\nbmp -> pdf: lossless via bmp-to-pdf\nbmp -> pgm: conditional via bmp-to-pgm\nbmp -> png: lossless via bmp-to-png\nbmp -> ppm: conditional via bmp-to-ppm\nbmp -> qoi: lossless via bmp-to-qoi\nbmp -> rtf: lossy via bmp-to-pdf -> pdf-to-docx -> docx-to-rtf\nbmp -> text: lossy via bmp-to-pdf -> pdf-to-text\nbmp -> tga: lossless via bmp-to-tga\nbmp -> tiff: lossless via bmp-to-tiff\nbmp -> toml: lossy via bmp-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-toml\nbmp -> tsv: lossy via bmp-to-pdf -> pdf-to-markdown -> markdown-to-tsv\nbmp -> webp: lossless via bmp-to-webp\nbmp -> xlsx: lossy via bmp-to-pdf -> pdf-to-markdown -> markdown-to-xlsx\nbmp -> xml: lossy via bmp-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-xml\nbmp -> yaml: lossy via bmp-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-yaml\ncsv -> bmp: conditional via csv-to-markdown -> markdown-to-pdf -> pdf-to-bmp\ncsv -> docx: conditional via csv-to-markdown -> markdown-to-docx\ncsv -> heic: lossy via csv-to-markdown -> markdown-to-pdf -> pdf-to-heic\ncsv -> html: conditional via csv-to-markdown -> markdown-to-html\ncsv -> ico: conditional via csv-to-markdown -> markdown-to-pdf -> pdf-to-ico\ncsv -> jpeg: lossy via csv-to-markdown -> markdown-to-pdf -> pdf-to-jpeg\ncsv -> json: lossless via csv-to-json\ncsv -> jsonl: lossless via csv-to-jsonl\ncsv -> markdown: conditional via csv-to-markdown\ncsv -> markdown-json: conditional via csv-to-markdown -> markdown-to-json\ncsv -> numbers: conditional via csv-to-numbers\ncsv -> pages: lossy via csv-to-markdown -> markdown-to-pages\ncsv -> pages-json: lossy via csv-to-markdown -> markdown-to-pages -> pages-to-json\ncsv -> pam: conditional via csv-to-markdown -> markdown-to-pdf -> pdf-to-pam\ncsv -> pbm: lossy via csv-to-markdown -> markdown-to-pdf -> pdf-to-pbm\ncsv -> pdf: conditional via csv-to-markdown -> markdown-to-pdf\ncsv -> pgm: conditional via csv-to-markdown -> markdown-to-pdf -> pdf-to-pgm\ncsv -> png: conditional via csv-to-markdown -> markdown-to-pdf -> pdf-to-png\ncsv -> ppm: conditional via csv-to-markdown -> markdown-to-pdf -> pdf-to-ppm\ncsv -> qoi: conditional via csv-to-markdown -> markdown-to-pdf -> pdf-to-qoi\ncsv -> rtf: lossy via csv-to-markdown -> markdown-to-rtf\ncsv -> text: lossy via csv-to-markdown -> markdown-to-text\ncsv -> tga: conditional via csv-to-markdown -> markdown-to-pdf -> pdf-to-tga\ncsv -> tiff: conditional via csv-to-markdown -> markdown-to-pdf -> pdf-to-tiff\ncsv -> toml: conditional via csv-to-json -> json-to-toml\ncsv -> tsv: lossless via csv-to-tsv\ncsv -> webp: conditional via csv-to-markdown -> markdown-to-pdf -> pdf-to-webp\ncsv -> xlsx: conditional via csv-to-xlsx\ncsv -> xml: conditional via csv-to-json -> json-to-xml\ncsv -> yaml: lossless via csv-to-json -> json-to-yaml\ncur -> bmp: conditional via cur-to-bmp\ncur -> csv: lossy via cur-to-pdf -> pdf-to-markdown -> markdown-to-csv\ncur -> docx: lossy via cur-to-pdf -> pdf-to-docx\ncur -> heic: lossy via cur-to-heic\ncur -> html: lossy via cur-to-pdf -> pdf-to-html\ncur -> ico: conditional via cur-to-ico\ncur -> jpeg: lossy via cur-to-jpeg\ncur -> json: lossy via cur-to-pdf -> pdf-to-markdown -> markdown-tables-to-json\ncur -> jsonl: lossy via cur-to-pdf -> pdf-to-markdown -> markdown-to-csv -> csv-to-jsonl\ncur -> markdown: lossy via cur-to-pdf -> pdf-to-markdown\ncur -> markdown-json: lossy via cur-to-pdf -> pdf-to-markdown -> markdown-to-json\ncur -> numbers: lossy via cur-to-pdf -> pdf-to-markdown -> markdown-to-numbers\ncur -> pages: lossy via cur-to-pdf -> pdf-to-docx -> docx-to-pages\ncur -> pages-json: lossy via cur-to-pdf -> pdf-to-docx -> docx-to-pages -> pages-to-json\ncur -> pam: conditional via cur-to-pam\ncur -> pbm: lossy via cur-to-pbm\ncur -> pdf: conditional via cur-to-pdf\ncur -> pgm: conditional via cur-to-pgm\ncur -> png: conditional via cur-to-png\ncur -> ppm: conditional via cur-to-ppm\ncur -> qoi: conditional via cur-to-qoi\ncur -> rtf: lossy via cur-to-pdf -> pdf-to-docx -> docx-to-rtf\ncur -> text: lossy via cur-to-pdf -> pdf-to-text\ncur -> tga: conditional via cur-to-tga\ncur -> tiff: conditional via cur-to-tiff\ncur -> toml: lossy via cur-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-toml\ncur -> tsv: lossy via cur-to-pdf -> pdf-to-markdown -> markdown-to-tsv\ncur -> webp: conditional via cur-to-webp\ncur -> xlsx: lossy via cur-to-pdf -> pdf-to-markdown -> markdown-to-xlsx\ncur -> xml: lossy via cur-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-xml\ncur -> yaml: lossy via cur-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-yaml\ndocx -> bmp: conditional via docx-to-pdf -> pdf-to-bmp\ndocx -> csv: lossy via docx-to-markdown -> markdown-to-csv\ndocx -> heic: lossy via docx-to-pdf -> pdf-to-heic\ndocx -> html: lossy via docx-to-html\ndocx -> ico: conditional via docx-to-pdf -> pdf-to-ico\ndocx -> jpeg: lossy via docx-to-pdf -> pdf-to-jpeg\ndocx -> json: lossy via docx-to-markdown -> markdown-tables-to-json\ndocx -> jsonl: lossy via docx-to-markdown -> markdown-to-csv -> csv-to-jsonl\ndocx -> markdown: lossy via docx-to-markdown\ndocx -> markdown-json: lossy via docx-to-markdown -> markdown-to-json\ndocx -> numbers: lossy via docx-to-markdown -> markdown-to-numbers\ndocx -> pages: lossy via docx-to-pages\ndocx -> pages-json: lossy via docx-to-pages -> pages-to-json\ndocx -> pam: conditional via docx-to-pdf -> pdf-to-pam\ndocx -> pbm: lossy via docx-to-pdf -> pdf-to-pbm\ndocx -> pdf: conditional via docx-to-pdf\ndocx -> pgm: conditional via docx-to-pdf -> pdf-to-pgm\ndocx -> png: conditional via docx-to-pdf -> pdf-to-png\ndocx -> ppm: conditional via docx-to-pdf -> pdf-to-ppm\ndocx -> qoi: conditional via docx-to-pdf -> pdf-to-qoi\ndocx -> rtf: lossy via docx-to-rtf\ndocx -> text: lossy via docx-to-text\ndocx -> tga: conditional via docx-to-pdf -> pdf-to-tga\ndocx -> tiff: conditional via docx-to-pdf -> pdf-to-tiff\ndocx -> toml: lossy via docx-to-markdown -> markdown-tables-to-json -> json-to-toml\ndocx -> tsv: lossy via docx-to-markdown -> markdown-to-tsv\ndocx -> webp: conditional via docx-to-pdf -> pdf-to-webp\ndocx -> xlsx: lossy via docx-to-markdown -> markdown-to-xlsx\ndocx -> xml: lossy via docx-to-markdown -> markdown-tables-to-json -> json-to-xml\ndocx -> yaml: lossy via docx-to-markdown -> markdown-tables-to-json -> json-to-yaml\nheic -> bmp: conditional via heic-to-bmp\nheic -> csv: lossy via heic-to-pdf -> pdf-to-markdown -> markdown-to-csv\nheic -> docx: lossy via heic-to-pdf -> pdf-to-docx\nheic -> html: lossy via heic-to-pdf -> pdf-to-html\nheic -> ico: conditional via heic-to-ico\nheic -> jpeg: lossy via heic-to-jpeg\nheic -> json: lossy via heic-to-pdf -> pdf-to-markdown -> markdown-tables-to-json\nheic -> jsonl: lossy via heic-to-pdf -> pdf-to-markdown -> markdown-to-csv -> csv-to-jsonl\nheic -> markdown: lossy via heic-to-pdf -> pdf-to-markdown\nheic -> markdown-json: lossy via heic-to-pdf -> pdf-to-markdown -> markdown-to-json\nheic -> numbers: lossy via heic-to-pdf -> pdf-to-markdown -> markdown-to-numbers\nheic -> pages: lossy via heic-to-pdf -> pdf-to-docx -> docx-to-pages\nheic -> pages-json: lossy via heic-to-pdf -> pdf-to-docx -> docx-to-pages -> pages-to-json\nheic -> pam: conditional via heic-to-pam\nheic -> pbm: lossy via heic-to-pbm\nheic -> pdf: conditional via heic-to-pdf\nheic -> pgm: conditional via heic-to-pgm\nheic -> png: conditional via heic-to-png\nheic -> ppm: conditional via heic-to-ppm\nheic -> qoi: conditional via heic-to-qoi\nheic -> rtf: lossy via heic-to-pdf -> pdf-to-docx -> docx-to-rtf\nheic -> text: lossy via heic-to-pdf -> pdf-to-text\nheic -> tga: conditional via heic-to-tga\nheic -> tiff: conditional via heic-to-tiff\nheic -> toml: lossy via heic-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-toml\nheic -> tsv: lossy via heic-to-pdf -> pdf-to-markdown -> markdown-to-tsv\nheic -> webp: conditional via heic-to-webp\nheic -> xlsx: lossy via heic-to-pdf -> pdf-to-markdown -> markdown-to-xlsx\nheic -> xml: lossy via heic-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-xml\nheic -> yaml: lossy via heic-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-yaml\nhtml -> bmp: conditional via html-to-pdf -> pdf-to-bmp\nhtml -> csv: lossy via html-to-markdown -> markdown-to-csv\nhtml -> docx: conditional via html-to-docx\nhtml -> heic: lossy via html-to-pdf -> pdf-to-heic\nhtml -> ico: conditional via html-to-pdf -> pdf-to-ico\nhtml -> jpeg: lossy via html-to-pdf -> pdf-to-jpeg\nhtml -> json: lossy via html-to-markdown -> markdown-tables-to-json\nhtml -> jsonl: lossy via html-to-markdown -> markdown-to-csv -> csv-to-jsonl\nhtml -> markdown: lossy via html-to-markdown\nhtml -> markdown-json: lossy via html-to-markdown -> markdown-to-json\nhtml -> numbers: lossy via html-to-markdown -> markdown-to-numbers\nhtml -> pages: lossy via html-to-pages\nhtml -> pages-json: lossy via html-to-pages -> pages-to-json\nhtml -> pam: conditional via html-to-pdf -> pdf-to-pam\nhtml -> pbm: lossy via html-to-pdf -> pdf-to-pbm\nhtml -> pdf: conditional via html-to-pdf\nhtml -> pgm: conditional via html-to-pdf -> pdf-to-pgm\nhtml -> png: conditional via html-to-pdf -> pdf-to-png\nhtml -> ppm: conditional via html-to-pdf -> pdf-to-ppm\nhtml -> qoi: conditional via html-to-pdf -> pdf-to-qoi\nhtml -> rtf: lossy via html-to-rtf\nhtml -> text: lossy via html-to-text\nhtml -> tga: conditional via html-to-pdf -> pdf-to-tga\nhtml -> tiff: conditional via html-to-pdf -> pdf-to-tiff\nhtml -> toml: lossy via html-to-markdown -> markdown-tables-to-json -> json-to-toml\nhtml -> tsv: lossy via html-to-markdown -> markdown-to-tsv\nhtml -> webp: conditional via html-to-pdf -> pdf-to-webp\nhtml -> xlsx: lossy via html-to-markdown -> markdown-to-xlsx\nhtml -> xml: lossy via html-to-markdown -> markdown-tables-to-json -> json-to-xml\nhtml -> yaml: lossy via html-to-markdown -> markdown-tables-to-json -> json-to-yaml\nico -> bmp: conditional via ico-to-bmp\nico -> csv: lossy via ico-to-pdf -> pdf-to-markdown -> markdown-to-csv\nico -> docx: lossy via ico-to-pdf -> pdf-to-docx\nico -> heic: lossy via ico-to-heic\nico -> html: lossy via ico-to-pdf -> pdf-to-html\nico -> jpeg: lossy via ico-to-jpeg\nico -> json: lossy via ico-to-pdf -> pdf-to-markdown -> markdown-tables-to-json\nico -> jsonl: lossy via ico-to-pdf -> pdf-to-markdown -> markdown-to-csv -> csv-to-jsonl\nico -> markdown: lossy via ico-to-pdf -> pdf-to-markdown\nico -> markdown-json: lossy via ico-to-pdf -> pdf-to-markdown -> markdown-to-json\nico -> numbers: lossy via ico-to-pdf -> pdf-to-markdown -> markdown-to-numbers\nico -> pages: lossy via ico-to-pdf -> pdf-to-docx -> docx-to-pages\nico -> pages-json: lossy via ico-to-pdf -> pdf-to-docx -> docx-to-pages -> pages-to-json\nico -> pam: conditional via ico-to-pam\nico -> pbm: lossy via ico-to-pbm\nico -> pdf: conditional via ico-to-pdf\nico -> pgm: conditional via ico-to-pgm\nico -> png: conditional via ico-to-png\nico -> ppm: conditional via ico-to-ppm\nico -> qoi: conditional via ico-to-qoi\nico -> rtf: lossy via ico-to-pdf -> pdf-to-docx -> docx-to-rtf\nico -> text: lossy via ico-to-pdf -> pdf-to-text\nico -> tga: conditional via ico-to-tga\nico -> tiff: conditional via ico-to-tiff\nico -> toml: lossy via ico-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-toml\nico -> tsv: lossy via ico-to-pdf -> pdf-to-markdown -> markdown-to-tsv\nico -> webp: conditional via ico-to-webp\nico -> xlsx: lossy via ico-to-pdf -> pdf-to-markdown -> markdown-to-xlsx\nico -> xml: lossy via ico-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-xml\nico -> yaml: lossy via ico-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-yaml\njpeg -> bmp: conditional via jpeg-to-bmp\njpeg -> csv: lossy via jpeg-to-pdf -> pdf-to-markdown -> markdown-to-csv\njpeg -> docx: lossy via jpeg-to-pdf -> pdf-to-docx\njpeg -> heic: lossy via jpeg-to-heic\njpeg -> html: lossy via jpeg-to-pdf -> pdf-to-html\njpeg -> ico: conditional via jpeg-to-ico\njpeg -> json: lossy via jpeg-to-pdf -> pdf-to-markdown -> markdown-tables-to-json\njpeg -> jsonl: lossy via jpeg-to-pdf -> pdf-to-markdown -> markdown-to-csv -> csv-to-jsonl\njpeg -> markdown: lossy via jpeg-to-pdf -> pdf-to-markdown\njpeg -> markdown-json: lossy via jpeg-to-pdf -> pdf-to-markdown -> markdown-to-json\njpeg -> numbers: lossy via jpeg-to-pdf -> pdf-to-markdown -> markdown-to-numbers\njpeg -> pages: lossy via jpeg-to-pdf -> pdf-to-docx -> docx-to-pages\njpeg -> pages-json: lossy via jpeg-to-pdf -> pdf-to-docx -> docx-to-pages -> pages-to-json\njpeg -> pam: conditional via jpeg-to-pam\njpeg -> pbm: lossy via jpeg-to-pbm\njpeg -> pdf: lossless via jpeg-to-pdf\njpeg -> pgm: conditional via jpeg-to-pgm\njpeg -> png: conditional via jpeg-to-png\njpeg -> ppm: conditional via jpeg-to-ppm\njpeg -> qoi: conditional via jpeg-to-qoi\njpeg -> rtf: lossy via jpeg-to-pdf -> pdf-to-docx -> docx-to-rtf\njpeg -> text: lossy via jpeg-to-pdf -> pdf-to-text\njpeg -> tga: conditional via jpeg-to-tga\njpeg -> tiff: conditional via jpeg-to-tiff\njpeg -> toml: lossy via jpeg-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-toml\njpeg -> tsv: lossy via jpeg-to-pdf -> pdf-to-markdown -> markdown-to-tsv\njpeg -> webp: conditional via jpeg-to-webp\njpeg -> xlsx: lossy via jpeg-to-pdf -> pdf-to-markdown -> markdown-to-xlsx\njpeg -> xml: lossy via jpeg-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-xml\njpeg -> yaml: lossy via jpeg-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-yaml\njson -> bmp: conditional via json-to-markdown -> markdown-to-pdf -> pdf-to-bmp\njson -> csv: conditional via json-to-csv\njson -> docx: conditional via json-to-markdown -> markdown-to-docx\njson -> heic: lossy via json-to-markdown -> markdown-to-pdf -> pdf-to-heic\njson -> html: conditional via json-to-markdown -> markdown-to-html\njson -> ico: conditional via json-to-markdown -> markdown-to-pdf -> pdf-to-ico\njson -> jpeg: lossy via json-to-markdown -> markdown-to-pdf -> pdf-to-jpeg\njson -> jsonl: conditional via json-to-jsonl\njson -> markdown: conditional via json-to-markdown\njson -> markdown-json: conditional via json-to-markdown -> markdown-to-json\njson -> numbers: conditional via json-to-numbers\njson -> pages: lossy via json-to-markdown -> markdown-to-pages\njson -> pages-json: lossy via json-to-markdown -> markdown-to-pages -> pages-to-json\njson -> pam: conditional via json-to-markdown -> markdown-to-pdf -> pdf-to-pam\njson -> pbm: lossy via json-to-markdown -> markdown-to-pdf -> pdf-to-pbm\njson -> pdf: conditional via json-to-markdown -> markdown-to-pdf\njson -> pgm: conditional via json-to-markdown -> markdown-to-pdf -> pdf-to-pgm\njson -> png: conditional via json-to-markdown -> markdown-to-pdf -> pdf-to-png\njson -> ppm: conditional via json-to-markdown -> markdown-to-pdf -> pdf-to-ppm\njson -> qoi: conditional via json-to-markdown -> markdown-to-pdf -> pdf-to-qoi\njson -> rtf: lossy via json-to-markdown -> markdown-to-rtf\njson -> text: lossy via json-to-markdown -> markdown-to-text\njson -> tga: conditional via json-to-markdown -> markdown-to-pdf -> pdf-to-tga\njson -> tiff: conditional via json-to-markdown -> markdown-to-pdf -> pdf-to-tiff\njson -> toml: conditional via json-to-toml\njson -> tsv: conditional via json-to-tsv\njson -> webp: conditional via json-to-markdown -> markdown-to-pdf -> pdf-to-webp\njson -> xlsx: conditional via json-to-xlsx\njson -> xml: conditional via json-to-xml\njson -> yaml: lossless via json-to-yaml\njsonl -> bmp: conditional via jsonl-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-bmp\njsonl -> csv: conditional via jsonl-to-csv\njsonl -> docx: conditional via jsonl-to-json -> json-to-markdown -> markdown-to-docx\njsonl -> heic: lossy via jsonl-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-heic\njsonl -> html: conditional via jsonl-to-json -> json-to-markdown -> markdown-to-html\njsonl -> ico: conditional via jsonl-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-ico\njsonl -> jpeg: lossy via jsonl-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-jpeg\njsonl -> json: lossless via jsonl-to-json\njsonl -> markdown: conditional via jsonl-to-json -> json-to-markdown\njsonl -> markdown-json: conditional via jsonl-to-json -> json-to-markdown -> markdown-to-json\njsonl -> numbers: conditional via jsonl-to-json -> json-to-numbers\njsonl -> pages: lossy via jsonl-to-json -> json-to-markdown -> markdown-to-pages\njsonl -> pages-json: lossy via jsonl-to-json -> json-to-markdown -> markdown-to-pages -> pages-to-json\njsonl -> pam: conditional via jsonl-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-pam\njsonl -> pbm: lossy via jsonl-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-pbm\njsonl -> pdf: conditional via jsonl-to-json -> json-to-markdown -> markdown-to-pdf\njsonl -> pgm: conditional via jsonl-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-pgm\njsonl -> png: conditional via jsonl-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-png\njsonl -> ppm: conditional via jsonl-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-ppm\njsonl -> qoi: conditional via jsonl-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-qoi\njsonl -> rtf: lossy via jsonl-to-json -> json-to-markdown -> markdown-to-rtf\njsonl -> text: lossy via jsonl-to-json -> json-to-markdown -> markdown-to-text\njsonl -> tga: conditional via jsonl-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-tga\njsonl -> tiff: conditional via jsonl-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-tiff\njsonl -> toml: conditional via jsonl-to-json -> json-to-toml\njsonl -> tsv: conditional via jsonl-to-tsv\njsonl -> webp: conditional via jsonl-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-webp\njsonl -> xlsx: conditional via jsonl-to-json -> json-to-xlsx\njsonl -> xml: conditional via jsonl-to-json -> json-to-xml\njsonl -> yaml: lossless via jsonl-to-json -> json-to-yaml\nmarkdown -> bmp: conditional via markdown-to-pdf -> pdf-to-bmp\nmarkdown -> csv: lossy via markdown-to-csv\nmarkdown -> docx: conditional via markdown-to-docx\nmarkdown -> heic: lossy via markdown-to-pdf -> pdf-to-heic\nmarkdown -> html: lossless via markdown-to-html\nmarkdown -> ico: conditional via markdown-to-pdf -> pdf-to-ico\nmarkdown -> jpeg: lossy via markdown-to-pdf -> pdf-to-jpeg\nmarkdown -> json: lossy via markdown-tables-to-json\nmarkdown -> jsonl: lossy via markdown-to-csv -> csv-to-jsonl\nmarkdown -> markdown-json: lossless via markdown-to-json\nmarkdown -> numbers: lossy via markdown-to-numbers\nmarkdown -> pages: lossy via markdown-to-pages\nmarkdown -> pages-json: lossy via markdown-to-pages -> pages-to-json\nmarkdown -> pam: conditional via markdown-to-pdf -> pdf-to-pam\nmarkdown -> pbm: lossy via markdown-to-pdf -> pdf-to-pbm\nmarkdown -> pdf: conditional via markdown-to-pdf\nmarkdown -> pgm: conditional via markdown-to-pdf -> pdf-to-pgm\nmarkdown -> png: conditional via markdown-to-pdf -> pdf-to-png\nmarkdown -> ppm: conditional via markdown-to-pdf -> pdf-to-ppm\nmarkdown -> qoi: conditional via markdown-to-pdf -> pdf-to-qoi\nmarkdown -> rtf: lossy via markdown-to-rtf\nmarkdown -> text: lossy via markdown-to-text\nmarkdown -> tga: conditional via markdown-to-pdf -> pdf-to-tga\nmarkdown -> tiff: conditional via markdown-to-pdf -> pdf-to-tiff\nmarkdown -> toml: lossy via markdown-tables-to-json -> json-to-toml\nmarkdown -> tsv: lossy via markdown-to-tsv\nmarkdown -> webp: conditional via markdown-to-pdf -> pdf-to-webp\nmarkdown -> xlsx: lossy via markdown-to-xlsx\nmarkdown -> xml: lossy via markdown-tables-to-json -> json-to-xml\nmarkdown -> yaml: lossy via markdown-tables-to-json -> json-to-yaml\nmarkdown-json -> bmp: conditional via markdown-json-to-markdown -> markdown-to-pdf -> pdf-to-bmp\nmarkdown-json -> csv: lossy via markdown-json-to-markdown -> markdown-to-csv\nmarkdown-json -> docx: conditional via markdown-json-to-markdown -> markdown-to-docx\nmarkdown-json -> heic: lossy via markdown-json-to-markdown -> markdown-to-pdf -> pdf-to-heic\nmarkdown-json -> html: lossless via markdown-json-to-markdown -> markdown-to-html\nmarkdown-json -> ico: conditional via markdown-json-to-markdown -> markdown-to-pdf -> pdf-to-ico\nmarkdown-json -> jpeg: lossy via markdown-json-to-markdown -> markdown-to-pdf -> pdf-to-jpeg\nmarkdown-json -> json: lossy via markdown-json-to-markdown -> markdown-tables-to-json\nmarkdown-json -> jsonl: lossy via markdown-json-to-markdown -> markdown-to-csv -> csv-to-jsonl\nmarkdown-json -> markdown: lossless via markdown-json-to-markdown\nmarkdown-json -> numbers: lossy via markdown-json-to-markdown -> markdown-to-numbers\nmarkdown-json -> pages: lossy via markdown-json-to-markdown -> markdown-to-pages\nmarkdown-json -> pages-json: lossy via markdown-json-to-markdown -> markdown-to-pages -> pages-to-json\nmarkdown-json -> pam: conditional via markdown-json-to-markdown -> markdown-to-pdf -> pdf-to-pam\nmarkdown-json -> pbm: lossy via markdown-json-to-markdown -> markdown-to-pdf -> pdf-to-pbm\nmarkdown-json -> pdf: conditional via markdown-json-to-markdown -> markdown-to-pdf\nmarkdown-json -> pgm: conditional via markdown-json-to-markdown -> markdown-to-pdf -> pdf-to-pgm\nmarkdown-json -> png: conditional via markdown-json-to-markdown -> markdown-to-pdf -> pdf-to-png\nmarkdown-json -> ppm: conditional via markdown-json-to-markdown -> markdown-to-pdf -> pdf-to-ppm\nmarkdown-json -> qoi: conditional via markdown-json-to-markdown -> markdown-to-pdf -> pdf-to-qoi\nmarkdown-json -> rtf: lossy via markdown-json-to-markdown -> markdown-to-rtf\nmarkdown-json -> text: lossy via markdown-json-to-markdown -> markdown-to-text\nmarkdown-json -> tga: conditional via markdown-json-to-markdown -> markdown-to-pdf -> pdf-to-tga\nmarkdown-json -> tiff: conditional via markdown-json-to-markdown -> markdown-to-pdf -> pdf-to-tiff\nmarkdown-json -> toml: lossy via markdown-json-to-markdown -> markdown-tables-to-json -> json-to-toml\nmarkdown-json -> tsv: lossy via markdown-json-to-markdown -> markdown-to-tsv\nmarkdown-json -> webp: conditional via markdown-json-to-markdown -> markdown-to-pdf -> pdf-to-webp\nmarkdown-json -> xlsx: lossy via markdown-json-to-markdown -> markdown-to-xlsx\nmarkdown-json -> xml: lossy via markdown-json-to-markdown -> markdown-tables-to-json -> json-to-xml\nmarkdown-json -> yaml: lossy via markdown-json-to-markdown -> markdown-tables-to-json -> json-to-yaml\nnumbers -> bmp: conditional via numbers-to-markdown -> markdown-to-pdf -> pdf-to-bmp\nnumbers -> csv: conditional via numbers-to-csv\nnumbers -> docx: conditional via numbers-to-markdown -> markdown-to-docx\nnumbers -> heic: lossy via numbers-to-markdown -> markdown-to-pdf -> pdf-to-heic\nnumbers -> html: conditional via numbers-to-markdown -> markdown-to-html\nnumbers -> ico: conditional via numbers-to-markdown -> markdown-to-pdf -> pdf-to-ico\nnumbers -> jpeg: lossy via numbers-to-markdown -> markdown-to-pdf -> pdf-to-jpeg\nnumbers -> json: conditional via numbers-to-json\nnumbers -> jsonl: conditional via numbers-to-csv -> csv-to-jsonl\nnumbers -> markdown: conditional via numbers-to-markdown\nnumbers -> markdown-json: conditional via numbers-to-markdown -> markdown-to-json\nnumbers -> pages: lossy via numbers-to-markdown -> markdown-to-pages\nnumbers -> pages-json: lossy via numbers-to-markdown -> markdown-to-pages -> pages-to-json\nnumbers -> pam: conditional via numbers-to-markdown -> markdown-to-pdf -> pdf-to-pam\nnumbers -> pbm: lossy via numbers-to-markdown -> markdown-to-pdf -> pdf-to-pbm\nnumbers -> pdf: conditional via numbers-to-markdown -> markdown-to-pdf\nnumbers -> pgm: conditional via numbers-to-markdown -> markdown-to-pdf -> pdf-to-pgm\nnumbers -> png: conditional via numbers-to-markdown -> markdown-to-pdf -> pdf-to-png\nnumbers -> ppm: conditional via numbers-to-markdown -> markdown-to-pdf -> pdf-to-ppm\nnumbers -> qoi: conditional via numbers-to-markdown -> markdown-to-pdf -> pdf-to-qoi\nnumbers -> rtf: lossy via numbers-to-markdown -> markdown-to-rtf\nnumbers -> text: lossy via numbers-to-markdown -> markdown-to-text\nnumbers -> tga: conditional via numbers-to-markdown -> markdown-to-pdf -> pdf-to-tga\nnumbers -> tiff: conditional via numbers-to-markdown -> markdown-to-pdf -> pdf-to-tiff\nnumbers -> toml: conditional via numbers-to-json -> json-to-toml\nnumbers -> tsv: conditional via numbers-to-tsv\nnumbers -> webp: conditional via numbers-to-markdown -> markdown-to-pdf -> pdf-to-webp\nnumbers -> xlsx: conditional via numbers-to-xlsx\nnumbers -> xml: conditional via numbers-to-json -> json-to-xml\nnumbers -> yaml: conditional via numbers-to-json -> json-to-yaml\npages -> bmp: conditional via pages-to-pdf -> pdf-to-bmp\npages -> csv: lossy via pages-to-markdown -> markdown-to-csv\npages -> docx: conditional via pages-to-docx\npages -> heic: lossy via pages-to-pdf -> pdf-to-heic\npages -> html: lossy via pages-to-html\npages -> ico: conditional via pages-to-pdf -> pdf-to-ico\npages -> jpeg: lossy via pages-to-pdf -> pdf-to-jpeg\npages -> json: lossy via pages-to-markdown -> markdown-tables-to-json\npages -> jsonl: lossy via pages-to-markdown -> markdown-to-csv -> csv-to-jsonl\npages -> markdown: lossy via pages-to-markdown\npages -> markdown-json: lossy via pages-to-markdown -> markdown-to-json\npages -> numbers: lossy via pages-to-markdown -> markdown-to-numbers\npages -> pages-json: lossless via pages-to-json\npages -> pam: conditional via pages-to-pdf -> pdf-to-pam\npages -> pbm: lossy via pages-to-pdf -> pdf-to-pbm\npages -> pdf: conditional via pages-to-pdf\npages -> pgm: conditional via pages-to-pdf -> pdf-to-pgm\npages -> png: conditional via pages-to-pdf -> pdf-to-png\npages -> ppm: conditional via pages-to-pdf -> pdf-to-ppm\npages -> qoi: conditional via pages-to-pdf -> pdf-to-qoi\npages -> rtf: lossy via pages-to-rtf\npages -> text: lossy via pages-to-text\npages -> tga: conditional via pages-to-pdf -> pdf-to-tga\npages -> tiff: conditional via pages-to-pdf -> pdf-to-tiff\npages -> toml: lossy via pages-to-markdown -> markdown-tables-to-json -> json-to-toml\npages -> tsv: lossy via pages-to-markdown -> markdown-to-tsv\npages -> webp: conditional via pages-to-pdf -> pdf-to-webp\npages -> xlsx: lossy via pages-to-markdown -> markdown-to-xlsx\npages -> xml: lossy via pages-to-markdown -> markdown-tables-to-json -> json-to-xml\npages -> yaml: lossy via pages-to-markdown -> markdown-tables-to-json -> json-to-yaml\npages-json -> bmp: conditional via json-to-pages -> pages-to-pdf -> pdf-to-bmp\npages-json -> csv: lossy via json-to-pages -> pages-to-markdown -> markdown-to-csv\npages-json -> docx: conditional via json-to-pages -> pages-to-docx\npages-json -> heic: lossy via json-to-pages -> pages-to-pdf -> pdf-to-heic\npages-json -> html: lossy via json-to-pages -> pages-to-html\npages-json -> ico: conditional via json-to-pages -> pages-to-pdf -> pdf-to-ico\npages-json -> jpeg: lossy via json-to-pages -> pages-to-pdf -> pdf-to-jpeg\npages-json -> json: lossy via json-to-pages -> pages-to-markdown -> markdown-tables-to-json\npages-json -> jsonl: lossy via json-to-pages -> pages-to-markdown -> markdown-to-csv -> csv-to-jsonl\npages-json -> markdown: lossy via json-to-pages -> pages-to-markdown\npages-json -> markdown-json: lossy via json-to-pages -> pages-to-markdown -> markdown-to-json\npages-json -> numbers: lossy via json-to-pages -> pages-to-markdown -> markdown-to-numbers\npages-json -> pages: lossless via json-to-pages\npages-json -> pam: conditional via json-to-pages -> pages-to-pdf -> pdf-to-pam\npages-json -> pbm: lossy via json-to-pages -> pages-to-pdf -> pdf-to-pbm\npages-json -> pdf: conditional via json-to-pages -> pages-to-pdf\npages-json -> pgm: conditional via json-to-pages -> pages-to-pdf -> pdf-to-pgm\npages-json -> png: conditional via json-to-pages -> pages-to-pdf -> pdf-to-png\npages-json -> ppm: conditional via json-to-pages -> pages-to-pdf -> pdf-to-ppm\npages-json -> qoi: conditional via json-to-pages -> pages-to-pdf -> pdf-to-qoi\npages-json -> rtf: lossy via json-to-pages -> pages-to-rtf\npages-json -> text: lossy via json-to-pages -> pages-to-text\npages-json -> tga: conditional via json-to-pages -> pages-to-pdf -> pdf-to-tga\npages-json -> tiff: conditional via json-to-pages -> pages-to-pdf -> pdf-to-tiff\npages-json -> toml: lossy via json-to-pages -> pages-to-markdown -> markdown-tables-to-json -> json-to-toml\npages-json -> tsv: lossy via json-to-pages -> pages-to-markdown -> markdown-to-tsv\npages-json -> webp: conditional via json-to-pages -> pages-to-pdf -> pdf-to-webp\npages-json -> xlsx: lossy via json-to-pages -> pages-to-markdown -> markdown-to-xlsx\npages-json -> xml: lossy via json-to-pages -> pages-to-markdown -> markdown-tables-to-json -> json-to-xml\npages-json -> yaml: lossy via json-to-pages -> pages-to-markdown -> markdown-tables-to-json -> json-to-yaml\npam -> bmp: conditional via pam-to-bmp\npam -> csv: lossy via pam-to-pdf -> pdf-to-markdown -> markdown-to-csv\npam -> docx: lossy via pam-to-pdf -> pdf-to-docx\npam -> heic: lossy via pam-to-heic\npam -> html: lossy via pam-to-pdf -> pdf-to-html\npam -> ico: conditional via pam-to-ico\npam -> jpeg: lossy via pam-to-jpeg\npam -> json: lossy via pam-to-pdf -> pdf-to-markdown -> markdown-tables-to-json\npam -> jsonl: lossy via pam-to-pdf -> pdf-to-markdown -> markdown-to-csv -> csv-to-jsonl\npam -> markdown: lossy via pam-to-pdf -> pdf-to-markdown\npam -> markdown-json: lossy via pam-to-pdf -> pdf-to-markdown -> markdown-to-json\npam -> numbers: lossy via pam-to-pdf -> pdf-to-markdown -> markdown-to-numbers\npam -> pages: lossy via pam-to-pdf -> pdf-to-docx -> docx-to-pages\npam -> pages-json: lossy via pam-to-pdf -> pdf-to-docx -> docx-to-pages -> pages-to-json\npam -> pbm: lossy via pam-to-pbm\npam -> pdf: conditional via pam-to-pdf\npam -> pgm: conditional via pam-to-pgm\npam -> png: conditional via pam-to-png\npam -> ppm: conditional via pam-to-ppm\npam -> qoi: conditional via pam-to-qoi\npam -> rtf: lossy via pam-to-pdf -> pdf-to-docx -> docx-to-rtf\npam -> text: lossy via pam-to-pdf -> pdf-to-text\npam -> tga: conditional via pam-to-tga\npam -> tiff: conditional via pam-to-tiff\npam -> toml: lossy via pam-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-toml\npam -> tsv: lossy via pam-to-pdf -> pdf-to-markdown -> markdown-to-tsv\npam -> webp: conditional via pam-to-webp\npam -> xlsx: lossy via pam-to-pdf -> pdf-to-markdown -> markdown-to-xlsx\npam -> xml: lossy via pam-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-xml\npam -> yaml: lossy via pam-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-yaml\npbm -> bmp: lossless via pbm-to-bmp\npbm -> csv: lossy via pbm-to-pdf -> pdf-to-markdown -> markdown-to-csv\npbm -> docx: lossy via pbm-to-pdf -> pdf-to-docx\npbm -> heic: lossy via pbm-to-heic\npbm -> html: lossy via pbm-to-pdf -> pdf-to-html\npbm -> ico: conditional via pbm-to-ico\npbm -> jpeg: lossy via pbm-to-jpeg\npbm -> json: lossy via pbm-to-pdf -> pdf-to-markdown -> markdown-tables-to-json\npbm -> jsonl: lossy via pbm-to-pdf -> pdf-to-markdown -> markdown-to-csv -> csv-to-jsonl\npbm -> markdown: lossy via pbm-to-pdf -> pdf-to-markdown\npbm -> markdown-json: lossy via pbm-to-pdf -> pdf-to-markdown -> markdown-to-json\npbm -> numbers: lossy via pbm-to-pdf -> pdf-to-markdown -> markdown-to-numbers\npbm -> pages: lossy via pbm-to-pdf -> pdf-to-docx -> docx-to-pages\npbm -> pages-json: lossy via pbm-to-pdf -> pdf-to-docx -> docx-to-pages -> pages-to-json\npbm -> pam: lossless via pbm-to-pam\npbm -> pdf: lossless via pbm-to-pdf\npbm -> pgm: conditional via pbm-to-pgm\npbm -> png: lossless via pbm-to-png\npbm -> ppm: conditional via pbm-to-ppm\npbm -> qoi: lossless via pbm-to-qoi\npbm -> rtf: lossy via pbm-to-pdf -> pdf-to-docx -> docx-to-rtf\npbm -> text: lossy via pbm-to-pdf -> pdf-to-text\npbm -> tga: lossless via pbm-to-tga\npbm -> tiff: lossless via pbm-to-tiff\npbm -> toml: lossy via pbm-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-toml\npbm -> tsv: lossy via pbm-to-pdf -> pdf-to-markdown -> markdown-to-tsv\npbm -> webp: lossless via pbm-to-webp\npbm -> xlsx: lossy via pbm-to-pdf -> pdf-to-markdown -> markdown-to-xlsx\npbm -> xml: lossy via pbm-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-xml\npbm -> yaml: lossy via pbm-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-yaml\npdf -> bmp: conditional via pdf-to-bmp\npdf -> csv: lossy via pdf-to-markdown -> markdown-to-csv\npdf -> docx: lossy via pdf-to-docx\npdf -> heic: lossy via pdf-to-heic\npdf -> html: lossy via pdf-to-html\npdf -> ico: conditional via pdf-to-ico\npdf -> jpeg: lossy via pdf-to-jpeg\npdf -> json: lossy via pdf-to-markdown -> markdown-tables-to-json\npdf -> jsonl: lossy via pdf-to-markdown -> markdown-to-csv -> csv-to-jsonl\npdf -> markdown: lossy via pdf-to-markdown\npdf -> markdown-json: lossy via pdf-to-markdown -> markdown-to-json\npdf -> numbers: lossy via pdf-to-markdown -> markdown-to-numbers\npdf -> pages: lossy via pdf-to-docx -> docx-to-pages\npdf -> pages-json: lossy via pdf-to-docx -> docx-to-pages -> pages-to-json\npdf -> pam: conditional via pdf-to-pam\npdf -> pbm: lossy via pdf-to-pbm\npdf -> pgm: conditional via pdf-to-pgm\npdf -> png: conditional via pdf-to-png\npdf -> ppm: conditional via pdf-to-ppm\npdf -> qoi: conditional via pdf-to-qoi\npdf -> rtf: lossy via pdf-to-docx -> docx-to-rtf\npdf -> text: lossy via pdf-to-text\npdf -> tga: conditional via pdf-to-tga\npdf -> tiff: conditional via pdf-to-tiff\npdf -> toml: lossy via pdf-to-markdown -> markdown-tables-to-json -> json-to-toml\npdf -> tsv: lossy via pdf-to-markdown -> markdown-to-tsv\npdf -> webp: conditional via pdf-to-webp\npdf -> xlsx: lossy via pdf-to-markdown -> markdown-to-xlsx\npdf -> xml: lossy via pdf-to-markdown -> markdown-tables-to-json -> json-to-xml\npdf -> yaml: lossy via pdf-to-markdown -> markdown-tables-to-json -> json-to-yaml\npgm -> bmp: conditional via pgm-to-bmp\npgm -> csv: lossy via pgm-to-pdf -> pdf-to-markdown -> markdown-to-csv\npgm -> docx: lossy via pgm-to-pdf -> pdf-to-docx\npgm -> heic: lossy via pgm-to-heic\npgm -> html: lossy via pgm-to-pdf -> pdf-to-html\npgm -> ico: conditional via pgm-to-ico\npgm -> jpeg: lossy via pgm-to-jpeg\npgm -> json: lossy via pgm-to-pdf -> pdf-to-markdown -> markdown-tables-to-json\npgm -> jsonl: lossy via pgm-to-pdf -> pdf-to-markdown -> markdown-to-csv -> csv-to-jsonl\npgm -> markdown: lossy via pgm-to-pdf -> pdf-to-markdown\npgm -> markdown-json: lossy via pgm-to-pdf -> pdf-to-markdown -> markdown-to-json\npgm -> numbers: lossy via pgm-to-pdf -> pdf-to-markdown -> markdown-to-numbers\npgm -> pages: lossy via pgm-to-pdf -> pdf-to-docx -> docx-to-pages\npgm -> pages-json: lossy via pgm-to-pdf -> pdf-to-docx -> docx-to-pages -> pages-to-json\npgm -> pam: conditional via pgm-to-pam\npgm -> pbm: lossy via pgm-to-pbm\npgm -> pdf: conditional via pgm-to-pdf\npgm -> png: conditional via pgm-to-png\npgm -> ppm: conditional via pgm-to-ppm\npgm -> qoi: conditional via pgm-to-qoi\npgm -> rtf: lossy via pgm-to-pdf -> pdf-to-docx -> docx-to-rtf\npgm -> text: lossy via pgm-to-pdf -> pdf-to-text\npgm -> tga: conditional via pgm-to-tga\npgm -> tiff: conditional via pgm-to-tiff\npgm -> toml: lossy via pgm-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-toml\npgm -> tsv: lossy via pgm-to-pdf -> pdf-to-markdown -> markdown-to-tsv\npgm -> webp: conditional via pgm-to-webp\npgm -> xlsx: lossy via pgm-to-pdf -> pdf-to-markdown -> markdown-to-xlsx\npgm -> xml: lossy via pgm-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-xml\npgm -> yaml: lossy via pgm-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-yaml\npng -> bmp: conditional via png-to-bmp\npng -> csv: lossy via png-to-pdf -> pdf-to-markdown -> markdown-to-csv\npng -> docx: lossy via png-to-pdf -> pdf-to-docx\npng -> heic: lossy via png-to-heic\npng -> html: lossy via png-to-pdf -> pdf-to-html\npng -> ico: conditional via png-to-ico\npng -> jpeg: lossy via png-to-jpeg\npng -> json: lossy via png-to-pdf -> pdf-to-markdown -> markdown-tables-to-json\npng -> jsonl: lossy via png-to-pdf -> pdf-to-markdown -> markdown-to-csv -> csv-to-jsonl\npng -> markdown: lossy via png-to-pdf -> pdf-to-markdown\npng -> markdown-json: lossy via png-to-pdf -> pdf-to-markdown -> markdown-to-json\npng -> numbers: lossy via png-to-pdf -> pdf-to-markdown -> markdown-to-numbers\npng -> pages: lossy via png-to-pdf -> pdf-to-docx -> docx-to-pages\npng -> pages-json: lossy via png-to-pdf -> pdf-to-docx -> docx-to-pages -> pages-to-json\npng -> pam: conditional via png-to-pam\npng -> pbm: lossy via png-to-pbm\npng -> pdf: conditional via png-to-pdf\npng -> pgm: conditional via png-to-pgm\npng -> ppm: conditional via png-to-ppm\npng -> qoi: conditional via png-to-qoi\npng -> rtf: lossy via png-to-pdf -> pdf-to-docx -> docx-to-rtf\npng -> text: lossy via png-to-pdf -> pdf-to-text\npng -> tga: conditional via png-to-tga\npng -> tiff: conditional via png-to-tiff\npng -> toml: lossy via png-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-toml\npng -> tsv: lossy via png-to-pdf -> pdf-to-markdown -> markdown-to-tsv\npng -> webp: conditional via png-to-webp\npng -> xlsx: lossy via png-to-pdf -> pdf-to-markdown -> markdown-to-xlsx\npng -> xml: lossy via png-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-xml\npng -> yaml: lossy via png-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-yaml\nppm -> bmp: conditional via ppm-to-bmp\nppm -> csv: lossy via ppm-to-pdf -> pdf-to-markdown -> markdown-to-csv\nppm -> docx: lossy via ppm-to-pdf -> pdf-to-docx\nppm -> heic: lossy via ppm-to-heic\nppm -> html: lossy via ppm-to-pdf -> pdf-to-html\nppm -> ico: conditional via ppm-to-ico\nppm -> jpeg: lossy via ppm-to-jpeg\nppm -> json: lossy via ppm-to-pdf -> pdf-to-markdown -> markdown-tables-to-json\nppm -> jsonl: lossy via ppm-to-pdf -> pdf-to-markdown -> markdown-to-csv -> csv-to-jsonl\nppm -> markdown: lossy via ppm-to-pdf -> pdf-to-markdown\nppm -> markdown-json: lossy via ppm-to-pdf -> pdf-to-markdown -> markdown-to-json\nppm -> numbers: lossy via ppm-to-pdf -> pdf-to-markdown -> markdown-to-numbers\nppm -> pages: lossy via ppm-to-pdf -> pdf-to-docx -> docx-to-pages\nppm -> pages-json: lossy via ppm-to-pdf -> pdf-to-docx -> docx-to-pages -> pages-to-json\nppm -> pam: conditional via ppm-to-pam\nppm -> pbm: lossy via ppm-to-pbm\nppm -> pdf: conditional via ppm-to-pdf\nppm -> pgm: conditional via ppm-to-pgm\nppm -> png: conditional via ppm-to-png\nppm -> qoi: conditional via ppm-to-qoi\nppm -> rtf: lossy via ppm-to-pdf -> pdf-to-docx -> docx-to-rtf\nppm -> text: lossy via ppm-to-pdf -> pdf-to-text\nppm -> tga: conditional via ppm-to-tga\nppm -> tiff: conditional via ppm-to-tiff\nppm -> toml: lossy via ppm-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-toml\nppm -> tsv: lossy via ppm-to-pdf -> pdf-to-markdown -> markdown-to-tsv\nppm -> webp: conditional via ppm-to-webp\nppm -> xlsx: lossy via ppm-to-pdf -> pdf-to-markdown -> markdown-to-xlsx\nppm -> xml: lossy via ppm-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-xml\nppm -> yaml: lossy via ppm-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-yaml\nqoi -> bmp: lossless via qoi-to-bmp\nqoi -> csv: lossy via qoi-to-pdf -> pdf-to-markdown -> markdown-to-csv\nqoi -> docx: lossy via qoi-to-pdf -> pdf-to-docx\nqoi -> heic: lossy via qoi-to-heic\nqoi -> html: lossy via qoi-to-pdf -> pdf-to-html\nqoi -> ico: conditional via qoi-to-ico\nqoi -> jpeg: lossy via qoi-to-jpeg\nqoi -> json: lossy via qoi-to-pdf -> pdf-to-markdown -> markdown-tables-to-json\nqoi -> jsonl: lossy via qoi-to-pdf -> pdf-to-markdown -> markdown-to-csv -> csv-to-jsonl\nqoi -> markdown: lossy via qoi-to-pdf -> pdf-to-markdown\nqoi -> markdown-json: lossy via qoi-to-pdf -> pdf-to-markdown -> markdown-to-json\nqoi -> numbers: lossy via qoi-to-pdf -> pdf-to-markdown -> markdown-to-numbers\nqoi -> pages: lossy via qoi-to-pdf -> pdf-to-docx -> docx-to-pages\nqoi -> pages-json: lossy via qoi-to-pdf -> pdf-to-docx -> docx-to-pages -> pages-to-json\nqoi -> pam: lossless via qoi-to-pam\nqoi -> pbm: lossy via qoi-to-pbm\nqoi -> pdf: lossless via qoi-to-pdf\nqoi -> pgm: conditional via qoi-to-pgm\nqoi -> png: lossless via qoi-to-png\nqoi -> ppm: conditional via qoi-to-ppm\nqoi -> rtf: lossy via qoi-to-pdf -> pdf-to-docx -> docx-to-rtf\nqoi -> text: lossy via qoi-to-pdf -> pdf-to-text\nqoi -> tga: lossless via qoi-to-tga\nqoi -> tiff: lossless via qoi-to-tiff\nqoi -> toml: lossy via qoi-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-toml\nqoi -> tsv: lossy via qoi-to-pdf -> pdf-to-markdown -> markdown-to-tsv\nqoi -> webp: lossless via qoi-to-webp\nqoi -> xlsx: lossy via qoi-to-pdf -> pdf-to-markdown -> markdown-to-xlsx\nqoi -> xml: lossy via qoi-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-xml\nqoi -> yaml: lossy via qoi-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-yaml\nrtf -> bmp: conditional via rtf-to-pdf -> pdf-to-bmp\nrtf -> csv: lossy via rtf-to-markdown -> markdown-to-csv\nrtf -> docx: lossy via rtf-to-docx\nrtf -> heic: lossy via rtf-to-pdf -> pdf-to-heic\nrtf -> html: lossy via rtf-to-html\nrtf -> ico: conditional via rtf-to-pdf -> pdf-to-ico\nrtf -> jpeg: lossy via rtf-to-pdf -> pdf-to-jpeg\nrtf -> json: lossy via rtf-to-markdown -> markdown-tables-to-json\nrtf -> jsonl: lossy via rtf-to-markdown -> markdown-to-csv -> csv-to-jsonl\nrtf -> markdown: lossy via rtf-to-markdown\nrtf -> markdown-json: lossy via rtf-to-markdown -> markdown-to-json\nrtf -> numbers: lossy via rtf-to-markdown -> markdown-to-numbers\nrtf -> pages: lossy via rtf-to-pages\nrtf -> pages-json: lossy via rtf-to-pages -> pages-to-json\nrtf -> pam: conditional via rtf-to-pdf -> pdf-to-pam\nrtf -> pbm: lossy via rtf-to-pdf -> pdf-to-pbm\nrtf -> pdf: conditional via rtf-to-pdf\nrtf -> pgm: conditional via rtf-to-pdf -> pdf-to-pgm\nrtf -> png: conditional via rtf-to-pdf -> pdf-to-png\nrtf -> ppm: conditional via rtf-to-pdf -> pdf-to-ppm\nrtf -> qoi: conditional via rtf-to-pdf -> pdf-to-qoi\nrtf -> text: lossy via rtf-to-text\nrtf -> tga: conditional via rtf-to-pdf -> pdf-to-tga\nrtf -> tiff: conditional via rtf-to-pdf -> pdf-to-tiff\nrtf -> toml: lossy via rtf-to-markdown -> markdown-tables-to-json -> json-to-toml\nrtf -> tsv: lossy via rtf-to-markdown -> markdown-to-tsv\nrtf -> webp: conditional via rtf-to-pdf -> pdf-to-webp\nrtf -> xlsx: lossy via rtf-to-markdown -> markdown-to-xlsx\nrtf -> xml: lossy via rtf-to-markdown -> markdown-tables-to-json -> json-to-xml\nrtf -> yaml: lossy via rtf-to-markdown -> markdown-tables-to-json -> json-to-yaml\ntext -> bmp: conditional via text-to-pdf -> pdf-to-bmp\ntext -> csv: lossy via text-to-markdown -> markdown-to-csv\ntext -> docx: conditional via text-to-docx\ntext -> heic: lossy via text-to-pdf -> pdf-to-heic\ntext -> html: conditional via text-to-html\ntext -> ico: conditional via text-to-pdf -> pdf-to-ico\ntext -> jpeg: lossy via text-to-pdf -> pdf-to-jpeg\ntext -> json: lossy via text-to-markdown -> markdown-tables-to-json\ntext -> jsonl: lossy via text-to-markdown -> markdown-to-csv -> csv-to-jsonl\ntext -> markdown: conditional via text-to-markdown\ntext -> markdown-json: conditional via text-to-markdown -> markdown-to-json\ntext -> numbers: lossy via text-to-markdown -> markdown-to-numbers\ntext -> pages: lossy via text-to-pages\ntext -> pages-json: lossy via text-to-pages -> pages-to-json\ntext -> pam: conditional via text-to-pdf -> pdf-to-pam\ntext -> pbm: lossy via text-to-pdf -> pdf-to-pbm\ntext -> pdf: conditional via text-to-pdf\ntext -> pgm: conditional via text-to-pdf -> pdf-to-pgm\ntext -> png: conditional via text-to-pdf -> pdf-to-png\ntext -> ppm: conditional via text-to-pdf -> pdf-to-ppm\ntext -> qoi: conditional via text-to-pdf -> pdf-to-qoi\ntext -> rtf: lossy via text-to-rtf\ntext -> tga: conditional via text-to-pdf -> pdf-to-tga\ntext -> tiff: conditional via text-to-pdf -> pdf-to-tiff\ntext -> toml: lossy via text-to-markdown -> markdown-tables-to-json -> json-to-toml\ntext -> tsv: lossy via text-to-markdown -> markdown-to-tsv\ntext -> webp: conditional via text-to-pdf -> pdf-to-webp\ntext -> xlsx: lossy via text-to-markdown -> markdown-to-xlsx\ntext -> xml: lossy via text-to-markdown -> markdown-tables-to-json -> json-to-xml\ntext -> yaml: lossy via text-to-markdown -> markdown-tables-to-json -> json-to-yaml\ntga -> bmp: conditional via tga-to-bmp\ntga -> csv: lossy via tga-to-pdf -> pdf-to-markdown -> markdown-to-csv\ntga -> docx: lossy via tga-to-pdf -> pdf-to-docx\ntga -> heic: lossy via tga-to-heic\ntga -> html: lossy via tga-to-pdf -> pdf-to-html\ntga -> ico: conditional via tga-to-ico\ntga -> jpeg: lossy via tga-to-jpeg\ntga -> json: lossy via tga-to-pdf -> pdf-to-markdown -> markdown-tables-to-json\ntga -> jsonl: lossy via tga-to-pdf -> pdf-to-markdown -> markdown-to-csv -> csv-to-jsonl\ntga -> markdown: lossy via tga-to-pdf -> pdf-to-markdown\ntga -> markdown-json: lossy via tga-to-pdf -> pdf-to-markdown -> markdown-to-json\ntga -> numbers: lossy via tga-to-pdf -> pdf-to-markdown -> markdown-to-numbers\ntga -> pages: lossy via tga-to-pdf -> pdf-to-docx -> docx-to-pages\ntga -> pages-json: lossy via tga-to-pdf -> pdf-to-docx -> docx-to-pages -> pages-to-json\ntga -> pam: conditional via tga-to-pam\ntga -> pbm: lossy via tga-to-pbm\ntga -> pdf: conditional via tga-to-pdf\ntga -> pgm: conditional via tga-to-pgm\ntga -> png: conditional via tga-to-png\ntga -> ppm: conditional via tga-to-ppm\ntga -> qoi: conditional via tga-to-qoi\ntga -> rtf: lossy via tga-to-pdf -> pdf-to-docx -> docx-to-rtf\ntga -> text: lossy via tga-to-pdf -> pdf-to-text\ntga -> tiff: conditional via tga-to-tiff\ntga -> toml: lossy via tga-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-toml\ntga -> tsv: lossy via tga-to-pdf -> pdf-to-markdown -> markdown-to-tsv\ntga -> webp: conditional via tga-to-webp\ntga -> xlsx: lossy via tga-to-pdf -> pdf-to-markdown -> markdown-to-xlsx\ntga -> xml: lossy via tga-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-xml\ntga -> yaml: lossy via tga-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-yaml\ntiff -> bmp: conditional via tiff-to-bmp\ntiff -> csv: lossy via tiff-to-pdf -> pdf-to-markdown -> markdown-to-csv\ntiff -> docx: lossy via tiff-to-pdf -> pdf-to-docx\ntiff -> heic: lossy via tiff-to-heic\ntiff -> html: lossy via tiff-to-pdf -> pdf-to-html\ntiff -> ico: conditional via tiff-to-ico\ntiff -> jpeg: lossy via tiff-to-jpeg\ntiff -> json: lossy via tiff-to-pdf -> pdf-to-markdown -> markdown-tables-to-json\ntiff -> jsonl: lossy via tiff-to-pdf -> pdf-to-markdown -> markdown-to-csv -> csv-to-jsonl\ntiff -> markdown: lossy via tiff-to-pdf -> pdf-to-markdown\ntiff -> markdown-json: lossy via tiff-to-pdf -> pdf-to-markdown -> markdown-to-json\ntiff -> numbers: lossy via tiff-to-pdf -> pdf-to-markdown -> markdown-to-numbers\ntiff -> pages: lossy via tiff-to-pdf -> pdf-to-docx -> docx-to-pages\ntiff -> pages-json: lossy via tiff-to-pdf -> pdf-to-docx -> docx-to-pages -> pages-to-json\ntiff -> pam: conditional via tiff-to-pam\ntiff -> pbm: lossy via tiff-to-pbm\ntiff -> pdf: conditional via tiff-to-pdf\ntiff -> pgm: conditional via tiff-to-pgm\ntiff -> png: conditional via tiff-to-png\ntiff -> ppm: conditional via tiff-to-ppm\ntiff -> qoi: conditional via tiff-to-qoi\ntiff -> rtf: lossy via tiff-to-pdf -> pdf-to-docx -> docx-to-rtf\ntiff -> text: lossy via tiff-to-pdf -> pdf-to-text\ntiff -> tga: conditional via tiff-to-tga\ntiff -> toml: lossy via tiff-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-toml\ntiff -> tsv: lossy via tiff-to-pdf -> pdf-to-markdown -> markdown-to-tsv\ntiff -> webp: conditional via tiff-to-webp\ntiff -> xlsx: lossy via tiff-to-pdf -> pdf-to-markdown -> markdown-to-xlsx\ntiff -> xml: lossy via tiff-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-xml\ntiff -> yaml: lossy via tiff-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-yaml\ntoml -> bmp: conditional via toml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-bmp\ntoml -> csv: conditional via toml-to-json -> json-to-csv\ntoml -> docx: conditional via toml-to-json -> json-to-markdown -> markdown-to-docx\ntoml -> heic: lossy via toml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-heic\ntoml -> html: conditional via toml-to-json -> json-to-markdown -> markdown-to-html\ntoml -> ico: conditional via toml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-ico\ntoml -> jpeg: lossy via toml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-jpeg\ntoml -> json: conditional via toml-to-json\ntoml -> jsonl: conditional via toml-to-json -> json-to-jsonl\ntoml -> markdown: conditional via toml-to-json -> json-to-markdown\ntoml -> markdown-json: conditional via toml-to-json -> json-to-markdown -> markdown-to-json\ntoml -> numbers: conditional via toml-to-json -> json-to-numbers\ntoml -> pages: lossy via toml-to-json -> json-to-markdown -> markdown-to-pages\ntoml -> pages-json: lossy via toml-to-json -> json-to-markdown -> markdown-to-pages -> pages-to-json\ntoml -> pam: conditional via toml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-pam\ntoml -> pbm: lossy via toml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-pbm\ntoml -> pdf: conditional via toml-to-json -> json-to-markdown -> markdown-to-pdf\ntoml -> pgm: conditional via toml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-pgm\ntoml -> png: conditional via toml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-png\ntoml -> ppm: conditional via toml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-ppm\ntoml -> qoi: conditional via toml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-qoi\ntoml -> rtf: lossy via toml-to-json -> json-to-markdown -> markdown-to-rtf\ntoml -> text: lossy via toml-to-json -> json-to-markdown -> markdown-to-text\ntoml -> tga: conditional via toml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-tga\ntoml -> tiff: conditional via toml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-tiff\ntoml -> tsv: conditional via toml-to-json -> json-to-tsv\ntoml -> webp: conditional via toml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-webp\ntoml -> xlsx: conditional via toml-to-json -> json-to-xlsx\ntoml -> xml: conditional via toml-to-xml\ntoml -> yaml: conditional via toml-to-yaml\ntsv -> bmp: conditional via tsv-to-markdown -> markdown-to-pdf -> pdf-to-bmp\ntsv -> csv: lossless via tsv-to-csv\ntsv -> docx: conditional via tsv-to-markdown -> markdown-to-docx\ntsv -> heic: lossy via tsv-to-markdown -> markdown-to-pdf -> pdf-to-heic\ntsv -> html: conditional via tsv-to-markdown -> markdown-to-html\ntsv -> ico: conditional via tsv-to-markdown -> markdown-to-pdf -> pdf-to-ico\ntsv -> jpeg: lossy via tsv-to-markdown -> markdown-to-pdf -> pdf-to-jpeg\ntsv -> json: lossless via tsv-to-json\ntsv -> jsonl: lossless via tsv-to-jsonl\ntsv -> markdown: conditional via tsv-to-markdown\ntsv -> markdown-json: conditional via tsv-to-markdown -> markdown-to-json\ntsv -> numbers: conditional via tsv-to-numbers\ntsv -> pages: lossy via tsv-to-markdown -> markdown-to-pages\ntsv -> pages-json: lossy via tsv-to-markdown -> markdown-to-pages -> pages-to-json\ntsv -> pam: conditional via tsv-to-markdown -> markdown-to-pdf -> pdf-to-pam\ntsv -> pbm: lossy via tsv-to-markdown -> markdown-to-pdf -> pdf-to-pbm\ntsv -> pdf: conditional via tsv-to-markdown -> markdown-to-pdf\ntsv -> pgm: conditional via tsv-to-markdown -> markdown-to-pdf -> pdf-to-pgm\ntsv -> png: conditional via tsv-to-markdown -> markdown-to-pdf -> pdf-to-png\ntsv -> ppm: conditional via tsv-to-markdown -> markdown-to-pdf -> pdf-to-ppm\ntsv -> qoi: conditional via tsv-to-markdown -> markdown-to-pdf -> pdf-to-qoi\ntsv -> rtf: lossy via tsv-to-markdown -> markdown-to-rtf\ntsv -> text: lossy via tsv-to-markdown -> markdown-to-text\ntsv -> tga: conditional via tsv-to-markdown -> markdown-to-pdf -> pdf-to-tga\ntsv -> tiff: conditional via tsv-to-markdown -> markdown-to-pdf -> pdf-to-tiff\ntsv -> toml: conditional via tsv-to-json -> json-to-toml\ntsv -> webp: conditional via tsv-to-markdown -> markdown-to-pdf -> pdf-to-webp\ntsv -> xlsx: conditional via tsv-to-xlsx\ntsv -> xml: conditional via tsv-to-json -> json-to-xml\ntsv -> yaml: lossless via tsv-to-json -> json-to-yaml\nwebp -> bmp: conditional via webp-to-bmp\nwebp -> csv: lossy via webp-to-pdf -> pdf-to-markdown -> markdown-to-csv\nwebp -> docx: lossy via webp-to-pdf -> pdf-to-docx\nwebp -> heic: lossy via webp-to-heic\nwebp -> html: lossy via webp-to-pdf -> pdf-to-html\nwebp -> ico: conditional via webp-to-ico\nwebp -> jpeg: lossy via webp-to-jpeg\nwebp -> json: lossy via webp-to-pdf -> pdf-to-markdown -> markdown-tables-to-json\nwebp -> jsonl: lossy via webp-to-pdf -> pdf-to-markdown -> markdown-to-csv -> csv-to-jsonl\nwebp -> markdown: lossy via webp-to-pdf -> pdf-to-markdown\nwebp -> markdown-json: lossy via webp-to-pdf -> pdf-to-markdown -> markdown-to-json\nwebp -> numbers: lossy via webp-to-pdf -> pdf-to-markdown -> markdown-to-numbers\nwebp -> pages: lossy via webp-to-pdf -> pdf-to-docx -> docx-to-pages\nwebp -> pages-json: lossy via webp-to-pdf -> pdf-to-docx -> docx-to-pages -> pages-to-json\nwebp -> pam: conditional via webp-to-pam\nwebp -> pbm: lossy via webp-to-pbm\nwebp -> pdf: conditional via webp-to-pdf\nwebp -> pgm: conditional via webp-to-pgm\nwebp -> png: conditional via webp-to-png\nwebp -> ppm: conditional via webp-to-ppm\nwebp -> qoi: conditional via webp-to-qoi\nwebp -> rtf: lossy via webp-to-pdf -> pdf-to-docx -> docx-to-rtf\nwebp -> text: lossy via webp-to-pdf -> pdf-to-text\nwebp -> tga: conditional via webp-to-tga\nwebp -> tiff: conditional via webp-to-tiff\nwebp -> toml: lossy via webp-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-toml\nwebp -> tsv: lossy via webp-to-pdf -> pdf-to-markdown -> markdown-to-tsv\nwebp -> xlsx: lossy via webp-to-pdf -> pdf-to-markdown -> markdown-to-xlsx\nwebp -> xml: lossy via webp-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-xml\nwebp -> yaml: lossy via webp-to-pdf -> pdf-to-markdown -> markdown-tables-to-json -> json-to-yaml\nxlsx -> bmp: conditional via xlsx-to-markdown -> markdown-to-pdf -> pdf-to-bmp\nxlsx -> csv: conditional via xlsx-to-csv\nxlsx -> docx: conditional via xlsx-to-markdown -> markdown-to-docx\nxlsx -> heic: lossy via xlsx-to-markdown -> markdown-to-pdf -> pdf-to-heic\nxlsx -> html: conditional via xlsx-to-markdown -> markdown-to-html\nxlsx -> ico: conditional via xlsx-to-markdown -> markdown-to-pdf -> pdf-to-ico\nxlsx -> jpeg: lossy via xlsx-to-markdown -> markdown-to-pdf -> pdf-to-jpeg\nxlsx -> json: conditional via xlsx-to-json\nxlsx -> jsonl: conditional via xlsx-to-csv -> csv-to-jsonl\nxlsx -> markdown: conditional via xlsx-to-markdown\nxlsx -> markdown-json: conditional via xlsx-to-markdown -> markdown-to-json\nxlsx -> numbers: conditional via xlsx-to-numbers\nxlsx -> pages: lossy via xlsx-to-markdown -> markdown-to-pages\nxlsx -> pages-json: lossy via xlsx-to-markdown -> markdown-to-pages -> pages-to-json\nxlsx -> pam: conditional via xlsx-to-markdown -> markdown-to-pdf -> pdf-to-pam\nxlsx -> pbm: lossy via xlsx-to-markdown -> markdown-to-pdf -> pdf-to-pbm\nxlsx -> pdf: conditional via xlsx-to-markdown -> markdown-to-pdf\nxlsx -> pgm: conditional via xlsx-to-markdown -> markdown-to-pdf -> pdf-to-pgm\nxlsx -> png: conditional via xlsx-to-markdown -> markdown-to-pdf -> pdf-to-png\nxlsx -> ppm: conditional via xlsx-to-markdown -> markdown-to-pdf -> pdf-to-ppm\nxlsx -> qoi: conditional via xlsx-to-markdown -> markdown-to-pdf -> pdf-to-qoi\nxlsx -> rtf: lossy via xlsx-to-markdown -> markdown-to-rtf\nxlsx -> text: lossy via xlsx-to-markdown -> markdown-to-text\nxlsx -> tga: conditional via xlsx-to-markdown -> markdown-to-pdf -> pdf-to-tga\nxlsx -> tiff: conditional via xlsx-to-markdown -> markdown-to-pdf -> pdf-to-tiff\nxlsx -> toml: conditional via xlsx-to-json -> json-to-toml\nxlsx -> tsv: conditional via xlsx-to-tsv\nxlsx -> webp: conditional via xlsx-to-markdown -> markdown-to-pdf -> pdf-to-webp\nxlsx -> xml: conditional via xlsx-to-json -> json-to-xml\nxlsx -> yaml: conditional via xlsx-to-json -> json-to-yaml\nxml -> bmp: conditional via xml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-bmp\nxml -> csv: conditional via xml-to-json -> json-to-csv\nxml -> docx: conditional via xml-to-json -> json-to-markdown -> markdown-to-docx\nxml -> heic: lossy via xml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-heic\nxml -> html: conditional via xml-to-json -> json-to-markdown -> markdown-to-html\nxml -> ico: conditional via xml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-ico\nxml -> jpeg: lossy via xml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-jpeg\nxml -> json: conditional via xml-to-json\nxml -> jsonl: conditional via xml-to-json -> json-to-jsonl\nxml -> markdown: conditional via xml-to-json -> json-to-markdown\nxml -> markdown-json: conditional via xml-to-json -> json-to-markdown -> markdown-to-json\nxml -> numbers: conditional via xml-to-json -> json-to-numbers\nxml -> pages: lossy via xml-to-json -> json-to-markdown -> markdown-to-pages\nxml -> pages-json: lossy via xml-to-json -> json-to-markdown -> markdown-to-pages -> pages-to-json\nxml -> pam: conditional via xml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-pam\nxml -> pbm: lossy via xml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-pbm\nxml -> pdf: conditional via xml-to-json -> json-to-markdown -> markdown-to-pdf\nxml -> pgm: conditional via xml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-pgm\nxml -> png: conditional via xml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-png\nxml -> ppm: conditional via xml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-ppm\nxml -> qoi: conditional via xml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-qoi\nxml -> rtf: lossy via xml-to-json -> json-to-markdown -> markdown-to-rtf\nxml -> text: lossy via xml-to-json -> json-to-markdown -> markdown-to-text\nxml -> tga: conditional via xml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-tga\nxml -> tiff: conditional via xml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-tiff\nxml -> toml: conditional via xml-to-toml\nxml -> tsv: conditional via xml-to-json -> json-to-tsv\nxml -> webp: conditional via xml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-webp\nxml -> xlsx: conditional via xml-to-json -> json-to-xlsx\nxml -> yaml: conditional via xml-to-yaml\nyaml -> bmp: conditional via yaml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-bmp\nyaml -> csv: conditional via yaml-to-json -> json-to-csv\nyaml -> docx: conditional via yaml-to-json -> json-to-markdown -> markdown-to-docx\nyaml -> heic: lossy via yaml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-heic\nyaml -> html: conditional via yaml-to-json -> json-to-markdown -> markdown-to-html\nyaml -> ico: conditional via yaml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-ico\nyaml -> jpeg: lossy via yaml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-jpeg\nyaml -> json: conditional via yaml-to-json\nyaml -> jsonl: conditional via yaml-to-json -> json-to-jsonl\nyaml -> markdown: conditional via yaml-to-json -> json-to-markdown\nyaml -> markdown-json: conditional via yaml-to-json -> json-to-markdown -> markdown-to-json\nyaml -> numbers: conditional via yaml-to-json -> json-to-numbers\nyaml -> pages: lossy via yaml-to-json -> json-to-markdown -> markdown-to-pages\nyaml -> pages-json: lossy via yaml-to-json -> json-to-markdown -> markdown-to-pages -> pages-to-json\nyaml -> pam: conditional via yaml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-pam\nyaml -> pbm: lossy via yaml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-pbm\nyaml -> pdf: conditional via yaml-to-json -> json-to-markdown -> markdown-to-pdf\nyaml -> pgm: conditional via yaml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-pgm\nyaml -> png: conditional via yaml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-png\nyaml -> ppm: conditional via yaml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-ppm\nyaml -> qoi: conditional via yaml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-qoi\nyaml -> rtf: lossy via yaml-to-json -> json-to-markdown -> markdown-to-rtf\nyaml -> text: lossy via yaml-to-json -> json-to-markdown -> markdown-to-text\nyaml -> tga: conditional via yaml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-tga\nyaml -> tiff: conditional via yaml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-tiff\nyaml -> toml: conditional via yaml-to-toml\nyaml -> tsv: conditional via yaml-to-json -> json-to-tsv\nyaml -> webp: conditional via yaml-to-json -> json-to-markdown -> markdown-to-pdf -> pdf-to-webp\nyaml -> xlsx: conditional via yaml-to-json -> json-to-xlsx\nyaml -> xml: conditional via yaml-to-xml\n"
    );

    let markdown = run(&["paths", "--markdown"]);
    assert!(stdout(&markdown).starts_with("# Formats\n"));
    assert!(!stdout(&markdown).contains("## Paths"));
    assert!(stdout(&markdown).contains("```mermaid\ngraph LR\n"));
    assert!(stdout(&markdown).contains("  pages --- docx\n"));

    let conversions = run(&["paths", "--conversions"]);
    assert_eq!(code(&conversions), 0);
    assert!(stdout(&conversions).starts_with("# Conversions\n"));
    assert!(stdout(&conversions).contains("## Converters\n"));
    assert!(stdout(&conversions).contains("| csv | json | lossless | csv-to-json |"));
}

#[test]
fn version_help_and_usage_errors() {
    let version = run(&["version"]);
    assert_eq!(
        stdout(&version),
        format!("sublime {}\n", env!("CARGO_PKG_VERSION"))
    );
    let help = run(&["--help"]);
    assert_eq!(code(&help), 0);
    assert!(stdout(&help).contains("USAGE"));
    let nothing = run(&[]);
    assert_eq!(code(&nothing), 4);
    let bogus = run(&["bogus"]);
    assert_eq!(code(&bogus), 4);
    let missing_input = run(&["convert"]);
    assert_eq!(code(&missing_input), 4);
}

#[test]
fn missing_input_file_is_an_error() {
    let output = run(&[
        "convert",
        "/nonexistent/definitely/missing.csv",
        "--to",
        "json",
    ]);
    assert_eq!(code(&output), 1);
    assert!(stderr(&output).contains("opening"));
}

#[test]
fn markdown_to_html_via_extension() {
    let input = fixture("markdown/sample.md");
    let output = run(&["convert", input.to_str().unwrap(), "--to", "html"]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "<h1>Sample</h1>\n<p>A paragraph with <em>emphasis</em> and a <a href=\"https://example.com\">link</a>.</p>\n<ul>\n<li>one</li>\n<li>two</li>\n</ul>\n<table>\n<thead>\n<tr>\n<th>a</th>\n<th>b</th>\n</tr>\n</thead>\n<tbody>\n<tr>\n<td>1</td>\n<td>2</td>\n</tr>\n</tbody>\n</table>\n"
    );
    let check = run(&["check", "markdown", "html"]);
    assert_eq!(code(&check), 0);
    assert_eq!(
        stdout(&check),
        "markdown -> html\n  1. markdown-to-html (native, lossless)\nfidelity: lossless\n"
    );
}

#[test]
fn markdown_to_text_via_extension() {
    let input = fixture("markdown/sample.md");
    let output = run(&["convert", input.to_str().unwrap(), "--to", "text"]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "Sample\n\nA paragraph with emphasis and a link (https://example.com).\n\n- one\n- two\n\na  b\n-  -\n1  2\n"
    );
}

#[test]
fn markdown_round_trips_through_json() {
    let input = fixture("markdown/sample.md");
    let json_path = temp_path("sample.events.json");
    let to_json = run(&[
        "convert",
        input.to_str().unwrap(),
        json_path.to_str().unwrap(),
        "--to",
        "markdown-json",
    ]);
    assert_eq!(code(&to_json), 0, "stderr: {}", stderr(&to_json));
    let back = run(&[
        "convert",
        json_path.to_str().unwrap(),
        "--from",
        "markdown-json",
        "--to",
        "markdown",
    ]);
    assert_eq!(code(&back), 0, "stderr: {}", stderr(&back));
    let original = std::fs::read_to_string(&input).unwrap();
    assert_eq!(stdout(&back), original);
    let check = run(&["check", "markdown-json", "html"]);
    assert_eq!(code(&check), 0);
    assert!(stdout(&check).contains("markdown-json-to-markdown"));
    std::fs::remove_file(json_path).ok();
}

#[test]
fn pages_round_trips_through_json_on_the_command_line() {
    let input = fixture("pages/text-styles.pages");
    let json_path = temp_path("text-styles.pages.json");
    let pages_path = temp_path("text-styles-again.pages");
    let to_json = run(&[
        "convert",
        input.to_str().unwrap(),
        json_path.to_str().unwrap(),
        "--to",
        "pages-json",
    ]);
    assert_eq!(code(&to_json), 0, "stderr: {}", stderr(&to_json));
    let json = std::fs::read_to_string(&json_path).unwrap();
    assert!(json.starts_with("{\"format\":\"pages-json\""));
    assert!(json.contains("\"@type\":\"TSWP.StorageArchive\""));
    assert!(json.contains("Heading One"));
    let back = run(&[
        "convert",
        json_path.to_str().unwrap(),
        pages_path.to_str().unwrap(),
        "--from",
        "pages-json",
    ]);
    assert_eq!(code(&back), 0, "stderr: {}", stderr(&back));
    let again = run(&[
        "convert",
        pages_path.to_str().unwrap(),
        "--to",
        "pages-json",
    ]);
    assert_eq!(code(&again), 0, "stderr: {}", stderr(&again));
    assert_eq!(stdout(&again), json);
    std::fs::remove_file(json_path).ok();
    std::fs::remove_file(pages_path).ok();
}

#[test]
fn pages_to_docx_via_extension() {
    let input = fixture("pages/text-styles.pages");
    let output_path = temp_path("text-styles.docx");
    let output = run(&[
        "convert",
        input.to_str().unwrap(),
        output_path.to_str().unwrap(),
    ]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let bytes = std::fs::read(&output_path).unwrap();
    assert!(bytes.starts_with(b"PK"));
    assert!(bytes.len() > 2_000);
    std::fs::remove_file(output_path).ok();
}

// ---- batch conversion ----

fn batch_dir(name: &str) -> PathBuf {
    let dir = temp_path(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("in/sub")).unwrap();
    fs::write(dir.join("in/a.csv"), "x,y\n1,2\n").unwrap();
    fs::write(dir.join("in/b.csv"), "x,y\n3,4\n").unwrap();
    fs::write(dir.join("in/sub/c.toml"), "k = 1\n").unwrap();
    fs::write(dir.join("in/notes.zzz"), "not a format").unwrap();
    dir
}

#[test]
fn batch_converts_a_directory_recursively_into_an_out_dir() {
    let dir = batch_dir("batch-dir");
    let out = run(&[
        "convert",
        dir.join("in").to_str().unwrap(),
        "-r",
        "--to",
        "json",
        "--out-dir",
        dir.join("out").to_str().unwrap(),
    ]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(
        fs::read_to_string(dir.join("out/a.json")).unwrap(),
        r#"[{"x":"1","y":"2"}]"#
    );
    assert_eq!(
        fs::read_to_string(dir.join("out/b.json")).unwrap(),
        r#"[{"x":"3","y":"4"}]"#
    );
    assert_eq!(
        fs::read_to_string(dir.join("out/sub/c.json")).unwrap(),
        r#"{"k":1}"#
    );
    assert!(!dir.join("out/notes.json").exists());
    assert!(
        stderr(&out).contains("3 converted, 1 skipped"),
        "{}",
        stderr(&out)
    );
    assert!(!dir.join("out/a.json.part").exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn two_inputs_are_not_mistaken_for_an_output_and_a_trailing_directory_is_a_batch() {
    let dir = batch_dir("batch-beside");
    let a = dir.join("in/a.csv");
    let b = dir.join("in/b.csv");
    // The old reading, "write a.csv's TSV over b.csv", is refused.
    let out = run(&[
        "convert",
        a.to_str().unwrap(),
        b.to_str().unwrap(),
        "--to",
        "tsv",
    ]);
    assert_eq!(code(&out), 4, "{}", stderr(&out));
    assert!(stderr(&out).contains("--out-dir"), "{}", stderr(&out));
    assert_eq!(fs::read_to_string(&b).unwrap(), "x,y\n3,4\n");
    let mut target = dir.join("target").into_os_string();
    target.push("/");
    let out = run(&[
        "convert",
        a.to_str().unwrap(),
        b.to_str().unwrap(),
        target.to_str().unwrap(),
        "--to",
        "yaml",
    ]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(
        fs::read_to_string(dir.join("target/b.yaml")).unwrap(),
        "- x: \"3\"\n  y: \"4\"\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn batch_expands_a_quoted_glob_and_dry_run_writes_nothing() {
    let dir = batch_dir("batch-glob");
    let pattern = format!("{}/in/*.csv", dir.display());
    let out = run(&["convert", &pattern, "--to", "json", "--dry-run"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stderr(&out);
    assert!(
        text.contains("a.csv ->") && text.contains("b.csv ->"),
        "{text}"
    );
    assert!(text.contains("2 would be written (dry run)"), "{text}");
    assert!(!dir.join("in/a.json").exists());
    let out = run(&["convert", &pattern, "--to", "json"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(dir.join("in/a.json").exists() && dir.join("in/b.json").exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn batch_goes_on_after_a_bad_file_and_refuses_collisions_and_missing_to() {
    let dir = batch_dir("batch-errors");
    fs::write(dir.join("in/bad.csv"), "\"never closed\n").unwrap();
    let out = run(&[
        "convert",
        dir.join("in").to_str().unwrap(),
        "--to",
        "json",
        "--out-dir",
        dir.join("out").to_str().unwrap(),
    ]);
    assert_eq!(code(&out), 1, "{}", stderr(&out));
    assert!(stderr(&out).contains("failed: "), "{}", stderr(&out));
    assert!(dir.join("out/a.json").exists());
    assert!(!dir.join("out/bad.json").exists());
    assert!(!dir.join("out/bad.json.part").exists());
    fs::write(dir.join("in/a.tsv"), "x\ty\n").unwrap();
    let out = run(&[
        "convert",
        dir.join("in/a.csv").to_str().unwrap(),
        dir.join("in/a.tsv").to_str().unwrap(),
        "--to",
        "json",
        "--out-dir",
        dir.join("out").to_str().unwrap(),
    ]);
    assert_eq!(code(&out), 4, "{}", stderr(&out));
    assert!(
        stderr(&out).contains("would both write"),
        "{}",
        stderr(&out)
    );
    let out = run(&[
        "convert",
        dir.join("in/a.csv").to_str().unwrap(),
        dir.join("in/b.csv").to_str().unwrap(),
        "--out-dir",
        dir.join("out").to_str().unwrap(),
    ]);
    assert_eq!(code(&out), 4, "{}", stderr(&out));
    assert!(stderr(&out).contains("--to"), "{}", stderr(&out));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn images_merge_into_one_pdf_a_page_each() {
    let dir = batch_dir("merge");
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let png = fixtures.join("png/basn2c08.png");
    let jpeg = fixtures.join("jpeg/photo-200x130-q75-420.jpg");
    let qoi = fixtures.join("qoi/photo-64x64-rgba.qoi");
    let output = dir.join("scan.pdf");
    let result = run(&[
        "-q",
        "convert",
        png.to_str().unwrap(),
        jpeg.to_str().unwrap(),
        qoi.to_str().unwrap(),
        output.to_str().unwrap(),
    ]);
    assert_eq!(
        code(&result),
        0,
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let pdf = fs::read(&output).unwrap();
    let text = String::from_utf8_lossy(&pdf);
    assert!(text.starts_with("%PDF-1.7"));
    assert!(text.contains("/Count 3"));
    assert!(text.contains("/MediaBox [0 0 32 32]"));
    assert!(text.contains("/MediaBox [0 0 200 130]"));
    assert!(text.contains("/MediaBox [0 0 64 64]"));
    assert!(text.contains("/SMask"));
    assert!(!dir.join("scan.pdf.part").exists());

    // A document among the inputs is refused before anything is written.
    let markdown = dir.join("notes.md");
    fs::write(&markdown, "# notes\n").unwrap();
    let refused = dir.join("refused.pdf");
    let result = run(&[
        "convert",
        png.to_str().unwrap(),
        markdown.to_str().unwrap(),
        refused.to_str().unwrap(),
    ]);
    assert_eq!(code(&result), 4);
    assert!(!refused.exists());
}

#[test]
fn a_workbook_into_csv_is_a_folder_of_sheets() {
    let input = fixture("xlsx/types.xlsx");
    let directory = temp_path("workbook");
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let output_path = directory.join("types.csv");
    let output = run(&[
        "convert",
        input.to_str().unwrap(),
        output_path.to_str().unwrap(),
    ]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    assert!(
        stderr(&output).contains("2 files in"),
        "{}",
        stderr(&output)
    );
    let folder = directory.join("types");
    let first = std::fs::read_to_string(folder.join("Data.csv")).unwrap();
    let second = std::fs::read_to_string(folder.join("Other sheet.csv")).unwrap();
    assert_eq!(
        first,
        std::fs::read_to_string(fixture("xlsx/types.csv")).unwrap()
    );
    assert_eq!(
        second,
        std::fs::read_to_string(fixture("xlsx/types.sheet2.csv")).unwrap()
    );
    assert!(!output_path.exists());
    // A workbook of one sheet is one file, as before.
    let single = directory.join("date1904.csv");
    let output = run(&[
        "convert",
        fixture("xlsx/date1904.xlsx").to_str().unwrap(),
        single.to_str().unwrap(),
    ]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    assert!(single.is_file());
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn a_workbook_on_stdout_is_its_first_sheet_and_a_loss() {
    let input = fixture("xlsx/types.xlsx");
    let output = run(&["convert", input.to_str().unwrap(), "--to", "csv"]);
    assert_eq!(code(&output), 2, "stderr: {}", stderr(&output));
    assert_eq!(
        stdout(&output),
        std::fs::read_to_string(fixture("xlsx/types.csv")).unwrap()
    );
    assert!(stderr(&output).contains("left out 1 more (Other sheet)"));
    // Picking the sheet is one part, no loss.
    let output = run(&[
        "convert",
        input.to_str().unwrap(),
        "--to",
        "csv",
        "--sheet",
        "Other sheet",
    ]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
}

#[test]
fn a_package_saved_as_a_folder_is_one_document() {
    // Unpack the Numbers fixture into `sheets.numbers/`, as Numbers saves
    // a package when asked for a folder.
    let bytes = std::fs::read(fixture("numbers/sheets.numbers")).unwrap();
    let archive = sublime::io::zip::ZipArchive::parse(&bytes).unwrap();
    let directory = temp_path("package-folder");
    let _ = std::fs::remove_dir_all(&directory);
    let package = directory.join("sheets.numbers");
    for entry in archive.entries() {
        if entry.is_directory() {
            continue;
        }
        let path = package.join(&entry.name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut data = Vec::new();
        archive.read(entry, &mut data).unwrap();
        std::fs::write(path, data).unwrap();
    }
    let output = run(&["convert", package.to_str().unwrap(), "-", "--to", "json"]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).starts_with("{\"ZZZ_Sheet_1 - ZZZ_Table_1\":["),
        "{}",
        stdout(&output)
    );
    // Among other inputs, the folder is still one document.
    let out_dir = directory.join("out");
    let batch = run(&[
        "convert",
        directory.to_str().unwrap(),
        "--to",
        "json",
        "--out-dir",
        out_dir.to_str().unwrap(),
    ]);
    assert_eq!(code(&batch), 0, "stderr: {}", stderr(&batch));
    assert!(out_dir.join("sheets.json").is_file());
    let _ = std::fs::remove_dir_all(&directory);
}
