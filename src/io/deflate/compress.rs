//! DEFLATE compression: LZ77 over a 32 KiB window with hash chains, then
//! dynamic Huffman blocks, falling back to stored blocks when they would
//! be smaller. Written for a good ratio at a sensible speed, not for the
//! last percent of either.

use super::inflate::{CODE_LENGTH_ORDER, DISTANCE_BASE, DISTANCE_EXTRA, LENGTH_BASE, LENGTH_EXTRA};

const WINDOW: usize = 32 * 1024;
const MIN_MATCH: usize = 3;
const MAX_MATCH: usize = 258;
const HASH_BITS: u32 = 15;
const HASH_SIZE: usize = 1 << HASH_BITS;
/// Positions examined per match search.
const MAX_CHAIN: usize = 48;
/// Symbols per block before its Huffman codes are rebuilt.
const BLOCK_SYMBOLS: usize = 32 * 1024;
const LITERALS: usize = 286;
const DISTANCES: usize = 30;
const END_OF_BLOCK: u16 = 256;

/// How hard the matcher looks: `Default` walks 48 chain positions for a
/// zlib level-6 ratio; `Fast` walks 4, for output nobody keeps compressed
/// long (a Word body a reader re-saves), at two to three times the speed.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Fast,
    Default,
}

/// Compresses `input` as a raw deflate stream, appending to `out`.
pub fn deflate(input: &[u8], out: &mut Vec<u8>) {
    if input.is_empty() {
        let mut writer = BitWriter::new(out);
        // One empty fixed block: final, type 1, end-of-block (7 zero bits).
        writer.bits(1, 1);
        writer.bits(1, 2);
        writer.bits(0, 7);
        writer.finish();
        return;
    }
    deflate_part(input, out, Level::Default, true);
}

/// Compresses one part of a stream that arrives in pieces. Matches never
/// reach into an earlier part, and a part that is not final ends on a
/// byte boundary with an empty stored block (a sync flush), so parts can
/// be concatenated; the final part ends the stream. An empty non-final
/// part writes nothing.
pub fn deflate_part(input: &[u8], out: &mut Vec<u8>, level: Level, is_final: bool) {
    if input.is_empty() {
        if is_final {
            deflate(input, out);
        }
        return;
    }
    let chain_limit = match level {
        Level::Fast => 4,
        Level::Default => MAX_CHAIN,
    };
    let mut writer = BitWriter::new(out);
    let mut matcher = Matcher::new(input.len(), chain_limit);
    let mut symbols: Vec<Symbol> = Vec::with_capacity(BLOCK_SYMBOLS);
    let mut position = 0usize;
    let mut block_start = 0usize;
    while position < input.len() {
        let (length, distance) = matcher.find(input, position);
        if length >= MIN_MATCH {
            symbols.push(Symbol::Match {
                length: length as u16,
                distance: distance as u16,
            });
            matcher.insert_range(input, position + 1, position + length);
            position += length;
        } else {
            symbols.push(Symbol::Literal(input[position]));
            position += 1;
        }
        if symbols.len() >= BLOCK_SYMBOLS {
            let ends_stream = is_final && position >= input.len();
            write_block(
                &mut writer,
                &input[block_start..position],
                &symbols,
                ends_stream,
            );
            symbols.clear();
            block_start = position;
        }
    }
    if !symbols.is_empty() || block_start < input.len() {
        write_block(&mut writer, &input[block_start..], &symbols, is_final);
    }
    if !is_final {
        write_stored(&mut writer, &[], false);
    }
    writer.finish();
}

#[derive(Clone, Copy)]
enum Symbol {
    Literal(u8),
    Match { length: u16, distance: u16 },
}

struct Matcher {
    head: Vec<u32>,
    prev: Vec<u32>,
    chain_limit: usize,
}

impl Matcher {
    fn new(length: usize, chain_limit: usize) -> Matcher {
        Matcher {
            head: vec![u32::MAX; HASH_SIZE],
            prev: vec![u32::MAX; length],
            chain_limit,
        }
    }

    fn hash(input: &[u8], position: usize) -> usize {
        let word = (u32::from(input[position]) << 16)
            | (u32::from(input[position + 1]) << 8)
            | u32::from(input[position + 2]);
        (word.wrapping_mul(0x9E37_79B1) >> (32 - HASH_BITS)) as usize
    }

    fn insert(&mut self, input: &[u8], position: usize) {
        if position + MIN_MATCH > input.len() {
            return;
        }
        let hash = Self::hash(input, position);
        self.prev[position] = self.head[hash];
        self.head[hash] = position as u32;
    }

