//! Writes lossless WebP (VP8L) from rows. The image is held as ARGB
//! (the format's backward references reach anywhere above), then coded
//! in two deterministic passes over the same decisions: the first counts
//! symbols for the prefix codes, the second writes them, so no symbol
//! stream is stored.
//!
//! What it uses: a palette with packed indices when the image has at
//! most 256 colors; otherwise subtract-green and a predictor chosen per
//! 16x16 tile by the smallest sum of residual magnitudes. Then runs that
//! copy the pixel to the left or the pixel above, a color cache for
//! recently seen colors, and one set of prefix codes.

use std::io::{self, Write};

use crate::image::ColorType;
use crate::io::deflate::compress::{canonical_codes, code_lengths};
use crate::io::png::RowSink;

/// Tiles of 16x16 choose their predictor.
const PREDICTOR_BITS: u32 = 4;
/// A 1024-entry color cache.
const CACHE_BITS: u32 = 10;
/// Copies shorter than this cost more than literals.
const MIN_COPY: usize = 3;
const MAX_COPY: usize = 4096;
/// The predictor modes tried per tile: top, the average of left and
/// top, select, and the gradient clamp. Left (1) and the half-gradient
/// (13) chose tiles that these four code within 0.3% of, for a third
/// more scoring.
const CANDIDATES: [u32; 4] = [2, 7, 11, 12];

struct BitWriter {
    out: Vec<u8>,
    buffer: u64,
    count: u32,
}

impl BitWriter {
    fn new() -> BitWriter {
        BitWriter {
            out: Vec::new(),
            buffer: 0,
            count: 0,
        }
    }

    /// Appends `length` bits (at most 32) of `value`, least significant first.
    #[inline]
    fn put(&mut self, value: u32, length: u32) {
        self.put_wide(u64::from(value), length);
    }

    /// Appends `length` bits (at most 63) of `value`: a whole literal
    /// pixel's four codes in one call. The buffer holds under 64 bits
    /// between calls and goes out a word at a time.
    #[inline]
    fn put_wide(&mut self, value: u64, length: u32) {
        self.buffer |= value << self.count;
        let total = self.count + length;
        if total >= 64 {
            self.out.extend_from_slice(&self.buffer.to_le_bytes());
            // What did not fit; `count` is not zero here, since a
            // single value is under 64 bits.
            self.buffer = value >> (64 - self.count);
            self.count = total - 64;
        } else {
            self.count = total;
        }
    }

    fn finish(mut self) -> Vec<u8> {
        while self.count > 0 {
            self.out.push(self.buffer as u8);
            self.buffer >>= 8;
            self.count = self.count.saturating_sub(8);
        }
        self.out
    }
}

/// A prefix code for writing: lengths and bit-reversed codes.
struct Code {
    lengths: Vec<u8>,
    codes: Vec<u16>,
}

impl Code {
    fn from_histogram(histogram: &[u32]) -> Code {
        let lengths = code_lengths(histogram, 15);
        let codes = canonical_codes(&lengths);
        Code { lengths, codes }
    }

    #[inline]
    fn put(&self, bits: &mut BitWriter, symbol: usize) {
        bits.put(
            u32::from(self.codes[symbol]),
            u32::from(self.lengths[symbol]),
        );
    }
}

const CODE_LENGTH_ORDER: [usize; 19] = [
    17, 18, 0, 1, 2, 3, 4, 5, 16, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
];

/// Writes a prefix code's description: the simple form for one or two
/// symbols under 256, the normal form (run-length-coded lengths under
/// their own code) otherwise.
fn write_code(bits: &mut BitWriter, code: &Code) {
    let used: Vec<usize> = code
        .lengths
        .iter()
        .enumerate()
        .filter(|(_, length)| **length > 0)
        .map(|(symbol, _)| symbol)
        .collect();
    if used.is_empty() || (used.len() <= 2 && used.iter().all(|symbol| *symbol < 256)) {
        let first = used.first().copied().unwrap_or(0);
        bits.put(1, 1);
        bits.put((used.len().max(1) - 1) as u32, 1);
        if first < 2 {
            bits.put(0, 1);
            bits.put(first as u32, 1);
        } else {
            bits.put(1, 1);
            bits.put(first as u32, 8);
        }
        if used.len() == 2 {
            bits.put(used[1] as u32, 8);
        }
        return;
    }
    bits.put(0, 1);
    // Run-length code the lengths: 16 repeats the previous nonzero length
    // (which starts at 8), 17 and 18 are runs of zeros.
    let mut tokens: Vec<(u8, u32, u32)> = Vec::new();
    let lengths = &code.lengths;
    let mut index = 0;
    let mut previous = 8u8;
    while index < lengths.len() {
        let value = lengths[index];
        let mut run = 1;
        while index + run < lengths.len() && lengths[index + run] == value {
            run += 1;
        }
        if value == 0 {
            let mut left = run;
            while left >= 11 {
                let take = left.min(138);
                tokens.push((18, (take - 11) as u32, 7));
                left -= take;
            }
            if left >= 3 {
                tokens.push((17, (left - 3) as u32, 3));
                left = 0;
            }
            for _ in 0..left {
                tokens.push((0, 0, 0));
            }
        } else {
            let mut left = run;
            if value != previous {
                tokens.push((value, 0, 0));
                left -= 1;
                previous = value;
            }
            while left >= 3 {
                let take = left.min(6);
                tokens.push((16, (take - 3) as u32, 2));
                left -= take;
            }
            for _ in 0..left {
                tokens.push((value, 0, 0));
            }
        }
        index += run;
    }
    let mut histogram = [0u32; 19];
    for (symbol, _, _) in &tokens {
        histogram[usize::from(*symbol)] += 1;
    }
    let length_code = Code::from_histogram_limited(&histogram, 7);
    let mut count = 19;
    while count > 4 && length_code.lengths[CODE_LENGTH_ORDER[count - 1]] == 0 {
        count -= 1;
    }
    bits.put((count - 4) as u32, 4);
    for position in CODE_LENGTH_ORDER.iter().take(count) {
        bits.put(u32::from(length_code.lengths[*position]), 3);
    }
    // Use the whole alphabet (no max_symbol).
    bits.put(0, 1);
    for (symbol, extra, extra_bits) in tokens {
        length_code.put(bits, usize::from(symbol));
        if extra_bits > 0 {
            bits.put(extra, extra_bits);
        }
    }
}

