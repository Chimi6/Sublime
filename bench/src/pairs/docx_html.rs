//! Word -> Html. Inputs are the Pages pair's generated documents written
//! to Word by our own writer; see `bench/pairs/docx-html.sh`.

use sublime::converters::docx_to_html::DocxToHtml;

use crate::common::run_ours;

pub fn run(mode: &str, args: &[String]) -> Result<(), String> {
    match (mode, args) {
        ("ours", [input, output]) => run_ours(&DocxToHtml, input, output),
        _ => Err("docx-html modes: ours <in> <out>".to_string()),
    }
}
