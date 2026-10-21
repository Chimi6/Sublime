//! `pages-json` -> Pages: rebuilds the package. Object contents are exactly
//! what the JSON holds; the ZIP and Snappy layers are rebuilt.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Entry, Fidelity, Input, Location, Tier};
use crate::converters::pages_to_json::package_error;
use crate::event::Context;
use crate::format::formats;
use crate::io::pages::json::{ReadError, read_json};

const NAME: &str = "json-to-pages";

pub struct JsonToPages;

impl Converter for JsonToPages {
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

impl JsonToPages {
    pub const fn entry(&'static self) -> Entry {
        Entry {
            name: NAME,
            from: &formats::PAGES_JSON,
            to: &formats::PAGES,
            fidelity: Fidelity::Lossless,
            tier: Tier::Native,
            converter: self,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declares_contract() {
        assert_eq!(JsonToPages.entry().name, "json-to-pages");
        assert_eq!(JsonToPages.entry().from.id, "pages-json");
        assert_eq!(JsonToPages.entry().to.id, "pages");
        assert_eq!(JsonToPages.entry().fidelity, Fidelity::Lossless);
    }
}
