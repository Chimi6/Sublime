//! Byte scanning eight bytes at a time without SIMD intrinsics.
//!
//! Both finders load a `u64` per step and use the classic bit tricks for
//! "does any byte in this word equal `c`" and "is any byte below `n`". The
//! tricks can mis-flag bytes *after* the first real match because of borrow
//! propagation, so only the lowest flagged byte is trusted. That is exactly
//! the byte we want.

const LOW_BITS: u64 = 0x0101_0101_0101_0101;
const HIGH_BITS: u64 = 0x8080_8080_8080_8080;

/// Index of the first `,`, `"`, `\n`, or `\r` in `bytes`.
pub fn find_csv_delimiter(bytes: &[u8]) -> Option<usize> {
    let mut offset = 0usize;
    while offset + 8 <= bytes.len() {
        let word = load_word(bytes, offset);
        let hits = has_byte(word, b',')
            | has_byte(word, b'"')
            | has_byte(word, b'\n')
            | has_byte(word, b'\r');
        if hits != 0 {
            let first = first_flagged_byte(hits);
            return Some(offset + first);
        }
        offset += 8;
    }
    let tail = &bytes[offset..];
    if tail.is_empty() {
        return None;
    }
    let word = load_padded_word(tail);
    let hits =
        has_byte(word, b',') | has_byte(word, b'"') | has_byte(word, b'\n') | has_byte(word, b'\r');
    if hits == 0 {
        return None;
    }
    let first = first_flagged_byte(hits);
    if first >= tail.len() {
        return None;
    }
    Some(offset + first)
}

/// Index of the first `"`, `\`, or control byte below 0x20 in `bytes`.
pub fn find_json_escape(bytes: &[u8]) -> Option<usize> {
    let mut offset = 0usize;
    while offset + 8 <= bytes.len() {
        let word = load_word(bytes, offset);
        let hits = has_byte(word, b'"') | has_byte(word, b'\\') | has_byte_below(word, 0x20);
        if hits != 0 {
            let first = first_flagged_byte(hits);
            return Some(offset + first);
        }
        offset += 8;
    }
    let tail = &bytes[offset..];
    if tail.is_empty() {
        return None;
    }
    let word = load_padded_word(tail);
    let hits = has_byte(word, b'"') | has_byte(word, b'\\') | has_byte_below(word, 0x20);
    if hits == 0 {
        return None;
    }
    let first = first_flagged_byte(hits);
    if first >= tail.len() {
        return None;
    }
    Some(offset + first)
}

/// Loads a tail shorter than eight bytes, padding with a byte that no finder
/// flags. Padding bytes can still be reported by the bit tricks after a real
/// hit, which is why callers check the index against the tail length.
fn load_padded_word(tail: &[u8]) -> u64 {
    const PADDING: u8 = b'a';
    let mut chunk = [PADDING; 8];
    chunk[..tail.len()].copy_from_slice(tail);
    u64::from_le_bytes(chunk)
}

/// Index of the first `&`, `<`, `>`, or `"` in `bytes`.
pub fn find_html_special(bytes: &[u8]) -> Option<usize> {
    let mut offset = 0usize;
    while offset + 8 <= bytes.len() {
        let word = load_word(bytes, offset);
        let hits = has_byte(word, b'&')
            | has_byte(word, b'<')
            | has_byte(word, b'>')
            | has_byte(word, b'"');
        if hits != 0 {
            let first = first_flagged_byte(hits);
            return Some(offset + first);
        }
        offset += 8;
    }
    let tail = &bytes[offset..];
    if tail.is_empty() {
        return None;
    }
    let word = load_padded_word(tail);
    let hits =
        has_byte(word, b'&') | has_byte(word, b'<') | has_byte(word, b'>') | has_byte(word, b'"');
    if hits == 0 {
        return None;
    }
    let first = first_flagged_byte(hits);
    if first >= tail.len() {
        return None;
    }
    Some(offset + first)
}

/// Index of the first `\n` or `\r` in `bytes`.
pub fn find_line_ending(bytes: &[u8]) -> Option<usize> {
    let mut offset = 0usize;
    while offset + 8 <= bytes.len() {
        let word = load_word(bytes, offset);
        let hits = has_byte(word, b'\n') | has_byte(word, b'\r');
        if hits != 0 {
            let first = first_flagged_byte(hits);
            return Some(offset + first);
        }
        offset += 8;
    }
    let tail = &bytes[offset..];
    if tail.is_empty() {
        return None;
    }
    let word = load_padded_word(tail);
    let hits = has_byte(word, b'\n') | has_byte(word, b'\r');
    if hits == 0 {
        return None;
    }
    let first = first_flagged_byte(hits);
    if first >= tail.len() {
        return None;
    }
    Some(offset + first)
}

