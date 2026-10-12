//! Format declarations and detection.

pub mod formats;

use std::fmt;
use std::hash::{Hash, Hasher};
use std::path::Path;

/// A loose grouping of formats for listings and docs. Not a hierarchy:
/// code is organized by format, never by category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    Data,
    Document,
    Image,
    Audio,
    Video,
    Archive,
}

impl Category {
    pub fn label(&self) -> &'static str {
        match self {
            Category::Data => "data",
            Category::Document => "document",
            Category::Image => "image",
            Category::Audio => "audio",
            Category::Video => "video",
            Category::Archive => "archive",
        }
    }
}

/// How much work a writer spends on making its output small, at the
/// same fidelity: one scale for every format that has the trade
/// (`--effort`). A format without it ignores it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum Effort {
    Fast,
    #[default]
    Balanced,
    Max,
}

impl Effort {
    pub fn name(&self) -> &'static str {
        match self {
            Effort::Fast => "fast",
            Effort::Balanced => "balanced",
            Effort::Max => "max",
        }
    }

    pub fn parse(text: &str) -> Option<Effort> {
        match text {
            "fast" => Some(Effort::Fast),
            "balanced" => Some(Effort::Balanced),
            "max" => Some(Effort::Max),
            _ => None,
        }
    }
}

/// A conversion option a format honours when it is read or written: the
/// one place that says which flags mean something for which formats.
/// Help, `sublime paths`, the docs, and the warning for a flag that does
/// nothing are all drawn from these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Setting {
    /// `--quality`: the fidelity a lossy writer keeps, 1 to 100, and its
    /// default.
    Quality { default: u8 },
    /// `--effort`: the work a writer spends on size (`Effort`).
    Effort,
    /// `--sheet`: the worksheet read, or the name of the one written.
    Sheet,
    /// `--page`: the page read.
    Page,
    /// `--font`: the font a document is set in.
    Font,
    /// `--delimiter`: the field delimiter read and written.
    Delimiter,
}

impl Setting {
    pub fn flag(&self) -> &'static str {
        match self {
            Setting::Quality { .. } => "--quality",
            Setting::Effort => "--effort",
            Setting::Sheet => "--sheet",
            Setting::Page => "--page",
            Setting::Font => "--font",
            Setting::Delimiter => "--delimiter",
        }
    }

    /// The flag with its default, as the docs list it.
    pub fn label(&self) -> String {
        match self {
            Setting::Quality { default } => format!("--quality (default {default})"),
            Setting::Effort => "--effort (default balanced)".to_string(),
            other => other.flag().to_string(),
        }
    }

    /// Whether `self` is the same option as `other`, whatever its default.
    pub fn same(&self, other: &Setting) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

/// A file format. Declared once as a `static`, referenced by pointer.
#[derive(Debug)]
pub struct Format {
    pub id: &'static str,
    pub display_name: &'static str,
    pub extensions: &'static [&'static str],
    pub magic: Option<&'static [u8]>,
    pub category: Category,
    /// The options that mean something when this format is read.
    pub read_options: &'static [Setting],
    /// The options that mean something when this format is written.
    pub write_options: &'static [Setting],
}

impl Format {
    /// Whether a file of this format holds one table or picture only (CSV,
    /// TSV, JSON Lines, an image), so a many-part input (a workbook's sheets,
    /// a document's tables, a PDF's pages) becomes one file per part.
    pub fn holds_one_part(&self) -> bool {
        matches!(self.id, "csv" | "tsv" | "jsonl") || self.category == Category::Image
    }
}

impl PartialEq for Format {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for Format {}

impl Hash for Format {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum FormatError {
    UnknownId(String),
    UnknownExtension(String),
    NoExtension(String),
    Undetectable,
}

impl fmt::Display for FormatError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FormatError::UnknownId(id) => {
                write!(
                    formatter,
                    "unknown format '{id}'; run `sublime formats` to list known formats"
                )
            }
            FormatError::UnknownExtension(extension) => {
                write!(
                    formatter,
                    "no known format uses the extension '.{extension}'; use --from or --to"
                )
            }
            FormatError::NoExtension(path) => {
                write!(formatter, "'{path}' has no extension; use --from or --to")
            }
            FormatError::Undetectable => {
                write!(
                    formatter,
                    "could not detect the format from content; use --from"
                )
            }
        }
    }
}

impl std::error::Error for FormatError {}