impl Code {
    fn from_histogram_limited(histogram: &[u32], limit: u8) -> Code {
        let mut lengths = code_lengths(histogram, limit);
        // A code of one symbol is written with a length of one; the
        // decoder reads it as a code of no bits. Two symbols are needed
        // for a complete code otherwise.
        let used = lengths.iter().filter(|length| **length > 0).count();
        if used == 1 {
            let other = if lengths[0] == 0 { 0 } else { 1 };
            lengths[other] = 1;
        }
        let codes = canonical_codes(&lengths);
        Code { lengths, codes }
    }
}

/// An LZ77 length or distance as its prefix symbol and extra bits.
#[inline]
fn prefix_encode(value: usize) -> (usize, u32, u32) {
    let x = (value - 1) as u32;
    if x < 4 {
        return (x as usize, 0, 0);
    }
    let high = 31 - x.leading_zeros();
    let second = (x >> (high - 1)) & 1;
    let extra_bits = high - 1;
    (
        (2 * high + second) as usize,
        x & ((1 << extra_bits) - 1),
        extra_bits,
    )
}

/// The histograms of one entropy-coded image's symbols.
struct Histograms {
    green: Vec<u32>,
    // One spare slot past the 256 symbols, counted for a pixel found in
    // the color cache, so the count is a select and not a branch.
    red: [u32; 257],
    blue: [u32; 257],
    alpha: [u32; 257],
    distance: [u32; 40],
}

/// The first pass's decisions, kept for the writing pass so it neither
/// searches for runs nor hashes again: a bit per pixel for a cache hit,
/// a bit per pixel for the start of a copy, and each copy's length
/// (shifted left one, the low bit set for a copy of the pixel above).
struct Decisions {
    cached_bits: Vec<u64>,
    copy_bits: Vec<u64>,
    copies: Vec<u32>,
}

/// Chooses each symbol of an image (runs copying the pixel to the left
/// or above, color cache hits, literals), counting them into
/// `histograms` and recording the choices. One function with its state
/// in locals: behind a closure's captured references the position and
/// the bit words were reloaded from memory on every pixel.
fn analyze(
    pixels: &[u32],
    width: usize,
    cache_bits: u32,
    histograms: &mut Histograms,
) -> Decisions {
    let total = pixels.len();
    let words = total.div_ceil(64);
    let mut cached_bits = vec![0u64; words];
    let mut copy_bits = vec![0u64; words];
    let mut copies: Vec<u32> = Vec::new();
    let mut cache = vec![0u32; if cache_bits > 0 { 1 << cache_bits } else { 0 }];
    let green = &mut histograms.green[..];
    let red = &mut histograms.red;
    let blue = &mut histograms.blue;
    let alpha = &mut histograms.alpha;
    let mut word = 0u64;
    let mut word_index = 0usize;
    let mut at = 0;
    while at < total {
        if at >> 6 != word_index {
            cached_bits[word_index] = word;
            word_index = at >> 6;
            word = 0;
        }
        let pixel = pixels[at];
        // Runs copying the pixel to the left or the pixel above, tried
        // only when that neighbor matches this pixel at all.
        let left_matches = at >= 1 && pixels[at - 1] == pixel;
        let above_matches = at >= width && width > 1 && pixels[at - width] == pixel;
        if left_matches || above_matches {
            let limit = (total - at).min(MAX_COPY);
            let run = |distance: usize| {
                let mut length = 1;
                while length < limit && pixels[at + length] == pixels[at + length - distance] {
                    length += 1;
                }
                length
            };
            let left_run = if left_matches { run(1) } else { 0 };
            let above_run = if above_matches { run(width) } else { 0 };
            let (length, distance) = if above_run > left_run {
                (above_run, width)
            } else {
                (left_run, 1)
            };
            if length >= MIN_COPY {
                copy_bits[at >> 6] |= 1 << (at & 63);
                copies.push(((length as u32) << 1) | u32::from(distance != 1));
                green[256 + prefix_encode(length).0] += 1;
                histograms.distance[prefix_encode(distance_code(distance, width)).0] += 1;
                if cache_bits > 0 {
                    for value in &pixels[at..at + length] {
                        cache[cache_index_bits(*value, cache_bits)] = *value;
                    }
                }
                at += length;
                continue;
            }
        }
        // A photo's cache hits come at no pattern: store every time (a
        // hit stores the same value) and count by selects, not branches.
        let (cached, index) = if cache_bits > 0 {
            let index = cache_index_bits(pixel, cache_bits);
            let cached = cache[index] == pixel;
            cache[index] = pixel;
            (cached, index)
        } else {
            (false, 0)
        };
        word |= u64::from(cached) << (at & 63);
        let green_symbol = if cached {
            280 + index
        } else {
            ((pixel >> 8) & 0xff) as usize
        };
        let spare = |symbol: u32| if cached { 256 } else { symbol as usize };
        green[green_symbol] += 1;
        red[spare((pixel >> 16) & 0xff)] += 1;
        blue[spare(pixel & 0xff)] += 1;
        alpha[spare(pixel >> 24)] += 1;
        at += 1;
    }
    if let Some(last) = cached_bits.get_mut(word_index) {
        *last = word;
    }
    Decisions {
        cached_bits,
        copy_bits,
        copies,
    }
}

