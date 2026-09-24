//! HTML -> Docx. Inputs are the markdown-html generator's documents written
//! to HTML by our own writer; see `bench/pairs/html-docx.sh`.

use sublime::converters::html_to_docx::HtmlToDocx;

use crate::common::run_ours;

pub fn run(mode: &str, args: &[String]) -> Result<(), String> {
    match (mode, args) {
        ("ours", [input, output]) => run_ours(&HtmlToDocx, input, output),
        _ => Err("html-docx modes: ours <in> <out>".to_string()),
    }
}
