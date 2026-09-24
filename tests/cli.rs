//! Runs the built binary as a subprocess and checks stdout, stderr, and exit codes.

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
        "csv -> json: lossless via csv-to-json\ndocx -> html: lossy via docx-to-html\ndocx -> markdown: lossy via docx-to-markdown\ndocx -> markdown-json: lossy via docx-to-markdown -> markdown-to-json\ndocx -> text: lossy via docx-to-text\njson -> csv: conditional via json-to-csv\nmarkdown -> docx: conditional via markdown-to-docx\nmarkdown -> html: lossless via markdown-to-html\nmarkdown -> markdown-json: lossless via markdown-to-json\nmarkdown -> text: lossy via markdown-to-text\nmarkdown-json -> docx: conditional via markdown-json-to-markdown -> markdown-to-docx\nmarkdown-json -> html: lossless via markdown-json-to-markdown -> markdown-to-html\nmarkdown-json -> markdown: lossless via markdown-json-to-markdown\nmarkdown-json -> text: lossy via markdown-json-to-markdown -> markdown-to-text\npages -> docx: conditional via pages-to-docx\npages -> html: lossy via pages-to-html\npages -> markdown: lossy via pages-to-markdown\npages -> markdown-json: lossy via pages-to-markdown -> markdown-to-json\npages -> pages-json: lossless via pages-to-json\npages -> text: lossy via pages-to-text\npages-json -> docx: conditional via json-to-pages -> pages-to-docx\npages-json -> html: lossy via json-to-pages -> pages-to-html\npages-json -> markdown: lossy via json-to-pages -> pages-to-markdown\npages-json -> markdown-json: lossy via json-to-pages -> pages-to-markdown -> markdown-to-json\npages-json -> pages: lossless via json-to-pages\npages-json -> text: lossy via json-to-pages -> pages-to-text\n"
    );

    let markdown = run(&["paths", "--markdown"]);
    assert!(stdout(&markdown).starts_with("# Formats and Conversion Paths"));
    assert!(stdout(&markdown).contains("```mermaid\ngraph LR\n"));
    assert!(stdout(&markdown).contains("  pages -- conditional --> docx\n"));
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