#[inline]
fn cache_index_bits(argb: u32, bits: u32) -> usize {
    (0x1e35_a7bd_u32.wrapping_mul(argb) >> (32 - bits)) as usize
}

/// The plane code for a distance: the two copies used here have short
/// codes (the pixel above is 1, the pixel to the left is 2).
#[inline]
fn distance_code(distance: usize, width: usize) -> usize {
    if distance == width {
        1
    } else if distance == 1 {
        2
    } else {
        distance + 120
    }
}

/// Writes one entropy-coded image: color cache, (for the main image) no
/// meta codes, five prefix codes, then the symbols.
fn write_image(bits: &mut BitWriter, pixels: &[u32], width: usize, main: bool, cache_bits: u32) {
    if cache_bits > 0 {
        bits.put(1, 1);
        bits.put(cache_bits, 4);
    } else {
        bits.put(0, 1);
    }
    if main {
        bits.put(0, 1);
    }
    let cache_size = if cache_bits > 0 {
        1usize << cache_bits
    } else {
        0
    };
    let mut histograms = Histograms {
        green: vec![0; 256 + 24 + cache_size],
        red: [0; 257],
        blue: [0; 257],
        alpha: [0; 257],
        distance: [0; 40],
    };
    let Decisions {
        cached_bits,
        copy_bits,
        copies,
    } = analyze(pixels, width, cache_bits, &mut histograms);
    let codes = [
        Code::from_histogram(&histograms.green),
        Code::from_histogram(&histograms.red[..256]),
        Code::from_histogram(&histograms.blue[..256]),
        Code::from_histogram(&histograms.alpha[..256]),
        Code::from_histogram(&histograms.distance),
    ];
    for code in &codes {
        write_code(bits, code);
    }
    // The exact size of what follows, so the output never regrows.
    let mut total_bits: u64 = 0;
    for (histogram, code) in [
        (&histograms.green[..], &codes[0]),
        (&histograms.red[..256], &codes[1]),
        (&histograms.blue[..256], &codes[2]),
        (&histograms.alpha[..256], &codes[3]),
        (&histograms.distance[..], &codes[4]),
    ] {
        for (count, length) in histogram.iter().zip(&code.lengths) {
            total_bits += u64::from(*count) * u64::from(*length);
        }
    }
    // Extra bits of lengths and distances: at most thirty per copy.
    total_bits += copies.len() as u64 * 30;
    bits.out.reserve((total_bits / 8) as usize + 64);
    // A code of one used symbol reads no bits: writing it with length
    // zero keeps the encoder in step without a branch per symbol.
    let mut codes = codes;
    for code in codes.iter_mut() {
        if code.lengths.iter().filter(|length| **length > 0).count() <= 1 {
            code.lengths.iter_mut().for_each(|length| *length = 0);
        }
    }
    // Each symbol's code and length packed in one word (`code | length
    // << 16`); the byte-indexed channels as fixed arrays.
    let pack = |code: &Code| -> Vec<u32> {
        code.codes
            .iter()
            .zip(&code.lengths)
            .map(|(bits, length)| u32::from(*bits) | (u32::from(*length) << 16))
            .collect()
    };
    // The spare 257th entry writes nothing, for a cached pixel.
    let byte_table = |code: &Code| -> [u32; 257] {
        let mut table = [0u32; 257];
        for (slot, value) in table.iter_mut().zip(pack(code)) {
            *slot = value;
        }
        table
    };
    let green = pack(&codes[0]);
    let red = byte_table(&codes[1]);
    let blue = byte_table(&codes[2]);
    let alpha = byte_table(&codes[3]);
    let distances = pack(&codes[4]);
    let put_packed = |bits: &mut BitWriter, entry: u32| bits.put(entry & 0xffff, entry >> 16);
    // A literal's codes as one value of at most sixty bits and its
    // length; a cached pixel is its index and three empty codes. The
    // hash is computed either way and the choice made by selects.
    let cache_shift = cache_bits.max(1);
    let literal_code = |pixel: u32, cached: bool| -> (u64, u32) {
        let index = cache_index_bits(pixel, cache_shift);
        let green_symbol = if cached {
            280 + index
        } else {
            ((pixel >> 8) & 0xff) as usize
        };
        let spare = |symbol: u32| if cached { 256 } else { symbol as usize };
        let g = green[green_symbol];
        let r = red[spare((pixel >> 16) & 0xff)];
        let b = blue[spare(pixel & 0xff)];
        let a = alpha[spare(pixel >> 24)];
        let mut value = u64::from(g & 0xffff);
        let mut length = g >> 16;
        value |= u64::from(r & 0xffff) << length;
        length += r >> 16;
        value |= u64::from(b & 0xffff) << length;
        length += b >> 16;
        value |= u64::from(a & 0xffff) << length;
        length += a >> 16;
        (value, length)
    };
    let literal = |bits: &mut BitWriter, pixel: u32, cached: bool| {
        let (value, length) = literal_code(pixel, cached);
        bits.put_wide(value, length);
    };
    let mut next_copy = 0;
    let mut at = 0;
    while at < pixels.len() {
        let word = at >> 6;
        let bit = at & 63;
        // A whole word of literals (most of a photo): no copy tests, the
        // cache bits shifted out of a register.
        if bit == 0 && copy_bits[word] == 0 && at + 64 <= pixels.len() {
            // The bit buffer in locals for the word: through `bits` it was
            // a load, an or, and a store per pixel, one chain.
            let mut cached_word = cached_bits[word];
            let mut buffer = bits.buffer;
            let mut count = bits.count;
            for pixel in &pixels[at..at + 64] {
                let (value, length) = literal_code(*pixel, cached_word & 1 != 0);
                cached_word >>= 1;
                buffer |= value << count;
                let total = count + length;
                if total >= 64 {
                    bits.out.extend_from_slice(&buffer.to_le_bytes());
                    buffer = value >> (64 - count);
                    count = total - 64;
                } else {
                    count = total;
                }
            }
            bits.buffer = buffer;
            bits.count = count;
            at += 64;
            continue;
        }
        if (copy_bits[word] >> bit) & 1 != 0 {
            let copy = copies[next_copy];
            next_copy += 1;
            let length = (copy >> 1) as usize;
            let distance = if copy & 1 != 0 { width } else { 1 };
            let (symbol, extra, extra_bits) = prefix_encode(length);
            put_packed(bits, green[256 + symbol]);
            bits.put(extra, extra_bits);
            let (symbol, extra, extra_bits) = prefix_encode(distance_code(distance, width));
            put_packed(bits, distances[symbol]);
            bits.put(extra, extra_bits);
            at += length;
            continue;
        }
        literal(bits, pixels[at], (cached_bits[word] >> bit) & 1 != 0);
        at += 1;
    }
}