/// Finds a format by its id, ignoring ASCII case.
pub fn find_by_id(id: &str, known: &[&'static Format]) -> Result<&'static Format, FormatError> {
    let lowered = id.to_ascii_lowercase();
    for format in known {
        if format.id == lowered {
            return Ok(format);
        }
    }
    Err(FormatError::UnknownId(id.to_string()))
}

/// Finds a format by the extension of `path`, ignoring ASCII case.
pub fn find_by_extension(
    path: &Path,
    known: &[&'static Format],
) -> Result<&'static Format, FormatError> {
    let extension_os = match path.extension() {
        Some(extension_os) => extension_os,
        None => return Err(FormatError::NoExtension(path.display().to_string())),
    };
    let extension = match extension_os.to_str() {
        Some(extension) => extension.to_ascii_lowercase(),
        None => return Err(FormatError::NoExtension(path.display().to_string())),
    };
    for format in known {
        for candidate in format.extensions {
            if *candidate == extension {
                return Ok(format);
            }
        }
    }
    Err(FormatError::UnknownExtension(extension))
}

/// A magic byte that matches any byte, for signatures with a field in
/// the middle (RIFF files carry their size before the form type).
pub const MAGIC_ANY: u8 = b'?';

/// Finds a format whose magic bytes prefix `head`; `MAGIC_ANY` in a
/// magic matches any byte.
pub fn find_by_magic(
    head: &[u8],
    known: &[&'static Format],
) -> Result<&'static Format, FormatError> {
    for format in known {
        let magic = match format.magic {
            Some(magic) => magic,
            None => continue,
        };
        if head.len() < magic.len() {
            continue;
        }
        let matches = magic
            .iter()
            .zip(head)
            .all(|(expected, actual)| *expected == MAGIC_ANY || expected == actual);
        if matches {
            return Ok(format);
        }
    }
    Err(FormatError::Undetectable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    static PNGISH: Format = Format {
        id: "pngish",
        display_name: "Fake PNG",
        extensions: &["pngish", "pgi"],
        magic: Some(&[0x89, b'P', b'N', b'G']),
        category: Category::Image,
        read_options: &[],
        write_options: &[],
    };

    fn known() -> Vec<&'static Format> {
        vec![&formats::CSV, &formats::JSON, &PNGISH]
    }

    #[test]
    fn finds_by_id_case_insensitively() {
        let found = find_by_id("CSV", &known()).unwrap();
        assert_eq!(found.id, "csv");
    }

    #[test]
    fn unknown_id_is_an_error() {
        let result = find_by_id("pdoc", &known());
        assert_eq!(
            result.unwrap_err(),
            FormatError::UnknownId("pdoc".to_string())
        );
    }

    #[test]
    fn finds_by_extension_including_secondary_extensions() {
        let first = find_by_extension(Path::new("a/b/data.JSON"), &known()).unwrap();
        let second = find_by_extension(Path::new("x.pgi"), &known()).unwrap();
        assert_eq!(first.id, "json");
        assert_eq!(second.id, "pngish");
    }

    #[test]
    fn missing_extension_is_an_error() {
        let result = find_by_extension(Path::new("noext"), &known());
        assert_eq!(
            result.unwrap_err(),
            FormatError::NoExtension("noext".to_string())
        );
    }

    #[test]
    fn unknown_extension_is_an_error() {
        let result = find_by_extension(Path::new("file.pdoc"), &known());
        assert_eq!(
            result.unwrap_err(),
            FormatError::UnknownExtension("pdoc".to_string())
        );
    }

    #[test]
    fn finds_by_magic_bytes() {
        let head = [0x89, b'P', b'N', b'G', 0x0D, 0x0A];
        let found = find_by_magic(&head, &known()).unwrap();
        assert_eq!(found.id, "pngish");
    }

    #[test]
    fn a_wildcard_byte_matches_any_byte() {
        let head = b"RIFF\x10\x20\x30\x40WEBPVP8L";
        let found = find_by_magic(head, &[&formats::WEBP]).unwrap();
        assert_eq!(found.id, "webp");
        assert!(find_by_magic(b"RIFF\x10\x20\x30\x40WAVEfmt ", &[&formats::WEBP]).is_err());
    }

    #[test]
    fn undetectable_when_no_magic_matches() {
        let result = find_by_magic(b"a,b,c", &known());
        assert_eq!(result.unwrap_err(), FormatError::Undetectable);
    }

    #[test]
    fn formats_compare_by_id() {
        let clone = Format {
            id: "csv",
            display_name: "other name",
            extensions: &[],
            magic: None,
            category: Category::Document,
            read_options: &[],
            write_options: &[],
        };
        assert_eq!(&formats::CSV, &clone);
    }
}
