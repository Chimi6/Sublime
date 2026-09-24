//! `pages-json` -> Pages: rebuilds the package. Object contents are exactly
//! what the JSON holds; the ZIP and Snappy layers are rebuilt.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Fidelity, Input, Location, Tier};
use crate::converters::pages_to_json::package_error;
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::pages::json::{ReadError, read_json};

const NAME: &str = "json-to-pages";

pub struct JsonToPages;

impl Converter for JsonToPages {
    fn name(&self) -> &'static str {
        NAME
    }

    fn from(&self) -> &'static Format {
        &formats::PAGES_JSON
    }

    fn to(&self) -> &'static Format {
        &formats::PAGES
    }

    fn fidelity(&self) -> Fidelity {
        Fidelity::Lossless
    }

    fn tier(&self) -> Tier {
        Tier::Native
    }

    fn convert(
        &self,
        input: Input<'_>,
        output: &mut dyn Write,
        _context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let package = read_json(input).map_err(|error| match error {
            ReadError::Json(error) => error.into(),
            other => ConvertError::Malformed {
                location: Location { line: 0, column: 0 },
                message: other.to_string(),
            },
        })?;
        package.write(&mut *output).map_err(package_error)?;
        output.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declares_contract() {
        assert_eq!(JsonToPages.name(), "json-to-pages");
        assert_eq!(JsonToPages.from().id, "pages-json");
        assert_eq!(JsonToPages.to().id, "pages");
        assert_eq!(JsonToPages.fidelity(), Fidelity::Lossless);
    }
}
