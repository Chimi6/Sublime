//! Inflate: stored, fixed-Huffman, and dynamic-Huffman blocks, decoded with
//! full lookup tables so each symbol costs one table read.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InflateError {
    /// The stream ended before the final block did.
    Truncated,
    /// A block type of 3, a bad stored-block length check, an over-subscribed
    /// or incomplete Huffman code, or a code with no symbol.
    Corrupt { offset: usize, what: &'static str },
    /// A back-reference before the start of the output.
    BadDistance { offset: usize },
    /// Output would exceed the caller's limit.
    TooLarge,
}

impl fmt::Display for InflateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InflateError::Truncated => write!(formatter, "deflate stream truncated"),
            InflateError::Corrupt { offset, what } => {
                write!(formatter, "deflate stream corrupt at byte {offset}: {what}")
            }
            InflateError::BadDistance { offset } => write!(
                formatter,
                "deflate back-reference at byte {offset} reaches before the output"
            ),
            InflateError::TooLarge => write!(formatter, "inflated output exceeds the limit"),
        }
    }
}

impl std::error::Error for InflateError {}

struct BitReader<'a> {
    bytes: &'a [u8],
    position: usize,
    buffer: u64,
    count: u32,
}

impl<'a> BitReader<'a> {
    fn new(bytes: &'a [u8]) -> BitReader<'a> {
        BitReader {
            bytes,
            position: 0,
            buffer: 0,
            count: 0,
        }
    }

    /// Fills the buffer with as many whole bytes as fit.
    fn refill(&mut self) {
        while self.count <= 56 {
            match self.bytes.get(self.position) {
                Some(byte) => {
                    self.buffer |= u64::from(*byte) << self.count;
                    self.count += 8;
                    self.position += 1;
                }
                None => return,
            }
        }
    }

    fn bits(&mut self, n: u32) -> Result<u32, InflateError> {
        if n == 0 {
            return Ok(0);
        }
        if self.count < n {
            self.refill();
            if self.count < n {
                return Err(InflateError::Truncated);
            }
        }
        let value = (self.buffer & ((1u64 << n) - 1)) as u32;
        self.buffer >>= n;
        self.count -= n;
        Ok(value)
    }

    fn align_to_byte(&mut self) {
        let drop = self.count % 8;
        self.buffer >>= drop;
        self.count -= drop;
    }

    /// Bytes consumed from the input, counting buffered whole bytes as not
    /// yet consumed.
    fn consumed(&self) -> usize {
        self.position - (self.count / 8) as usize
    }
}

/// A canonical Huffman code as a table indexed by the next `bits` input
/// bits (least significant first, as deflate packs them). Each entry is
/// `symbol << 4 | code length`; a length of zero marks an unused code.
struct Huffman {
    table: Vec<u16>,
    bits: u32,
}

impl Huffman {
    fn build(lengths: &[u8], offset: usize) -> Result<Huffman, InflateError> {
        let max_length = u32::from(lengths.iter().copied().max().unwrap_or(0));
        if max_length == 0 {
            return Ok(Huffman {
                table: vec![0; 2],
                bits: 1,
            });
        }
        let mut counts = [0u32; 16];
        for length in lengths {
            counts[usize::from(*length)] += 1;
        }
        counts[0] = 0;
        let mut left = 1i32;
        for count in &counts[1..] {
            left = (left << 1) - *count as i32;
            if left < 0 {
                return Err(InflateError::Corrupt {
                    offset,
                    what: "over-subscribed Huffman code",
                });
            }
        }
        let mut next_code = [0u32; 16];
        let mut code = 0u32;
        for length in 1..16 {
            code = (code + counts[length - 1]) << 1;
            next_code[length] = code;
        }
        let size = 1usize << max_length;
        let mut table = vec![0u16; size];
        for (symbol, length) in lengths.iter().enumerate() {
            let length = u32::from(*length);
            if length == 0 {
                continue;
            }
            let code = next_code[length as usize];
            next_code[length as usize] += 1;
            let reversed = reverse_bits(code, length) as usize;
            let entry = ((symbol as u16) << 4) | length as u16;
            let step = 1usize << length;
            let mut index = reversed;
            while index < size {
                table[index] = entry;
                index += step;
            }
        }
        Ok(Huffman {
            table,
            bits: max_length,
        })
    }

