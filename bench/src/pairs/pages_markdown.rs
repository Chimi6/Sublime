//! Pages -> Markdown. Inputs come from the `pages-json` pair's generator; there is
//! no peer implementation, so the pair script compares against our own
//! package round trip on the same input.

use sublime::converters::pages_to_markdown::PagesToMarkdown;

use crate::common::run_ours;

pub fn run(mode: &str, args: &[String]) -> Result<(), String> {
    match (mode, args) {
        ("ours", [input, output]) => run_ours(&PagesToMarkdown, input, output),
        _ => Err("pages-markdown modes: ours <in> <out>".to_string()),
    }
}
