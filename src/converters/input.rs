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

/// The bytes read to find a CSV input's delimiter: enough for its first
/// records.
const SNIFF_BYTES: usize = 64 * 1024;

/// The delimiter delimited text is written with: `--delimiter`'s for
/// comma-separated output (CSV), else the format's own (TSV's tab).
pub fn written_delimiter(format_delimiter: u8, context: &crate::event::Context<'_>) -> u8 {
    match context.options.delimiter {
        Some(delimiter) if format_delimiter == b',' => delimiter,
        _ => format_delimiter,
    }
}

/// A stream's first bytes, read to find its delimiter, then the rest.
pub type HeadAndRest<'a> = std::io::Chain<std::io::Cursor<Vec<u8>>, Input<'a>>;

/// Delimited input and the delimiter to read it with: TSV's tab, or for
/// CSV `--delimiter`'s, else the one its first records use (a semicolon
/// export reads as one). A rewindable input is rewound after the look and
/// handed back as it was; a stream comes back as the bytes read first and
/// then the rest, held in `rest`. Either way the readers see an `Input`,
/// the type they are compiled and tuned for.
pub fn delimited_input<'a: 'b, 'b>(
    mut input: Input<'a>,
    format_delimiter: u8,
    context: &crate::event::Context<'_>,
    rest: &'b mut Option<HeadAndRest<'a>>,
) -> std::io::Result<(u8, Input<'b>)> {
    let delimiter = match (format_delimiter, context.options.delimiter) {
        (b',', Some(delimiter)) => delimiter,
        (b',', None) => {
            let mut head = Vec::new();
            let whole = (&mut input)
                .take(SNIFF_BYTES as u64)
                .read_to_end(&mut head)?
                < SNIFF_BYTES;
            let delimiter = crate::io::csv::sniff_delimiter(&head, whole);
            return Ok(match input {
                Input::Rewindable(reader) => {
                    reader.rewind()?;
                    (delimiter, shorten(Input::Rewindable(reader)))
                }
                stream => {
                    let chained = rest.insert(std::io::Cursor::new(head).chain(stream));
                    (delimiter, Input::Stream(chained))
                }
            });
        }
        (other, _) => other,
    };
    Ok((delimiter, shorten(input)))
}

/// `input` for a shorter lifetime (a reborrow of what it reads from): the
/// match rebuilds it so its trait object may coerce to the shorter one.
#[allow(clippy::needless_match)]
fn shorten<'a: 'b, 'b>(input: Input<'a>) -> Input<'b> {
    match input {
        Input::Stream(reader) => Input::Stream(reader),
        Input::Rewindable(reader) => Input::Rewindable(reader),
    }
}
