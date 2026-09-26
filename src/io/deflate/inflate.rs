//! Inflate: stored, fixed-Huffman, and dynamic-Huffman blocks, decoded with
//! full lookup tables so each symbol costs one table read, from a 64-bit
//! bit buffer refilled eight bytes at a time.
//!
//! `Inflater` is resumable: it takes input in any pieces and produces
//! output up to a limit per call, so a reader can feed a file chunk by
//! chunk and take rows as they come without holding either whole. It
//! keeps the last 32 KiB of output for back-references. `inflate` is the
//! one-shot form over a complete buffer.

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

/// History kept behind the drained output for back-references.
const WINDOW: usize = 32 * 1024;
/// Drained bytes are compacted away once this many pile up.
const COMPACT_AT: usize = 4 * WINDOW;

/// A bit reader over one input piece; its buffer persists across pieces.
struct BitReader<'a> {
    bytes: &'a [u8],
    position: usize,
    buffer: u64,
    count: u32,
}

impl<'a> BitReader<'a> {
    /// Tops the buffer up: eight bytes at once while the input has them,
    /// then one at a time to the end.
    #[inline(always)]
    fn refill(&mut self) {
        if self.count <= 56 {
            if let Some(word) = self.bytes.get(self.position..self.position + 8) {
                let word = u64::from_le_bytes([
                    word[0], word[1], word[2], word[3], word[4], word[5], word[6], word[7],
                ]);
                // Only the bytes taken go in: bits above `count` stay
                // zero, as a stored block may consume the rest of the
                // word from the slice directly.
                let taken = (63 - self.count) / 8;
                let mask = (1u64 << (taken * 8)) - 1;
                self.buffer |= (word & mask) << self.count;
                self.position += taken as usize;
                self.count += taken * 8;
                return;
            }
        }
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

    #[inline]
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

    /// Whole bytes taken into the buffer but not yet decoded; they may
    /// have come from an earlier piece.
    fn buffered_bytes(&self) -> usize {
        (self.count / 8) as usize
    }

    /// True when the piece cannot refill a whole word: the end is near.
    #[inline]
    fn near_end(&self) -> bool {
        self.position + 8 > self.bytes.len()
    }
}

/// The primary table covers this many input bits; longer codes go
/// through a second-level table. Twelve bits keeps the table in L1 and
/// holds two literals in one entry when their codes are short, which
/// halves the lookups on literal-heavy data.
const PRIMARY_BITS: u32 = 12;
const PRIMARY_SIZE: usize = 1 << PRIMARY_BITS;
/// Deflate codes are at most this long.
const MAX_CODE_BITS: u32 = 15;
/// A literal or match is at most this long: two codes of fifteen bits
/// with five and thirteen extra bits.
const MAX_SYMBOL_BITS: u32 = 48;
/// The longest match.
const MAX_MATCH: usize = 258;
/// The fast loop sizes its output this much at a time.
const SLAB: usize = 1 << 20;

/// Entry layout of the full table. Bits 0 to 3: the code length. Bits
/// 4 to 7: the same (kept so the packed layout can tell entries apart).
/// Bits 8 up: the symbol, or the offset of a second-level table (whose
/// width sits in bits 0 to 3). `LITERAL` marks a literal of the
/// literal/length alphabet; `POINTER` marks a second-level pointer.
const LITERAL: u32 = 1 << 31;
const POINTER: u32 = 1 << 29;
const SYMBOL_MASK: u32 = 0x1FF;
/// The packed primary table the fast path reads: bits 0 to 3 are the
/// bits to consume, bits 4 and 5 how many literals the entry holds (zero
/// for a length code, end of block, or pointer), and bits 8 up hold one
/// to three literal bytes. Non-literal entries keep the full table's
/// symbol and pointer layout with bits 4 to 7 clear.
const PACKED_COUNT_SHIFT: u32 = 4;
const MAX_PACKED: u32 = 3;

/// A canonical Huffman code as a two-level lookup table indexed by the
/// next input bits (least significant first, as deflate packs them).
/// An entry of zero marks an unused code; see the layouts above. The
/// literal/length alphabet also carries `packed`, its primary table
/// with runs of short literal codes folded into one entry.
struct Huffman {
    table: Vec<u32>,
    packed: Vec<u32>,
}

/// Why a code could not be read.
enum Fault {
    Truncated,
    NoSymbol,
}

/// Which alphabet a table decodes: only the literal/length alphabet has
/// literals to flag and pair.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Alphabet {
    Literals,
    Other,
}

