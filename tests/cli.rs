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
        "bmp -> csv: lossy via bmp-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv\nbmp -> docx: lossy via bmp-to-pdf -> pdf-to-text -> text-to-docx\nbmp -> html: lossy via bmp-to-pdf -> pdf-to-text -> text-to-html\nbmp -> ico: conditional via bmp-to-ico\nbmp -> jpeg: lossy via bmp-to-jpeg\nbmp -> json: lossy via bmp-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json\nbmp -> jsonl: lossy via bmp-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-jsonl\nbmp -> markdown: lossy via bmp-to-pdf -> pdf-to-text -> text-to-markdown\nbmp -> markdown-json: lossy via bmp-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-json\nbmp -> pam: lossless via bmp-to-pam\nbmp -> pbm: lossy via bmp-to-pbm\nbmp -> pdf: lossless via bmp-to-pdf\nbmp -> pgm: conditional via bmp-to-pgm\nbmp -> png: lossless via bmp-to-png\nbmp -> ppm: conditional via bmp-to-ppm\nbmp -> qoi: lossless via bmp-to-qoi\nbmp -> text: lossy via bmp-to-pdf -> pdf-to-text\nbmp -> tga: lossless via bmp-to-tga\nbmp -> tiff: lossless via bmp-to-tiff\nbmp -> toml: lossy via bmp-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-toml\nbmp -> tsv: lossy via bmp-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-tsv\nbmp -> webp: lossless via bmp-to-webp\nbmp -> xlsx: lossy via bmp-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-xlsx\nbmp -> xml: lossy via bmp-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-xml\nbmp -> yaml: lossy via bmp-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-yaml\ncsv -> docx: conditional via csv-to-markdown -> markdown-to-docx\ncsv -> html: conditional via csv-to-markdown -> markdown-to-html\ncsv -> json: lossless via csv-to-json\ncsv -> jsonl: lossless via csv-to-jsonl\ncsv -> markdown: conditional via csv-to-markdown\ncsv -> markdown-json: conditional via csv-to-markdown -> markdown-to-json\ncsv -> text: lossy via csv-to-markdown -> markdown-to-text\ncsv -> toml: conditional via csv-to-json -> json-to-toml\ncsv -> tsv: lossless via csv-to-tsv\ncsv -> xlsx: conditional via csv-to-xlsx\ncsv -> xml: conditional via csv-to-json -> json-to-xml\ncsv -> yaml: lossless via csv-to-json -> json-to-yaml\ncur -> bmp: conditional via cur-to-bmp\ncur -> csv: lossy via cur-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv\ncur -> docx: lossy via cur-to-pdf -> pdf-to-text -> text-to-docx\ncur -> html: lossy via cur-to-pdf -> pdf-to-text -> text-to-html\ncur -> ico: conditional via cur-to-ico\ncur -> jpeg: lossy via cur-to-jpeg\ncur -> json: lossy via cur-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json\ncur -> jsonl: lossy via cur-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-jsonl\ncur -> markdown: lossy via cur-to-pdf -> pdf-to-text -> text-to-markdown\ncur -> markdown-json: lossy via cur-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-json\ncur -> pam: conditional via cur-to-pam\ncur -> pbm: lossy via cur-to-pbm\ncur -> pdf: conditional via cur-to-pdf\ncur -> pgm: conditional via cur-to-pgm\ncur -> png: conditional via cur-to-png\ncur -> ppm: conditional via cur-to-ppm\ncur -> qoi: conditional via cur-to-qoi\ncur -> text: lossy via cur-to-pdf -> pdf-to-text\ncur -> tga: conditional via cur-to-tga\ncur -> tiff: conditional via cur-to-tiff\ncur -> toml: lossy via cur-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-toml\ncur -> tsv: lossy via cur-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-tsv\ncur -> webp: conditional via cur-to-webp\ncur -> xlsx: lossy via cur-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-xlsx\ncur -> xml: lossy via cur-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-xml\ncur -> yaml: lossy via cur-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-yaml\ndocx -> csv: lossy via docx-to-markdown -> markdown-to-csv\ndocx -> html: lossy via docx-to-html\ndocx -> json: lossy via docx-to-markdown -> markdown-to-csv -> csv-to-json\ndocx -> jsonl: lossy via docx-to-markdown -> markdown-to-csv -> csv-to-jsonl\ndocx -> markdown: lossy via docx-to-markdown\ndocx -> markdown-json: lossy via docx-to-markdown -> markdown-to-json\ndocx -> text: lossy via docx-to-text\ndocx -> toml: lossy via docx-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-toml\ndocx -> tsv: lossy via docx-to-markdown -> markdown-to-tsv\ndocx -> xlsx: lossy via docx-to-markdown -> markdown-to-csv -> csv-to-xlsx\ndocx -> xml: lossy via docx-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-xml\ndocx -> yaml: lossy via docx-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-yaml\nhtml -> csv: lossy via html-to-markdown -> markdown-to-csv\nhtml -> docx: conditional via html-to-docx\nhtml -> json: lossy via html-to-markdown -> markdown-to-csv -> csv-to-json\nhtml -> jsonl: lossy via html-to-markdown -> markdown-to-csv -> csv-to-jsonl\nhtml -> markdown: lossy via html-to-markdown\nhtml -> markdown-json: lossy via html-to-markdown -> markdown-to-json\nhtml -> text: lossy via html-to-text\nhtml -> toml: lossy via html-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-toml\nhtml -> tsv: lossy via html-to-markdown -> markdown-to-tsv\nhtml -> xlsx: lossy via html-to-markdown -> markdown-to-csv -> csv-to-xlsx\nhtml -> xml: lossy via html-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-xml\nhtml -> yaml: lossy via html-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-yaml\nico -> bmp: conditional via ico-to-bmp\nico -> csv: lossy via ico-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv\nico -> docx: lossy via ico-to-pdf -> pdf-to-text -> text-to-docx\nico -> html: lossy via ico-to-pdf -> pdf-to-text -> text-to-html\nico -> jpeg: lossy via ico-to-jpeg\nico -> json: lossy via ico-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json\nico -> jsonl: lossy via ico-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-jsonl\nico -> markdown: lossy via ico-to-pdf -> pdf-to-text -> text-to-markdown\nico -> markdown-json: lossy via ico-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-json\nico -> pam: conditional via ico-to-pam\nico -> pbm: lossy via ico-to-pbm\nico -> pdf: conditional via ico-to-pdf\nico -> pgm: conditional via ico-to-pgm\nico -> png: conditional via ico-to-png\nico -> ppm: conditional via ico-to-ppm\nico -> qoi: conditional via ico-to-qoi\nico -> text: lossy via ico-to-pdf -> pdf-to-text\nico -> tga: conditional via ico-to-tga\nico -> tiff: conditional via ico-to-tiff\nico -> toml: lossy via ico-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-toml\nico -> tsv: lossy via ico-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-tsv\nico -> webp: conditional via ico-to-webp\nico -> xlsx: lossy via ico-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-xlsx\nico -> xml: lossy via ico-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-xml\nico -> yaml: lossy via ico-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-yaml\njpeg -> bmp: conditional via jpeg-to-bmp\njpeg -> csv: lossy via jpeg-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv\njpeg -> docx: lossy via jpeg-to-pdf -> pdf-to-text -> text-to-docx\njpeg -> html: lossy via jpeg-to-pdf -> pdf-to-text -> text-to-html\njpeg -> ico: conditional via jpeg-to-ico\njpeg -> json: lossy via jpeg-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json\njpeg -> jsonl: lossy via jpeg-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-jsonl\njpeg -> markdown: lossy via jpeg-to-pdf -> pdf-to-text -> text-to-markdown\njpeg -> markdown-json: lossy via jpeg-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-json\njpeg -> pam: conditional via jpeg-to-pam\njpeg -> pbm: lossy via jpeg-to-pbm\njpeg -> pdf: lossless via jpeg-to-pdf\njpeg -> pgm: conditional via jpeg-to-pgm\njpeg -> png: conditional via jpeg-to-png\njpeg -> ppm: conditional via jpeg-to-ppm\njpeg -> qoi: conditional via jpeg-to-qoi\njpeg -> text: lossy via jpeg-to-pdf -> pdf-to-text\njpeg -> tga: conditional via jpeg-to-tga\njpeg -> tiff: conditional via jpeg-to-tiff\njpeg -> toml: lossy via jpeg-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-toml\njpeg -> tsv: lossy via jpeg-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-tsv\njpeg -> webp: conditional via jpeg-to-webp\njpeg -> xlsx: lossy via jpeg-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-xlsx\njpeg -> xml: lossy via jpeg-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-xml\njpeg -> yaml: lossy via jpeg-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-yaml\njson -> csv: conditional via json-to-csv\njson -> docx: conditional via json-to-csv -> csv-to-markdown -> markdown-to-docx\njson -> html: conditional via json-to-csv -> csv-to-markdown -> markdown-to-html\njson -> jsonl: conditional via json-to-jsonl\njson -> markdown: conditional via json-to-csv -> csv-to-markdown\njson -> markdown-json: conditional via json-to-csv -> csv-to-markdown -> markdown-to-json\njson -> text: lossy via json-to-csv -> csv-to-markdown -> markdown-to-text\njson -> toml: conditional via json-to-toml\njson -> tsv: conditional via json-to-tsv\njson -> xlsx: conditional via json-to-csv -> csv-to-xlsx\njson -> xml: conditional via json-to-xml\njson -> yaml: lossless via json-to-yaml\njsonl -> csv: conditional via jsonl-to-csv\njsonl -> docx: conditional via jsonl-to-csv -> csv-to-markdown -> markdown-to-docx\njsonl -> html: conditional via jsonl-to-csv -> csv-to-markdown -> markdown-to-html\njsonl -> json: lossless via jsonl-to-json\njsonl -> markdown: conditional via jsonl-to-csv -> csv-to-markdown\njsonl -> markdown-json: conditional via jsonl-to-csv -> csv-to-markdown -> markdown-to-json\njsonl -> text: lossy via jsonl-to-csv -> csv-to-markdown -> markdown-to-text\njsonl -> toml: conditional via jsonl-to-json -> json-to-toml\njsonl -> tsv: conditional via jsonl-to-tsv\njsonl -> xlsx: conditional via jsonl-to-csv -> csv-to-xlsx\njsonl -> xml: conditional via jsonl-to-json -> json-to-xml\njsonl -> yaml: lossless via jsonl-to-json -> json-to-yaml\nmarkdown -> csv: lossy via markdown-to-csv\nmarkdown -> docx: conditional via markdown-to-docx\nmarkdown -> html: lossless via markdown-to-html\nmarkdown -> json: lossy via markdown-to-csv -> csv-to-json\nmarkdown -> jsonl: lossy via markdown-to-csv -> csv-to-jsonl\nmarkdown -> markdown-json: lossless via markdown-to-json\nmarkdown -> text: lossy via markdown-to-text\nmarkdown -> toml: lossy via markdown-to-csv -> csv-to-json -> json-to-toml\nmarkdown -> tsv: lossy via markdown-to-tsv\nmarkdown -> xlsx: lossy via markdown-to-csv -> csv-to-xlsx\nmarkdown -> xml: lossy via markdown-to-csv -> csv-to-json -> json-to-xml\nmarkdown -> yaml: lossy via markdown-to-csv -> csv-to-json -> json-to-yaml\nmarkdown-json -> csv: lossy via markdown-json-to-markdown -> markdown-to-csv\nmarkdown-json -> docx: conditional via markdown-json-to-markdown -> markdown-to-docx\nmarkdown-json -> html: lossless via markdown-json-to-markdown -> markdown-to-html\nmarkdown-json -> json: lossy via markdown-json-to-markdown -> markdown-to-csv -> csv-to-json\nmarkdown-json -> jsonl: lossy via markdown-json-to-markdown -> markdown-to-csv -> csv-to-jsonl\nmarkdown-json -> markdown: lossless via markdown-json-to-markdown\nmarkdown-json -> text: lossy via markdown-json-to-markdown -> markdown-to-text\nmarkdown-json -> toml: lossy via markdown-json-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-toml\nmarkdown-json -> tsv: lossy via markdown-json-to-markdown -> markdown-to-tsv\nmarkdown-json -> xlsx: lossy via markdown-json-to-markdown -> markdown-to-csv -> csv-to-xlsx\nmarkdown-json -> xml: lossy via markdown-json-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-xml\nmarkdown-json -> yaml: lossy via markdown-json-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-yaml\npages -> csv: lossy via pages-to-markdown -> markdown-to-csv\npages -> docx: conditional via pages-to-docx\npages -> html: lossy via pages-to-html\npages -> json: lossy via pages-to-markdown -> markdown-to-csv -> csv-to-json\npages -> jsonl: lossy via pages-to-markdown -> markdown-to-csv -> csv-to-jsonl\npages -> markdown: lossy via pages-to-markdown\npages -> markdown-json: lossy via pages-to-markdown -> markdown-to-json\npages -> pages-json: lossless via pages-to-json\npages -> text: lossy via pages-to-text\npages -> toml: lossy via pages-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-toml\npages -> tsv: lossy via pages-to-markdown -> markdown-to-tsv\npages -> xlsx: lossy via pages-to-markdown -> markdown-to-csv -> csv-to-xlsx\npages -> xml: lossy via pages-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-xml\npages -> yaml: lossy via pages-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-yaml\npages-json -> csv: lossy via json-to-pages -> pages-to-markdown -> markdown-to-csv\npages-json -> docx: conditional via json-to-pages -> pages-to-docx\npages-json -> html: lossy via json-to-pages -> pages-to-html\npages-json -> json: lossy via json-to-pages -> pages-to-markdown -> markdown-to-csv -> csv-to-json\npages-json -> jsonl: lossy via json-to-pages -> pages-to-markdown -> markdown-to-csv -> csv-to-jsonl\npages-json -> markdown: lossy via json-to-pages -> pages-to-markdown\npages-json -> markdown-json: lossy via json-to-pages -> pages-to-markdown -> markdown-to-json\npages-json -> pages: lossless via json-to-pages\npages-json -> text: lossy via json-to-pages -> pages-to-text\npages-json -> toml: lossy via json-to-pages -> pages-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-toml\npages-json -> tsv: lossy via json-to-pages -> pages-to-markdown -> markdown-to-tsv\npages-json -> xlsx: lossy via json-to-pages -> pages-to-markdown -> markdown-to-csv -> csv-to-xlsx\npages-json -> xml: lossy via json-to-pages -> pages-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-xml\npages-json -> yaml: lossy via json-to-pages -> pages-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-yaml\npam -> bmp: conditional via pam-to-bmp\npam -> csv: lossy via pam-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv\npam -> docx: lossy via pam-to-pdf -> pdf-to-text -> text-to-docx\npam -> html: lossy via pam-to-pdf -> pdf-to-text -> text-to-html\npam -> ico: conditional via pam-to-ico\npam -> jpeg: lossy via pam-to-jpeg\npam -> json: lossy via pam-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json\npam -> jsonl: lossy via pam-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-jsonl\npam -> markdown: lossy via pam-to-pdf -> pdf-to-text -> text-to-markdown\npam -> markdown-json: lossy via pam-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-json\npam -> pbm: lossy via pam-to-pbm\npam -> pdf: conditional via pam-to-pdf\npam -> pgm: conditional via pam-to-pgm\npam -> png: conditional via pam-to-png\npam -> ppm: conditional via pam-to-ppm\npam -> qoi: conditional via pam-to-qoi\npam -> text: lossy via pam-to-pdf -> pdf-to-text\npam -> tga: conditional via pam-to-tga\npam -> tiff: conditional via pam-to-tiff\npam -> toml: lossy via pam-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-toml\npam -> tsv: lossy via pam-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-tsv\npam -> webp: conditional via pam-to-webp\npam -> xlsx: lossy via pam-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-xlsx\npam -> xml: lossy via pam-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-xml\npam -> yaml: lossy via pam-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-yaml\npbm -> bmp: lossless via pbm-to-bmp\npbm -> csv: lossy via pbm-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv\npbm -> docx: lossy via pbm-to-pdf -> pdf-to-text -> text-to-docx\npbm -> html: lossy via pbm-to-pdf -> pdf-to-text -> text-to-html\npbm -> ico: conditional via pbm-to-ico\npbm -> jpeg: lossy via pbm-to-jpeg\npbm -> json: lossy via pbm-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json\npbm -> jsonl: lossy via pbm-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-jsonl\npbm -> markdown: lossy via pbm-to-pdf -> pdf-to-text -> text-to-markdown\npbm -> markdown-json: lossy via pbm-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-json\npbm -> pam: lossless via pbm-to-pam\npbm -> pdf: lossless via pbm-to-pdf\npbm -> pgm: conditional via pbm-to-pgm\npbm -> png: lossless via pbm-to-png\npbm -> ppm: conditional via pbm-to-ppm\npbm -> qoi: lossless via pbm-to-qoi\npbm -> text: lossy via pbm-to-pdf -> pdf-to-text\npbm -> tga: lossless via pbm-to-tga\npbm -> tiff: lossless via pbm-to-tiff\npbm -> toml: lossy via pbm-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-toml\npbm -> tsv: lossy via pbm-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-tsv\npbm -> webp: lossless via pbm-to-webp\npbm -> xlsx: lossy via pbm-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-xlsx\npbm -> xml: lossy via pbm-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-xml\npbm -> yaml: lossy via pbm-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-yaml\npdf -> bmp: conditional via pdf-to-bmp\npdf -> csv: lossy via pdf-to-text -> text-to-markdown -> markdown-to-csv\npdf -> docx: lossy via pdf-to-text -> text-to-docx\npdf -> html: lossy via pdf-to-text -> text-to-html\npdf -> ico: conditional via pdf-to-ico\npdf -> jpeg: lossy via pdf-to-jpeg\npdf -> json: lossy via pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json\npdf -> jsonl: lossy via pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-jsonl\npdf -> markdown: lossy via pdf-to-text -> text-to-markdown\npdf -> markdown-json: lossy via pdf-to-text -> text-to-markdown -> markdown-to-json\npdf -> pam: conditional via pdf-to-pam\npdf -> pbm: lossy via pdf-to-pbm\npdf -> pgm: conditional via pdf-to-pgm\npdf -> png: conditional via pdf-to-png\npdf -> ppm: conditional via pdf-to-ppm\npdf -> qoi: conditional via pdf-to-qoi\npdf -> text: lossy via pdf-to-text\npdf -> tga: conditional via pdf-to-tga\npdf -> tiff: conditional via pdf-to-tiff\npdf -> toml: lossy via pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-toml\npdf -> tsv: lossy via pdf-to-text -> text-to-markdown -> markdown-to-tsv\npdf -> webp: conditional via pdf-to-webp\npdf -> xlsx: lossy via pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-xlsx\npdf -> xml: lossy via pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-xml\npdf -> yaml: lossy via pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-yaml\npgm -> bmp: conditional via pgm-to-bmp\npgm -> csv: lossy via pgm-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv\npgm -> docx: lossy via pgm-to-pdf -> pdf-to-text -> text-to-docx\npgm -> html: lossy via pgm-to-pdf -> pdf-to-text -> text-to-html\npgm -> ico: conditional via pgm-to-ico\npgm -> jpeg: lossy via pgm-to-jpeg\npgm -> json: lossy via pgm-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json\npgm -> jsonl: lossy via pgm-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-jsonl\npgm -> markdown: lossy via pgm-to-pdf -> pdf-to-text -> text-to-markdown\npgm -> markdown-json: lossy via pgm-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-json\npgm -> pam: conditional via pgm-to-pam\npgm -> pbm: lossy via pgm-to-pbm\npgm -> pdf: conditional via pgm-to-pdf\npgm -> png: conditional via pgm-to-png\npgm -> ppm: conditional via pgm-to-ppm\npgm -> qoi: conditional via pgm-to-qoi\npgm -> text: lossy via pgm-to-pdf -> pdf-to-text\npgm -> tga: conditional via pgm-to-tga\npgm -> tiff: conditional via pgm-to-tiff\npgm -> toml: lossy via pgm-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-toml\npgm -> tsv: lossy via pgm-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-tsv\npgm -> webp: conditional via pgm-to-webp\npgm -> xlsx: lossy via pgm-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-xlsx\npgm -> xml: lossy via pgm-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-xml\npgm -> yaml: lossy via pgm-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-yaml\npng -> bmp: conditional via png-to-bmp\npng -> csv: lossy via png-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv\npng -> docx: lossy via png-to-pdf -> pdf-to-text -> text-to-docx\npng -> html: lossy via png-to-pdf -> pdf-to-text -> text-to-html\npng -> ico: conditional via png-to-ico\npng -> jpeg: lossy via png-to-jpeg\npng -> json: lossy via png-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json\npng -> jsonl: lossy via png-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-jsonl\npng -> markdown: lossy via png-to-pdf -> pdf-to-text -> text-to-markdown\npng -> markdown-json: lossy via png-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-json\npng -> pam: conditional via png-to-pam\npng -> pbm: lossy via png-to-pbm\npng -> pdf: conditional via png-to-pdf\npng -> pgm: conditional via png-to-pgm\npng -> ppm: conditional via png-to-ppm\npng -> qoi: conditional via png-to-qoi\npng -> text: lossy via png-to-pdf -> pdf-to-text\npng -> tga: conditional via png-to-tga\npng -> tiff: conditional via png-to-tiff\npng -> toml: lossy via png-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-toml\npng -> tsv: lossy via png-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-tsv\npng -> webp: conditional via png-to-webp\npng -> xlsx: lossy via png-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-xlsx\npng -> xml: lossy via png-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-xml\npng -> yaml: lossy via png-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-yaml\nppm -> bmp: conditional via ppm-to-bmp\nppm -> csv: lossy via ppm-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv\nppm -> docx: lossy via ppm-to-pdf -> pdf-to-text -> text-to-docx\nppm -> html: lossy via ppm-to-pdf -> pdf-to-text -> text-to-html\nppm -> ico: conditional via ppm-to-ico\nppm -> jpeg: lossy via ppm-to-jpeg\nppm -> json: lossy via ppm-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json\nppm -> jsonl: lossy via ppm-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-jsonl\nppm -> markdown: lossy via ppm-to-pdf -> pdf-to-text -> text-to-markdown\nppm -> markdown-json: lossy via ppm-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-json\nppm -> pam: conditional via ppm-to-pam\nppm -> pbm: lossy via ppm-to-pbm\nppm -> pdf: conditional via ppm-to-pdf\nppm -> pgm: conditional via ppm-to-pgm\nppm -> png: conditional via ppm-to-png\nppm -> qoi: conditional via ppm-to-qoi\nppm -> text: lossy via ppm-to-pdf -> pdf-to-text\nppm -> tga: conditional via ppm-to-tga\nppm -> tiff: conditional via ppm-to-tiff\nppm -> toml: lossy via ppm-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-toml\nppm -> tsv: lossy via ppm-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-tsv\nppm -> webp: conditional via ppm-to-webp\nppm -> xlsx: lossy via ppm-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-xlsx\nppm -> xml: lossy via ppm-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-xml\nppm -> yaml: lossy via ppm-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-yaml\nqoi -> bmp: lossless via qoi-to-bmp\nqoi -> csv: lossy via qoi-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv\nqoi -> docx: lossy via qoi-to-pdf -> pdf-to-text -> text-to-docx\nqoi -> html: lossy via qoi-to-pdf -> pdf-to-text -> text-to-html\nqoi -> ico: conditional via qoi-to-ico\nqoi -> jpeg: lossy via qoi-to-jpeg\nqoi -> json: lossy via qoi-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json\nqoi -> jsonl: lossy via qoi-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-jsonl\nqoi -> markdown: lossy via qoi-to-pdf -> pdf-to-text -> text-to-markdown\nqoi -> markdown-json: lossy via qoi-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-json\nqoi -> pam: lossless via qoi-to-pam\nqoi -> pbm: lossy via qoi-to-pbm\nqoi -> pdf: lossless via qoi-to-pdf\nqoi -> pgm: conditional via qoi-to-pgm\nqoi -> png: lossless via qoi-to-png\nqoi -> ppm: conditional via qoi-to-ppm\nqoi -> text: lossy via qoi-to-pdf -> pdf-to-text\nqoi -> tga: lossless via qoi-to-tga\nqoi -> tiff: lossless via qoi-to-tiff\nqoi -> toml: lossy via qoi-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-toml\nqoi -> tsv: lossy via qoi-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-tsv\nqoi -> webp: lossless via qoi-to-webp\nqoi -> xlsx: lossy via qoi-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-xlsx\nqoi -> xml: lossy via qoi-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-xml\nqoi -> yaml: lossy via qoi-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-yaml\ntext -> csv: lossy via text-to-markdown -> markdown-to-csv\ntext -> docx: conditional via text-to-docx\ntext -> html: conditional via text-to-html\ntext -> json: lossy via text-to-markdown -> markdown-to-csv -> csv-to-json\ntext -> jsonl: lossy via text-to-markdown -> markdown-to-csv -> csv-to-jsonl\ntext -> markdown: conditional via text-to-markdown\ntext -> markdown-json: conditional via text-to-markdown -> markdown-to-json\ntext -> toml: lossy via text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-toml\ntext -> tsv: lossy via text-to-markdown -> markdown-to-tsv\ntext -> xlsx: lossy via text-to-markdown -> markdown-to-csv -> csv-to-xlsx\ntext -> xml: lossy via text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-xml\ntext -> yaml: lossy via text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-yaml\ntga -> bmp: conditional via tga-to-bmp\ntga -> csv: lossy via tga-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv\ntga -> docx: lossy via tga-to-pdf -> pdf-to-text -> text-to-docx\ntga -> html: lossy via tga-to-pdf -> pdf-to-text -> text-to-html\ntga -> ico: conditional via tga-to-ico\ntga -> jpeg: lossy via tga-to-jpeg\ntga -> json: lossy via tga-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json\ntga -> jsonl: lossy via tga-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-jsonl\ntga -> markdown: lossy via tga-to-pdf -> pdf-to-text -> text-to-markdown\ntga -> markdown-json: lossy via tga-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-json\ntga -> pam: conditional via tga-to-pam\ntga -> pbm: lossy via tga-to-pbm\ntga -> pdf: conditional via tga-to-pdf\ntga -> pgm: conditional via tga-to-pgm\ntga -> png: conditional via tga-to-png\ntga -> ppm: conditional via tga-to-ppm\ntga -> qoi: conditional via tga-to-qoi\ntga -> text: lossy via tga-to-pdf -> pdf-to-text\ntga -> tiff: conditional via tga-to-tiff\ntga -> toml: lossy via tga-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-toml\ntga -> tsv: lossy via tga-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-tsv\ntga -> webp: conditional via tga-to-webp\ntga -> xlsx: lossy via tga-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-xlsx\ntga -> xml: lossy via tga-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-xml\ntga -> yaml: lossy via tga-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-yaml\ntiff -> bmp: conditional via tiff-to-bmp\ntiff -> csv: lossy via tiff-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv\ntiff -> docx: lossy via tiff-to-pdf -> pdf-to-text -> text-to-docx\ntiff -> html: lossy via tiff-to-pdf -> pdf-to-text -> text-to-html\ntiff -> ico: conditional via tiff-to-ico\ntiff -> jpeg: lossy via tiff-to-jpeg\ntiff -> json: lossy via tiff-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json\ntiff -> jsonl: lossy via tiff-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-jsonl\ntiff -> markdown: lossy via tiff-to-pdf -> pdf-to-text -> text-to-markdown\ntiff -> markdown-json: lossy via tiff-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-json\ntiff -> pam: conditional via tiff-to-pam\ntiff -> pbm: lossy via tiff-to-pbm\ntiff -> pdf: conditional via tiff-to-pdf\ntiff -> pgm: conditional via tiff-to-pgm\ntiff -> png: conditional via tiff-to-png\ntiff -> ppm: conditional via tiff-to-ppm\ntiff -> qoi: conditional via tiff-to-qoi\ntiff -> text: lossy via tiff-to-pdf -> pdf-to-text\ntiff -> tga: conditional via tiff-to-tga\ntiff -> toml: lossy via tiff-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-toml\ntiff -> tsv: lossy via tiff-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-tsv\ntiff -> webp: conditional via tiff-to-webp\ntiff -> xlsx: lossy via tiff-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-xlsx\ntiff -> xml: lossy via tiff-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-xml\ntiff -> yaml: lossy via tiff-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-yaml\ntoml -> csv: conditional via toml-to-json -> json-to-csv\ntoml -> docx: conditional via toml-to-json -> json-to-csv -> csv-to-markdown -> markdown-to-docx\ntoml -> html: conditional via toml-to-json -> json-to-csv -> csv-to-markdown -> markdown-to-html\ntoml -> json: conditional via toml-to-json\ntoml -> jsonl: conditional via toml-to-json -> json-to-jsonl\ntoml -> markdown: conditional via toml-to-json -> json-to-csv -> csv-to-markdown\ntoml -> markdown-json: conditional via toml-to-json -> json-to-csv -> csv-to-markdown -> markdown-to-json\ntoml -> text: lossy via toml-to-json -> json-to-csv -> csv-to-markdown -> markdown-to-text\ntoml -> tsv: conditional via toml-to-json -> json-to-tsv\ntoml -> xlsx: conditional via toml-to-json -> json-to-csv -> csv-to-xlsx\ntoml -> xml: conditional via toml-to-xml\ntoml -> yaml: conditional via toml-to-yaml\ntsv -> csv: lossless via tsv-to-csv\ntsv -> docx: conditional via tsv-to-markdown -> markdown-to-docx\ntsv -> html: conditional via tsv-to-markdown -> markdown-to-html\ntsv -> json: lossless via tsv-to-json\ntsv -> jsonl: lossless via tsv-to-jsonl\ntsv -> markdown: conditional via tsv-to-markdown\ntsv -> markdown-json: conditional via tsv-to-markdown -> markdown-to-json\ntsv -> text: lossy via tsv-to-markdown -> markdown-to-text\ntsv -> toml: conditional via tsv-to-json -> json-to-toml\ntsv -> xlsx: conditional via tsv-to-xlsx\ntsv -> xml: conditional via tsv-to-json -> json-to-xml\ntsv -> yaml: lossless via tsv-to-json -> json-to-yaml\nwebp -> bmp: conditional via webp-to-bmp\nwebp -> csv: lossy via webp-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv\nwebp -> docx: lossy via webp-to-pdf -> pdf-to-text -> text-to-docx\nwebp -> html: lossy via webp-to-pdf -> pdf-to-text -> text-to-html\nwebp -> ico: conditional via webp-to-ico\nwebp -> jpeg: lossy via webp-to-jpeg\nwebp -> json: lossy via webp-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json\nwebp -> jsonl: lossy via webp-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-jsonl\nwebp -> markdown: lossy via webp-to-pdf -> pdf-to-text -> text-to-markdown\nwebp -> markdown-json: lossy via webp-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-json\nwebp -> pam: conditional via webp-to-pam\nwebp -> pbm: lossy via webp-to-pbm\nwebp -> pdf: conditional via webp-to-pdf\nwebp -> pgm: conditional via webp-to-pgm\nwebp -> png: conditional via webp-to-png\nwebp -> ppm: conditional via webp-to-ppm\nwebp -> qoi: conditional via webp-to-qoi\nwebp -> text: lossy via webp-to-pdf -> pdf-to-text\nwebp -> tga: conditional via webp-to-tga\nwebp -> tiff: conditional via webp-to-tiff\nwebp -> toml: lossy via webp-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-toml\nwebp -> tsv: lossy via webp-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-tsv\nwebp -> xlsx: lossy via webp-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-xlsx\nwebp -> xml: lossy via webp-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-xml\nwebp -> yaml: lossy via webp-to-pdf -> pdf-to-text -> text-to-markdown -> markdown-to-csv -> csv-to-json -> json-to-yaml\nxlsx -> csv: conditional via xlsx-to-csv\nxlsx -> docx: conditional via xlsx-to-csv -> csv-to-markdown -> markdown-to-docx\nxlsx -> html: conditional via xlsx-to-csv -> csv-to-markdown -> markdown-to-html\nxlsx -> json: conditional via xlsx-to-csv -> csv-to-json\nxlsx -> jsonl: conditional via xlsx-to-csv -> csv-to-jsonl\nxlsx -> markdown: conditional via xlsx-to-csv -> csv-to-markdown\nxlsx -> markdown-json: conditional via xlsx-to-csv -> csv-to-markdown -> markdown-to-json\nxlsx -> text: lossy via xlsx-to-csv -> csv-to-markdown -> markdown-to-text\nxlsx -> toml: conditional via xlsx-to-csv -> csv-to-json -> json-to-toml\nxlsx -> tsv: conditional via xlsx-to-tsv\nxlsx -> xml: conditional via xlsx-to-csv -> csv-to-json -> json-to-xml\nxlsx -> yaml: conditional via xlsx-to-csv -> csv-to-json -> json-to-yaml\nxml -> csv: conditional via xml-to-json -> json-to-csv\nxml -> docx: conditional via xml-to-json -> json-to-csv -> csv-to-markdown -> markdown-to-docx\nxml -> html: conditional via xml-to-json -> json-to-csv -> csv-to-markdown -> markdown-to-html\nxml -> json: conditional via xml-to-json\nxml -> jsonl: conditional via xml-to-json -> json-to-jsonl\nxml -> markdown: conditional via xml-to-json -> json-to-csv -> csv-to-markdown\nxml -> markdown-json: conditional via xml-to-json -> json-to-csv -> csv-to-markdown -> markdown-to-json\nxml -> text: lossy via xml-to-json -> json-to-csv -> csv-to-markdown -> markdown-to-text\nxml -> toml: conditional via xml-to-toml\nxml -> tsv: conditional via xml-to-json -> json-to-tsv\nxml -> xlsx: conditional via xml-to-json -> json-to-csv -> csv-to-xlsx\nxml -> yaml: conditional via xml-to-yaml\nyaml -> csv: conditional via yaml-to-json -> json-to-csv\nyaml -> docx: conditional via yaml-to-json -> json-to-csv -> csv-to-markdown -> markdown-to-docx\nyaml -> html: conditional via yaml-to-json -> json-to-csv -> csv-to-markdown -> markdown-to-html\nyaml -> json: conditional via yaml-to-json\nyaml -> jsonl: conditional via yaml-to-json -> json-to-jsonl\nyaml -> markdown: conditional via yaml-to-json -> json-to-csv -> csv-to-markdown\nyaml -> markdown-json: conditional via yaml-to-json -> json-to-csv -> csv-to-markdown -> markdown-to-json\nyaml -> text: lossy via yaml-to-json -> json-to-csv -> csv-to-markdown -> markdown-to-text\nyaml -> toml: conditional via yaml-to-toml\nyaml -> tsv: conditional via yaml-to-json -> json-to-tsv\nyaml -> xlsx: conditional via yaml-to-json -> json-to-csv -> csv-to-xlsx\nyaml -> xml: conditional via yaml-to-xml\n"
    );

    let markdown = run(&["paths", "--markdown"]);
    assert!(stdout(&markdown).starts_with("# Formats and Conversion Paths"));
    assert!(stdout(&markdown).contains("```mermaid\ngraph LR\n"));
    assert!(stdout(&markdown).contains("  pages --- docx\n"));
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
