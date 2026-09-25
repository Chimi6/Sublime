//! Static format declarations. Add one `pub static` per format.

use super::{Category, Format};

pub static CSV: Format = Format {
    id: "csv",
    display_name: "Comma-Separated Values",
    extensions: &["csv"],
    magic: None,
    category: Category::Data,
};

pub static TSV: Format = Format {
    id: "tsv",
    display_name: "Tab-Separated Values",
    extensions: &["tsv", "tab"],
    magic: None,
    category: Category::Data,
};

pub static JSONL: Format = Format {
    id: "jsonl",
    display_name: "JSON Lines",
    extensions: &["jsonl", "ndjson"],
    magic: None,
    category: Category::Data,
};

pub static JSON: Format = Format {
    id: "json",
    display_name: "JSON",
    extensions: &["json"],
    magic: None,
    category: Category::Data,
};

pub static TOML: Format = Format {
    id: "toml",
    display_name: "TOML",
    extensions: &["toml"],
    magic: None,
    category: Category::Data,
};

pub static YAML: Format = Format {
    id: "yaml",
    display_name: "YAML",
    extensions: &["yaml", "yml"],
    magic: None,
    category: Category::Data,
};

pub static XML: Format = Format {
    id: "xml",
    display_name: "XML",
    extensions: &["xml"],
    magic: None,
    category: Category::Data,
};

pub static MARKDOWN: Format = Format {
    id: "markdown",
    display_name: "Markdown",
    extensions: &["md", "markdown"],
    magic: None,
    category: Category::Document,
};

pub static HTML: Format = Format {
    id: "html",
    display_name: "HTML",
    extensions: &["html", "htm"],
    magic: None,
    category: Category::Document,
};

pub static TEXT: Format = Format {
    id: "text",
    display_name: "Plain text",
    extensions: &["txt"],
    magic: None,
    category: Category::Document,
};

pub static DOCX: Format = Format {
    id: "docx",
    display_name: "Word document",
    extensions: &["docx"],
    magic: None,
    category: Category::Document,
};

pub static PAGES: Format = Format {
    id: "pages",
    display_name: "Apple Pages",
    extensions: &["pages"],
    magic: None,
    category: Category::Document,
};

/// A Pages package as JSON (see `io::pages::json`). No extension of its
/// own; select it with `--to` or `--from`.
pub static PAGES_JSON: Format = Format {
    id: "pages-json",
    display_name: "Apple Pages package as JSON",
    extensions: &[],
    magic: None,
    category: Category::Document,
};

/// The Markdown event stream as JSON (see `io::markdown::events_json`).
/// It has no extension of its own; select it with `--to` or `--from`.
pub static MARKDOWN_JSON: Format = Format {
    id: "markdown-json",
    display_name: "Markdown events as JSON",
    extensions: &[],
    magic: None,
    category: Category::Document,
};