// ------------------------------------------------------------- transforms

/// One byte's prediction under `MODE` from its left, top, and top-left
/// neighbors (the same channel of the neighboring pixels). Select (11)
/// looks at all four channels at once and is predicted per pixel.
#[inline(always)]
fn predict_byte<const MODE: u32>(left: u8, top: u8, top_left: u8) -> u8 {
    let (left, top, top_left) = (i16::from(left), i16::from(top), i16::from(top_left));
    let value = match MODE {
        2 => top,
        7 => (left + top) >> 1,
        _ => (left + top - top_left).clamp(0, 255),
    };
    value as u8
}

/// Select (11): the top pixel when the image changes less across than
/// down (summed over the four channels), the left pixel otherwise.
#[inline(always)]
fn select_pixel(left: &[u8], top: &[u8], top_left: &[u8]) -> bool {
    let mut difference = 0i16;
    for channel in 0..4 {
        let corner = i16::from(top_left[channel]);
        difference +=
            (i16::from(left[channel]) - corner).abs() - (i16::from(top[channel]) - corner).abs();
    }
    difference <= 0
}

fn subtract_pixels(a: u32, b: u32) -> u32 {
    let alpha_green = (a | 0x00ff_00ff).wrapping_sub(b & 0xff00_ff00);
    let red_blue = (a | 0xff00_ff00).wrapping_sub(b & 0x00ff_00ff);
    (alpha_green & 0xff00_ff00) | (red_blue & 0x00ff_00ff)
}

/// A row's pixels as bytes, blue first (the byte order of the
/// little-endian words), so predictors run as byte loops.
fn row_bytes(row: &[u32], bytes: &mut [u8]) {
    for (cell, pixel) in bytes.chunks_exact_mut(4).zip(row) {
        cell.copy_from_slice(&pixel.to_le_bytes());
    }
}

/// The sum of residual magnitudes predicting the bytes `start..end` of
/// `row` under `MODE` (`start` is past the first pixel).
fn span_cost<const MODE: u32>(row: &[u8], above: &[u8], start: usize, end: usize) -> u32 {
    let current = &row[start..end];
    let left = &row[start - 4..end - 4];
    let top = &above[start..end];
    let top_left = &above[start - 4..end - 4];
    if MODE == 11 {
        let mut cost = 0u32;
        for (((c, l), t), tl) in current
            .chunks_exact(4)
            .zip(left.chunks_exact(4))
            .zip(top.chunks_exact(4))
            .zip(top_left.chunks_exact(4))
        {
            let prediction = if select_pixel(l, t, tl) { t } else { l };
            for channel in 0..4 {
                let residual = c[channel].wrapping_sub(prediction[channel]);
                cost += u32::from((residual as i8).unsigned_abs());
            }
        }
        return cost;
    }
    let mut cost = 0u32;
    for (((c, l), t), tl) in current.iter().zip(left).zip(top).zip(top_left) {
        let residual = c.wrapping_sub(predict_byte::<MODE>(*l, *t, *tl));
        cost += u32::from((residual as i8).unsigned_abs());
    }
    cost
}

/// Writes the residuals of the bytes `start..end` of `row` under `MODE`
/// into `out`.
fn span_residuals<const MODE: u32>(
    row: &[u8],
    above: &[u8],
    out: &mut [u8],
    start: usize,
    end: usize,
) {
    let current = &row[start..end];
    let left = &row[start - 4..end - 4];
    let top = &above[start..end];
    let top_left = &above[start - 4..end - 4];
    let out = &mut out[start..end];
    if MODE == 11 {
        for ((((o, c), l), t), tl) in out
            .chunks_exact_mut(4)
            .zip(current.chunks_exact(4))
            .zip(left.chunks_exact(4))
            .zip(top.chunks_exact(4))
            .zip(top_left.chunks_exact(4))
        {
            let prediction = if select_pixel(l, t, tl) { t } else { l };
            for channel in 0..4 {
                o[channel] = c[channel].wrapping_sub(prediction[channel]);
            }
        }
        return;
    }
    for ((((o, c), l), t), tl) in out.iter_mut().zip(current).zip(left).zip(top).zip(top_left) {
        *o = c.wrapping_sub(predict_byte::<MODE>(*l, *t, *tl));
    }
}

