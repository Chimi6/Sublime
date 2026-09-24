//! HTML -> Markdown: our reader and writer, and `htmd` (an html5ever-based
//! HTML to Markdown converter) as the reference.

use std::fs::File;
use std::io::{BufWriter, Write};

use sublime::converters::html_to_markdown::HtmlToMarkdown;

use crate::common::run_ours;

pub fn run(mode: &str, args: &[String]) -> Result<(), String> {
    match (mode, args) {
        ("ours", [input, output]) => run_ours(&HtmlToMarkdown, input, output),
        ("crates", [input, output]) => crates_html_to_markdown(input, output),
        _ => Err("html-markdown modes: ours <in> <out> | crates <in> <out>".to_string()),
    }
}

fn crates_html_to_markdown(input: &str, output: &str) -> Result<(), String> {
    let html = std::fs::read_to_string(input).map_err(|error| error.to_string())?;
    let markdown = htmd::convert(&html).map_err(|error| error.to_string())?;
    let file = File::create(output).map_err(|error| error.to_string())?;
    let mut writer = BufWriter::new(file);
    writer
        .write_all(markdown.as_bytes())
        .map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())
}
