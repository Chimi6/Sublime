//! One module per conversion pair. Add a module, a name in `NAMES`, and a
//! match arm in `run`; the pair's run script lives in `bench/pairs/`.

pub mod csv_json;
pub mod docx_html;
pub mod docx_markdown;
pub mod docx_text;
pub mod html_docx;
pub mod html_markdown;
pub mod html_text;
pub mod jsonl_json;
pub mod markdown_docx;
pub mod markdown_html;
pub mod markdown_json;
pub mod markdown_text;
pub mod pages_docx;
pub mod pages_html;
pub mod pages_json;
pub mod pages_markdown;
pub mod pages_text;
pub mod text_markdown;
pub mod toml_json;
pub mod xlsx_csv;
pub mod xml_json;
pub mod yaml_json;

pub const NAMES: &[&str] = &[
    "csv-json",
    "tsv-json",
    "jsonl-json",
    "docx-markdown",
    "docx-html",
    "docx-text",
    "html-markdown",
    "html-text",
    "html-docx",
    "markdown-docx",
    "markdown-html",
    "markdown-text",
    "markdown-json",
    "pages-json",
    "pages-docx",
    "pages-markdown",
    "pages-html",
    "pages-text",
    "text-markdown",
    "toml-json",
    "xml-json",
    "xlsx-csv",
    "yaml-json",
];

pub fn run(pair: &str, mode: &str, args: &[String]) -> Result<(), String> {
    match pair {
        "csv-json" => csv_json::run(mode, args),
        "tsv-json" => csv_json::run(mode, args),
        "jsonl-json" => jsonl_json::run(mode, args),
        "docx-markdown" => docx_markdown::run(mode, args),
        "docx-html" => docx_html::run(mode, args),
        "docx-text" => docx_text::run(mode, args),
        "html-markdown" => html_markdown::run(mode, args),
        "html-text" => html_text::run(mode, args),
        "html-docx" => html_docx::run(mode, args),
        "markdown-docx" => markdown_docx::run(mode, args),
        "markdown-html" => markdown_html::run(mode, args),
        "markdown-text" => markdown_text::run(mode, args),
        "markdown-json" => markdown_json::run(mode, args),
        "pages-json" => pages_json::run(mode, args),
        "pages-docx" => pages_docx::run(mode, args),
        "pages-markdown" => pages_markdown::run(mode, args),
        "pages-html" => pages_html::run(mode, args),
        "pages-text" => pages_text::run(mode, args),
        "text-markdown" => text_markdown::run(mode, args),
        "toml-json" => toml_json::run(mode, args),
        "xml-json" => xml_json::run(mode, args),
        "xlsx-csv" => xlsx_csv::run(mode, args),
        "yaml-json" => yaml_json::run(mode, args),
        other => Err(format!(
            "unknown pair '{other}'; known: {}",
            NAMES.join(", ")
        )),
    }
}