fn mode_cost(mode: u32, row: &[u8], above: &[u8], start: usize, end: usize) -> u32 {
    match mode {
        2 => span_cost::<2>(row, above, start, end),
        7 => span_cost::<7>(row, above, start, end),
        11 => span_cost::<11>(row, above, start, end),
        _ => span_cost::<12>(row, above, start, end),
    }
}

fn mode_residuals(mode: u32, row: &[u8], above: &[u8], out: &mut [u8], start: usize, end: usize) {
    match mode {
        2 => span_residuals::<2>(row, above, out, start, end),
        7 => span_residuals::<7>(row, above, out, start, end),
        11 => span_residuals::<11>(row, above, out, start, end),
        _ => span_residuals::<12>(row, above, out, start, end),
    }
}

/// How hard the encoder works, from `--quality` as cwebp reads it for
/// lossless output: 50 and under writes with one predictor (the
/// gradient clamp, the best single mode on real images) and no
/// search, 90 and over chooses predictors by an entropy estimate on
/// every row, and anything else (or no quality) scores four predictors
/// on every other row by residual magnitude.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Effort {
    Fast,
    Default,
    Best,
}

impl Effort {
    pub fn from_quality(quality: Option<u8>) -> Effort {
        match quality {
            Some(quality) if quality <= 50 => Effort::Fast,
            Some(quality) if quality >= 90 => Effort::Best,
            _ => Effort::Default,
        }
    }
}

/// The byte span of each predictor tile in a row, the first pixel left
/// out (it predicts from the pixel above whatever the mode).
struct Tiles {
    size: usize,
    wide: usize,
    high: usize,
    spans: Vec<(usize, usize)>,
    /// The index of the first tile with a span (0, or 1 for a one-pixel
    /// wide image).
    first: usize,
}

impl Tiles {
    fn new(width: usize, height: usize) -> Tiles {
        let size = 1usize << PREDICTOR_BITS;
        let tile_bytes = size * 4;
        let row_length = width * 4;
        let wide = width.div_ceil(size);
        let spans: Vec<(usize, usize)> = (0..wide)
            .map(|tile_x| {
                let start = (tile_x * tile_bytes).max(4);
                let end = ((tile_x + 1) * tile_bytes).min(row_length);
                (start, end)
            })
            .filter(|(start, end)| start < end)
            .collect();
        Tiles {
            size,
            wide,
            high: height.div_ceil(size),
            first: wide - spans.len(),
            spans,
        }
    }
}

/// Each tile's mode by the smallest sum of residual magnitudes on every
/// other row.
fn magnitude_modes(pixels: &[u32], width: usize, height: usize, tiles: &Tiles) -> Vec<u32> {
    let spans = &tiles.spans;
    let mut row = vec![0u8; width * 4];
    let mut above = vec![0u8; width * 4];
    let mut costs = vec![0u32; CANDIDATES.len() * spans.len()];
    let mut modes = vec![2u32; tiles.wide * tiles.high];
    for tile_y in 0..tiles.high {
        let first_row = (tile_y * tiles.size).max(1);
        let last_row = ((tile_y + 1) * tiles.size).min(height);
        if first_row >= last_row {
            continue;
        }
        costs.fill(0);
        for y in (first_row..last_row).step_by(2) {
            row_bytes(&pixels[y * width..(y + 1) * width], &mut row);
            row_bytes(&pixels[(y - 1) * width..y * width], &mut above);
            for (candidate, mode) in CANDIDATES.iter().enumerate() {
                let tile_costs = &mut costs[candidate * spans.len()..(candidate + 1) * spans.len()];
                for (cost, (start, end)) in tile_costs.iter_mut().zip(spans) {
                    *cost += mode_cost(*mode, &row, &above, *start, *end);
                }
            }
        }
        for index in 0..spans.len() {
            let mut best = (u32::MAX, 2u32);
            for (candidate, mode) in CANDIDATES.iter().enumerate() {
                let cost = costs[candidate * spans.len() + index];
                if cost < best.0 {
                    best = (cost, *mode);
                }
            }
            modes[tile_y * tiles.wide + tiles.first + index] = best.1;
        }
    }
    modes
}

/// The cost in bits (times 16) of each residual byte value per channel,
/// from the residuals the image has chosen so far: a Shannon estimate
/// of what the prefix codes will spend, starting from a Laplacian guess.
struct ResidualModel {
    counts: [[u32; 256]; 4],
    table: Vec<u16>,
}

impl ResidualModel {
    fn new() -> ResidualModel {
        let mut table = vec![0u16; 1024];
        for (at, cost) in table.iter_mut().enumerate() {
            let magnitude = f32::from((at as u8 as i8).unsigned_abs());
            *cost = (16.0 * (1.0 + 2.0 * (1.0 + magnitude).log2())) as u16;
        }
        ResidualModel {
            counts: [[0; 256]; 4],
            table,
        }
    }

    /// The estimated bits of `bytes`, whose first byte is at `offset` in
    /// its row (so `offset & 3` is its channel).
    fn cost(&self, bytes: &[u8], offset: usize) -> u32 {
        let mut cost = 0u32;
        for (at, byte) in bytes.iter().enumerate() {
            let channel = (offset + at) & 3;
            cost += u32::from(self.table[(channel << 8) | usize::from(*byte)]);
        }
        cost
    }

    fn add(&mut self, bytes: &[u8], offset: usize) {
        for (at, byte) in bytes.iter().enumerate() {
            self.counts[(offset + at) & 3][usize::from(*byte)] += 1;
        }
    }

