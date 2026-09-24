//! Word -> Markdown. Inputs are the Pages pair's generated documents written
//! to Word by our own writer; see `bench/pairs/docx-markdown.sh`.

use sublime::converters::docx_to_markdown::DocxToMarkdown;

use crate::common::run_ours;

pub fn run(mode: &str, args: &[String]) -> Result<(), String> {
    match (mode, args) {
        ("ours", [input, output]) => run_ours(&DocxToMarkdown, input, output),
        _ => Err("docx-markdown modes: ours <in> <out>".to_string()),
    }
}
