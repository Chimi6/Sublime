//! One module per converter. Add a `pub mod` line here and a registry line
//! in `src/registry.rs`.

pub mod errors;
pub mod input;

pub mod csv_to_json;
pub mod docx_to_html;
pub mod docx_to_markdown;
pub mod docx_to_text;
pub mod html_to_docx;
pub mod html_to_markdown;
pub mod html_to_text;
pub mod json_to_csv;
pub mod json_to_pages;
pub mod json_to_toml;
pub mod json_to_xml;
pub mod json_to_yaml;
pub mod markdown_json_to_markdown;
pub mod markdown_to_docx;
pub mod markdown_to_html;
pub mod markdown_to_json;
pub mod markdown_to_text;
pub mod pages_to_docx;
pub mod pages_to_html;
pub mod pages_to_json;
pub mod pages_to_markdown;
pub mod pages_to_text;
pub mod text_to_docx;
pub mod text_to_html;
pub mod text_to_markdown;
pub mod toml_to_json;
pub mod xml_to_json;
pub mod yaml_to_json;
