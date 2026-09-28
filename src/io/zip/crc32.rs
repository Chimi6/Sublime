//! CRC-32 (IEEE, reflected), as ZIP, PNG, and gzip use it.

const fn make_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut index = 0usize;
    while index < 256 {
        let mut value = index as u32;
        let mut bit = 0;
        while bit < 8 {
            value = if value & 1 == 1 {
                0xEDB8_8320 ^ (value >> 1)
            } else {
                value >> 1
            };
            bit += 1;
        }
        table[index] = value;
        index += 1;
    }
    table
}

/// Slicing-by-sixteen: sixteen tables let sixteen bytes fold in per
/// step with independent lookups, where the one-table loop is a
/// dependent chain per byte.
const SLICES: usize = 16;

const fn make_tables() -> [[u32; 256]; SLICES] {
    let mut tables = [[0u32; 256]; SLICES];
    tables[0] = make_table();
    let mut index = 0usize;
    while index < 256 {
        let mut slice = 1usize;
        while slice < SLICES {
            let previous = tables[slice - 1][index];
            tables[slice][index] = (previous >> 8) ^ tables[0][(previous & 0xFF) as usize];
            slice += 1;
        }
        index += 1;
    }
    tables
}

// A static, not a const: indexing a const at runtime copies the whole table
// onto the stack in unoptimized builds (sixteen copies of 16 KB in one
// function overflowed the 1 MB main-thread stack on Windows).
static TABLES: [[u32; 256]; SLICES] = make_tables();

/// Continues a checksum: pass `0` to start, and the previous result to
/// extend it over more bytes. A long input is checked as two halves in
/// one interleaved loop (two independent chains keep the core busy where
/// one waits on itself) and the halves are joined with `combine`.
pub fn crc32_update(previous: u32, bytes: &[u8]) -> u32 {
    const SPLIT_AT: usize = 8 * SLICES;
    if bytes.len() < SPLIT_AT {
        return crc32_serial(previous, bytes);
    }
    // Four equal parts (a multiple of a piece each) run interleaved in
    // one loop, four independent chains, and join with `combine`.
    let part = (bytes.len() / 4) & !(SLICES - 1);
    let (head, tail) = bytes.split_at(part * 4);
    let (stream_a, rest) = head.split_at(part);
    let (stream_b, rest) = rest.split_at(part);
    let (stream_c, stream_d) = rest.split_at(part);
    let mut value_a = !previous;
    let mut value_b = !0u32;
    let mut value_c = !0u32;
    let mut value_d = !0u32;
    let pieces = stream_a
        .chunks_exact(SLICES)
        .zip(stream_b.chunks_exact(SLICES))
        .zip(stream_c.chunks_exact(SLICES))
        .zip(stream_d.chunks_exact(SLICES));
    for (((piece_a, piece_b), piece_c), piece_d) in pieces {
        value_a = fold_piece(value_a, piece_a);
        value_b = fold_piece(value_b, piece_b);
        value_c = fold_piece(value_c, piece_c);
        value_d = fold_piece(value_d, piece_d);
    }
    // The three joins carry a checksum over the same length, so the
    // operator for that length is built once and applied three times.
    let carry = zero_operator(part);
    let mut crc = times(&carry, !value_a) ^ !value_b;
    crc = times(&carry, crc) ^ !value_c;
    crc = times(&carry, crc) ^ !value_d;
    crc32_serial(crc, tail)
}

/// One chain over `bytes`, sixteen at a time.
fn crc32_serial(previous: u32, bytes: &[u8]) -> u32 {
    let mut value = !previous;
    let mut pieces = bytes.chunks_exact(SLICES);
    for piece in &mut pieces {
        value = fold_piece(value, piece);
    }
    for byte in pieces.remainder() {
        let index = ((value ^ u32::from(*byte)) & 0xFF) as usize;
        value = TABLES[0][index] ^ (value >> 8);
    }
    !value
}