fn entry_for(alphabet: Alphabet, symbol: usize, length: u32) -> u32 {
    let flag = if alphabet == Alphabet::Literals && symbol < 256 {
        LITERAL
    } else {
        0
    };
    flag | ((symbol as u32) << 8) | (length << 4) | length
}

impl Huffman {
    fn empty() -> Huffman {
        Huffman {
            table: vec![0; PRIMARY_SIZE],
            packed: Vec::new(),
        }
    }

    fn build(lengths: &[u8], offset: usize, alphabet: Alphabet) -> Result<Huffman, InflateError> {
        let max_length = u32::from(lengths.iter().copied().max().unwrap_or(0));
        if max_length == 0 {
            return Ok(Huffman::empty());
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
        // Each symbol's code, bit-reversed for indexing by the input.
        let mut reversed_codes = vec![0u32; lengths.len()];
        for (symbol, length) in lengths.iter().enumerate() {
            let length = u32::from(*length);
            if length == 0 {
                continue;
            }
            let code = next_code[length as usize];
            next_code[length as usize] += 1;
            reversed_codes[symbol] = reverse_bits(code, length);
        }
        let mut table = vec![0u32; PRIMARY_SIZE];
        // Long codes: each primary prefix gets a second-level table wide
        // enough for the longest code behind it.
        let mut sub_bits = vec![0u32; PRIMARY_SIZE];
        for (symbol, length) in lengths.iter().enumerate() {
            let length = u32::from(*length);
            if length > PRIMARY_BITS {
                let prefix = (reversed_codes[symbol] as usize) & (PRIMARY_SIZE - 1);
                sub_bits[prefix] = sub_bits[prefix].max(length - PRIMARY_BITS);
            }
        }
        for prefix in 0..PRIMARY_SIZE {
            if sub_bits[prefix] > 0 {
                let start = table.len() as u32;
                table.resize(table.len() + (1usize << sub_bits[prefix]), 0);
                table[prefix] = POINTER | (start << 8) | sub_bits[prefix];
            }
        }
        for (symbol, length) in lengths.iter().enumerate() {
            let length = u32::from(*length);
            if length == 0 {
                continue;
            }
            let reversed = reversed_codes[symbol] as usize;
            let entry = entry_for(alphabet, symbol, length);
            if length <= PRIMARY_BITS {
                let step = 1usize << length;
                let mut index = reversed;
                while index < PRIMARY_SIZE {
                    table[index] = entry;
                    index += step;
                }
            } else {
                let prefix = reversed & (PRIMARY_SIZE - 1);
                let pointer = table[prefix];
                let start = ((pointer >> 8) & 0xF_FFFF) as usize;
                let width = pointer & 15;
                let size = 1usize << width;
                let step = 1usize << (length - PRIMARY_BITS);
                let mut index = reversed >> PRIMARY_BITS;
                while index < size {
                    table[start + index] = entry;
                    index += step;
                }
            }
        }
        // Fold literals: where a literal's code leaves room in the
        // primary bits for whole further literal codes, the packed entry
        // holds up to three and the fast path consumes them in one step.
        let mut packed = vec![0u32; PRIMARY_SIZE];
        for (index, slot) in packed.iter_mut().enumerate() {
            let first = table[index];
            if first & LITERAL == 0 {
                *slot = if first & POINTER != 0 {
                    first
                } else {
                    first & !0xF0
                };
                continue;
            }
            let mut total = first & 15;
            let mut literals = 1u32;
            let mut bytes = first & 0xFF00;
            while literals < MAX_PACKED {
                let room = PRIMARY_BITS - total;
                if room == 0 {
                    break;
                }
                let next = table[index >> total];
                let next_length = next & 15;
                if next & LITERAL == 0 || next_length > room {
                    break;
                }
                bytes |= ((next >> 8) & 0xFF) << (8 * (literals + 1));
                total += next_length;
                literals += 1;
            }
            *slot = bytes | (literals << PACKED_COUNT_SHIFT) | total;
        }
        Ok(Huffman { table, packed })
    }

    /// One symbol, consuming its code alone (a paired entry gives its
    /// first literal).
    #[inline(always)]
    fn decode(&self, reader: &mut BitReader<'_>) -> Result<u16, Fault> {
        if reader.count < MAX_CODE_BITS {
            reader.refill();
        }
        let mut entry = self.table[(reader.buffer as usize) & (PRIMARY_SIZE - 1)];
        if entry & POINTER != 0 {
            let start = ((entry >> 8) & 0xF_FFFF) as usize;
            let width = entry & 15;
            let index = ((reader.buffer >> PRIMARY_BITS) as usize) & ((1usize << width) - 1);
            entry = self.table[start + index];
        }
        let length = entry & 15;
        if length == 0 {
            return Err(Fault::NoSymbol);
        }
        if reader.count < length {
            return Err(Fault::Truncated);
        }
        reader.buffer >>= length;
        reader.count -= length;
        Ok(((entry >> 8) & SYMBOL_MASK) as u16)
    }

    /// `decode` with the fault turned into the stream's error.
    #[inline(always)]
    fn decode_at(&self, reader: &mut BitReader<'_>, offset: usize) -> Result<u16, InflateError> {
        self.decode(reader).map_err(|fault| match fault {
            Fault::Truncated => InflateError::Truncated,
            Fault::NoSymbol => InflateError::Corrupt {
                offset,
                what: "code with no symbol",
            },
        })
    }
}

/// Tops the bit buffer up from a word at `position`, which must be in
/// bounds, without a branch: the whole word is folded in above `count`,
/// the position moves past the bytes that fit, and `count` lands in 56
/// to 63. Bits of the next byte may sit above `count`; every refill
/// folds that byte in at the same place, so they stay right, and the
/// fast path clears them before handing the reader back.
#[inline(always)]
fn refill_word(bytes: &[u8], position: &mut usize, buffer: &mut u64, count: &mut u32) {
    let word = &bytes[*position..*position + 8];
    let word = u64::from_le_bytes([
        word[0], word[1], word[2], word[3], word[4], word[5], word[6], word[7],
    ]);
    *buffer |= word << *count;
    *position += ((63 - *count) >> 3) as usize;
    *count |= 56;
}

/// Writes `length` bytes at `at` repeating the `distance` bytes before
/// it; `out` must have the room. A short distance doubles the copy each
/// step: after `copied` bytes (a multiple of the distance) the pattern
/// is that much longer.
#[inline(always)]
fn copy_match_at(out: &mut [u8], at: usize, distance: usize, length: usize) {
    let from = at - distance;
    if distance >= length {
        out.copy_within(from..from + length, at);
    } else if distance == 1 {
        let byte = out[from];
        out[at..at + length].fill(byte);
    } else {
        let mut copied = 0;
        while copied < length {
            let piece = (distance + copied).min(length - copied);
            out.copy_within(from..from + piece, at + copied);
            copied += piece;
        }
    }
}

fn reverse_bits(code: u32, length: u32) -> u32 {
    let mut reversed = 0u32;
    for bit in 0..length {
        reversed |= ((code >> bit) & 1) << (length - 1 - bit);
    }
    reversed
}

pub(super) const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
pub(super) const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
pub(super) const DISTANCE_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
pub(super) const DISTANCE_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
pub(super) const CODE_LENGTH_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

/// Where the decoder stands between pieces of input.
enum State {
    /// Before a block header.
    Header,
    /// Inside a stored block with this many bytes to copy.
    Stored(usize),
    /// Inside a Huffman block with the current tables.
    Huffman,
    Done,
}

/// Why `push` returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// The input piece is used up; feed the rest.
    NeedInput,
    /// The output limit is reached; drain and call again.
    OutputFull,
    /// The final block has ended.
    Done,
}