    fn decode(&self, reader: &mut BitReader<'_>, offset: usize) -> Result<u16, InflateError> {
        if reader.count < self.bits {
            reader.refill();
        }
        let index = (reader.buffer & ((1u64 << self.bits) - 1)) as usize;
        let entry = self.table[index];
        let length = u32::from(entry & 15);
        if length == 0 {
            return Err(InflateError::Corrupt {
                offset,
                what: "code with no symbol",
            });
        }
        if reader.count < length {
            return Err(InflateError::Truncated);
        }
        reader.buffer >>= length;
        reader.count -= length;
        Ok(entry >> 4)
    }
}

fn reverse_bits(code: u32, length: u32) -> u32 {
    let mut reversed = 0u32;
    for bit in 0..length {
        reversed |= ((code >> bit) & 1) << (length - 1 - bit);
    }
    reversed
}

const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DISTANCE_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DISTANCE_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
const CODE_LENGTH_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

/// Inflates a raw deflate stream (no zlib or gzip header) from `input`,
/// appending to `out`. Returns the number of input bytes the stream used.
/// `limit` caps the output.
pub fn inflate(input: &[u8], out: &mut Vec<u8>, limit: usize) -> Result<usize, InflateError> {
    let mut reader = BitReader::new(input);
    let start = out.len();
    loop {
        let is_final = reader.bits(1)? == 1;
        let kind = reader.bits(2)?;
        match kind {
            0 => stored_block(&mut reader, out, start, limit)?,
            1 => {
                let (literals, distances) = fixed_tables()?;
                compressed_block(&mut reader, out, start, limit, &literals, &distances)?;
            }
            2 => {
                let (literals, distances) = dynamic_tables(&mut reader)?;
                compressed_block(&mut reader, out, start, limit, &literals, &distances)?;
            }
            _ => {
                return Err(InflateError::Corrupt {
                    offset: reader.consumed(),
                    what: "block type 3",
                });
            }
        }
        if is_final {
            return Ok(reader.consumed());
        }
    }
}

fn stored_block(
    reader: &mut BitReader<'_>,
    out: &mut Vec<u8>,
    start: usize,
    limit: usize,
) -> Result<(), InflateError> {
    reader.align_to_byte();
    let length = reader.bits(16)? as usize;
    let check = reader.bits(16)? as usize;
    if length != (!check & 0xFFFF) {
        return Err(InflateError::Corrupt {
            offset: reader.consumed(),
            what: "stored block length check",
        });
    }
    if out.len() - start + length > limit {
        return Err(InflateError::TooLarge);
    }
    // The buffer holds whole bytes now; drain it before reading raw bytes.
    let mut remaining = length;
    while remaining > 0 && reader.count >= 8 {
        out.push(reader.bits(8)? as u8);
        remaining -= 1;
    }
    let end = reader.position + remaining;
    if end > reader.bytes.len() {
        return Err(InflateError::Truncated);
    }
    out.extend_from_slice(&reader.bytes[reader.position..end]);
    reader.position = end;
    Ok(())
}

fn fixed_tables() -> Result<(Huffman, Huffman), InflateError> {
    let mut lengths = [0u8; 288];
    for (symbol, length) in lengths.iter_mut().enumerate() {
        *length = match symbol {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    let literals = Huffman::build(&lengths, 0)?;
    let distances = Huffman::build(&[5u8; 30], 0)?;
    Ok((literals, distances))
}

fn dynamic_tables(reader: &mut BitReader<'_>) -> Result<(Huffman, Huffman), InflateError> {
    let offset = reader.consumed();
    let literal_count = reader.bits(5)? as usize + 257;
    let distance_count = reader.bits(5)? as usize + 1;
    let code_length_count = reader.bits(4)? as usize + 4;
    if literal_count > 286 || distance_count > 30 {
        return Err(InflateError::Corrupt {
            offset,
            what: "too many codes",
        });
    }
    let mut code_lengths = [0u8; 19];
    for index in CODE_LENGTH_ORDER.iter().take(code_length_count) {
        code_lengths[*index] = reader.bits(3)? as u8;
    }
    let code_length_code = Huffman::build(&code_lengths, offset)?;
    let total = literal_count + distance_count;
    let mut lengths = vec![0u8; total];
    let mut index = 0usize;
    while index < total {
        let symbol = code_length_code.decode(reader, offset)?;
        match symbol {
            0..=15 => {
                lengths[index] = symbol as u8;
                index += 1;
            }
            16 => {
                if index == 0 {
                    return Err(InflateError::Corrupt {
                        offset,
                        what: "repeat with no previous length",
                    });
                }
                let previous = lengths[index - 1];
                let repeat = 3 + reader.bits(2)? as usize;
                fill(&mut lengths, &mut index, previous, repeat, offset)?;
            }
            17 => {
                let repeat = 3 + reader.bits(3)? as usize;
                fill(&mut lengths, &mut index, 0, repeat, offset)?;
            }
            _ => {
                let repeat = 11 + reader.bits(7)? as usize;
                fill(&mut lengths, &mut index, 0, repeat, offset)?;
            }
        }
    }
    if lengths[256] == 0 {
        return Err(InflateError::Corrupt {
            offset,
            what: "no end-of-block code",
        });
    }
    let literals = Huffman::build(&lengths[..literal_count], offset)?;
    let distances = Huffman::build(&lengths[literal_count..], offset)?;
    Ok((literals, distances))
}

fn fill(
    lengths: &mut [u8],
    index: &mut usize,
    value: u8,
    repeat: usize,
    offset: usize,
) -> Result<(), InflateError> {
    if *index + repeat > lengths.len() {
        return Err(InflateError::Corrupt {
            offset,
            what: "code lengths overflow",
        });
    }
    for slot in &mut lengths[*index..*index + repeat] {
        *slot = value;
    }
    *index += repeat;
    Ok(())
}

fn compressed_block(
    reader: &mut BitReader<'_>,
    out: &mut Vec<u8>,
    start: usize,
    limit: usize,
    literals: &Huffman,
    distances: &Huffman,
) -> Result<(), InflateError> {
    loop {
        let offset = reader.consumed();
        let symbol = literals.decode(reader, offset)?;
        if symbol < 256 {
            if out.len() - start >= limit {
                return Err(InflateError::TooLarge);
            }
            out.push(symbol as u8);
            continue;
        }
        if symbol == 256 {
            return Ok(());
        }
        let length_index = usize::from(symbol - 257);
        if length_index >= LENGTH_BASE.len() {
            return Err(InflateError::Corrupt {
                offset,
                what: "length code out of range",
            });
        }
        let length = usize::from(LENGTH_BASE[length_index])
            + reader.bits(u32::from(LENGTH_EXTRA[length_index]))? as usize;
        let distance_symbol = usize::from(distances.decode(reader, offset)?);
        if distance_symbol >= DISTANCE_BASE.len() {
            return Err(InflateError::Corrupt {
                offset,
                what: "distance code out of range",
            });
        }
        let distance = usize::from(DISTANCE_BASE[distance_symbol])
            + reader.bits(u32::from(DISTANCE_EXTRA[distance_symbol]))? as usize;
        if distance > out.len() - start {
            return Err(InflateError::BadDistance { offset });
        }
        if out.len() - start + length > limit {
            return Err(InflateError::TooLarge);
        }
        let from = out.len() - distance;
        if distance >= length {
            out.extend_from_within(from..from + length);
        } else {
            for index in 0..length {
                let byte = out[from + index];
                out.push(byte);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(input: &[u8]) -> Result<Vec<u8>, InflateError> {
        let mut out = Vec::new();
        inflate(input, &mut out, 1 << 20)?;
        Ok(out)
    }

    #[test]
    fn stored_block_round_trips() {
        let input = [0x01, 0x05, 0x00, 0xFA, 0xFF, b'h', b'e', b'l', b'l', b'o'];
        assert_eq!(run(&input).unwrap(), b"hello");
    }

    #[test]
    fn fixed_huffman_block() {
        let input = [75, 76, 74, 78, 68, 66, 0];
        assert_eq!(run(&input).unwrap(), b"abcabcabcabcabc");
    }

    #[test]
    fn dynamic_huffman_block() {
        let input = [
            203, 72, 205, 201, 201, 87, 200, 64, 39, 117, 20, 138, 19, 51, 83, 20, 74, 50, 82, 21,
            82, 82, 211, 114, 18, 75, 82, 21, 74, 82, 139, 75, 244, 48, 85, 82, 89, 61, 0,
        ];
        let expected = b"hello hello hello hello, said the deflate test. ".repeat(3);
        assert_eq!(run(&input).unwrap(), expected);
    }

    #[test]
    fn errors_do_not_panic() {
        assert_eq!(
            run(&[0x01, 0x05, 0x00, 0x00, 0x00]),
            Err(InflateError::Corrupt {
                offset: 5,
                what: "stored block length check"
            })
        );
        assert_eq!(
            run(&[0x07]),
            Err(InflateError::Corrupt {
                offset: 1,
                what: "block type 3"
            })
        );
        assert_eq!(run(&[]), Err(InflateError::Truncated));
        let mut out = Vec::new();
        assert_eq!(
            inflate(&[75, 76, 74, 78, 68, 66, 0], &mut out, 4),
            Err(InflateError::TooLarge)
        );
    }
}
