//! Byte scanning eight bytes at a time without SIMD intrinsics.
//!
//! Every finder loads a `u64` per step and uses the classic bit tricks for
//! "does any byte in this word equal `c`" and "is any byte below `n`". The
//! tricks can mis-flag bytes *after* the first real match because of borrow
//! propagation, so only the lowest flagged byte is trusted. That is exactly
//! the byte we want.

const LOW_BITS: u64 = 0x0101_0101_0101_0101;
const HIGH_BITS: u64 = 0x8080_8080_8080_8080;

/// Index of the first `,`, `"`, `\n`, or `\r` in `bytes`.
pub fn find_csv_delimiter(bytes: &[u8]) -> Option<usize> {
    scan(bytes, |word| {
        has_byte(word, b',') | has_byte(word, b'"') | has_byte(word, b'\n') | has_byte(word, b'\r')
    })
}

/// Index of the first `"`, `\`, or control byte below 0x20 in `bytes`.
pub fn find_json_escape(bytes: &[u8]) -> Option<usize> {
    scan(bytes, |word| {
        has_byte(word, b'"') | has_byte(word, b'\\') | has_byte_below(word, 0x20)
    })
}

/// Index of the first `&`, `<`, `>`, or `"` in `bytes`.
pub fn find_html_special(bytes: &[u8]) -> Option<usize> {
    scan(bytes, |word| {
        has_byte(word, b'&') | has_byte(word, b'<') | has_byte(word, b'>') | has_byte(word, b'"')
    })
}

/// Index of the first `\n` or `\r` in `bytes`.
pub fn find_line_ending(bytes: &[u8]) -> Option<usize> {
    scan(bytes, |word| has_byte(word, b'\n') | has_byte(word, b'\r'))
}

/// Index of the first byte equal to any of the three needles.
pub fn find_any_of3(bytes: &[u8], first: u8, second: u8, third: u8) -> Option<usize> {
    scan(bytes, |word| {
        has_byte(word, first) | has_byte(word, second) | has_byte(word, third)
    })
}

/// Index of the first `needle` in `bytes`.
pub fn find_byte(bytes: &[u8], needle: u8) -> Option<usize> {
    scan(bytes, |word| has_byte(word, needle))
}

/// Runs `flag` over `bytes` a word at a time and returns the index of the
/// lowest flagged byte. `flag` sets the high bit of every byte it matches.
fn scan(bytes: &[u8], flag: impl Fn(u64) -> u64) -> Option<usize> {
    let mut offset = 0usize;
    while offset + 8 <= bytes.len() {
        let hits = flag(load_word(bytes, offset));
        if hits != 0 {
            return Some(offset + first_flagged_byte(hits));
        }
        offset += 8;
    }
    if offset == bytes.len() {
        return None;
    }
    // The tail. A slice of eight bytes or more reloads its last eight: they
    // overlap the previous word, which held no hit, so any hit found is in
    // the tail. A shorter slice is padded with a byte no finder flags.
    if bytes.len() >= 8 {
        let start = bytes.len() - 8;
        let hits = flag(load_word(bytes, start));
        if hits == 0 {
            return None;
        }
        return Some(start + first_flagged_byte(hits));
    }
    let hits = flag(load_padded_word(bytes));
    if hits == 0 {
        return None;
    }
    let first = first_flagged_byte(hits);
    if first >= bytes.len() {
        return None;
    }
    Some(first)
}

/// Loads a tail shorter than eight bytes, padding with `a`. The word is
/// built with shifts rather than through a stack buffer: reloading a
/// buffer as one word right after filling it byte by byte stalls the CPU.
/// A padding byte can still be flagged by the bit tricks after a real hit
/// (or when a finder looks for `a`), which is why `scan` checks the index
/// against the slice length.
fn load_padded_word(tail: &[u8]) -> u64 {
    const PADDING: u8 = b'a';
    let mut word = LOW_BITS * u64::from(PADDING);
    for (index, byte) in tail.iter().enumerate() {
        let shift = index * 8;
        word ^= u64::from(PADDING ^ *byte) << shift;
    }
    word
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