pub struct Inflater {
    buffer: u64,
    count: u32,
    state: State,
    final_block: bool,
    literals: Huffman,
    distances: Huffman,
    /// Output produced, the drained prefix kept as history. The vector
    /// is a slab reused across calls: `filled` bytes of it are valid,
    /// the rest is room already zeroed once.
    out: Vec<u8>,
    filled: usize,
    drained: usize,
    /// Output compacted away, so `total_out` is this plus `filled`.
    compacted: usize,
    total_in: usize,
    /// Input a block header straddled: kept here and read before the
    /// next piece, so a caller never re-sends bytes.
    pending: Vec<u8>,
}

impl Default for Inflater {
    fn default() -> Self {
        Inflater::new()
    }
}

impl Inflater {
    pub fn new() -> Inflater {
        Inflater {
            buffer: 0,
            count: 0,
            state: State::Header,
            final_block: false,
            literals: Huffman::empty(),
            distances: Huffman::empty(),
            out: Vec::new(),
            filled: 0,
            drained: 0,
            compacted: 0,
            total_in: 0,
            pending: Vec::new(),
        }
    }

    /// Output not yet taken.
    pub fn output(&self) -> &[u8] {
        &self.out[self.drained..self.filled]
    }

    /// Marks `count` bytes of `output` as taken.
    pub fn drain(&mut self, count: usize) {
        self.drained += count;
        if self.drained >= COMPACT_AT {
            let keep_from = self.drained - WINDOW;
            self.out.copy_within(keep_from..self.filled, 0);
            self.filled -= keep_from;
            self.compacted += keep_from;
            self.drained = WINDOW;
        }
    }