    fn insert_range(&mut self, input: &[u8], from: usize, to: usize) {
        for position in from..to.min(input.len()) {
            self.insert(input, position);
        }
    }

    /// Longest match ending the chain search early on a maximal one.
    /// Returns `(length, distance)`; a length under `MIN_MATCH` means none.
    fn find(&mut self, input: &[u8], position: usize) -> (usize, usize) {
        if position + MIN_MATCH > input.len() {
            return (0, 0);
        }
        let hash = Self::hash(input, position);
        let mut candidate = self.head[hash];
        self.prev[position] = candidate;
        self.head[hash] = position as u32;
        let limit = (input.len() - position).min(MAX_MATCH);
        let mut best_length = 0usize;
        let mut best_distance = 0usize;
        let mut chain = 0usize;
        while candidate != u32::MAX && chain < self.chain_limit {
            let start = candidate as usize;
            let distance = position - start;
            if distance > WINDOW {
                break;
            }
            if input[start + best_length.min(limit - 1)]
                == input[position + best_length.min(limit - 1)]
            {
                let length = common_prefix(input, start, position, limit);
                if length > best_length {
                    best_length = length;
                    best_distance = distance;
                    if length == limit {
                        break;
                    }
                }
            }
            candidate = self.prev[start];
            chain += 1;
        }
        (best_length, best_distance)
    }
}

fn common_prefix(input: &[u8], left: usize, right: usize, limit: usize) -> usize {
    let mut length = 0usize;
    while length + 8 <= limit {
        let a = u64::from_le_bytes(eight(input, left + length));
        let b = u64::from_le_bytes(eight(input, right + length));
        if a != b {
            return length + ((a ^ b).trailing_zeros() / 8) as usize;
        }
        length += 8;
    }
    while length < limit && input[left + length] == input[right + length] {
        length += 1;
    }
    length
}

fn eight(input: &[u8], at: usize) -> [u8; 8] {
    let mut array = [0u8; 8];
    array.copy_from_slice(&input[at..at + 8]);
    array
}

// ----- Huffman codes -----

/// Canonical code lengths for `frequencies`, none longer than `limit`.
/// Frequencies are halved and the tree rebuilt until the limit holds.
fn code_lengths(frequencies: &[u32], limit: u8) -> Vec<u8> {
    let mut weights: Vec<u32> = frequencies.to_vec();
    loop {
        let lengths = huffman_lengths(&weights);
        if lengths.iter().all(|length| *length <= limit) {
            return lengths;
        }
        for weight in &mut weights {
            if *weight > 0 {
                *weight = (*weight >> 1) | 1;
            }
        }
    }
}

/// Unlimited Huffman code lengths by repeated merging of the two lightest
/// nodes. A single used symbol gets length one, as deflate requires.
fn huffman_lengths(weights: &[u32]) -> Vec<u8> {
    let count = weights.len();
    let mut lengths = vec![0u8; count];
    let mut nodes: Vec<(u64, usize)> = weights
        .iter()
        .enumerate()
        .filter(|(_, weight)| **weight > 0)
        .map(|(symbol, weight)| (u64::from(*weight), symbol))
        .collect();
    if nodes.is_empty() {
        return lengths;
    }
    if nodes.len() == 1 {
        lengths[nodes[0].1] = 1;
        return lengths;
    }
    // Node ids: symbols first, then internal nodes; parents track depth.
    let mut parents: Vec<usize> = vec![usize::MAX; count];
    let mut next_id = count;
    let mut heap: Vec<(u64, usize)> = std::mem::take(&mut nodes);
    while heap.len() > 1 {
        heap.sort_unstable_by_key(|node| std::cmp::Reverse(node.0));
        let (weight_a, node_a) = heap.pop().unwrap_or((0, 0));
        let (weight_b, node_b) = heap.pop().unwrap_or((0, 0));
        let id = next_id;
        next_id += 1;
        parents.push(usize::MAX);
        parents[node_a] = id;
        parents[node_b] = id;
        heap.push((weight_a + weight_b, id));
    }
    for (symbol, length) in lengths.iter_mut().enumerate() {
        if weights[symbol] == 0 {
            continue;
        }
        let mut depth = 0u8;
        let mut node = symbol;
        while parents[node] != usize::MAX {
            node = parents[node];
            depth = depth.saturating_add(1);
        }
        *length = depth;
    }
    lengths
}

