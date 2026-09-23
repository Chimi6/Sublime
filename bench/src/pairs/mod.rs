//! One module per conversion pair. Add a module, a name in `NAMES`, and a
//! match arm in `run`; the pair's run script lives in `bench/pairs/`.

pub mod csv_json;
pub mod markdown_html;
pub mod markdown_json;
pub mod markdown_text;

pub const NAMES: &[&str] = &["csv-json", "markdown-html", "markdown-text", "markdown-json"];

pub fn run(pair: &str, mode: &str, args: &[String]) -> Result<(), String> {
    match pair {
        "csv-json" => csv_json::run(mode, args),
        "markdown-html" => markdown_html::run(mode, args),
        "markdown-text" => markdown_text::run(mode, args),
        "markdown-json" => markdown_json::run(mode, args),
        other => Err(format!(
            "unknown pair '{other}'; known: {}",
            NAMES.join(", ")
        )),
    }
}