    /// Makes sure `room` more bytes fit behind `filled`.
    fn reserve_room(&mut self, room: usize) {
        let needed = self.filled + room;
        if self.out.len() < needed {
            let grown = needed.max(self.out.len() * 2).max(SLAB);
            self.out.resize(grown, 0);
        }
    }

    fn push_byte(&mut self, byte: u8) {
        self.reserve_room(1);
        self.out[self.filled] = byte;
        self.filled += 1;
    }

    pub fn is_done(&self) -> bool {
        matches!(self.state, State::Done)
    }

    /// Bytes of input consumed over the stream's life, including any
    /// read ahead of the stream's end (see `leftover()`).
    pub fn total_in(&self) -> usize {
        self.total_in
    }

    pub fn total_out(&self) -> usize {
        self.compacted + self.filled
    }

    /// Decodes from `input` until it is used up, `output()` holds at
    /// least `want` bytes, or the stream ends. Returns how many bytes of
    /// `input` were consumed and why it stopped. On `NeedInput` the whole
    /// piece is consumed (a straddling header is kept inside); on
    /// `OutputFull` the caller drains and pushes the rest of the piece.
    pub fn push(&mut self, input: &[u8], want: usize) -> Result<(usize, Progress), InflateError> {
        if self.pending.is_empty() {
            let (outcome, position) = self.run_piece(input, want);
            let progress = outcome?;
            let consumed = match progress {
                Progress::NeedInput => {
                    self.pending.extend_from_slice(&input[position..]);
                    input.len()
                }
                Progress::OutputFull | Progress::Done => position,
            };
            self.total_in += consumed;
            return Ok((consumed, progress));
        }
        let mut pending = std::mem::take(&mut self.pending);
        pending.extend_from_slice(input);
        let (outcome, position) = self.run_piece(&pending, want);
        pending.drain(..position);
        self.pending = pending;
        self.total_in += input.len();
        Ok((input.len(), outcome?))
    }

    #[cold]
    fn fault(&self, fault: Fault, reader: &BitReader<'_>) -> InflateError {
        match fault {
            Fault::Truncated => InflateError::Truncated,
            Fault::NoSymbol => InflateError::Corrupt {
                offset: self.offset(reader),
                what: "code with no symbol",
            },
        }
    }

    /// Where the decoder stands in the stream, for error messages.
    fn offset(&self, reader: &BitReader<'_>) -> usize {
        (self.total_in + reader.position).saturating_sub(reader.buffered_bytes())
    }

    /// Runs the decoder over one slice; returns where it stopped.
    fn run_piece(&mut self, input: &[u8], want: usize) -> (Result<Progress, InflateError>, usize) {
        let mut reader = BitReader {
            bytes: input,
            position: 0,
            buffer: self.buffer,
            count: self.count,
        };
        let outcome = self.run(&mut reader, want);
        self.buffer = reader.buffer;
        self.count = reader.count;
        (outcome, reader.position)
    }

    /// Bytes taken past the end of a finished stream (the reader looks
    /// ahead), in stream order. Empty until `is_done()`.
    pub fn leftover(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(8 + self.pending.len());
        let buffered = (self.count / 8) as usize;
        for index in 0..buffered {
            bytes.push((self.buffer >> (index * 8)) as u8);
        }
        bytes.extend_from_slice(&self.pending);
        bytes
    }