/// Canonical codes from lengths, bit-reversed for LSB-first output.
fn canonical_codes(lengths: &[u8]) -> Vec<u16> {
    let mut counts = [0u16; 16];
    for length in lengths {
        counts[usize::from(*length)] += 1;
    }
    counts[0] = 0;
    let mut next = [0u16; 16];
    let mut code = 0u16;
    for length in 1..16 {
        code = (code + counts[length - 1]) << 1;
        next[length] = code;
    }
    lengths
        .iter()
        .map(|length| {
            if *length == 0 {
                return 0;
            }
            let code = next[usize::from(*length)];
            next[usize::from(*length)] += 1;
            reverse(code, *length)
        })
        .collect()
}

fn reverse(code: u16, length: u8) -> u16 {
    let mut reversed = 0u16;
    for bit in 0..length {
        reversed |= ((code >> bit) & 1) << (length - 1 - bit);
    }
    reversed
}

// ----- blocks -----

fn length_symbol(length: u16) -> (u16, u8, u16) {
    let index = LENGTH_BASE
        .iter()
        .rposition(|base| *base <= length)
        .unwrap_or(0);
    let extra_bits = LENGTH_EXTRA[index];
    (257 + index as u16, extra_bits, length - LENGTH_BASE[index])
}

fn distance_symbol(distance: u16) -> (u16, u8, u16) {
    let index = DISTANCE_BASE
        .iter()
        .rposition(|base| *base <= distance)
        .unwrap_or(0);
    let extra_bits = DISTANCE_EXTRA[index];
    (index as u16, extra_bits, distance - DISTANCE_BASE[index])
}

fn write_block(writer: &mut BitWriter<'_>, raw: &[u8], symbols: &[Symbol], is_final: bool) {
    let mut literal_frequencies = [0u32; LITERALS];
    let mut distance_frequencies = [0u32; DISTANCES];
    for symbol in symbols {
        match *symbol {
            Symbol::Literal(byte) => literal_frequencies[usize::from(byte)] += 1,
            Symbol::Match { length, distance } => {
                literal_frequencies[usize::from(length_symbol(length).0)] += 1;
                distance_frequencies[usize::from(distance_symbol(distance).0)] += 1;
            }
        }
    }
    literal_frequencies[usize::from(END_OF_BLOCK)] += 1;
    let literal_lengths = code_lengths(&literal_frequencies, 15);
    let mut distance_lengths = code_lengths(&distance_frequencies, 15);
    // At least one distance code must be present.
    if distance_lengths.iter().all(|length| *length == 0) {
        distance_lengths[0] = 1;
    }
    let literal_codes = canonical_codes(&literal_lengths);
    let distance_codes = canonical_codes(&distance_lengths);

    // Trim trailing unused codes from the header counts.
    let literal_count = literal_lengths
        .iter()
        .rposition(|length| *length > 0)
        .map_or(257, |last| (last + 1).max(257));
    let distance_count = distance_lengths
        .iter()
        .rposition(|length| *length > 0)
        .map_or(1, |last| last + 1);
    let mut all_lengths: Vec<u8> = Vec::with_capacity(literal_count + distance_count);
    all_lengths.extend_from_slice(&literal_lengths[..literal_count]);
    all_lengths.extend_from_slice(&distance_lengths[..distance_count]);
    let runs = run_length_encode(&all_lengths);
    let mut code_length_frequencies = [0u32; 19];
    for (symbol, _, _) in &runs {
        code_length_frequencies[usize::from(*symbol)] += 1;
    }
    let code_length_lengths = code_lengths(&code_length_frequencies, 7);
    let code_length_codes = canonical_codes(&code_length_lengths);
    let code_length_count = CODE_LENGTH_ORDER
        .iter()
        .rposition(|index| code_length_lengths[*index] > 0)
        .map_or(4, |last| (last + 1).max(4));

    // Size of the dynamic block in bits, to compare with stored blocks.
    let mut dynamic_bits = 3 + 5 + 5 + 4 + 3 * code_length_count;
    for (symbol, extra_bits, _) in &runs {
        dynamic_bits +=
            usize::from(code_length_lengths[usize::from(*symbol)]) + usize::from(*extra_bits);
    }
    for symbol in symbols {
        dynamic_bits += match *symbol {
            Symbol::Literal(byte) => usize::from(literal_lengths[usize::from(byte)]),
            Symbol::Match { length, distance } => {
                let (length_code, length_extra, _) = length_symbol(length);
                let (distance_code, distance_extra, _) = distance_symbol(distance);
                usize::from(literal_lengths[usize::from(length_code)])
                    + usize::from(length_extra)
                    + usize::from(distance_lengths[usize::from(distance_code)])
                    + usize::from(distance_extra)
            }
        };
    }
    dynamic_bits += usize::from(literal_lengths[usize::from(END_OF_BLOCK)]);
    let stored_bits = raw.len().div_ceil(65535).max(1) * 40 + raw.len() * 8;
    if stored_bits <= dynamic_bits {
        write_stored(writer, raw, is_final);
        return;
    }

    writer.bits(u32::from(is_final), 1);
    writer.bits(2, 2);
    writer.bits((literal_count - 257) as u32, 5);
    writer.bits((distance_count - 1) as u32, 5);
    writer.bits((code_length_count - 4) as u32, 4);
    for index in CODE_LENGTH_ORDER.iter().take(code_length_count) {
        writer.bits(u32::from(code_length_lengths[*index]), 3);
    }
    for (symbol, extra_bits, extra) in &runs {
        writer.code(
            code_length_codes[usize::from(*symbol)],
            code_length_lengths[usize::from(*symbol)],
        );
        if *extra_bits > 0 {
            writer.bits(u32::from(*extra), u32::from(*extra_bits));
        }
    }
    for symbol in symbols {
        match *symbol {
            Symbol::Literal(byte) => writer.code(
                literal_codes[usize::from(byte)],
                literal_lengths[usize::from(byte)],
            ),
            Symbol::Match { length, distance } => {
                let (length_code, length_extra, length_rest) = length_symbol(length);
                writer.code(
                    literal_codes[usize::from(length_code)],
                    literal_lengths[usize::from(length_code)],
                );
                if length_extra > 0 {
                    writer.bits(u32::from(length_rest), u32::from(length_extra));
                }
                let (distance_code, distance_extra, distance_rest) = distance_symbol(distance);
                writer.code(
                    distance_codes[usize::from(distance_code)],
                    distance_lengths[usize::from(distance_code)],
                );
                if distance_extra > 0 {
                    writer.bits(u32::from(distance_rest), u32::from(distance_extra));
                }
            }
        }
    }
    writer.code(
        literal_codes[usize::from(END_OF_BLOCK)],
        literal_lengths[usize::from(END_OF_BLOCK)],
    );
}

