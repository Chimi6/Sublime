//! Text -> Markdown. Inputs are the markdown-html generator's prose written
//! to plain text by our own writer, and a dense shape of short lines; see
//! `bench/pairs/text-markdown.sh`.

use std::fs::File;
use std::io::{BufWriter, Write};

use sublime::converters::text_to_markdown::TextToMarkdown;

use crate::common::{parse_rows, run_ours};

pub fn run(mode: &str, args: &[String]) -> Result<(), String> {
    match (mode, args) {
        ("gen-lines", [units, path]) => generate_lines(units, path),
        ("ours", [input, output]) => run_ours(&TextToMarkdown, input, output),
        _ => Err("text-markdown modes: gen-lines <units> <path> | ours <in> <out>".to_string()),
    }
}

/// Short lines with characters Markdown would read as markup, a blank
/// line every fourth: the shape that makes the writer escape the most.
fn generate_lines(units: &str, path: &str) -> Result<(), String> {
    let count = parse_rows(units)?;
    let file = File::create(path).map_err(|error| error.to_string())?;
    let mut writer = BufWriter::new(file);
    for index in 0..count {
        writeln!(writer, "* item {index} with _underscores_ and #hashes")
            .map_err(|error| error.to_string())?;
        writeln!(
            writer,
            "1. not a list [nor a link](x) <nor a tag> `nor code`"
        )
        .map_err(|error| error.to_string())?;
        writeln!(writer, "  indented line with a | pipe and a \\ backslash")
            .map_err(|error| error.to_string())?;
        if index % 4 == 3 {
            writeln!(writer).map_err(|error| error.to_string())?;
        }
    }
    writer.flush().map_err(|error| error.to_string())
}