    fn run(&mut self, reader: &mut BitReader<'_>, want: usize) -> Result<Progress, InflateError> {
        loop {
            if self.filled - self.drained >= want {
                return Ok(Progress::OutputFull);
            }
            match self.state {
                State::Done => return Ok(Progress::Done),
                State::Header => {
                    // A header is decoded whole or not at all: on a short
                    // read the reader is put back to where it started.
                    let checkpoint = (reader.position, reader.buffer, reader.count);
                    match self.header(reader) {
                        Ok(()) => {}
                        Err(InflateError::Truncated) => {
                            // Back to the start of the header; the bytes
                            // the attempt took are re-buffered as far as
                            // the buffer holds, and `push` keeps the rest.
                            (reader.position, reader.buffer, reader.count) = checkpoint;
                            reader.refill();
                            return Ok(Progress::NeedInput);
                        }
                        Err(error) => return Err(error),
                    }
                }
                State::Stored(remaining) => {
                    let taken = self.stored(reader, remaining);
                    if taken < remaining {
                        self.state = State::Stored(remaining - taken);
                        return Ok(Progress::NeedInput);
                    }
                    self.end_block(reader);
                }
                State::Huffman => match self.symbols(reader, want)? {
                    Some(progress) => return Ok(progress),
                    None => self.end_block(reader),
                },
            }
        }
    }

    fn end_block(&mut self, reader: &mut BitReader<'_>) {
        if self.final_block {
            // The stream ends at the last code; the rest of that byte is
            // padding, and whatever follows (a zlib trailer) is whole
            // bytes the caller reads back through `leftover()`.
            reader.align_to_byte();
            self.state = State::Done;
        } else {
            self.state = State::Header;
        }
    }

    fn header(&mut self, reader: &mut BitReader<'_>) -> Result<(), InflateError> {
        let offset = self.offset(reader);
        self.final_block = reader.bits(1)? == 1;
        match reader.bits(2)? {
            0 => {
                reader.align_to_byte();
                let length = reader.bits(16)? as usize;
                let check = reader.bits(16)? as usize;
                if length != (!check & 0xFFFF) {
                    return Err(InflateError::Corrupt {
                        offset: self.offset(reader),
                        what: "stored block length check",
                    });
                }
                self.state = State::Stored(length);
            }
            1 => {
                let (literals, distances) = fixed_tables()?;
                self.literals = literals;
                self.distances = distances;
                self.state = State::Huffman;
            }
            2 => {
                let (literals, distances) = dynamic_tables(reader, offset)?;
                self.literals = literals;
                self.distances = distances;
                self.state = State::Huffman;
            }
            _ => {
                return Err(InflateError::Corrupt {
                    offset: self.offset(reader),
                    what: "block type 3",
                });
            }
        }
        Ok(())
    }

    /// Copies up to `remaining` stored bytes; returns how many it could.
    fn stored(&mut self, reader: &mut BitReader<'_>, remaining: usize) -> usize {
        let mut taken = 0;
        while taken < remaining && reader.count >= 8 {
            self.push_byte((reader.buffer & 0xff) as u8);
            reader.buffer >>= 8;
            reader.count -= 8;
            taken += 1;
        }
        let available = reader.bytes.len() - reader.position;
        let direct = (remaining - taken).min(available);
        self.reserve_room(direct);
        self.out[self.filled..self.filled + direct]
            .copy_from_slice(&reader.bytes[reader.position..reader.position + direct]);
        self.filled += direct;
        reader.position += direct;
        taken += direct;
        taken
    }

    /// Decodes symbols until the block ends (`None`), the output is full,
    /// or the input runs out at a symbol boundary. While the piece has
    /// bytes to spare, one refill guarantees a whole symbol (at most 48
    /// bits) and no checkpoint is needed; only near the end of the piece
    /// is each symbol attempted under a checkpoint.
    fn symbols(
        &mut self,
        reader: &mut BitReader<'_>,
        want: usize,
    ) -> Result<Option<Progress>, InflateError> {
        loop {
            if self.filled - self.drained >= want {
                return Ok(Some(Progress::OutputFull));
            }
            if !reader.near_end() {
                // A refused symbol (`None`) falls through to the careful
                // path below, which decodes it once and reports whatever
                // is wrong with it.
                if let Some(block_over) = self.fast_symbols(reader, want)? {
                    return Ok(if block_over {
                        None
                    } else {
                        Some(Progress::OutputFull)
                    });
                }
            }
            if reader.count < MAX_SYMBOL_BITS {
                if reader.near_end() {
                    // On a short read the reader goes back to where the
                    // symbol started, with the bytes the attempt took
                    // re-buffered (one refill covers a symbol).
                    let checkpoint = (reader.position, reader.buffer, reader.count);
                    match self.symbol(reader) {
                        Ok(true) => continue,
                        Ok(false) => return Ok(None),
                        Err(InflateError::Truncated) => {
                            (reader.position, reader.buffer, reader.count) = checkpoint;
                            reader.refill();
                            return Ok(Some(Progress::NeedInput));
                        }
                        Err(error) => return Err(error),
                    }
                }
                reader.refill();
            }
            if !self.symbol(reader)? {
                return Ok(None);
            }
        }
    }

