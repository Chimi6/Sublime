//! Input helpers shared by converters that parse a whole text document.

use std::io::Read;

use crate::converter::{ConvertError, Input, Location};

/// Reads the whole input as UTF-8, replacing NUL with U+FFFD as the
/// CommonMark specification requires for security. An invalid sequence is
/// reported with the line it starts on.
pub fn read_text_document(input: &mut Input<'_>) -> Result<String, ConvertError> {
    let mut bytes = Vec::new();
    input.read_to_end(&mut bytes)?;
    let text = match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) => {
            let offset = error.utf8_error().valid_up_to();
            let newlines = error.as_bytes()[..offset]
                .iter()
                .filter(|byte| **byte == b'\n')
                .count();
            return Err(ConvertError::Malformed {
                location: Location {
                    line: newlines as u64 + 1,
                    column: 0,
                },
                message: "invalid UTF-8".to_string(),
            });
        }
    };
    if text.contains('\0') {
        return Ok(text.replace('\0', "\u{FFFD}"));
    }
    Ok(text)
}
