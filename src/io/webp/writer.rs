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
/// The predictor modes tried per tile: left, top, the average of the
/// two, select, and the two gradient clamps.
const CANDIDATES: [u32; 6] = [1, 2, 7, 11, 12, 13];

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
        self.buffer |= u64::from(value) << self.count;
        self.count += length;
        if self.count >= 32 {
            self.out
                .extend_from_slice(&(self.buffer as u32).to_le_bytes());
            self.buffer >>= 32;
            self.count -= 32;
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

#[inline]
fn cache_index(argb: u32) -> usize {
    (0x1e35_a7bd_u32.wrapping_mul(argb) >> (32 - CACHE_BITS)) as usize
}

/// The histograms of one entropy-coded image's symbols.
struct Histograms {
    green: Vec<u32>,
    red: [u32; 256],
    blue: [u32; 256],
    alpha: [u32; 256],
    distance: [u32; 40],
}

/// One symbol of an image: a literal pixel (or its color cache index),
/// or a copy of `length` pixels from `distance` back.
enum Token {
    Literal(u32, Option<usize>),
    Copy(usize, usize),
}

/// One image's symbols, from the same decisions in both passes; `emit`
/// counts them in the first pass and writes them in the second.
fn tokenize(pixels: &[u32], width: usize, cache_bits: u32, mut emit: impl FnMut(Token)) {
    let total = pixels.len();
    let mut cache = vec![0u32; if cache_bits > 0 { 1 << cache_bits } else { 0 }];
    let mut at = 0;
    while at < total {
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
            let (best_length, best_distance) = if above_run > left_run {
                (above_run, width)
            } else {
                (left_run, 1)
            };
            if best_length >= MIN_COPY {
                emit(Token::Copy(best_length, best_distance));
                if cache_bits > 0 {
                    for value in &pixels[at..at + best_length] {
                        cache[cache_index_bits(*value, cache_bits)] = *value;
                    }
                }
                at += best_length;
                continue;
            }
        }
        if cache_bits > 0 {
            let index = cache_index_bits(pixel, cache_bits);
            if cache[index] == pixel {
                emit(Token::Literal(pixel, Some(index)));
            } else {
                emit(Token::Literal(pixel, None));
                cache[index] = pixel;
            }
        } else {
            emit(Token::Literal(pixel, None));
        }
        at += 1;
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
        red: [0; 256],
        blue: [0; 256],
        alpha: [0; 256],
        distance: [0; 40],
    };
    tokenize(pixels, width, cache_bits, |token| match token {
        Token::Literal(_, Some(index)) => histograms.green[280 + index] += 1,
        Token::Literal(pixel, None) => {
            histograms.green[((pixel >> 8) & 0xff) as usize] += 1;
            histograms.red[((pixel >> 16) & 0xff) as usize] += 1;
            histograms.blue[(pixel & 0xff) as usize] += 1;
            histograms.alpha[(pixel >> 24) as usize] += 1;
        }
        Token::Copy(length, distance) => {
            histograms.green[256 + prefix_encode(length).0] += 1;
            histograms.distance[prefix_encode(distance_code(distance, width)).0] += 1;
        }
    });
    let codes = [
        Code::from_histogram(&histograms.green),
        Code::from_histogram(&histograms.red),
        Code::from_histogram(&histograms.blue),
        Code::from_histogram(&histograms.alpha),
        Code::from_histogram(&histograms.distance),
    ];
    for code in &codes {
        write_code(bits, code);
    }
    // The exact size of what follows, so the output never regrows.
    let mut total_bits: u64 = 0;
    for (histogram, code) in [
        (&histograms.green[..], &codes[0]),
        (&histograms.red[..], &codes[1]),
        (&histograms.blue[..], &codes[2]),
        (&histograms.alpha[..], &codes[3]),
        (&histograms.distance[..], &codes[4]),
    ] {
        for (count, length) in histogram.iter().zip(&code.lengths) {
            total_bits += u64::from(*count) * u64::from(*length);
        }
    }
    // Extra bits of lengths and distances: at most thirty per copy.
    let copies: u64 = histograms.green[256..280]
        .iter()
        .map(|count| u64::from(*count))
        .sum();
    total_bits += copies * 30;
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
    let byte_table = |code: &Code| -> [u32; 256] {
        let mut table = [0u32; 256];
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
    tokenize(pixels, width, cache_bits, |token| match token {
        Token::Literal(_, Some(index)) => put_packed(bits, green[280 + index]),
        Token::Literal(pixel, None) => {
            // Two writes of at most thirty bits: green and red, blue and alpha.
            let g = green[((pixel >> 8) & 0xff) as usize];
            let r = red[((pixel >> 16) & 0xff) as usize];
            let b = blue[(pixel & 0xff) as usize];
            let a = alpha[(pixel >> 24) as usize];
            let green_length = g >> 16;
            bits.put(
                (g & 0xffff) | ((r & 0xffff) << green_length),
                green_length + (r >> 16),
            );
            let blue_length = b >> 16;
            bits.put(
                (b & 0xffff) | ((a & 0xffff) << blue_length),
                blue_length + (a >> 16),
            );
        }
        Token::Copy(length, distance) => {
            let (symbol, extra, extra_bits) = prefix_encode(length);
            put_packed(bits, green[256 + symbol]);
            bits.put(extra, extra_bits);
            let (symbol, extra, extra_bits) = prefix_encode(distance_code(distance, width));
            put_packed(bits, distances[symbol]);
            bits.put(extra, extra_bits);
        }
    });
}

// ------------------------------------------------------------- transforms

#[inline]
fn channel(value: u32, shift: u32) -> i32 {
    ((value >> shift) & 0xff) as i32
}

#[inline]
fn average(a: u32, b: u32) -> u32 {
    (((a ^ b) & 0xfefe_fefe) >> 1) + (a & b)
}

#[inline(always)]
fn predict<const MODE: u32>(left: u32, top: u32, top_left: u32) -> u32 {
    match MODE {
        1 => left,
        2 => top,
        7 => average(left, top),
        11 => {
            let mut difference = 0i32;
            for shift in [24, 16, 8, 0] {
                difference += (channel(left, shift) - channel(top_left, shift)).abs()
                    - (channel(top, shift) - channel(top_left, shift)).abs();
            }
            if difference <= 0 { top } else { left }
        }
        12 => {
            let mut out = 0u32;
            for shift in [24, 16, 8, 0] {
                let value = (channel(left, shift) + channel(top, shift) - channel(top_left, shift))
                    .clamp(0, 255);
                out |= (value as u32) << shift;
            }
            out
        }
        _ => {
            // 13: half the gradient from the average of left and top.
            let mean = average(left, top);
            let mut out = 0u32;
            for shift in [24, 16, 8, 0] {
                let x = channel(mean, shift);
                let value = (x + (x - channel(top_left, shift)) / 2).clamp(0, 255);
                out |= (value as u32) << shift;
            }
            out
        }
    }
}

#[inline]
fn subtract_pixels(a: u32, b: u32) -> u32 {
    let alpha_green = (a | 0x00ff_00ff).wrapping_sub(b & 0xff00_ff00);
    let red_blue = (a | 0xff00_ff00).wrapping_sub(b & 0x00ff_00ff);
    (alpha_green & 0xff00_ff00) | (red_blue & 0x00ff_00ff)
}

#[inline(always)]
fn residual_cost(residual: u32) -> u32 {
    let mut cost = 0;
    for shift in [24, 16, 8, 0] {
        cost += u32::from(((residual >> shift) as u8 as i8).unsigned_abs());
    }
    cost
}

/// The cost of predicting a tile with `MODE`, from every other row and
/// column of it (the choice is a heuristic; a quarter of the samples
/// chooses nearly as well).
fn tile_cost<const MODE: u32>(
    pixels: &[u32],
    width: usize,
    xs: (usize, usize),
    ys: (usize, usize),
    budget: u32,
) -> u32 {
    let mut cost = 0u32;
    for y in (ys.0..ys.1).step_by(2) {
        let row = y * width;
        for x in (xs.0..xs.1).step_by(2) {
            let at = row + x;
            let prediction =
                predict::<MODE>(pixels[at - 1], pixels[at - width], pixels[at - width - 1]);
            cost += residual_cost(subtract_pixels(pixels[at], prediction));
        }
        if cost >= budget {
            return cost;
        }
    }
    cost
}

/// Replaces one row segment's pixels with their residuals under `MODE`,
/// right to left, so every prediction still sees original neighbors
/// (the row above is replaced only after this one).
fn residual_segment<const MODE: u32>(
    pixels: &mut [u32],
    width: usize,
    row: usize,
    xs: (usize, usize),
) {
    for x in (xs.0..xs.1).rev() {
        let at = row + x;
        let prediction =
            predict::<MODE>(pixels[at - 1], pixels[at - width], pixels[at - width - 1]);
        pixels[at] = subtract_pixels(pixels[at], prediction);
    }
}

/// Chooses a predictor per tile, then turns the pixels into residuals in
/// place (bottom row first, each row right to left). Returns the modes.
fn predictor_residuals(pixels: &mut [u32], width: usize, height: usize) -> Vec<u32> {
    let tile = 1usize << PREDICTOR_BITS;
    let tiles_wide = width.div_ceil(tile);
    let tiles_high = height.div_ceil(tile);
    let mut modes = vec![1u32; tiles_wide * tiles_high];
    for tile_y in 0..tiles_high {
        let ys = ((tile_y * tile).max(1), ((tile_y + 1) * tile).min(height));
        if ys.0 >= ys.1 {
            continue;
        }
        for tile_x in 0..tiles_wide {
            let xs = ((tile_x * tile).max(1), ((tile_x + 1) * tile).min(width));
            if xs.0 >= xs.1 {
                continue;
            }
            let mut best = (u32::MAX, 1u32);
            for mode in CANDIDATES {
                let cost = match mode {
                    1 => tile_cost::<1>(pixels, width, xs, ys, best.0),
                    2 => tile_cost::<2>(pixels, width, xs, ys, best.0),
                    7 => tile_cost::<7>(pixels, width, xs, ys, best.0),
                    11 => tile_cost::<11>(pixels, width, xs, ys, best.0),
                    12 => tile_cost::<12>(pixels, width, xs, ys, best.0),
                    _ => tile_cost::<13>(pixels, width, xs, ys, best.0),
                };
                if cost < best.0 {
                    best = (cost, mode);
                }
            }
            modes[tile_y * tiles_wide + tile_x] = best.1;
        }
    }
    for y in (1..height).rev() {
        let row = y * width;
        let tile_row = (y >> PREDICTOR_BITS) * tiles_wide;
        for tile_x in (0..tiles_wide).rev() {
            let xs = ((tile_x * tile).max(1), ((tile_x + 1) * tile).min(width));
            if xs.0 >= xs.1 {
                continue;
            }
            match modes[tile_row + tile_x] {
                1 => residual_segment::<1>(pixels, width, row, xs),
                2 => residual_segment::<2>(pixels, width, row, xs),
                7 => residual_segment::<7>(pixels, width, row, xs),
                11 => residual_segment::<11>(pixels, width, row, xs),
                12 => residual_segment::<12>(pixels, width, row, xs),
                _ => residual_segment::<13>(pixels, width, row, xs),
            }
        }
        // The left column predicts from the pixel above.
        pixels[row] = subtract_pixels(pixels[row], pixels[row - width]);
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

/// The image's colors, sorted, when there are at most 256.
fn palette(pixels: &[u32]) -> Option<Vec<u32>> {
    let mut colors: Vec<u32> = Vec::with_capacity(257);
    let mut last = None;
    for pixel in pixels {
        if last == Some(*pixel) {
            continue;
        }
        last = Some(*pixel);
        if !colors.contains(pixel) {
            if colors.len() == 256 {
                return None;
            }
            colors.push(*pixel);
        }
    }
    colors.sort_unstable();
    Some(colors)
}

/// Encodes ARGB pixels as a VP8L bitstream.
pub(crate) fn encode(pixels: &mut [u32], width: usize, height: usize, has_alpha: bool) -> Vec<u8> {
    let mut bits = BitWriter::new();
    bits.put(0x2f, 8);
    bits.put((width - 1) as u32, 14);
    bits.put((height - 1) as u32, 14);
    bits.put(u32::from(has_alpha), 1);
    bits.put(0, 3);
    if let Some(colors) = palette(pixels) {
        // Color indexing: the palette as deltas, then indices packed
        // several to a pixel when there are few colors.
        bits.put(1, 1);
        bits.put(3, 2);
        bits.put((colors.len() - 1) as u32, 8);
        let mut deltas = colors.clone();
        for index in (1..deltas.len()).rev() {
            deltas[index] = subtract_pixels(colors[index], colors[index - 1]);
        }
        write_image(&mut bits, &deltas, deltas.len(), false, 0);
        let width_bits = match colors.len() {
            0..=2 => 3,
            3..=4 => 2,
            5..=16 => 1,
            _ => 0,
        };
        let per_pixel = 8 >> width_bits;
        let packed_width = width.div_ceil(1 << width_bits);
        let mut packed = vec![0xff00_0000u32; packed_width * height];
        for y in 0..height {
            for x in 0..width {
                let index = colors.binary_search(&pixels[y * width + x]).unwrap_or(0) as u32;
                let slot = &mut packed[y * packed_width + (x >> width_bits)];
                let shift = (x & ((1 << width_bits) - 1)) as u32 * per_pixel;
                *slot |= index << (8 + shift);
            }
        }
        bits.put(0, 1);
        write_image(&mut bits, &packed, packed_width, true, 0);
        return bits.finish();
    }
    // Subtract green, then the predictor.
    bits.put(1, 1);
    bits.put(2, 2);
    for pixel in pixels.iter_mut() {
        let green = (*pixel >> 8) & 0xff;
        // The green byte set to 0xff absorbs the blue lane's borrow.
        let red_blue = (*pixel | 0xff00_ff00).wrapping_sub((green << 16) | green) & 0x00ff_00ff;
        *pixel = (*pixel & 0xff00_ff00) | red_blue;
    }
    let modes = predictor_residuals(pixels, width, height);
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
pub struct WebpRows<'a> {
    sink: &'a mut dyn Write,
    width: usize,
    height: usize,
    color: ColorType,
    pixels: Vec<u32>,
    has_alpha: bool,
}

impl<'a> WebpRows<'a> {
    pub fn new(sink: &'a mut dyn Write) -> WebpRows<'a> {
        WebpRows {
            sink,
            width: 0,
            height: 0,
            color: ColorType::Rgb,
            pixels: Vec::new(),
            has_alpha: false,
        }
    }

    fn finish(&mut self) -> io::Result<()> {
        let mut pixels = std::mem::take(&mut self.pixels);
        let stream = encode(&mut pixels, self.width, self.height, self.has_alpha);
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
        self.pixels = Vec::with_capacity(self.width * self.height);
        Ok(())
    }

    fn row(&mut self, pixels: &[u8]) -> io::Result<()> {
        let start = self.pixels.len();
        self.pixels.resize(start + self.width, 0);
        let target = &mut self.pixels[start..];
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
        if self.pixels.len() == self.width * self.height {
            self.finish()?;
        }
        Ok(())
    }
}

/// Writes a whole image as lossless WebP.
pub fn write_webp(image: &crate::image::Image, sink: &mut dyn Write) -> io::Result<()> {
    let mut rows = WebpRows::new(sink);
    rows.start(image.width, image.height, image.color)?;
    for y in 0..image.height {
        rows.row(image.row(y))?;
    }
    Ok(())
}

#[allow(dead_code)]
fn unused_cache_index(argb: u32) -> usize {
    cache_index(argb)
}
