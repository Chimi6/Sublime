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

pub static XLSX: Format = Format {
    id: "xlsx",
    display_name: "Excel Workbook",
    extensions: &["xlsx"],
    magic: None,
    category: Category::Data,
};

pub static PNG: Format = Format {
    id: "png",
    display_name: "PNG image",
    extensions: &["png"],
    magic: Some(&[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n']),
    category: Category::Image,
};

pub static JPEG: Format = Format {
    id: "jpeg",
    display_name: "JPEG image",
    extensions: &["jpg", "jpeg", "jpe"],
    magic: Some(&[0xFF, 0xD8, 0xFF]),
    category: Category::Image,
};

pub static WEBP: Format = Format {
    id: "webp",
    display_name: "WebP image",
    extensions: &["webp"],
    magic: Some(b"RIFF????WEBP"),
    category: Category::Image,
};

pub static PBM: Format = Format {
    id: "pbm",
    display_name: "Netpbm bitmap (PBM)",
    extensions: &["pbm"],
    magic: Some(b"P4"),
    category: Category::Image,
};

pub static PGM: Format = Format {
    id: "pgm",
    display_name: "Netpbm graymap (PGM)",
    extensions: &["pgm"],
    magic: Some(b"P5"),
    category: Category::Image,
};

pub static PPM: Format = Format {
    id: "ppm",
    display_name: "Netpbm pixmap (PPM)",
    extensions: &["ppm", "pnm"],
    magic: Some(b"P6"),
    category: Category::Image,
};

pub static PAM: Format = Format {
    id: "pam",
    display_name: "Netpbm arbitrary map (PAM)",
    extensions: &["pam"],
    magic: Some(b"P7"),
    category: Category::Image,
};

pub static TGA: Format = Format {
    id: "tga",
    display_name: "Truevision TGA image",
    extensions: &["tga", "icb", "vda", "vst"],
    // No signature at the start; a TGA 2.0 file ends in one.
    magic: None,
    category: Category::Image,
};

pub static ICO: Format = Format {
    id: "ico",
    display_name: "Windows icon",
    extensions: &["ico"],
    magic: Some(b"\0\0\x01\0"),
    category: Category::Image,
};

pub static CUR: Format = Format {
    id: "cur",
    display_name: "Windows cursor",
    extensions: &["cur"],
    magic: Some(b"\0\0\x02\0"),
    category: Category::Image,
};

pub static TIFF: Format = Format {
    id: "tiff",
    display_name: "TIFF image",
    extensions: &["tif", "tiff"],
    // Little-endian; a big-endian file ("MM\0*") is known by extension.
    magic: Some(b"II*\0"),
    category: Category::Image,
};

pub static PDF: Format = Format {
    id: "pdf",
    display_name: "PDF document",
    extensions: &["pdf"],
    magic: Some(b"%PDF-"),
    category: Category::Document,
};

pub static QOI: Format = Format {
    id: "qoi",
    display_name: "QOI image",
    extensions: &["qoi"],
    magic: Some(b"qoif"),
    category: Category::Image,
};

pub static BMP: Format = Format {
    id: "bmp",
    display_name: "Windows bitmap",
    extensions: &["bmp", "dib"],
    magic: Some(b"BM"),
    category: Category::Image,
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

pub static RTF: Format = Format {
    id: "rtf",
    display_name: "Rich Text Format",
    extensions: &["rtf"],
    magic: Some(b"{\\rtf"),
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