    /// The hot loop: while the piece has a word to spare and the output
    /// has room, symbols decode from register copies of the reader with
    /// no checkpoints (a refill covers a whole symbol). The next entry is
    /// looked up before the refill and the literal store, so the refill
    /// stays off the chain from one symbol to the next. Output goes
    /// through an index into a slab sized ahead of time, so a paired
    /// entry stores two bytes without a branch. Returns `Some(true)` at
    /// the end of the block, `Some(false)` when the output is full,
    /// `None` when the piece runs low or a symbol is refused.
    #[inline(never)]
    fn fast_symbols(
        &mut self,
        reader: &mut BitReader<'_>,
        want: usize,
    ) -> Result<Option<bool>, InflateError> {
        let bytes = reader.bytes;
        let mut position = reader.position;
        let mut buffer = reader.buffer;
        let mut count = reader.count;
        if bytes.len() < position + 8 {
            return Ok(None);
        }
        // The slab: what the caller wants, taken a megabyte at a time,
        // plus room for the longest match and a literal store.
        let held = self.filled - self.drained;
        let slab = want.saturating_sub(held).min(SLAB);
        let mut at = self.filled;
        let stop_at = at + slab;
        self.reserve_room(slab + MAX_MATCH + 16);
        let mut out = std::mem::take(&mut self.out);
        // Locals, not fields: the compiler keeps a local slice's pointer
        // and a local vector's length in registers across the loop,
        // where fields behind `self` are reloaded at every symbol.
        // The packed table as a fixed-size array: an index masked to
        // its size then needs no bounds check.
        let literals: &[u32; PRIMARY_SIZE] = match self.literals.packed.as_slice().try_into() {
            Ok(table) => table,
            Err(_) => {
                self.out = out;
                return Ok(None);
            }
        };
        let subtables: &[u32] = self.literals.table.as_slice();
        let distances: &[u32] = self.distances.table.as_slice();
        // The last position a whole word can be read from.
        let last_start = bytes.len() - 8;
        let mut outcome = None;
        refill_word(bytes, &mut position, &mut buffer, &mut count);
        let mut entry = literals[(buffer as usize) & (PRIMARY_SIZE - 1)];
        loop {
            if at >= stop_at {
                // The slab is used up: full if that satisfies the caller,
                // otherwise the next round sizes another.
                let produced = at - (stop_at - slab);
                outcome = if held + produced >= want {
                    Some(false)
                } else {
                    None
                };
                break;
            }
            if position > last_start {
                break;
            }
            // Literals first: the third literal byte shares bits with
            // the flags, which only mean something when the count is
            // zero.
            let packed_literals = (entry >> PACKED_COUNT_SHIFT) & 3;
            if packed_literals != 0 {
                // Up to three entries decode from the buffer as it stands:
                // each lookup depends on the one before only through a
                // code length feeding a shift, never through a buffer
                // update, and the buffer holds at least 56 bits, enough
                // for three entries (36 bits at most) and the lookup of a
                // fourth, which the next round starts from. One consume
                // and one refill cover the whole group. (The shape of
                // fdeflate's loop, with three literals per entry rather
                // than two.)
                let bits_1 = entry & 15;
                let entry_2 = literals[((buffer >> bits_1) as usize) & (PRIMARY_SIZE - 1)];
                let bits_2 = entry_2 & 15;
                let entry_3 =
                    literals[((buffer >> (bits_1 + bits_2)) as usize) & (PRIMARY_SIZE - 1)];
                let bits_3 = entry_3 & 15;
                let entry_4 = literals
                    [((buffer >> (bits_1 + bits_2 + bits_3)) as usize) & (PRIMARY_SIZE - 1)];
                // Four-byte stores of one to three literals each; the
                // slab has room for the bytes past the count.
                out[at..at + 4].copy_from_slice(&(entry >> 8).to_le_bytes());
                at += packed_literals as usize;
                let literals_2 = (entry_2 >> PACKED_COUNT_SHIFT) & 3;
                if literals_2 == 0 {
                    buffer >>= bits_1;
                    count -= bits_1;
                    refill_word(bytes, &mut position, &mut buffer, &mut count);
                    entry = entry_2;
                    continue;
                }
                out[at..at + 4].copy_from_slice(&(entry_2 >> 8).to_le_bytes());
                at += literals_2 as usize;
                let literals_3 = (entry_3 >> PACKED_COUNT_SHIFT) & 3;
                if literals_3 == 0 {
                    let taken = bits_1 + bits_2;
                    buffer >>= taken;
                    count -= taken;
                    refill_word(bytes, &mut position, &mut buffer, &mut count);
                    entry = entry_3;
                    continue;
                }
                out[at..at + 4].copy_from_slice(&(entry_3 >> 8).to_le_bytes());
                at += literals_3 as usize;
                let taken = bits_1 + bits_2 + bits_3;
                buffer >>= taken;
                count -= taken;
                refill_word(bytes, &mut position, &mut buffer, &mut count);
                entry = entry_4;
                continue;
            }
            // A refused symbol is handed back whole to the careful path.
            let checkpoint = (position, buffer, count);
            if entry & POINTER != 0 {
                let start = ((entry >> 8) & 0xF_FFFF) as usize;
                let width = entry & 15;
                let index = ((buffer >> PRIMARY_BITS) as usize) & ((1usize << width) - 1);
                entry = subtables[start + index];
                if entry & LITERAL != 0 {
                    // A long literal code: one literal in the full layout.
                    let total = entry & 15;
                    buffer >>= total;
                    count -= total;
                    out[at] = (entry >> 8) as u8;
                    at += 1;
                    refill_word(bytes, &mut position, &mut buffer, &mut count);
                    entry = literals[(buffer as usize) & (PRIMARY_SIZE - 1)];
                    continue;
                }
            }
            let total = entry & 15;
            if total == 0 {
                break;
            }
            buffer >>= total;
            count -= total;
            let symbol = (entry >> 8) & SYMBOL_MASK;
            if symbol == 256 {
                outcome = Some(true);
                break;
            }
            let length_index = (symbol - 257) as usize;
            if length_index >= LENGTH_BASE.len() {
                (position, buffer, count) = checkpoint;
                break;
            }
            let extra = u32::from(LENGTH_EXTRA[length_index]);
            let length =
                usize::from(LENGTH_BASE[length_index]) + (buffer & ((1u64 << extra) - 1)) as usize;
            buffer >>= extra;
            count -= extra;
            let mut distance_entry = distances[(buffer as usize) & (PRIMARY_SIZE - 1)];
            if distance_entry & POINTER != 0 {
                let start = ((distance_entry >> 8) & 0xF_FFFF) as usize;
                let width = distance_entry & 15;
                let index = ((buffer >> PRIMARY_BITS) as usize) & ((1usize << width) - 1);
                distance_entry = distances[start + index];
            }
            let code_length = distance_entry & 15;
            if code_length == 0 {
                (position, buffer, count) = checkpoint;
                break;
            }
            buffer >>= code_length;
            count -= code_length;
            let distance_index = ((distance_entry >> 8) & SYMBOL_MASK) as usize;
            if distance_index >= DISTANCE_BASE.len() {
                (position, buffer, count) = checkpoint;
                break;
            }
            let extra = u32::from(DISTANCE_EXTRA[distance_index]);
            let distance = usize::from(DISTANCE_BASE[distance_index])
                + (buffer & ((1u64 << extra) - 1)) as usize;
            buffer >>= extra;
            count -= extra;
            if distance > at {
                (position, buffer, count) = checkpoint;
                break;
            }
            copy_match_at(&mut out, at, distance, length);
            at += length;
            refill_word(bytes, &mut position, &mut buffer, &mut count);
            entry = literals[(buffer as usize) & (PRIMARY_SIZE - 1)];
        }
        self.out = out;
        self.filled = at;
        reader.position = position;
        // The word refill leaves bits of the byte at `position` above
        // `count`; the careful path expects zeros there.
        reader.buffer = buffer & ((1u64 << count) - 1);
        reader.count = count;
        // Anything the loop refused (a bad code, a bad distance, a short
        // piece) is decoded again by the careful path, which reports it.
        Ok(outcome)
    }