    fn refresh(&mut self) {
        for channel in 0..4 {
            let total: u32 = self.counts[channel].iter().sum::<u32>() + 256;
            for value in 0..256 {
                let count = self.counts[channel][value] + 1;
                let bits = (total as f32 / count as f32).log2();
                self.table[(channel << 8) | value] = (16.0 * bits).min(65535.0) as u16;
            }
        }
    }
}

/// Each tile's mode by an entropy estimate on every row: the residuals
/// priced by what the tiles above have chosen (libwebp's approach, in
/// its simplest form). About 1% smaller than the magnitude choice on
/// real images, at 40% more encode time.
fn entropy_modes(pixels: &[u32], width: usize, height: usize, tiles: &Tiles) -> Vec<u32> {
    let spans = &tiles.spans;
    let row_length = width * 4;
    let mut row = vec![0u8; row_length];
    let mut above = vec![0u8; row_length];
    let mut costs = vec![0u32; CANDIDATES.len() * spans.len()];
    let mut modes = vec![2u32; tiles.wide * tiles.high];
    // Each candidate's residuals on each row of a tile row.
    let mut residuals = vec![0u8; CANDIDATES.len() * tiles.size * row_length];
    let mut model = ResidualModel::new();
    for tile_y in 0..tiles.high {
        let first_row = (tile_y * tiles.size).max(1);
        let last_row = ((tile_y + 1) * tiles.size).min(height);
        if first_row >= last_row {
            continue;
        }
        costs.fill(0);
        let mut rows = 0;
        for y in first_row..last_row {
            row_bytes(&pixels[y * width..(y + 1) * width], &mut row);
            row_bytes(&pixels[(y - 1) * width..y * width], &mut above);
            for (candidate, mode) in CANDIDATES.iter().enumerate() {
                let at = (candidate * tiles.size + rows) * row_length;
                let buffer = &mut residuals[at..at + row_length];
                let tile_costs = &mut costs[candidate * spans.len()..(candidate + 1) * spans.len()];
                for (cost, (start, end)) in tile_costs.iter_mut().zip(spans) {
                    mode_residuals(*mode, &row, &above, buffer, *start, *end);
                    *cost += model.cost(&buffer[*start..*end], *start);
                }
            }
            rows += 1;
        }
        for (index, (start, end)) in spans.iter().enumerate() {
            let mut best = (u32::MAX, 0usize);
            for candidate in 0..CANDIDATES.len() {
                let cost = costs[candidate * spans.len() + index];
                if cost < best.0 {
                    best = (cost, candidate);
                }
            }
            modes[tile_y * tiles.wide + tiles.first + index] = CANDIDATES[best.1];
            for sample in 0..rows {
                let at = (best.1 * tiles.size + sample) * row_length;
                model.add(&residuals[at + start..at + end], *start);
            }
        }
        model.refresh();
    }
    modes
}

/// Chooses a predictor per tile as `effort` says, then turns the pixels
/// into residuals in place (bottom row first, so each row still sees the
/// original row above). Returns the modes.
///
/// Rows are worked as bytes, a whole row per mode at a time, so every
/// loop is a straight byte loop the compiler vectorizes.
fn predictor_residuals(
    pixels: &mut [u32],
    width: usize,
    height: usize,
    effort: Effort,
) -> Vec<u32> {
    let tiles = Tiles::new(width, height);
    let modes = match effort {
        Effort::Fast => vec![12u32; tiles.wide * tiles.high],
        Effort::Default => magnitude_modes(pixels, width, height, &tiles),
        Effort::Best => entropy_modes(pixels, width, height, &tiles),
    };
    let spans = &tiles.spans;
    let first_tile = tiles.first;
    let tiles_wide = tiles.wide;
    let row_length = width * 4;
    let mut row = vec![0u8; row_length];
    let mut above = vec![0u8; row_length];
    let mut out = vec![0u8; row_length];
    for y in (1..height).rev() {
        let start_of_row = y * width;
        row_bytes(&pixels[start_of_row..start_of_row + width], &mut row);
        row_bytes(&pixels[start_of_row - width..start_of_row], &mut above);
        let tile_row = (y >> PREDICTOR_BITS) * tiles_wide;
        for (index, (start, end)) in spans.iter().enumerate() {
            let mode = modes[tile_row + first_tile + index];
            mode_residuals(mode, &row, &above, &mut out, *start, *end);
        }
        // The left column predicts from the pixel above.
        let first = subtract_pixels(pixels[start_of_row], pixels[start_of_row - width]);
        let target = &mut pixels[start_of_row..start_of_row + width];
        for (pixel, cell) in target.iter_mut().zip(out.chunks_exact(4)).skip(1) {
            *pixel = u32::from_le_bytes([cell[0], cell[1], cell[2], cell[3]]);
        }
        target[0] = first;
    }
    // The top row predicts from the left, its first pixel from black.
    for x in (1..width).rev() {
        pixels[x] = subtract_pixels(pixels[x], pixels[x - 1]);
    }
    if let Some(first) = pixels.first_mut() {
        *first = subtract_pixels(*first, 0xff00_0000);
    }
    modes.iter().map(|mode| 0xff00_0000 | (mode << 8)).collect()
}

/// The colors seen so far, in the order met, while there are at most
/// 256, with a color's index found by open addressing (1024 slots, so a
/// probe rarely goes past its first).
struct Palette {
    colors: Vec<u32>,
    slots: Vec<(u32, u8)>,
    used: Vec<bool>,
}

impl Palette {
    const BITS: u32 = 10;

    fn new() -> Palette {
        let size = 1 << Self::BITS;
        Palette {
            colors: Vec::with_capacity(256),
            slots: vec![(0, 0); size],
            used: vec![false; size],
        }
    }

