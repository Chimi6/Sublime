//! HTML -> Text. Inputs are the markdown-html generator's documents written
//! to HTML by our own writer; see `bench/pairs/html-text.sh`. The reference is
//! `html2text`, a pure-Rust HTML to text renderer built on html5ever.

use std::fs::File;
use std::io::{BufWriter, Write};

use sublime::converters::html_to_text::HtmlToText;

use crate::common::run_ours;

pub fn run(mode: &str, args: &[String]) -> Result<(), String> {
    match (mode, args) {
        ("ours", [input, output]) => run_ours(&HtmlToText, input, output),
        ("crates", [input, output]) => crates_html_to_text(input, output),
        _ => Err("html-text modes: ours <in> <out> | crates <in> <out>".to_string()),
    }
}

fn crates_html_to_text(input: &str, output: &str) -> Result<(), String> {
    let html = std::fs::read(input).map_err(|error| error.to_string())?;
    let text = html2text::from_read(html.as_slice(), 80);
    let file = File::create(output).map_err(|error| error.to_string())?;
    let mut writer = BufWriter::new(file);
    writer
        .write_all(text.as_bytes())
        .map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())
}
