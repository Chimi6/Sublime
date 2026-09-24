//! Pages -> `pages-json`: the whole package as JSON, lossless at the
//! object level. The first step of every Pages path, and the form the
//! format map is built from.

use std::io::{Read, Write};

use crate::converter::{ConvertError, Converter, Fidelity, Input, Location, Tier};
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::pages::json::write_json;
use crate::io::pages::{Package, PackageError};

const NAME: &str = "pages-to-json";

pub struct PagesToJson;

impl Converter for PagesToJson {
    fn name(&self) -> &'static str {
        NAME
    }

    fn from(&self) -> &'static Format {
        &formats::PAGES
    }

    fn to(&self) -> &'static Format {
        &formats::PAGES_JSON
    }

    fn fidelity(&self) -> Fidelity {
        Fidelity::Lossless
    }

    fn tier(&self) -> Tier {
        Tier::Native
    }

    fn convert(
        &self,
        mut input: Input<'_>,
        output: &mut dyn Write,
        _context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let mut bytes = Vec::new();
        input.read_to_end(&mut bytes)?;
        let package = Package::read(&bytes).map_err(package_error)?;
        write_json(&package, &mut *output)?;
        output.flush()?;
        Ok(())
    }
}

pub(crate) fn package_error(error: PackageError) -> ConvertError {
    match error {
        PackageError::Io(error) => ConvertError::Io(error),
        other => ConvertError::Malformed {
            location: Location { line: 0, column: 0 },
            message: other.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declares_contract() {
        assert_eq!(PagesToJson.name(), "pages-to-json");
        assert_eq!(PagesToJson.from().id, "pages");
        assert_eq!(PagesToJson.to().id, "pages-json");
        assert_eq!(PagesToJson.fidelity(), Fidelity::Lossless);
    }
}