fn write_stored(writer: &mut BitWriter<'_>, raw: &[u8], is_final: bool) {
    let mut chunks = raw.chunks(65535).peekable();
    if raw.is_empty() {
        writer.bits(u32::from(is_final), 1);
        writer.bits(0, 2);
        writer.align();
        writer.bytes(&[0, 0, 0xFF, 0xFF]);
        return;
    }
    while let Some(chunk) = chunks.next() {
        let last = chunks.peek().is_none();
        writer.bits(u32::from(is_final && last), 1);
        writer.bits(0, 2);
        writer.align();
        let length = chunk.len() as u16;
        writer.bytes(&length.to_le_bytes());
        writer.bytes(&(!length).to_le_bytes());
        writer.bytes(chunk);
    }
}

/// Code lengths as (symbol, extra bit count, extra value) with the 16, 17,
/// and 18 repeat codes.
fn run_length_encode(lengths: &[u8]) -> Vec<(u8, u8, u16)> {
    let mut runs = Vec::new();
    let mut index = 0usize;
    while index < lengths.len() {
        let value = lengths[index];
        let mut run = 1usize;
        while index + run < lengths.len() && lengths[index + run] == value {
            run += 1;
        }
        if value == 0 && run >= 3 {
            let mut remaining = run;
            while remaining >= 3 {
                if remaining >= 11 {
                    let piece = remaining.min(138);
                    runs.push((18, 7, (piece - 11) as u16));
                    remaining -= piece;
                } else {
                    let piece = remaining.min(10);
                    runs.push((17, 3, (piece - 3) as u16));
                    remaining -= piece;
                }
            }
            for _ in 0..remaining {
                runs.push((0, 0, 0));
            }
            index += run;
            continue;
        }
        runs.push((value, 0, 0));
        let mut remaining = run - 1;
        while remaining >= 3 {
            let piece = remaining.min(6);
            runs.push((16, 2, (piece - 3) as u16));
            remaining -= piece;
        }
        for _ in 0..remaining {
            runs.push((value, 0, 0));
        }
        index += run;
    }
    runs
}

// ----- bits -----

struct BitWriter<'o> {
    out: &'o mut Vec<u8>,
    buffer: u64,
    count: u32,
}

