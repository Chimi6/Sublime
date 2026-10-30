//! Japanese in the legacy encodings: every input in
//! tests/fixtures/japanese-input (from `make.py` there) converts, and its
//! Japanese reads as written. Text, HTML, and RTF go to text; CSV to CSV
//! (as UTF-8); XML to JSON.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/japanese-input")
}

fn convert(input: &PathBuf, to: &str) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_sublime"))
        .args(["-q", "convert"])
        .arg(input)
        .args(["--to", to])
        .output()
        .expect("runs");
    assert!(
        output
            .status
            .code()
            .is_some_and(|code| code == 0 || code == 2),
        "{}: {}",
        input.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("UTF-8 out")
}

fn squeezed(text: &str) -> String {
    text.chars().filter(|ch| !ch.is_whitespace()).collect()
}

#[test]
fn every_japanese_input_reads_as_written() {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir())
        .expect("fixture dir")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| {
            let extension = path.extension().and_then(|extension| extension.to_str());
            matches!(extension, Some("txt" | "html" | "csv" | "xml" | "rtf"))
        })
        .collect();
    paths.sort();
    assert_eq!(paths.len(), 8, "the eight inputs make.py writes");
    for path in paths {
        let to = match path.extension().and_then(|extension| extension.to_str()) {
            Some("csv") => "tsv",
            Some("xml") => "json",
            Some("txt") => "markdown",
            _ => "text",
        };
        let mut read = convert(&path, to);
        if to == "tsv" {
            read = read.replace('\t', ",");
        }
        let expected_path = PathBuf::from(format!("{}.expected", path.display()));
        let expected = fs::read_to_string(expected_path).expect("expected beside the input");
        assert_eq!(squeezed(&read), squeezed(&expected), "{}", path.display());
    }
}
