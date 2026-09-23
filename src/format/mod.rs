//! Format declarations and detection.

pub mod formats;

use std::fmt;
use std::hash::{Hash, Hasher};
use std::path::Path;

/// A file format. Declared once as a `static`, referenced by pointer.
#[derive(Debug)]
pub struct Format {
    pub id: &'static str,
    pub display_name: &'static str,
    pub extensions: &'static [&'static str],
    pub magic: Option<&'static [u8]>,
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

/// Finds a format whose magic bytes prefix `head`.
pub fn find_by_magic(
    head: &[u8],
    known: &[&'static Format],
) -> Result<&'static Format, FormatError> {
    for format in known {
        let magic = match format.magic {
            Some(magic) => magic,
            None => continue,
        };
        if head.starts_with(magic) {
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
        };
        assert_eq!(&formats::CSV, &clone);
    }
}