    /// The index of `color`, added if new; `None` when it would be the
    /// 257th color.
    fn index(&mut self, color: u32) -> Option<u8> {
        let mask = self.slots.len() - 1;
        let mut slot = (0x1e35_a7bd_u32.wrapping_mul(color) >> (32 - Self::BITS)) as usize;
        while self.used[slot] {
            if self.slots[slot].0 == color {
                return Some(self.slots[slot].1);
            }
            slot = (slot + 1) & mask;
        }
        if self.colors.len() == 256 {
            return None;
        }
        let index = self.colors.len() as u8;
        self.colors.push(color);
        self.slots[slot] = (color, index);
        self.used[slot] = true;
        Some(index)
    }
}

fn header(width: usize, height: usize, has_alpha: bool) -> BitWriter {
    let mut bits = BitWriter::new();
    bits.put(0x2f, 8);
    bits.put((width - 1) as u32, 14);
    bits.put((height - 1) as u32, 14);
    bits.put(u32::from(has_alpha), 1);
    bits.put(0, 3);
    bits
}

/// Encodes an image of at most 256 colors, given as indices into
/// `colors`, as a VP8L bitstream: color indexing with the palette
/// sorted and written as deltas, then the indices packed several to a
/// pixel when there are few colors.
fn encode_indexed(
    indices: &[u8],
    colors: &[u32],
    width: usize,
    height: usize,
    has_alpha: bool,
) -> Vec<u8> {
    let mut bits = header(width, height, has_alpha);
    let mut sorted = colors.to_vec();
    sorted.sort_unstable();
    // Each index as met to its place in the sorted palette.
    let mut remap = [0u32; 256];
    for (index, color) in colors.iter().enumerate() {
        remap[index] = sorted.binary_search(color).unwrap_or(0) as u32;
    }
    bits.put(1, 1);
    bits.put(3, 2);
    bits.put((sorted.len() - 1) as u32, 8);
    let mut deltas = sorted.clone();
    for index in (1..deltas.len()).rev() {
        deltas[index] = subtract_pixels(sorted[index], sorted[index - 1]);
    }
    write_image(&mut bits, &deltas, deltas.len(), false, 0);
    let width_bits = match sorted.len() {
        0..=2 => 3,
        3..=4 => 2,
        5..=16 => 1,
        _ => 0,
    };
    let per_pixel = 8 >> width_bits;
    let packed_width = width.div_ceil(1 << width_bits);
    let mut packed = vec![0xff00_0000u32; packed_width * height];
    for (packed_row, row) in packed
        .chunks_exact_mut(packed_width)
        .zip(indices.chunks_exact(width))
    {
        for (x, index) in row.iter().enumerate() {
            let shift = (x & ((1 << width_bits) - 1)) as u32 * per_pixel;
            packed_row[x >> width_bits] |= remap[usize::from(*index)] << (8 + shift);
        }
    }
    bits.put(0, 1);
    write_image(&mut bits, &packed, packed_width, true, 0);
    bits.finish()
}

/// Encodes ARGB pixels of more than 256 colors as a VP8L bitstream:
/// subtract green, the predictor, then the residuals.
fn encode(
    pixels: &mut [u32],
    width: usize,
    height: usize,
    has_alpha: bool,
    effort: Effort,
) -> Vec<u8> {
    let mut bits = header(width, height, has_alpha);
    bits.put(1, 1);
    bits.put(2, 2);
    for pixel in pixels.iter_mut() {
        let green = (*pixel >> 8) & 0xff;
        // The green byte set to 0xff absorbs the blue lane's borrow.
        let red_blue = (*pixel | 0xff00_ff00).wrapping_sub((green << 16) | green) & 0x00ff_00ff;
        *pixel = (*pixel & 0xff00_ff00) | red_blue;
    }
    let modes = predictor_residuals(pixels, width, height, effort);
    bits.put(1, 1);
    bits.put(0, 2);
    bits.put(PREDICTOR_BITS - 2, 3);
    write_image(
        &mut bits,
        &modes,
        width.div_ceil(1 << PREDICTOR_BITS),
        false,
        0,
    );
    bits.put(0, 1);
    write_image(&mut bits, pixels, width, true, CACHE_BITS);
    bits.finish()
}

/// Writes a lossless WebP from rows as a `RowSink`.
///
/// While the image has at most 256 colors its rows are held as a byte
/// per pixel (the palette index); the 257th color turns what is held
/// into ARGB words, and later rows are held that way.
pub struct WebpRows<'a> {
    sink: &'a mut dyn Write,
    width: usize,
    height: usize,
    color: ColorType,
    rows_seen: usize,
    palette: Option<Palette>,
    indices: Vec<u8>,
    scratch: Vec<u32>,
    pixels: Vec<u32>,
    has_alpha: bool,
    effort: Effort,
}

impl<'a> WebpRows<'a> {
    pub fn new(sink: &'a mut dyn Write) -> WebpRows<'a> {
        WebpRows {
            sink,
            width: 0,
            height: 0,
            color: ColorType::Rgb,
            rows_seen: 0,
            palette: None,
            indices: Vec::new(),
            scratch: Vec::new(),
            pixels: Vec::new(),
            has_alpha: false,
            effort: Effort::Default,
        }
    }

    /// Sets how hard the encoder works (`Effort::from_quality`).
    pub fn with_effort(mut self, effort: Effort) -> WebpRows<'a> {
        self.effort = effort;
        self
    }

