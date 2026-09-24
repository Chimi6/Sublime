//! Raw Snappy block decoding.

use std::fmt;

use crate::io::protobuf::read_varint;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnappyError {
    /// The block ended in the middle of a tag or its payload.
    Truncated { offset: usize },
    /// A copy reached before the start of the output.
    BadOffset { offset: usize },
    /// The declared length does not match what the tags produced.
    LengthMismatch { declared: usize, actual: usize },
    /// Longer than the caller allows.
    TooLarge { declared: u64 },
}

impl fmt::Display for SnappyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SnappyError::Truncated { offset } => {
                write!(formatter, "snappy block truncated at byte {offset}")
            }
            SnappyError::BadOffset { offset } => write!(
                formatter,
                "snappy copy at byte {offset} reaches before the output"
            ),
            SnappyError::LengthMismatch { declared, actual } => {
                write!(
                    formatter,
                    "snappy block declares {declared} bytes but holds {actual}"
                )
            }
            SnappyError::TooLarge { declared } => write!(
                formatter,
                "snappy block declares {declared} bytes, over the limit"
            ),
        }
    }
}

impl std::error::Error for SnappyError {}

/// The uncompressed length a block declares, and the bytes it takes.
pub fn uncompressed_length(block: &[u8]) -> Result<(u64, usize), SnappyError> {
    read_varint(block, 0).map_err(|_| SnappyError::Truncated { offset: 0 })
}

/// Decompresses one block, appending to `out`. `limit` caps the declared
/// length, so a corrupt header cannot ask for arbitrary memory.
pub fn decompress_block(block: &[u8], out: &mut Vec<u8>, limit: usize) -> Result<(), SnappyError> {
    let (declared, header_length) = uncompressed_length(block)?;
    if declared > limit as u64 {
        return Err(SnappyError::TooLarge { declared });
    }
    let declared = declared as usize;
    let start = out.len();
    out.reserve(declared);
    let mut position = header_length;
    while position < block.len() {
        let tag = block[position];
        let tag_offset = position;
        position += 1;
        match tag & 0b11 {
            0 => {
                let mut length = usize::from(tag >> 2) + 1;
                if length > 60 {
                    let extra = length - 60;
                    let mut value = 0usize;
                    for index in 0..extra {
                        let byte = *block
                            .get(position + index)
                            .ok_or(SnappyError::Truncated { offset: tag_offset })?;
                        value |= usize::from(byte) << (8 * index);
                    }
                    position += extra;
                    length = value + 1;
                }
                let end = position
                    .checked_add(length)
                    .ok_or(SnappyError::Truncated { offset: tag_offset })?;
                if end > block.len() {
                    return Err(SnappyError::Truncated { offset: tag_offset });
                }
                out.extend_from_slice(&block[position..end]);
                position = end;
            }
            1 => {
                let length = 4 + usize::from((tag >> 2) & 0b111);
                let high = usize::from(tag >> 5);
                let low = usize::from(
                    *block
                        .get(position)
                        .ok_or(SnappyError::Truncated { offset: tag_offset })?,
                );
                position += 1;
                copy(out, start, (high << 8) | low, length, tag_offset)?;
            }
            2 => {
                let length = usize::from(tag >> 2) + 1;
                let bytes = block
                    .get(position..position + 2)
                    .ok_or(SnappyError::Truncated { offset: tag_offset })?;
                position += 2;
                let offset = usize::from(bytes[0]) | (usize::from(bytes[1]) << 8);
                copy(out, start, offset, length, tag_offset)?;
            }
            _ => {
                let length = usize::from(tag >> 2) + 1;
                let bytes = block
                    .get(position..position + 4)
                    .ok_or(SnappyError::Truncated { offset: tag_offset })?;
                position += 4;
                let offset = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
                copy(out, start, offset, length, tag_offset)?;
            }
        }
    }
    let actual = out.len() - start;
    if actual != declared {
        return Err(SnappyError::LengthMismatch { declared, actual });
    }
    Ok(())
}

/// Appends `length` bytes copied from `offset` bytes back. The regions may
/// overlap (a run), so this copies byte by byte when they do.
fn copy(
    out: &mut Vec<u8>,
    start: usize,
    offset: usize,
    length: usize,
    tag_offset: usize,
) -> Result<(), SnappyError> {
    let produced = out.len() - start;
    if offset == 0 || offset > produced {
        return Err(SnappyError::BadOffset { offset: tag_offset });
    }
    let from = out.len() - offset;
    if offset >= length {
        out.extend_from_within(from..from + length);
        return Ok(());
    }
    // An overlapping copy repeats the last `offset` bytes; each chunk
    // copied makes the next chunk up to twice as long, so a run of any
    // length takes a few bulk copies instead of a byte at a time.
    let mut remaining = length;
    while remaining > 0 {
        let available = out.len() - from;
        let chunk = remaining.min(available);
        out.extend_from_within(from..from + chunk);
        remaining -= chunk;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(block: &[u8]) -> Result<Vec<u8>, SnappyError> {
        let mut out = Vec::new();
        decompress_block(block, &mut out, 1 << 20)?;
        Ok(out)
    }

    #[test]
    fn literals_and_copies() {
        // "abcabcabcabc": length 12, literal "abc", copy of 9 bytes from 3 back.
        let block = [12, 0x08, b'a', b'b', b'c', (5 << 2) | 1, 3];
        assert_eq!(decode(&block).unwrap(), b"abcabcabcabc");
    }

    #[test]
    fn long_literal_uses_extra_length_bytes() {
        let mut block = vec![100, (60 << 2), 99];
        block.extend(std::iter::repeat_n(b'x', 100));
        assert_eq!(decode(&block).unwrap(), vec![b'x'; 100]);
    }

    #[test]
    fn two_byte_offset_copy() {
        let mut block = vec![8, 0x0C, 1, 2, 3, 4];
        block.extend([(3 << 2) | 2, 4, 0]);
        assert_eq!(decode(&block).unwrap(), [1, 2, 3, 4, 1, 2, 3, 4]);
    }

    #[test]
    fn bad_input_is_an_error() {
        assert_eq!(
            decode(&[3, 0x08, b'a']),
            Err(SnappyError::Truncated { offset: 1 })
        );
        assert_eq!(
            decode(&[4, 0x00, b'a', 1, 9]),
            Err(SnappyError::BadOffset { offset: 3 })
        );
        assert!(matches!(
            decode(&[5, 0x00, b'a']),
            Err(SnappyError::LengthMismatch { .. })
        ));
    }
}
