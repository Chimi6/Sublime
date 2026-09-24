//! One module per conversion pair. Add a module, a name in `NAMES`, and a
//! match arm in `run`; the pair's run script lives in `bench/pairs/`.

pub mod csv_json;
pub mod docx_html;
pub mod docx_markdown;
pub mod docx_text;
pub mod markdown_html;
pub mod markdown_json;
pub mod markdown_text;
pub mod pages_docx;
pub mod pages_html;
pub mod pages_json;
pub mod pages_markdown;
pub mod pages_text;

pub const NAMES: &[&str] = &[
    "csv-json",
    "docx-markdown",
    "docx-html",
    "docx-text",
    "markdown-html",
    "markdown-text",
    "markdown-json",
    "pages-json",
    "pages-docx",
    "pages-markdown",
    "pages-html",
    "pages-text",
];

pub fn run(pair: &str, mode: &str, args: &[String]) -> Result<(), String> {
    match pair {
        "csv-json" => csv_json::run(mode, args),
        "docx-markdown" => docx_markdown::run(mode, args),
        "docx-html" => docx_html::run(mode, args),
        "docx-text" => docx_text::run(mode, args),
        "markdown-html" => markdown_html::run(mode, args),
        "markdown-text" => markdown_text::run(mode, args),
        "markdown-json" => markdown_json::run(mode, args),
        "pages-json" => pages_json::run(mode, args),
        "pages-docx" => pages_docx::run(mode, args),
        "pages-markdown" => pages_markdown::run(mode, args),
        "pages-html" => pages_html::run(mode, args),
        "pages-text" => pages_text::run(mode, args),
        other => Err(format!(
            "unknown pair '{other}'; known: {}",
            NAMES.join(", ")
        )),
    }
}