    /// One literal or match; false at the end-of-block code.
    #[inline]
    fn symbol(&mut self, reader: &mut BitReader<'_>) -> Result<bool, InflateError> {
        let symbol = match self.literals.decode(reader) {
            Ok(symbol) => symbol,
            Err(fault) => return Err(self.fault(fault, reader)),
        };
        if symbol < 256 {
            self.push_byte(symbol as u8);
            return Ok(true);
        }
        if symbol == 256 {
            return Ok(false);
        }
        let length_index = usize::from(symbol - 257);
        if length_index >= LENGTH_BASE.len() {
            return Err(InflateError::Corrupt {
                offset: self.offset(reader),
                what: "length code out of range",
            });
        }
        let length = usize::from(LENGTH_BASE[length_index])
            + reader.bits(u32::from(LENGTH_EXTRA[length_index]))? as usize;
        let distance_symbol = match self.distances.decode(reader) {
            Ok(symbol) => usize::from(symbol),
            Err(fault) => return Err(self.fault(fault, reader)),
        };
        if distance_symbol >= DISTANCE_BASE.len() {
            return Err(InflateError::Corrupt {
                offset: self.offset(reader),
                what: "distance code out of range",
            });
        }
        let distance = usize::from(DISTANCE_BASE[distance_symbol])
            + reader.bits(u32::from(DISTANCE_EXTRA[distance_symbol]))? as usize;
        if distance > self.filled {
            return Err(InflateError::BadDistance {
                offset: self.offset(reader),
            });
        }
        self.reserve_room(length);
        copy_match_at(&mut self.out, self.filled, distance, length);
        self.filled += length;
        Ok(true)
    }
}

