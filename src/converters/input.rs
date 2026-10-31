//! Input helpers shared by converters that parse a whole text document.

use std::io::Read;

use crate::converter::{ConvertError, Input, Location};

/// Reads the whole input as text, replacing NUL with U+FFFD as the
/// CommonMark specification requires for security. UTF-8 is read as it
/// is (a byte-order mark dropped), and UTF-16 by its byte-order mark;
/// anything else is Japanese if it reads as Japanese: in the encoding the
/// document declares (an HTML `charset`, an XML `encoding`), else the
/// more plausible of Shift_JIS, EUC-JP, and ISO-2022-JP. Input that is
/// none of these is refused with the line its first bad byte is on.
pub fn read_text_document(input: &mut Input<'_>) -> Result<String, ConvertError> {
    let mut bytes = Vec::new();
    input.read_to_end(&mut bytes)?;
    let text = match decode_text(bytes) {
        Ok(text) => text,
        Err((bytes, offset)) => {
            let newlines = bytes[..offset]
                .iter()
                .filter(|byte| **byte == b'\n')
                .count();
            return Err(ConvertError::Malformed {
                location: Location {
                    line: newlines as u64 + 1,
                    column: 0,
                },
                message: "invalid UTF-8 (and not Japanese in Shift_JIS, EUC-JP, or ISO-2022-JP)"
                    .to_string(),
            });
        }
    };
    if text.contains('\0') {
        return Ok(text.replace('\0', "\u{FFFD}"));
    }
    Ok(text)
}

/// Text from bytes in UTF-8, UTF-16 (by its mark), or a Japanese
/// encoding; the bytes back, with where UTF-8 failed, when none fits.
fn decode_text(bytes: Vec<u8>) -> Result<String, (Vec<u8>, usize)> {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF])
        && let Ok(text) = std::str::from_utf8(rest)
    {
        return Ok(text.to_string());
    }
    for (mark, big_endian) in [([0xFE, 0xFF], true), ([0xFF, 0xFE], false)] {
        if let Some(rest) = bytes.strip_prefix(&mark) {
            let units = rest.chunks(2).map(|pair| {
                let pair = [pair[0], *pair.get(1).unwrap_or(&0)];
                if big_endian {
                    u16::from_be_bytes(pair)
                } else {
                    u16::from_le_bytes(pair)
                }
            });
            return Ok(char::decode_utf16(units)
                .map(|unit| unit.unwrap_or('\u{FFFD}'))
                .collect());
        }
    }
    let (bytes, offset) = match String::from_utf8(bytes) {
        Ok(text) => return Ok(text),
        Err(error) => {
            let offset = error.utf8_error().valid_up_to();
            (error.into_bytes(), offset)
        }
    };
    let encoding = crate::io::cjk::declared(&bytes).or_else(|| crate::io::cjk::sniff(&bytes));
    match encoding {
        Some(encoding) => Ok(encoding.decode(&bytes)),
        None => Err((bytes, offset)),
    }
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

/// What delimited input is read through once its first bytes have been
/// looked at: those bytes and then the rest, decoded from Shift_JIS or
/// EUC-JP when they are Japanese.
pub type HeadAndRest<'a> = Box<dyn Read + Send + 'a>;

