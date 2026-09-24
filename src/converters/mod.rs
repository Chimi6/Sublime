//! One module per converter. Add a `pub mod` line here and a registry line
//! in `src/registry.rs`.

pub mod errors;
pub mod input;

pub mod csv_to_json;
pub mod json_to_csv;
pub mod json_to_pages;
pub mod markdown_json_to_markdown;
pub mod markdown_to_html;
pub mod markdown_to_json;
pub mod markdown_to_text;
pub mod pages_to_docx;
pub mod pages_to_json;