/// Folds sixteen bytes into the running (inverted) value.
#[inline(always)]
fn fold_piece(value: u32, piece: &[u8]) -> u32 {
    let first = u32::from_le_bytes([piece[0], piece[1], piece[2], piece[3]]) ^ value;
    // Four independent chains of four, joined at the end; a single
    // chain of sixteen would wait a cycle per lookup.
    let quarter_a = TABLES[15][(first & 0xFF) as usize]
        ^ TABLES[14][((first >> 8) & 0xFF) as usize]
        ^ TABLES[13][((first >> 16) & 0xFF) as usize]
        ^ TABLES[12][(first >> 24) as usize];
    let quarter_b = TABLES[11][usize::from(piece[4])]
        ^ TABLES[10][usize::from(piece[5])]
        ^ TABLES[9][usize::from(piece[6])]
        ^ TABLES[8][usize::from(piece[7])];
    let quarter_c = TABLES[7][usize::from(piece[8])]
        ^ TABLES[6][usize::from(piece[9])]
        ^ TABLES[5][usize::from(piece[10])]
        ^ TABLES[4][usize::from(piece[11])];
    let quarter_d = TABLES[3][usize::from(piece[12])]
        ^ TABLES[2][usize::from(piece[13])]
        ^ TABLES[1][usize::from(piece[14])]
        ^ TABLES[0][usize::from(piece[15])];
    (quarter_a ^ quarter_b) ^ (quarter_c ^ quarter_d)
}

/// The checksum of two byte strings joined, from their own checksums
/// and the second one's length: the first is carried over `length`
/// zero bytes by GF(2) matrix powers of the polynomial (zlib's method).
pub fn combine(first: u32, second: u32, length: usize) -> u32 {
    if length == 0 {
        return first;
    }
    times(&zero_operator(length), first) ^ second
}

/// The GF(2) matrix that carries a checksum over `length` zero bytes,
/// by repeated squaring of the one-bit operator.
fn zero_operator(length: usize) -> [u32; 32] {
    let mut odd = [0u32; 32];
    odd[0] = 0xEDB8_8320;
    for (bit, cell) in odd.iter_mut().enumerate().skip(1) {
        *cell = 1 << (bit - 1);
    }
    let mut even = square(&odd);
    odd = square(&even);
    let mut result = [0u32; 32];
    for (bit, cell) in result.iter_mut().enumerate() {
        *cell = 1 << bit;
    }
    let mut remaining = length;
    loop {
        even = square(&odd);
        if remaining & 1 == 1 {
            result = compose(&even, &result);
        }
        remaining >>= 1;
        if remaining == 0 {
            break;
        }
        odd = square(&even);
        if remaining & 1 == 1 {
            result = compose(&odd, &result);
        }
        remaining >>= 1;
        if remaining == 0 {
            break;
        }
    }
    result
}

/// The matrix product `left * right` (apply `right`, then `left`).
fn compose(left: &[u32; 32], right: &[u32; 32]) -> [u32; 32] {
    let mut result = [0u32; 32];
    for (cell, column) in result.iter_mut().zip(right) {
        *cell = times(left, *column);
    }
    result
}

fn times(matrix: &[u32; 32], mut vector: u32) -> u32 {
    let mut sum = 0u32;
    let mut row = 0usize;
    while vector != 0 {
        if vector & 1 == 1 {
            sum ^= matrix[row];
        }
        vector >>= 1;
        row += 1;
    }
    sum
}

fn square(matrix: &[u32; 32]) -> [u32; 32] {
    let mut result = [0u32; 32];
    for (cell, column) in result.iter_mut().zip(matrix) {
        *cell = times(matrix, *column);
    }
    result
}

pub fn crc32(bytes: &[u8]) -> u32 {
    crc32_update(0, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_inputs_split_and_combine_to_the_serial_value() {
        let text: Vec<u8> = (0..10_000u32).map(|i| (i * 31 + i / 7) as u8).collect();
        for length in [64usize, 65, 100, 255, 256, 1000, 4097, 10_000] {
            let bytes = &text[..length];
            let serial = crc32_serial(0, bytes);
            assert_eq!(crc32(bytes), serial, "length {length}");
            let (a, b) = bytes.split_at(length / 3);
            assert_eq!(
                combine(crc32(a), crc32(b), b.len()),
                serial,
                "combine {length}"
            );
            assert_eq!(crc32_update(crc32(a), b), serial, "update {length}");
        }
    }

    #[test]
    fn known_values() {
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32_update(crc32(b"1234"), b"56789"), 0xCBF4_3926);
        // Longer than a word, split at every offset, matches the whole.
        let text = b"The quick brown fox jumps over the lazy dog, twice over.";
        let whole = crc32(text);
        for split in 0..text.len() {
            assert_eq!(crc32_update(crc32(&text[..split]), &text[split..]), whole);
        }
        assert_eq!(
            crc32(b"The quick brown fox jumps over the lazy dog"),
            0x414F_A339
        );
    }
}