/// Index of the first `needle` in `bytes`.
pub fn find_byte(bytes: &[u8], needle: u8) -> Option<usize> {
    let mut offset = 0usize;
    while offset + 8 <= bytes.len() {
        let word = load_word(bytes, offset);
        let hits = has_byte(word, needle);
        if hits != 0 {
            let first = first_flagged_byte(hits);
            return Some(offset + first);
        }
        offset += 8;
    }
    let tail = &bytes[offset..];
    for (index, byte) in tail.iter().enumerate() {
        if *byte == needle {
            return Some(offset + index);
        }
    }
    None
}

fn load_word(bytes: &[u8], offset: usize) -> u64 {
    let mut chunk = [0u8; 8];
    chunk.copy_from_slice(&bytes[offset..offset + 8]);
    u64::from_le_bytes(chunk)
}

/// High bit set in every byte of `word` that is zero (lowest match exact).
fn has_zero_byte(word: u64) -> u64 {
    word.wrapping_sub(LOW_BITS) & !word & HIGH_BITS
}

/// High bit set in every byte of `word` equal to `value` (lowest match exact).
fn has_byte(word: u64, value: u8) -> u64 {
    let spread = LOW_BITS * u64::from(value);
    has_zero_byte(word ^ spread)
}

/// High bit set in every byte of `word` below `limit` (lowest match exact).
/// `limit` must be at most 128.
fn has_byte_below(word: u64, limit: u8) -> u64 {
    let spread = LOW_BITS * u64::from(limit);
    word.wrapping_sub(spread) & !word & HIGH_BITS
}

/// Byte index (0 = lowest address) of the lowest flagged byte.
fn first_flagged_byte(hits: u64) -> usize {
    let bit = hits.trailing_zeros() as usize;
    bit / 8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scalar_csv(bytes: &[u8]) -> Option<usize> {
        bytes
            .iter()
            .position(|byte| matches!(byte, b',' | b'"' | b'\n' | b'\r'))
    }

    fn scalar_json(bytes: &[u8]) -> Option<usize> {
        bytes
            .iter()
            .position(|byte| *byte == b'"' || *byte == b'\\' || *byte < 0x20)
    }

    #[test]
    fn csv_finder_matches_scalar_on_varied_inputs() {
        let samples: Vec<&[u8]> = vec![
            b"",
            b"abc",
            b"abcdefgh",
            b"abcdefg,",
            b",abcdefgh",
            b"abcdefghijklmnop\"x",
            b"no delimiters here at all really",
            b"line\nbreak",
            b"carriage\rreturn",
            b"\xFF\xFE\x80\x7F,tail",
            b"12345678901234567890123456789,",
        ];
        for sample in samples {
            assert_eq!(find_csv_delimiter(sample), scalar_csv(sample), "{sample:?}");
        }
    }

    #[test]
    fn json_finder_matches_scalar_on_varied_inputs() {
        let samples: Vec<&[u8]> = vec![
            b"",
            b"plain",
            b"eightbyte",
            b"quote\"here",
            b"back\\slash",
            b"tab\there",
            b"\x01",
            b"abcdefghijklmnopqrstuvwxyz\x1f",
            "héllo ✓ world".as_bytes(),
            b"\xFF\xFF\xFF\xFF\xFF\xFF\xFF\xFF\xFF",
        ];
        for sample in samples {
            assert_eq!(find_json_escape(sample), scalar_json(sample), "{sample:?}");
        }
    }

    #[test]
    fn finders_agree_with_scalar_on_every_byte_value_in_every_lane() {
        for value in 0..=255u8 {
            for lane in 0..8usize {
                let mut sample = [b'a'; 16];
                sample[lane] = value;
                assert_eq!(
                    find_csv_delimiter(&sample),
                    scalar_csv(&sample),
                    "csv {value} {lane}"
                );
                assert_eq!(
                    find_json_escape(&sample),
                    scalar_json(&sample),
                    "json {value} {lane}"
                );
            }
        }
    }
}
