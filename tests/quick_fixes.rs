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
