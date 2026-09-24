//! HTML -> Text. Inputs are the markdown-html generator's documents written
//! to HTML by our own writer; see `bench/pairs/html-text.sh`.

use sublime::converters::html_to_text::HtmlToText;

use crate::common::run_ours;

pub fn run(mode: &str, args: &[String]) -> Result<(), String> {
    match (mode, args) {
        ("ours", [input, output]) => run_ours(&HtmlToText, input, output),
        _ => Err("html-text modes: ours <in> <out>".to_string()),
    }
}
