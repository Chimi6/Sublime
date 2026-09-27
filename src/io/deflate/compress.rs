//! DEFLATE compression: LZ77 over a 32 KiB window with hash chains, then
//! dynamic Huffman blocks, falling back to stored blocks when they would
//! be smaller. The default level is zlib's lazy evaluation with a price
//! check on short matches; it writes zlib level 6's ratio or better at
//! about its speed.

use super::inflate::{CODE_LENGTH_ORDER, DISTANCE_BASE, DISTANCE_EXTRA, LENGTH_BASE, LENGTH_EXTRA};

const WINDOW: usize = 32 * 1024;
const MIN_MATCH: usize = 3;
const MAX_MATCH: usize = 258;
const HASH_BITS: u32 = 15;
const HASH_SIZE: usize = 1 << HASH_BITS;
/// Symbols per block before its Huffman codes are rebuilt.
const BLOCK_SYMBOLS: usize = 32 * 1024;
const LITERALS: usize = 286;
const DISTANCES: usize = 30;
const END_OF_BLOCK: u16 = 256;

/// How hard the matcher looks: `Default` is lazy matching over chains of
/// up to 32 positions; `Fast` is greedy on the first candidate, for
/// output nobody keeps compressed long (a Word body a reader re-saves).
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
    if level == Level::Default {
        deflate_lazy(input, out, is_final);
        return;
    }
    let mut writer = BitWriter::new(out);
    let mut matcher = Matcher::new();
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
            // The inside of a long match is not indexed past eight bytes,
            // as zlib's fast strategy does.
            if length <= 8 {
                matcher.insert_range(input, position + 1, position + length);
            }
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

/// A held match shorter than this is searched past (lazy evaluation);
/// zlib's 16 searched past the five- to seven-byte matches that fill a
/// photograph, nearly doubling the searches for 1.5% of size.
const LAZY_LENGTH: usize = 4;
/// A match this long ends a search.
const NICE_LENGTH: usize = 128;
/// Chain positions walked per search where matches pay (text, flat
/// images). zlib's 128 bought 0.6% on a real photograph for 40% more
/// time; libdeflate's middle levels walk about this many.
const LAZY_CHAIN: usize = 32;
/// Where accepted matches average under `SHORT` bytes (photographs),
/// the walk is this long: a longer one found little better, at two to
/// four times the time.
const SHORT_CHAIN: usize = 2;
const SHORT: u32 = 8 * 16;
/// A three-byte match farther than this costs more than its literals.
const TOO_FAR: usize = 4096;
/// Of a maximal match (a run or a long repeat), only this many final
/// positions are indexed: enough that the next search finds a near
/// copy, where indexing all of a flat image's runs cost 10 times the
/// time and indexing none tripled its size.
const MAXIMAL_TAIL: usize = 8;
/// The accepted-length average (times 16) under which searches stop at
/// the first candidate: under a byte means noise, where walking on found
/// nothing worth its bits.
const NOISE: u32 = 16;

/// What each symbol cost in the last block written, in bits: the price
/// list for deciding whether a short match is cheaper than its literals.
/// On filtered photographs most three- to five-byte matches are not
/// (literals alone beat zlib's level 6 there by 5%).
struct Costs {
    literal: [u8; LITERALS],
    distance: [u8; DISTANCES],
}

impl Costs {
    /// Matches this long are always taken.
    const SURE: usize = 16;

    fn new() -> Costs {
        let mut literal = [8u8; LITERALS];
        for cost in &mut literal[257..] {
            *cost = 7;
        }
        Costs {
            literal,
            distance: [5; DISTANCES],
        }
    }

    fn learn(&mut self, literal_lengths: &[u8], distance_lengths: &[u8]) {
        // An unseen symbol is priced as a rare one would be.
        for (cost, length) in self.literal.iter_mut().zip(literal_lengths) {
            *cost = if *length == 0 { 14 } else { *length };
        }
        for (cost, length) in self.distance.iter_mut().zip(distance_lengths) {
            *cost = if *length == 0 { 10 } else { *length };
        }
    }

    fn worth(&self, length: usize, distance: usize, bytes: &[u8]) -> bool {
        if length >= Self::SURE {
            return true;
        }
        let (length_code, length_extra, _) = length_symbol(length as u16);
        let (distance_code, distance_extra, _) = distance_symbol(distance as u16);
        let match_bits = u32::from(self.literal[usize::from(length_code)])
            + u32::from(length_extra)
            + u32::from(self.distance[usize::from(distance_code)])
            + u32::from(distance_extra);
        let literal_bits: u32 = bytes
            .iter()
            .map(|byte| u32::from(self.literal[usize::from(*byte)]))
            .sum();
        match_bits < literal_bits
    }
}

