//! Snappy block compression: the greedy hash-table matcher the format was
//! designed around. Matches are found on four-byte hashes; a copy is
//! written whenever one is found, literals cover the rest.

use crate::io::protobuf::tree::write_varint;

/// Hash table entries; a power of two.
const TABLE_SIZE: usize = 1 << 14;
/// Copies reach back at most this far (the two-byte offset form).
const MAX_OFFSET: usize = 65535;
/// Shortest match worth a copy.
const MIN_MATCH: usize = 4;

/// Compresses `data` into one block, appending to `out`.
pub fn compress_block(data: &[u8], out: &mut Vec<u8>) {
    write_varint(out, data.len() as u64);
    if data.len() < MIN_MATCH + 1 {
        if !data.is_empty() {
            emit_literal(data, out);
        }
        return;
    }
    // Positions plus one, so zero means empty.
    let mut table = vec![0u32; TABLE_SIZE];
    let mut literal_start = 0usize;
    let mut position = 0usize;
    let last_start = data.len() - MIN_MATCH;
    while position <= last_start {
        let hash = hash_at(data, position);
        let candidate = table[hash] as usize;
        table[hash] = position as u32 + 1;
        let offset = candidate
            .checked_sub(1)
            .map(|candidate| position - candidate);
        let matched = match offset {
            Some(offset) if offset > 0 && offset <= MAX_OFFSET => {
                let candidate = position - offset;
                if data[candidate..candidate + MIN_MATCH] == data[position..position + MIN_MATCH] {
                    Some((offset, match_length(data, candidate, position)))
                } else {
                    None
                }
            }
            _ => None,
        };
        let (offset, length) = match matched {
            Some(found) => found,
            None => {
                position += 1;
                continue;
            }
        };
        if literal_start < position {
            emit_literal(&data[literal_start..position], out);
        }
        emit_copies(offset, length, out);
        position += length;
        literal_start = position;
        // Remember the position just before the new literal so the next
        // match can reach across what was copied.
        if position >= 1 && position <= last_start {
            let hash = hash_at(data, position - 1);
            table[hash] = position as u32;
        }
    }
    if literal_start < data.len() {
        emit_literal(&data[literal_start..], out);
    }
}

/// Encodes `data` as one block holding a single literal. Every decoder
/// accepts it; used where speed of writing matters more than size.
pub fn encode_literal_block(data: &[u8], out: &mut Vec<u8>) {
    write_varint(out, data.len() as u64);
    if !data.is_empty() {
        emit_literal(data, out);
    }
}

fn hash_at(data: &[u8], position: usize) -> usize {
    let word = u32::from_le_bytes([
        data[position],
        data[position + 1],
        data[position + 2],
        data[position + 3],
    ]);
    (word.wrapping_mul(0x1E35_A7BD) >> (32 - 14)) as usize
}

/// Length of the match between `candidate` and `position`, at least
/// `MIN_MATCH`, compared eight bytes at a time.
fn match_length(data: &[u8], candidate: usize, position: usize) -> usize {
    let limit = data.len() - position;
    let mut length = MIN_MATCH;
    while length + 8 <= limit {
        let left = u64::from_le_bytes(slice8(data, candidate + length));
        let right = u64::from_le_bytes(slice8(data, position + length));
        if left != right {
            return length + ((left ^ right).trailing_zeros() / 8) as usize;
        }
        length += 8;
    }
    while length < limit && data[candidate + length] == data[position + length] {
        length += 1;
    }
    length
}

fn slice8(data: &[u8], at: usize) -> [u8; 8] {
    let mut array = [0u8; 8];
    array.copy_from_slice(&data[at..at + 8]);
    array
}

fn emit_literal(data: &[u8], out: &mut Vec<u8>) {
    let length = data.len() - 1;
    if length < 60 {
        out.push((length as u8) << 2);
    } else if length < 1 << 8 {
        out.push(60 << 2);
        out.push(length as u8);
    } else if length < 1 << 16 {
        out.push(61 << 2);
        out.extend_from_slice(&(length as u16).to_le_bytes());
    } else if length < 1 << 24 {
        out.push(62 << 2);
        out.extend_from_slice(&(length as u32).to_le_bytes()[..3]);
    } else {
        out.push(63 << 2);
        out.extend_from_slice(&(length as u32).to_le_bytes());
    }
    out.extend_from_slice(data);
}

/// Writes a match as one or more copy tags: the one-byte-offset form for
/// short, near matches, the two-byte form otherwise, 64 bytes at a time.
fn emit_copies(offset: usize, mut length: usize, out: &mut Vec<u8>) {
    while length > 0 {
        if (4..=11).contains(&length) && offset < 2048 {
            out.push(1 | (((length - 4) as u8) << 2) | (((offset >> 8) as u8) << 5));
            out.push(offset as u8);
            return;
        }
        let piece = length.min(64);
        // Never leave a remainder shorter than a one-byte copy can hold.
        let piece = if length - piece > 0 && length - piece < 4 {
            length - 4
        } else {
            piece
        };
        out.push(2 | (((piece - 1) as u8) << 2));
        out.extend_from_slice(&(offset as u16).to_le_bytes());
        length -= piece;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::snappy::decompress_block;

    fn round_trip(data: &[u8]) -> usize {
        let mut block = Vec::new();
        compress_block(data, &mut block);
        let mut back = Vec::new();
        decompress_block(&block, &mut back, 1 << 24).unwrap();
        assert_eq!(back, data);
        block.len()
    }

    #[test]
    fn literal_blocks_decode_back() {
        for size in [0usize, 1, 59, 60, 61, 255, 256, 65535, 65536, 70000] {
            let data: Vec<u8> = (0..size).map(|index| (index * 7) as u8).collect();
            let mut block = Vec::new();
            encode_literal_block(&data, &mut block);
            let mut back = Vec::new();
            decompress_block(&block, &mut back, 1 << 20).unwrap();
            assert_eq!(back, data, "size {size}");
        }
    }

    #[test]
    fn compressed_blocks_decode_back_and_shrink() {
        assert_eq!(round_trip(b""), 1);
        round_trip(b"abc");
        let repeated = b"the quick brown fox jumps over the lazy dog. ".repeat(200);
        assert!(round_trip(&repeated) < repeated.len() / 10);
        let mut mixed = Vec::new();
        for index in 0..100_000u32 {
            mixed.extend_from_slice(&(index.wrapping_mul(2_654_435_761) % 977).to_le_bytes());
        }
        round_trip(&mixed);
        let random: Vec<u8> = (0..50_000u32)
            .map(|index| (index.wrapping_mul(2_654_435_761) >> 24) as u8)
            .collect();
        assert!(round_trip(&random) <= random.len() + 10);
        let long_run = vec![7u8; 300_000];
        assert!(round_trip(&long_run) < 16_000);
    }
}