    fn finish(&mut self) -> io::Result<()> {
        let stream = match self.palette.take() {
            Some(palette) => encode_indexed(
                &self.indices,
                &palette.colors,
                self.width,
                self.height,
                self.has_alpha,
            ),
            None => {
                let mut pixels = std::mem::take(&mut self.pixels);
                encode(
                    &mut pixels,
                    self.width,
                    self.height,
                    self.has_alpha,
                    self.effort,
                )
            }
        };
        let padded = stream.len() + (stream.len() & 1);
        let mut head = Vec::with_capacity(20);
        head.extend_from_slice(b"RIFF");
        head.extend_from_slice(&((4 + 8 + padded) as u32).to_le_bytes());
        head.extend_from_slice(b"WEBPVP8L");
        head.extend_from_slice(&(stream.len() as u32).to_le_bytes());
        self.sink.write_all(&head)?;
        self.sink.write_all(&stream)?;
        if stream.len() & 1 == 1 {
            self.sink.write_all(&[0])?;
        }
        self.sink.flush()
    }

    /// Converts one row to ARGB words in `target`, noting alpha.
    fn argb_row(&mut self, pixels: &[u8], target: &mut [u32]) {
        match self.color {
            ColorType::Gray => {
                for (argb, value) in target.iter_mut().zip(pixels) {
                    let v = u32::from(*value);
                    *argb = 0xff00_0000 | (v << 16) | (v << 8) | v;
                }
            }
            ColorType::GrayAlpha => {
                let mut opaque = true;
                for (argb, cell) in target.iter_mut().zip(pixels.chunks_exact(2)) {
                    let v = u32::from(cell[0]);
                    let a = u32::from(cell[1]);
                    opaque &= a == 255;
                    *argb = (a << 24) | (v << 16) | (v << 8) | v;
                }
                self.has_alpha |= !opaque;
            }
            ColorType::Rgb => {
                for (argb, cell) in target.iter_mut().zip(pixels.chunks_exact(3)) {
                    *argb = 0xff00_0000
                        | (u32::from(cell[0]) << 16)
                        | (u32::from(cell[1]) << 8)
                        | u32::from(cell[2]);
                }
            }
            ColorType::Rgba => {
                let mut opaque = true;
                for (argb, cell) in target.iter_mut().zip(pixels.chunks_exact(4)) {
                    let a = u32::from(cell[3]);
                    opaque &= a == 255;
                    *argb = (a << 24)
                        | (u32::from(cell[0]) << 16)
                        | (u32::from(cell[1]) << 8)
                        | u32::from(cell[2]);
                }
                self.has_alpha |= !opaque;
            }
        }
    }

    /// Indexes a row held in `scratch` while the colors fit a palette;
    /// on the 257th color, turns the rows held so far and this one into
    /// ARGB words.
    fn index_row(&mut self) {
        let Some(palette) = self.palette.as_mut() else {
            return;
        };
        let row_start = self.indices.len();
        self.indices.resize(row_start + self.width, 0);
        // The row's indices written into a slice with the last color in
        // locals: a push per pixel through `self` reloaded the vector's
        // length every time.
        let targets = &mut self.indices[row_start..];
        let mut last_color = self.scratch[0];
        let mut last_index = palette.index(last_color);
        let mut overflow = last_index.is_none();
        if !overflow {
            for (target, pixel) in targets.iter_mut().zip(&self.scratch) {
                if *pixel != last_color {
                    last_color = *pixel;
                    last_index = palette.index(last_color);
                    if last_index.is_none() {
                        overflow = true;
                        break;
                    }
                }
                *target = last_index.unwrap_or(0);
            }
        }
        if overflow {
            // The 257th color: what is held becomes ARGB words.
            self.indices.truncate(row_start);
            let colors = &palette.colors;
            self.pixels = Vec::with_capacity(self.width * self.height);
            self.pixels
                .extend(self.indices.iter().map(|index| colors[usize::from(*index)]));
            self.pixels.extend_from_slice(&self.scratch);
            self.indices = Vec::new();
            self.palette = None;
        }
    }
}

impl RowSink for WebpRows<'_> {
    fn start(&mut self, width: u32, height: u32, color: ColorType) -> io::Result<()> {
        if width == 0 || height == 0 || width > 16_384 || height > 16_384 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "WebP holds 1 to 16384 pixels on a side",
            ));
        }
        self.width = width as usize;
        self.height = height as usize;
        self.color = color;
        self.rows_seen = 0;
        self.palette = Some(Palette::new());
        self.indices = Vec::with_capacity(self.width * self.height);
        self.scratch = vec![0; self.width];
        Ok(())
    }

    fn row(&mut self, pixels: &[u8]) -> io::Result<()> {
        if self.palette.is_some() {
            let mut scratch = std::mem::take(&mut self.scratch);
            self.argb_row(pixels, &mut scratch);
            self.scratch = scratch;
            self.index_row();
        } else {
            let start = self.pixels.len();
            self.pixels.resize(start + self.width, 0);
            let mut target = std::mem::take(&mut self.pixels);
            self.argb_row(pixels, &mut target[start..]);
            self.pixels = target;
        }
        self.rows_seen += 1;
        if self.rows_seen == self.height {
            self.finish()?;
        }
        Ok(())
    }
}

/// Writes a whole image as lossless WebP.
pub fn write_webp(image: &crate::image::Image, sink: &mut dyn Write) -> io::Result<()> {
    write_webp_with(image, sink, Effort::Default)
}

/// Writes a whole image as lossless WebP at `effort`.
pub fn write_webp_with(
    image: &crate::image::Image,
    sink: &mut dyn Write,
    effort: Effort,
) -> io::Result<()> {
    let mut rows = WebpRows::new(sink).with_effort(effort);
    rows.start(image.width, image.height, image.color)?;
    for y in 0..image.height {
        rows.row(image.row(y))?;
    }
    Ok(())
}