/// Delimited input and the delimiter to read it with: TSV's tab, or for
/// CSV `--delimiter`'s, else the one its first records use (a semicolon
/// export reads as one). Input that is not UTF-8 but reads as Japanese
/// (Shift_JIS, as Excel saves a Japanese CSV, or EUC-JP) is decoded as it
/// is read. A rewindable UTF-8 input is rewound after the look and handed
/// back as it was; anything else comes back as the bytes read first and
/// then the rest, held in `rest`. Either way the readers see an `Input`,
/// the type they are compiled and tuned for.
pub fn delimited_input<'a: 'b, 'b>(
    mut input: Input<'a>,
    format_delimiter: u8,
    context: &crate::event::Context<'_>,
    rest: &'b mut Option<HeadAndRest<'a>>,
) -> std::io::Result<(u8, Input<'b>)> {
    let mut head = Vec::new();
    let whole = (&mut input)
        .take(SNIFF_BYTES as u64)
        .read_to_end(&mut head)?
        < SNIFF_BYTES;
    let encoding = match std::str::from_utf8(&head) {
        Ok(_) => None,
        // Cut at the end of the sample: still UTF-8.
        Err(error) if error.error_len().is_none() && !whole => None,
        Err(_) => crate::io::cjk::sniff(&head),
    };
    let delimiter = match (format_delimiter, context.options.delimiter) {
        (b',', Some(delimiter)) => delimiter,
        (b',', None) => {
            let sample = match encoding {
                Some(encoding) => encoding.decode(&head).into_bytes(),
                None => head.clone(),
            };
            crate::io::csv::sniff_delimiter(&sample, whole)
        }
        (other, _) => other,
    };
    let Some(encoding) = encoding else {
        return Ok(match input {
            Input::Rewindable(reader) => {
                reader.rewind()?;
                (delimiter, shorten(Input::Rewindable(reader)))
            }
            stream => {
                let chained = rest.insert(Box::new(std::io::Cursor::new(head).chain(stream)));
                (delimiter, Input::Stream(&mut **chained))
            }
        });
    };
    let source = std::io::Cursor::new(head).chain(input);
    let decoded: HeadAndRest<'a> = match encoding {
        // ISO-2022-JP keeps a mode across the text: decoded whole.
        crate::io::cjk::Encoding::Iso2022Jp => {
            let mut bytes = Vec::new();
            let mut source = source;
            source.read_to_end(&mut bytes)?;
            Box::new(std::io::Cursor::new(encoding.decode(&bytes).into_bytes()))
        }
        _ => Box::new(crate::io::cjk::Utf8Reader::new(source, encoding)),
    };
    let reader = rest.insert(decoded);
    Ok((delimiter, Input::Stream(&mut **reader)))
}

/// `input` decoded as it is read when it is Japanese rather than UTF-8:
/// in the encoding its first bytes declare (an XML `encoding`), else the
/// one they read as. UTF-8 input comes back as it was (rewound, or its
/// first bytes and then the rest).
pub fn decoded_input<'a: 'b, 'b>(
    mut input: Input<'a>,
    rest: &'b mut Option<HeadAndRest<'a>>,
) -> std::io::Result<Input<'b>> {
    let mut head = Vec::new();
    let whole = (&mut input)
        .take(SNIFF_BYTES as u64)
        .read_to_end(&mut head)?
        < SNIFF_BYTES;
    let encoding = match std::str::from_utf8(&head) {
        Ok(_) => crate::io::cjk::declared(&head)
            .filter(|_| head.iter().any(|byte| *byte >= 0x80 || *byte == 0x1B)),
        Err(error) if error.error_len().is_none() && !whole => None,
        Err(_) => crate::io::cjk::declared(&head).or_else(|| crate::io::cjk::sniff(&head)),
    };
    let Some(encoding) = encoding else {
        return Ok(match input {
            Input::Rewindable(reader) => {
                reader.rewind()?;
                shorten(Input::Rewindable(reader))
            }
            stream => {
                let chained = rest.insert(Box::new(std::io::Cursor::new(head).chain(stream)));
                Input::Stream(&mut **chained)
            }
        });
    };
    let source = std::io::Cursor::new(head).chain(input);
    let decoded: HeadAndRest<'a> = match encoding {
        crate::io::cjk::Encoding::Iso2022Jp => {
            let mut bytes = Vec::new();
            let mut source = source;
            source.read_to_end(&mut bytes)?;
            Box::new(std::io::Cursor::new(encoding.decode(&bytes).into_bytes()))
        }
        _ => Box::new(crate::io::cjk::Utf8Reader::new(source, encoding)),
    };
    let reader = rest.insert(decoded);
    Ok(Input::Stream(&mut **reader))
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
