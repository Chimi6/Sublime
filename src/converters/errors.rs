//! Error conversions shared by converters.

use crate::converter::{ConvertError, Location};
use crate::io::csv::CsvError;
use crate::io::json::JsonError;
use crate::io::toml::TomlError;
use crate::io::yaml::YamlError;

impl From<CsvError> for ConvertError {
    fn from(error: CsvError) -> Self {
        match error {
            CsvError::Io(io_error) => ConvertError::Io(io_error),
            CsvError::InvalidUtf8 { line } => ConvertError::Malformed {
                location: Location { line, column: 0 },
                message: "invalid UTF-8".to_string(),
            },
            CsvError::UnterminatedQuote { line } => ConvertError::Malformed {
                location: Location { line, column: 0 },
                message: "quoted field never closed".to_string(),
            },
        }
    }
}

impl From<JsonError> for ConvertError {
    fn from(error: JsonError) -> Self {
        match error {
            JsonError::Io(io_error) => ConvertError::Io(io_error),
            JsonError::Unexpected { location, message } => {
                ConvertError::Malformed { location, message }
            }
            JsonError::InvalidUtf8 { location } => ConvertError::Malformed {
                location,
                message: "invalid UTF-8 in string".to_string(),
            },
        }
    }
}

impl From<TomlError> for ConvertError {
    fn from(error: TomlError) -> Self {
        ConvertError::Malformed {
            location: error.location,
            message: error.message,
        }
    }
}

impl From<YamlError> for ConvertError {
    fn from(error: YamlError) -> Self {
        ConvertError::Malformed {
            location: error.location,
            message: error.message,
        }
    }
}