/// The default level: zlib's lazy evaluation (`deflate_slow`). Every
/// position is indexed; each match found is held while the next position
/// is searched for a longer one.
fn deflate_lazy(input: &[u8], out: &mut Vec<u8>, is_final: bool) {
    let mut accepted = NOISE;
    let mut costs = Costs::new();
    let mut writer = BitWriter::new(out);
    let mut matcher = Matcher::new();
    let mut symbols: Vec<Symbol> = Vec::with_capacity(BLOCK_SYMBOLS);
    let mut block_start = 0usize;
    let mut position = 0usize;
    // The match found at the previous position, and whether that
    // position's byte is still owed (as a literal or the match's start).
    let mut previous_length = 0usize;
    let mut previous_distance = 0usize;
    let mut pending = false;
    let total = input.len();
    while position < total {
        // Chains hash four bytes, as libdeflate's do: a three-byte hash
        // chained every short repeat of a photograph, walked for matches
        // the price check then refused (4% larger and slower).
        let candidate = if position + 4 <= total {
            let hash = Matcher::hash4(input, position);
            matcher.link(hash, position)
        } else {
            u32::MAX
        };
        let mut length = 0;
        let mut distance = 0;
        if previous_length < LAZY_LENGTH {
            let chain = if accepted < NOISE {
                1
            } else if accepted < SHORT {
                SHORT_CHAIN
            } else {
                LAZY_CHAIN
            };
            (length, distance) = matcher.longest(
                input,
                position,
                candidate,
                previous_length,
                chain,
                NICE_LENGTH,
            );
            if length == MIN_MATCH && distance > TOO_FAR {
                length = 0;
            }
            if length >= MIN_MATCH
                && !costs.worth(length, distance, &input[position..position + length])
            {
                length = 0;
            }
            // A running average of accepted match lengths (times 16): on
            // noise, where nothing past the first candidate pays, the walk
            // stops there.
            accepted = accepted - accepted / 16 + length as u32;
        }
        if previous_length >= MIN_MATCH && length <= previous_length {
            // The held match wins: it starts at the previous position.
            symbols.push(Symbol::Match {
                length: previous_length as u16,
                distance: previous_distance as u16,
            });
            let end = position - 1 + previous_length;
            let index_from = if previous_length == MAX_MATCH {
                (end - MAXIMAL_TAIL).max(position + 1)
            } else {
                position + 1
            };
            for inside in index_from..end.min(total) {
                if inside + 4 <= total {
                    let hash = Matcher::hash4(input, inside);
                    matcher.link(hash, inside);
                }
            }
            position = end;
            previous_length = 0;
            pending = false;
        } else {
            if pending {
                symbols.push(Symbol::Literal(input[position - 1]));
            }
            pending = true;
            previous_length = length;
            previous_distance = distance;
            position += 1;
        }
        if symbols.len() >= BLOCK_SYMBOLS {
            // The block ends before any byte still owed.
            let block_end = if pending { position - 1 } else { position };
            let (literal_lengths, distance_lengths) =
                write_block(&mut writer, &input[block_start..block_end], &symbols, false);
            costs.learn(&literal_lengths, &distance_lengths);
            symbols.clear();
            block_start = block_end;
        }
    }
    if pending {
        symbols.push(Symbol::Literal(input[total - 1]));
    }
    write_block(&mut writer, &input[block_start..], &symbols, is_final);
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

/// Finds earlier occurrences through hash chains. The chain links are a
/// ring of sixteen-bit distances, one slot per window position, so the
/// walk (a dependent load per step) stays in cache; an array of
/// positions the size of the input missed on every step.
struct Matcher {
    head: Vec<u32>,
    /// Distance from a position to the previous one with the same hash,
    /// indexed by position modulo the window; zero ends the chain.
    prev: Vec<u16>,
}

impl Matcher {
    fn new() -> Matcher {
        Matcher {
            head: vec![u32::MAX; HASH_SIZE],
            prev: vec![0; WINDOW],
        }
    }

    fn hash(input: &[u8], position: usize) -> usize {
        let word = (u32::from(input[position]) << 16)
            | (u32::from(input[position + 1]) << 8)
            | u32::from(input[position + 2]);
        (word.wrapping_mul(0x9E37_79B1) >> (32 - HASH_BITS)) as usize
    }

    fn hash4(input: &[u8], position: usize) -> usize {
        let bytes: [u8; 4] = input[position..position + 4].try_into().unwrap();
        let word = u32::from_le_bytes(bytes);
        (word.wrapping_mul(0x9E37_79B1) >> (32 - HASH_BITS)) as usize
    }

    /// Links `position` into its chain; returns the previous head.
    fn link(&mut self, hash: usize, position: usize) -> u32 {
        let previous = self.head[hash];
        let step = if previous == u32::MAX || position - previous as usize >= WINDOW {
            0
        } else {
            (position - previous as usize) as u16
        };
        self.prev[position & (WINDOW - 1)] = step;
        self.head[hash] = position as u32;
        previous
    }

    fn insert(&mut self, input: &[u8], position: usize) {
        if position + MIN_MATCH > input.len() {
            return;
        }
        let hash = Self::hash(input, position);
        self.link(hash, position);
    }

    fn insert_range(&mut self, input: &[u8], from: usize, to: usize) {
        for position in from..to.min(input.len()) {
            self.insert(input, position);
        }
    }

    /// zlib's `longest_match`: walks the chain from `candidate` for a
    /// match longer than `previous`, at most `chain` steps, stopping at
    /// `nice`. A candidate is compared in full only when the two bytes
    /// ending a longer match agree (miniz's and zlib's `scan_end`).
    fn longest(
        &self,
        input: &[u8],
        position: usize,
        candidate: u32,
        previous: usize,
        mut chain: usize,
        nice: usize,
    ) -> (usize, usize) {
        let limit = (input.len() - position).min(MAX_MATCH);
        let nice = nice.min(limit);
        let mut best_length = previous.max(MIN_MATCH - 1);
        let mut best_distance = 0usize;
        if candidate == u32::MAX || best_length >= limit {
            return (0, 0);
        }
        let mut start = candidate as usize;
        loop {
            let distance = position - start;
            if distance >= WINDOW || distance == 0 {
                break;
            }
            // The two bytes at the end of a match one longer than the best.
            let end = best_length - 1;
            if input[start + end] == input[position + end]
                && input[start + end + 1] == input[position + end + 1]
                && input[start] == input[position]
            {
                let length = common_prefix(input, start, position, limit);
                if length > best_length {
                    best_length = length;
                    best_distance = distance;
                    if length >= nice {
                        break;
                    }
                }
            }
            chain -= 1;
            if chain == 0 {
                break;
            }
            let step = usize::from(self.prev[start & (WINDOW - 1)]);
            if step == 0 || step > start {
                break;
            }
            start -= step;
        }
        if best_distance == 0 {
            (0, 0)
        } else {
            (best_length, best_distance)
        }
    }

    /// The fast level's search: the chain's first candidate only.
    /// Returns `(length, distance)`; a length under `MIN_MATCH` means none.
    fn find(&mut self, input: &[u8], position: usize) -> (usize, usize) {
        if position + MIN_MATCH > input.len() {
            return (0, 0);
        }
        let hash = Self::hash(input, position);
        let candidate = self.link(hash, position);
        if candidate == u32::MAX {
            return (0, 0);
        }
        let start = candidate as usize;
        let distance = position - start;
        if distance >= WINDOW {
            return (0, 0);
        }
        let limit = (input.len() - position).min(MAX_MATCH);
        (common_prefix(input, start, position, limit), distance)
    }
}

#[inline(always)]
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
pub(crate) fn code_lengths(frequencies: &[u32], limit: u8) -> Vec<u8> {
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

/// Unlimited Huffman code lengths by merging the two lightest nodes, with
/// the leaves sorted once and merged nodes kept in a second queue (they
/// are made in nondecreasing weight order): O(n log n). A single used
/// symbol gets length one, as deflate requires.
fn huffman_lengths(weights: &[u32]) -> Vec<u8> {
    let count = weights.len();
    let mut lengths = vec![0u8; count];
    let mut leaves: Vec<(u64, usize)> = weights
        .iter()
        .enumerate()
        .filter(|(_, weight)| **weight > 0)
        .map(|(symbol, weight)| (u64::from(*weight), symbol))
        .collect();
    if leaves.is_empty() {
        return lengths;
    }
    if leaves.len() == 1 {
        lengths[leaves[0].1] = 1;
        return lengths;
    }
    leaves.sort_unstable();
    let leaf_count = leaves.len();
    // Node ids: leaves 0..leaf_count in sorted order, then merged nodes.
    let mut parents: Vec<usize> = vec![usize::MAX; 2 * leaf_count - 1];
    let mut merged: Vec<u64> = Vec::with_capacity(leaf_count - 1);
    let (mut next_leaf, mut next_merged) = (0usize, 0usize);
    for id in leaf_count..2 * leaf_count - 1 {
        let mut take = || {
            let leaf_first = next_merged >= merged.len()
                || (next_leaf < leaf_count && leaves[next_leaf].0 <= merged[next_merged]);
            if leaf_first {
                next_leaf += 1;
                (leaves[next_leaf - 1].0, next_leaf - 1)
            } else {
                next_merged += 1;
                (merged[next_merged - 1], leaf_count + next_merged - 1)
            }
        };
        let (weight_a, node_a) = take();
        let (weight_b, node_b) = take();
        parents[node_a] = id;
        parents[node_b] = id;
        merged.push(weight_a + weight_b);
    }
    // Parents always have higher ids: depths fill from the root down.
    let root = 2 * leaf_count - 2;
    let mut depths = vec![0u8; 2 * leaf_count - 1];
    for id in (0..root).rev() {
        depths[id] = depths[parents[id]].saturating_add(1);
    }
    for (index, (_, symbol)) in leaves.iter().enumerate() {
        lengths[*symbol] = depths[index];
    }
    lengths
}

/// Canonical codes from lengths, bit-reversed for LSB-first output.
pub(crate) fn canonical_codes(lengths: &[u8]) -> Vec<u16> {
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

/// Length to length-code index, for every length 0 to 258.
const fn length_codes() -> [u8; MAX_MATCH + 1] {
    let mut table = [0u8; MAX_MATCH + 1];
    let mut length = MIN_MATCH;
    while length <= MAX_MATCH {
        let mut index = 0;
        while index + 1 < LENGTH_BASE.len() && LENGTH_BASE[index + 1] as usize <= length {
            index += 1;
        }
        table[length] = index as u8;
        length += 1;
    }
    table
}

/// Distance to distance-code index: distances 1 to 256 directly, and
/// beyond that by 128s, since every code boundary past 256 falls on one.
const fn distance_codes() -> [u8; 512] {
    let mut table = [0u8; 512];
    let mut index = 0;
    while index < DISTANCE_BASE.len() {
        let first = DISTANCE_BASE[index] as usize;
        let last = if index + 1 < DISTANCE_BASE.len() {
            DISTANCE_BASE[index + 1] as usize - 1
        } else {
            WINDOW
        };
        if last <= 256 {
            let mut distance = first;
            while distance <= last {
                table[distance - 1] = index as u8;
                distance += 1;
            }
        } else {
            let mut slot = 256 + ((first - 1) >> 7);
            let end = 256 + ((last - 1) >> 7);
            while slot <= end {
                table[slot] = index as u8;
                slot += 1;
            }
        }
        index += 1;
    }
    table
}

static LENGTH_CODES: [u8; MAX_MATCH + 1] = length_codes();
static DISTANCE_CODES: [u8; 512] = distance_codes();

fn length_symbol(length: u16) -> (u16, u8, u16) {
    let index = usize::from(LENGTH_CODES[usize::from(length)]);
    (
        257 + index as u16,
        LENGTH_EXTRA[index],
        length - LENGTH_BASE[index],
    )
}

fn distance_symbol(distance: u16) -> (u16, u8, u16) {
    let slot = if distance <= 256 {
        usize::from(distance) - 1
    } else {
        256 + ((usize::from(distance) - 1) >> 7)
    };
    let index = usize::from(DISTANCE_CODES[slot]);
    (
        index as u16,
        DISTANCE_EXTRA[index],
        distance - DISTANCE_BASE[index],
    )
}

/// Writes one block; returns its literal/length and distance code
/// lengths (the lazy level prices its next block's matches by them).
fn write_block(
    writer: &mut BitWriter<'_>,
    raw: &[u8],
    symbols: &[Symbol],
    is_final: bool,
) -> (Vec<u8>, Vec<u8>) {
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
        return (literal_lengths, distance_lengths);
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
    (literal_lengths, distance_lengths)
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

    /// Appends `count` bits (at most 32); whole 32-bit words go out at
    /// once, where a push per byte was a tenth of a photo's encode.
    #[inline]
    fn bits(&mut self, value: u32, count: u32) {
        self.buffer |= u64::from(value) << self.count;
        self.count += count;
        if self.count >= 32 {
            self.out
                .extend_from_slice(&(self.buffer as u32).to_le_bytes());
            self.buffer >>= 32;
            self.count -= 32;
        }
    }

    fn code(&mut self, code: u16, length: u8) {
        self.bits(u32::from(code), u32::from(length));
    }

    fn align(&mut self) {
        while self.count > 0 {
            self.out.push(self.buffer as u8);
            self.buffer >>= 8;
            self.count = self.count.saturating_sub(8);
        }
        self.buffer = 0;
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