/// Inflates a complete raw deflate stream (no zlib or gzip header) from
/// `input`, appending to `out`. Returns the number of input bytes the
/// stream used. `limit` caps the output.
pub fn inflate(input: &[u8], out: &mut Vec<u8>, limit: usize) -> Result<usize, InflateError> {
    let mut inflater = Inflater::new();
    let mut at = 0usize;
    loop {
        let (consumed, progress) = inflater.push(&input[at..], usize::MAX)?;
        at += consumed;
        if inflater.output().len() > limit {
            return Err(InflateError::TooLarge);
        }
        match progress {
            Progress::Done => break,
            Progress::NeedInput => {
                if at >= input.len() || consumed == 0 {
                    return Err(InflateError::Truncated);
                }
            }
            Progress::OutputFull => {}
        }
    }
    out.extend_from_slice(inflater.output());
    Ok(inflater.total_in() - inflater.leftover().len())
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
    let literals = Huffman::build(&lengths, 0, Alphabet::Literals)?;
    let distances = Huffman::build(&[5u8; 30], 0, Alphabet::Other)?;
    Ok((literals, distances))
}

fn dynamic_tables(
    reader: &mut BitReader<'_>,
    offset: usize,
) -> Result<(Huffman, Huffman), InflateError> {
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
    let code_length_code = Huffman::build(&code_lengths, offset, Alphabet::Other)?;
    let total = literal_count + distance_count;
    let mut lengths = vec![0u8; total];
    let mut index = 0usize;
    while index < total {
        let symbol = code_length_code.decode_at(reader, offset)?;
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
    let literals = Huffman::build(&lengths[..literal_count], offset, Alphabet::Literals)?;
    let distances = Huffman::build(&lengths[literal_count..], offset, Alphabet::Other)?;
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
    fn pieces_of_any_size_decode_like_the_whole() {
        let text = b"pieces of a stream, fed a few bytes at a time. ".repeat(4000);
        let mut compressed = Vec::new();
        super::super::compress::deflate(&text, &mut compressed);
        compressed.extend_from_slice(b"tail");
        let whole = run(&compressed[..compressed.len() - 4]).unwrap();
        assert_eq!(whole, text);
        for piece_size in [1usize, 3, 7, 8, 9, 100, 4093] {
            let mut inflater = Inflater::new();
            let mut back = Vec::new();
            let mut fed = 0usize;
            let mut done = false;
            for piece in compressed.chunks(piece_size) {
                let mut piece = piece;
                while !piece.is_empty() && !done {
                    let (consumed, progress) = inflater.push(piece, 1000).unwrap();
                    fed += consumed;
                    piece = &piece[consumed..];
                    let produced = inflater.output().len();
                    back.extend_from_slice(inflater.output());
                    inflater.drain(produced);
                    done = matches!(progress, Progress::Done);
                }
                if done {
                    break;
                }
            }
            assert!(done, "piece size {piece_size} never finished");
            assert_eq!(back, text, "piece size {piece_size}");
            assert_eq!(inflater.total_out(), text.len());
            let leftover = inflater.leftover();
            let stream_len = fed - leftover.len();
            assert_eq!(stream_len, compressed.len() - 4, "piece size {piece_size}");
            assert!(
                leftover.len() <= 4,
                "piece size {piece_size} over-read {leftover:?}"
            );
            assert_eq!(leftover, b"tail"[..leftover.len()].to_vec());
        }
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
