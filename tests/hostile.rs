//! Hostile inputs through the built binary: each must end in a clean error
//! or a valid output, never a signal (a panic aborts in release, a stack
//! overflow always does) and never a hang. A subprocess is the only way to
//! see a stack overflow, so every input runs as its own process.
//!
//! The small inputs are files in `tests/fixtures/hostile`, listed in its
//! `cases.txt`; the large ones (deep nesting, a ragged table) are made here.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Far above any case's time: each takes a fraction of a second when the
/// guards hold.
const TIME_LIMIT: Duration = Duration::from_secs(60);

fn hostile(name: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests/fixtures/hostile");
    path.push(name);
    path
}

fn scratch(test: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("sublime-{test}-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("scratch directory");
    directory
}

/// Converts `input` to `extension` and returns the exit code, failing the
/// test on a signal or past the time limit.
fn convert(input: &Path, extension: &str, output_directory: &Path) -> i32 {
    let stem = input
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("input");
    let output = output_directory.join(format!("{stem}.{extension}"));
    let mut child = Command::new(env!("CARGO_BIN_EXE_sublime"))
        .arg("convert")
        .arg("-q")
        .arg(input)
        .arg(&output)
        .env_remove("SUBLIME_LOG")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("binary spawns");
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("child status") {
            break status;
        }
        if started.elapsed() > TIME_LIMIT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{stem} -> {extension}: still running after {TIME_LIMIT:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let _ = std::fs::remove_file(&output);
    let _ = std::fs::remove_dir_all(&output);
    match status.code() {
        Some(code @ 0..=2) => code,
        Some(code) => panic!("{stem} -> {extension}: exit code {code}"),
        None => panic!("{stem} -> {extension}: killed by a signal ({status})"),
    }
}

#[test]
fn audit_inputs_end_cleanly() {
    let manifest = std::fs::read_to_string(hostile("cases.txt")).expect("cases.txt");
    let output = scratch("hostile-files");
    let mut failures = Vec::new();
    for line in manifest.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        let [name, extension, expect] = fields[..] else {
            panic!("bad line in cases.txt: {line}");
        };
        let code = convert(&hostile(name), extension, &output);
        let ended = if code == 1 { "error" } else { "ok" };
        if ended != expect {
            failures.push(format!("{name} -> {extension}: {ended}, expected {expect}"));
        }
    }
    let _ = std::fs::remove_dir_all(&output);
    assert!(failures.is_empty(), "{failures:#?}");
}

/// Inputs too large to keep as files: nesting far past the limit in every
/// recursive reader, and a ragged Markdown table.
#[test]
fn generated_inputs_end_cleanly() {
    const DEEP: usize = 100_000;
    let deep = |open: &str, middle: &str, close: &str| {
        format!("{}{middle}{}", open.repeat(DEEP), close.repeat(DEEP))
    };
    let mut table = format!("|{}\n|{}\n", "a|".repeat(3_000), "-|".repeat(3_000));
    table.push_str(&"|x|\n".repeat(30_000));
    let cases = [
        ("deep.json", deep("[", "", "]"), "yaml", "error"),
        ("deep.jsonl", deep("[", "", "]"), "json", "error"),
        (
            "deep-object.json",
            deep("{\"a\":", "1", "}"),
            "toml",
            "error",
        ),
        ("deep-flow.yaml", deep("[", "", "]"), "json", "error"),
        ("deep-dash.yaml", "- ".repeat(DEEP) + "x\n", "json", "error"),
        ("deep-key.yaml", "? ".repeat(DEEP) + "x\n", "json", "error"),
        (
            "deep-array.toml",
            format!("a = {}", deep("[", "1", "]")),
            "json",
            "error",
        ),
        (
            "deep-inline.toml",
            format!("a = {}", deep("{a = ", "1", "}")),
            "json",
            "error",
        ),
        (
            "deep-dotted.toml",
            "a.".repeat(DEEP) + "a = 1\n",
            "json",
            "error",
        ),
        ("deep.xml", deep("<a>", "x", "</a>"), "json", "error"),
        ("deep-quote.md", ">".repeat(DEEP) + " x\n", "html", "ok"),
        ("deep-list.md", "- ".repeat(DEEP) + "x\n", "html", "ok"),
        (
            "deep-emphasis.md",
            "*a ".repeat(20_000) + &" b*".repeat(20_000),
            "html",
            "ok",
        ),
        ("ragged-table.md", table, "docx", "ok"),
    ];
    let directory = scratch("hostile-generated");
    let mut failures = Vec::new();
    for (name, text, extension, expect) in cases {
        let input = directory.join(name);
        std::fs::write(&input, text).expect("input written");
        let code = convert(&input, extension, &directory);
        let ended = if code == 1 { "error" } else { "ok" };
        if ended != expect {
            failures.push(format!("{name} -> {extension}: {ended}, expected {expect}"));
        }
    }
    let _ = std::fs::remove_dir_all(&directory);
    assert!(failures.is_empty(), "{failures:#?}");
}