impl<'o> BitWriter<'o> {
    fn new(out: &'o mut Vec<u8>) -> BitWriter<'o> {
        BitWriter {
            out,
            buffer: 0,
            count: 0,
        }
    }

    fn bits(&mut self, value: u32, count: u32) {
        self.buffer |= u64::from(value) << self.count;
        self.count += count;
        while self.count >= 8 {
            self.out.push(self.buffer as u8);
            self.buffer >>= 8;
            self.count -= 8;
        }
    }

    fn code(&mut self, code: u16, length: u8) {
        self.bits(u32::from(code), u32::from(length));
    }

    fn align(&mut self) {
        if self.count > 0 {
            self.out.push(self.buffer as u8);
            self.buffer = 0;
            self.count = 0;
        }
    }

    fn bytes(&mut self, bytes: &[u8]) {
        self.out.extend_from_slice(bytes);
    }

    fn finish(&mut self) {
        self.align();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::deflate::inflate;

    fn round_trip(data: &[u8]) -> usize {
        let mut compressed = Vec::new();
        deflate(data, &mut compressed);
        let mut back = Vec::new();
        let consumed = inflate(&compressed, &mut back, 1 << 26).unwrap();
        assert_eq!(consumed, compressed.len());
        assert_eq!(back, data);
        compressed.len()
    }

    #[test]
    fn compresses_a_large_input_quickly() {
        let mut data = Vec::with_capacity(8 << 20);
        let words: [&[u8]; 8] = [
            b"alpha ",
            b"beta ",
            b"gamma ",
            b"delta ",
            b"epsilon ",
            b"zeta ",
            b"eta ",
            b"theta ",
        ];
        let mut state = 12345u32;
        while data.len() < 8 << 20 {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12345);
            data.extend_from_slice(words[(state >> 16) as usize % 8]);
            if state % 7 == 0 {
                data.push(b'\n');
            }
        }
        let started = std::time::Instant::now();
        let mut compressed = Vec::new();
        deflate(&data, &mut compressed);
        let elapsed = started.elapsed();
        let mut back = Vec::new();
        inflate(&compressed, &mut back, 1 << 26).unwrap();
        assert_eq!(back, data);
        eprintln!(
            "deflate 8 MiB of words: {} bytes ({:.1}%) in {:.0} ms ({:.0} MB/s)",
            compressed.len(),
            compressed.len() as f64 * 100.0 / data.len() as f64,
            elapsed.as_secs_f64() * 1000.0,
            data.len() as f64 / 1_048_576.0 / elapsed.as_secs_f64()
        );
    }

    #[test]
    fn parts_concatenate_into_one_stream() {
        let text = b"parts of a stream, each compressed on its own. ".repeat(3000);
        let mut out = Vec::new();
        for (index, part) in text.chunks(50_000).enumerate() {
            let last = index == text.chunks(50_000).count() - 1;
            deflate_part(part, &mut out, Level::Fast, last);
        }
        let mut back = Vec::new();
        inflate(&out, &mut back, 1 << 26).unwrap();
        assert_eq!(back, text);
        // An empty final part after non-final ones still ends the stream.
        let mut out = Vec::new();
        deflate_part(b"abc", &mut out, Level::Default, false);
        deflate_part(&[], &mut out, Level::Default, true);
        let mut back = Vec::new();
        inflate(&out, &mut back, 1 << 26).unwrap();
        assert_eq!(back, b"abc");
    }

    #[test]
    fn round_trips_all_shapes() {
        assert_eq!(round_trip(b""), 2);
        round_trip(b"a");
        round_trip(b"abc");
        let text =
            b"the quick brown fox jumps over the lazy dog, said the deflate test. ".repeat(3000);
        let text_size = round_trip(&text);
        assert!(text_size < text.len() / 20);
        let random: Vec<u8> = (0..200_000u32)
            .map(|index| (index.wrapping_mul(2_654_435_761) >> 24) as u8)
            .collect();
        assert!(round_trip(&random) <= random.len() + random.len() / 200 + 64);
        let mut mixed = Vec::new();
        for index in 0..60_000u32 {
            mixed.extend_from_slice(&(index % 1_003).to_le_bytes());
            mixed.push(b'x');
        }
        let mixed_size = round_trip(&mixed);
        assert!(mixed_size < mixed.len() / 2);
        eprintln!("deflate sizes: text {text_size}, mixed {mixed_size}");
        let long_run = vec![9u8; 300_000];
        assert!(round_trip(&long_run) < 2_000);
        for size in [65_534usize, 65_535, 65_536, 65_537, 131_071] {
            let data: Vec<u8> = (0..size)
                .map(|index| ((index * 31) ^ (index >> 3)) as u8)
                .collect();
            round_trip(&data);
        }
    }
}
