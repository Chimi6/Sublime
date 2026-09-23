//! Static format declarations. Add one `pub static` per format.

use super::{Category, Format};

pub static CSV: Format = Format {
    id: "csv",
    display_name: "Comma-Separated Values",
    extensions: &["csv"],
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
