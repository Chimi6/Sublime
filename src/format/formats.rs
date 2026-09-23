//! Static format declarations. Add one `pub static` per format.

use super::Format;

pub static CSV: Format = Format {
    id: "csv",
    display_name: "Comma-Separated Values",
    extensions: &["csv"],
    magic: None,
};

pub static JSON: Format = Format {
    id: "json",
    display_name: "JSON",
    extensions: &["json"],
    magic: None,
};
