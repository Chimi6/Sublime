//! The WebP lossless bitstream (VP8L, RFC 9649): prefix-coded ARGB pixels
//! with LZ77 back-references and a color cache, under up to four
//! transforms (predictor, cross-color, subtract-green, color-indexing).
//! Decodes to an ARGB buffer, one `u32` per pixel; the arithmetic of
//! every predictor and transform follows libwebp's exactly.

use super::WebpError;

fn fail<T>(message: &str) -> Result<T, WebpError> {
    Err(WebpError(message.to_string()))
}

// ------------------------------------------------------------- bit reader

/// Least-significant-bit-first reader.
pub(crate) struct BitReader<'a> {
    bytes: &'a [u8],
    position: usize,
    buffer: u64,
    count: u32,
}

impl<'a> BitReader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> BitReader<'a> {
        BitReader {
            bytes,
            position: 0,
            buffer: 0,
            count: 0,
        }
    }

    #[inline]
    fn refill(&mut self) {
        if self.count <= 56 && self.position + 8 <= self.bytes.len() {
            let word = &self.bytes[self.position..self.position + 8];
            let word = u64::from_le_bytes([
                word[0], word[1], word[2], word[3], word[4], word[5], word[6], word[7],
            ]);
            let taken = (63 - self.count) / 8;
            let mask = (1u64 << (taken * 8)) - 1;
            self.buffer |= (word & mask) << self.count;
            self.position += taken as usize;
            self.count += taken * 8;
            return;
        }
        while self.count <= 56 && self.position < self.bytes.len() {
            self.buffer |= u64::from(self.bytes[self.position]) << self.count;
            self.position += 1;
            self.count += 8;
        }
    }

    #[inline]
    pub(crate) fn bits(&mut self, n: u32) -> Result<u32, WebpError> {
        if n == 0 {
            return Ok(0);
        }
        if self.count < n {
            self.refill();
            if self.count < n {
                return fail("lossless data cut short");
            }
        }
        let value = (self.buffer & ((1u64 << n) - 1)) as u32;
        self.buffer >>= n;
        self.count -= n;
        Ok(value)
    }
}

// ------------------------------------------------------------ prefix codes

const PRIMARY_BITS: u32 = 8;
const PRIMARY_SIZE: usize = 1 << PRIMARY_BITS;
const POINTER: u32 = 1 << 31;

/// A canonical prefix code as a two-level lookup table indexed by the
/// next input bits. An entry is `symbol << 4 | length`; a pointer entry
/// holds its second-level table's offset and width. A code with one
/// symbol reads no bits.
struct Prefix {
    table: Vec<u32>,
    single: Option<u16>,
}

impl Prefix {
    fn build(lengths: &[u8]) -> Result<Prefix, WebpError> {
        let mut used = 0;
        let mut only = 0usize;
        for (symbol, length) in lengths.iter().enumerate() {
            if *length > 0 {
                used += 1;
                only = symbol;
            }
        }
        if used == 0 {
            return fail("a prefix code with no symbols");
        }
        if used == 1 {
            return Ok(Prefix {
                table: Vec::new(),
                single: Some(only as u16),
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
                return fail("over-subscribed prefix code");
            }
        }
        if left != 0 {
            return fail("incomplete prefix code");
        }
        let mut next_code = [0u32; 16];
        let mut code = 0u32;
        for length in 1..16 {
            code = (code + counts[length - 1]) << 1;
            next_code[length] = code;
        }
        let mut reversed = vec![0u32; lengths.len()];
        for (symbol, length) in lengths.iter().enumerate() {
            let length = u32::from(*length);
            if length == 0 {
                continue;
            }
            let code = next_code[length as usize];
            next_code[length as usize] += 1;
            let mut value = 0u32;
            for bit in 0..length {
                value |= ((code >> bit) & 1) << (length - 1 - bit);
            }
            reversed[symbol] = value;
        }
        let mut table = vec![0u32; PRIMARY_SIZE];
        let mut sub_bits = vec![0u32; PRIMARY_SIZE];
        for (symbol, length) in lengths.iter().enumerate() {
            let length = u32::from(*length);
            if length > PRIMARY_BITS {
                let prefix = (reversed[symbol] as usize) & (PRIMARY_SIZE - 1);
                sub_bits[prefix] = sub_bits[prefix].max(length - PRIMARY_BITS);
            }
        }
        for prefix in 0..PRIMARY_SIZE {
            if sub_bits[prefix] > 0 {
                let start = table.len() as u32;
                table.resize(table.len() + (1usize << sub_bits[prefix]), 0);
                table[prefix] = POINTER | (start << 4) | sub_bits[prefix];
            }
        }
        for (symbol, length) in lengths.iter().enumerate() {
            let length = u32::from(*length);
            if length == 0 {
                continue;
            }
            let code = reversed[symbol] as usize;
            let entry = ((symbol as u32) << 4) | length;
            if length <= PRIMARY_BITS {
                let mut index = code;
                while index < PRIMARY_SIZE {
                    table[index] = entry;
                    index += 1 << length;
                }
            } else {
                let pointer = table[code & (PRIMARY_SIZE - 1)];
                let start = ((pointer & !POINTER) >> 4) as usize;
                let width = pointer & 15;
                let mut index = code >> PRIMARY_BITS;
                while index < (1 << width) {
                    table[start + index] = entry;
                    index += 1 << (length - PRIMARY_BITS);
                }
            }
        }
        Ok(Prefix {
            table,
            single: None,
        })
    }

    #[inline]
    fn read(&self, reader: &mut BitReader<'_>) -> Result<u16, WebpError> {
        if let Some(symbol) = self.single {
            return Ok(symbol);
        }
        if reader.count < 15 {
            reader.refill();
        }
        let mut entry = self.table[(reader.buffer as usize) & (PRIMARY_SIZE - 1)];
        if entry & POINTER != 0 {
            let start = ((entry & !POINTER) >> 4) as usize;
            let width = entry & 15;
            let index = ((reader.buffer >> PRIMARY_BITS) as usize) & ((1 << width) - 1);
            entry = self.table[start + index];
        }
        let length = entry & 15;
        if length == 0 || length > reader.count {
            return fail("bad prefix code in lossless data");
        }
        reader.buffer >>= length;
        reader.count -= length;
        Ok((entry >> 4) as u16)
    }
}

const CODE_LENGTH_ORDER: [usize; 19] = [
    17, 18, 0, 1, 2, 3, 4, 5, 16, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
];

/// Reads one prefix code of `alphabet` symbols (a simple code of one or
/// two symbols, or a normal code whose lengths are themselves coded).
fn read_prefix(reader: &mut BitReader<'_>, alphabet: usize) -> Result<Prefix, WebpError> {
    let mut lengths = vec![0u8; alphabet];
    if reader.bits(1)? == 1 {
        let symbols = reader.bits(1)? + 1;
        let first_is_eight_bits = reader.bits(1)?;
        let first = reader.bits(1 + 7 * first_is_eight_bits)? as usize;
        if first >= alphabet {
            return fail("prefix code symbol out of range");
        }
        lengths[first] = 1;
        if symbols == 2 {
            let second = reader.bits(8)? as usize;
            if second >= alphabet {
                return fail("prefix code symbol out of range");
            }
            lengths[second] = 1;
        }
        return Prefix::build(&lengths);
    }
    let count = reader.bits(4)? as usize + 4;
    let mut code_length_lengths = [0u8; 19];
    for position in CODE_LENGTH_ORDER.iter().take(count) {
        code_length_lengths[*position] = reader.bits(3)? as u8;
    }
    let length_code = Prefix::build(&code_length_lengths)?;
    let mut max_symbol = alphabet;
    if reader.bits(1)? == 1 {
        let length_bits = 2 + 2 * reader.bits(3)?;
        max_symbol = 2 + reader.bits(length_bits)? as usize;
        if max_symbol > alphabet {
            return fail("prefix code length count out of range");
        }
    }
    let mut previous = 8u8;
    let mut symbol = 0usize;
    while symbol < alphabet {
        if max_symbol == 0 {
            break;
        }
        max_symbol -= 1;
        let code = length_code.read(reader)?;
        if code < 16 {
            lengths[symbol] = code as u8;
            symbol += 1;
            if code != 0 {
                previous = code as u8;
            }
            continue;
        }
        let (extra, offset, value) = match code {
            16 => (2, 3, previous),
            17 => (3, 3, 0),
            _ => (7, 11, 0),
        };
        let repeat = reader.bits(extra)? as usize + offset;
        if symbol + repeat > alphabet {
            return fail("prefix code lengths run past the alphabet");
        }
        for slot in &mut lengths[symbol..symbol + repeat] {
            *slot = value;
        }
        symbol += repeat;
    }
    Prefix::build(&lengths)
}

/// The five codes that decode one tile's pixels.
struct Group {
    green: Prefix,
    red: Prefix,
    blue: Prefix,
    alpha: Prefix,
    distance: Prefix,
}

// ----------------------------------------------------------- image data

const DISTANCE_MAP: [(i8, i8); 120] = [
    (0, 1),
    (1, 0),
    (1, 1),
    (-1, 1),
    (0, 2),
    (2, 0),
    (1, 2),
    (-1, 2),
    (2, 1),
    (-2, 1),
    (2, 2),
    (-2, 2),
    (0, 3),
    (3, 0),
    (1, 3),
    (-1, 3),
    (3, 1),
    (-3, 1),
    (2, 3),
    (-2, 3),
    (3, 2),
    (-3, 2),
    (0, 4),
    (4, 0),
    (1, 4),
    (-1, 4),
    (4, 1),
    (-4, 1),
    (3, 3),
    (-3, 3),
    (2, 4),
    (-2, 4),
    (4, 2),
    (-4, 2),
    (0, 5),
    (3, 4),
    (-3, 4),
    (4, 3),
    (-4, 3),
    (5, 0),
    (1, 5),
    (-1, 5),
    (5, 1),
    (-5, 1),
    (2, 5),
    (-2, 5),
    (5, 2),
    (-5, 2),
    (4, 4),
    (-4, 4),
    (3, 5),
    (-3, 5),
    (5, 3),
    (-5, 3),
    (0, 6),
    (6, 0),
    (1, 6),
    (-1, 6),
    (6, 1),
    (-6, 1),
    (2, 6),
    (-2, 6),
    (6, 2),
    (-6, 2),
    (4, 5),
    (-4, 5),
    (5, 4),
    (-5, 4),
    (3, 6),
    (-3, 6),
    (6, 3),
    (-6, 3),
    (0, 7),
    (7, 0),
    (1, 7),
    (-1, 7),
    (5, 5),
    (-5, 5),
    (7, 1),
    (-7, 1),
    (4, 6),
    (-4, 6),
    (6, 4),
    (-6, 4),
    (2, 7),
    (-2, 7),
    (7, 2),
    (-7, 2),
    (3, 7),
    (-3, 7),
    (7, 3),
    (-7, 3),
    (5, 6),
    (-5, 6),
    (6, 5),
    (-6, 5),
    (8, 0),
    (4, 7),
    (-4, 7),
    (7, 4),
    (-7, 4),
    (8, 1),
    (8, 2),
    (6, 6),
    (-6, 6),
    (8, 3),
    (5, 7),
    (-5, 7),
    (7, 5),
    (-7, 5),
    (8, 4),
    (6, 7),
    (-6, 7),
    (7, 6),
    (-7, 6),
    (8, 5),
    (7, 7),
    (-7, 7),
    (8, 6),
    (8, 7),
];

/// An LZ77 length or distance from its prefix symbol and extra bits.
#[inline]
fn prefix_value(reader: &mut BitReader<'_>, symbol: u32) -> Result<usize, WebpError> {
    if symbol < 4 {
        return Ok(symbol as usize + 1);
    }
    let extra = (symbol - 2) >> 1;
    let offset = (2 + (symbol & 1)) << extra;
    Ok((offset + reader.bits(extra)?) as usize + 1)
}

#[inline]
fn cache_index(argb: u32, bits: u32) -> usize {
    (0x1e35_a7bd_u32.wrapping_mul(argb) >> (32 - bits)) as usize
}

fn div_round_up(value: usize, bits: u32) -> usize {
    (value + (1 << bits) - 1) >> bits
}

/// Decodes one entropy-coded image of `width` by `height` pixels. The
/// main image (`level0`) may carry meta prefix codes chosen per tile by
/// an entropy image; sub-images (transform data) may not.
fn decode_image(
    reader: &mut BitReader<'_>,
    width: usize,
    height: usize,
    level0: bool,
) -> Result<Vec<u32>, WebpError> {
    let cache_bits = if reader.bits(1)? == 1 {
        let bits = reader.bits(4)?;
        if !(1..=11).contains(&bits) {
            return fail("bad color cache size");
        }
        bits
    } else {
        0
    };
    let mut tile_bits = 0u32;
    let mut tiles_wide = 0usize;
    let mut meta: Vec<u32> = Vec::new();
    let mut group_count = 1usize;
    if level0 && reader.bits(1)? == 1 {
        tile_bits = reader.bits(3)? + 2;
        tiles_wide = div_round_up(width, tile_bits);
        let tiles_high = div_round_up(height, tile_bits);
        let entropy = decode_image(reader, tiles_wide, tiles_high, false)?;
        meta = entropy.iter().map(|argb| (argb >> 8) & 0xffff).collect();
        group_count = meta.iter().copied().max().unwrap_or(0) as usize + 1;
    }
    let cache_size = if cache_bits > 0 {
        1usize << cache_bits
    } else {
        0
    };
    let mut groups = Vec::with_capacity(group_count);
    for _ in 0..group_count {
        groups.push(Group {
            green: read_prefix(reader, 256 + 24 + cache_size)?,
            red: read_prefix(reader, 256)?,
            blue: read_prefix(reader, 256)?,
            alpha: read_prefix(reader, 256)?,
            distance: read_prefix(reader, 40)?,
        });
    }
    let total = width * height;
    let mut pixels = vec![0u32; total];
    let mut cache = vec![0u32; cache_size];
    let mut cached_up_to = 0usize;
    let tile_mask = if tile_bits == 0 {
        usize::MAX
    } else {
        (1 << tile_bits) - 1
    };
    let group_at = |x: usize, y: usize| -> usize {
        if meta.is_empty() {
            0
        } else {
            meta[(y >> tile_bits) * tiles_wide + (x >> tile_bits)] as usize
        }
    };
    let mut at = 0usize;
    let (mut x, mut y) = (0usize, 0usize);
    let mut group = &groups[0];
    while at < total {
        if x & tile_mask == 0 {
            group = &groups[group_at(x, y)];
        }
        let code = u32::from(group.green.read(reader)?);
        if code < 256 {
            let red = u32::from(group.red.read(reader)?);
            let blue = u32::from(group.blue.read(reader)?);
            let alpha = u32::from(group.alpha.read(reader)?);
            pixels[at] = (alpha << 24) | (red << 16) | (code << 8) | blue;
            at += 1;
            x += 1;
            if x == width {
                x = 0;
                y += 1;
            }
        } else if code < 256 + 24 {
            let length = prefix_value(reader, code - 256)?;
            let distance_symbol = u32::from(group.distance.read(reader)?);
            let distance_code = prefix_value(reader, distance_symbol)?;
            let distance = if distance_code > 120 {
                distance_code - 120
            } else {
                let (dx, dy) = DISTANCE_MAP[distance_code - 1];
                let value = i64::from(dx) + i64::from(dy) * width as i64;
                value.max(1) as usize
            };
            if distance > at || at + length > total {
                return fail("back-reference outside the image");
            }
            for index in at..at + length {
                pixels[index] = pixels[index - distance];
            }
            at += length;
            x += length;
            while x >= width {
                x -= width;
                y += 1;
            }
            if at < total {
                group = &groups[group_at(x, y)];
            }
        } else {
            let index = (code - 280) as usize;
            if index >= cache_size {
                return fail("color cache index out of range");
            }
            // The cache holds every pixel before this one; it is brought
            // up to date only when looked up.
            while cached_up_to < at {
                let argb = pixels[cached_up_to];
                cache[cache_index(argb, cache_bits)] = argb;
                cached_up_to += 1;
            }
            pixels[at] = cache[index];
            at += 1;
            x += 1;
            if x == width {
                x = 0;
                y += 1;
            }
        }
    }
    Ok(pixels)
}

// ------------------------------------------------------------- transforms

enum Transform {
    Predictor {
        bits: u32,
        data: Vec<u32>,
        width: usize,
    },
    CrossColor {
        bits: u32,
        data: Vec<u32>,
        width: usize,
    },
    SubtractGreen,
    ColorIndexing {
        table: Vec<u32>,
        width_bits: u32,
        width: usize,
    },
}

#[inline]
fn average(a: u32, b: u32) -> u32 {
    (((a ^ b) & 0xfefe_fefe) >> 1) + (a & b)
}

#[inline]
fn channel(value: u32, shift: u32) -> i32 {
    ((value >> shift) & 0xff) as i32
}

#[inline]
fn clamp_add_subtract_full(a: u32, b: u32, c: u32) -> u32 {
    let mut out = 0u32;
    for shift in [24, 16, 8, 0] {
        let value = (channel(a, shift) + channel(b, shift) - channel(c, shift)).clamp(0, 255);
        out |= (value as u32) << shift;
    }
    out
}

#[inline]
fn clamp_add_subtract_half(a: u32, b: u32) -> u32 {
    let mut out = 0u32;
    for shift in [24, 16, 8, 0] {
        let x = channel(a, shift);
        let value = (x + (x - channel(b, shift)) / 2).clamp(0, 255);
        out |= (value as u32) << shift;
    }
    out
}

#[inline]
fn select(top: u32, left: u32, top_left: u32) -> u32 {
    let mut difference = 0i32;
    for shift in [24, 16, 8, 0] {
        let from_left = (channel(left, shift) - channel(top_left, shift)).abs();
        let from_top = (channel(top, shift) - channel(top_left, shift)).abs();
        difference += from_left - from_top;
    }
    if difference <= 0 { top } else { left }
}

#[inline]
fn add_pixels(a: u32, b: u32) -> u32 {
    let alpha_green = (a & 0xff00_ff00).wrapping_add(b & 0xff00_ff00);
    let red_blue = (a & 0x00ff_00ff).wrapping_add(b & 0x00ff_00ff);
    (alpha_green & 0xff00_ff00) | (red_blue & 0x00ff_00ff)
}

#[inline]
fn predict(mode: u32, left: u32, top: u32, top_left: u32, top_right: u32) -> u32 {
    match mode {
        0 => 0xff00_0000,
        1 => left,
        2 => top,
        3 => top_right,
        4 => top_left,
        5 => average(average(left, top_right), top),
        6 => average(left, top_left),
        7 => average(left, top),
        8 => average(top_left, top),
        9 => average(top, top_right),
        10 => average(average(left, top_left), average(top, top_right)),
        11 => select(top, left, top_left),
        12 => clamp_add_subtract_full(left, top, top_left),
        13 => clamp_add_subtract_half(average(left, top), top_left),
        _ => 0xff00_0000,
    }
}

fn inverse_predictor(
    pixels: &mut [u32],
    width: usize,
    height: usize,
    bits: u32,
    data: &[u32],
    tiles_wide: usize,
) {
    if width == 0 || height == 0 {
        return;
    }
    // Top-left is predicted from black, the rest of the top row from the left.
    pixels[0] = add_pixels(pixels[0], 0xff00_0000);
    for x in 1..width {
        pixels[x] = add_pixels(pixels[x], pixels[x - 1]);
    }
    for y in 1..height {
        let row = y * width;
        // The left column from the pixel above.
        pixels[row] = add_pixels(pixels[row], pixels[row - width]);
        let tile_row = (y >> bits) * tiles_wide;
        for x in 1..width {
            let mode = (data[tile_row + (x >> bits)] >> 8) & 15;
            let at = row + x;
            let left = pixels[at - 1];
            let top = pixels[at - width];
            let top_left = pixels[at - width - 1];
            // The rightmost pixel's top-right is the first pixel of its own row.
            let top_right = pixels[at - width + 1];
            pixels[at] = add_pixels(pixels[at], predict(mode, left, top, top_left, top_right));
        }
    }
}

#[inline]
fn color_delta(multiplier: u8, value: u8) -> i32 {
    (i32::from(multiplier as i8) * i32::from(value as i8)) >> 5
}

fn inverse_cross_color(
    pixels: &mut [u32],
    width: usize,
    height: usize,
    bits: u32,
    data: &[u32],
    tiles_wide: usize,
) {
    for y in 0..height {
        let tile_row = (y >> bits) * tiles_wide;
        for x in 0..width {
            let element = data[tile_row + (x >> bits)];
            let green_to_red = element as u8;
            let green_to_blue = (element >> 8) as u8;
            let red_to_blue = (element >> 16) as u8;
            let argb = pixels[y * width + x];
            let green = (argb >> 8) as u8;
            let mut red = ((argb >> 16) & 0xff) as i32;
            let mut blue = (argb & 0xff) as i32;
            red += color_delta(green_to_red, green);
            red &= 0xff;
            blue += color_delta(green_to_blue, green);
            blue += color_delta(red_to_blue, red as u8);
            blue &= 0xff;
            pixels[y * width + x] = (argb & 0xff00_ff00) | ((red as u32) << 16) | blue as u32;
        }
    }
}

/// Reads a VP8L image stream (after the header): the transforms, then
/// the main image, then applies the transforms in reverse.
pub(crate) fn decode_stream(
    reader: &mut BitReader<'_>,
    width: usize,
    height: usize,
) -> Result<Vec<u32>, WebpError> {
    let mut transforms: Vec<Transform> = Vec::new();
    let mut coded_width = width;
    let mut seen = [false; 4];
    while reader.bits(1)? == 1 {
        let kind = reader.bits(2)? as usize;
        if seen[kind] {
            return fail("a transform appears twice");
        }
        seen[kind] = true;
        match kind {
            0 | 1 => {
                let bits = reader.bits(3)? + 2;
                let tiles_wide = div_round_up(coded_width, bits);
                let tiles_high = div_round_up(height, bits);
                let data = decode_image(reader, tiles_wide, tiles_high, false)?;
                if kind == 0 {
                    transforms.push(Transform::Predictor {
                        bits,
                        data,
                        width: coded_width,
                    });
                } else {
                    transforms.push(Transform::CrossColor {
                        bits,
                        data,
                        width: coded_width,
                    });
                }
            }
            2 => transforms.push(Transform::SubtractGreen),
            _ => {
                let size = reader.bits(8)? as usize + 1;
                let mut table = decode_image(reader, size, 1, false)?;
                for index in 1..size {
                    table[index] = add_pixels(table[index], table[index - 1]);
                }
                let width_bits = match size {
                    0..=2 => 3,
                    3..=4 => 2,
                    5..=16 => 1,
                    _ => 0,
                };
                transforms.push(Transform::ColorIndexing {
                    table,
                    width_bits,
                    width: coded_width,
                });
                coded_width = div_round_up(coded_width, width_bits);
            }
        }
    }
    let mut pixels = decode_image(reader, coded_width, height, true)?;
    for transform in transforms.iter().rev() {
        match transform {
            Transform::Predictor { bits, data, width } => {
                let tiles_wide = div_round_up(*width, *bits);
                inverse_predictor(&mut pixels, *width, height, *bits, data, tiles_wide);
            }
            Transform::CrossColor { bits, data, width } => {
                let tiles_wide = div_round_up(*width, *bits);
                inverse_cross_color(&mut pixels, *width, height, *bits, data, tiles_wide);
            }
            Transform::SubtractGreen => {
                for argb in pixels.iter_mut() {
                    let green = (*argb >> 8) & 0xff;
                    let red_blue = (*argb & 0x00ff_00ff).wrapping_add((green << 16) | green);
                    *argb = (*argb & 0xff00_ff00) | (red_blue & 0x00ff_00ff);
                }
            }
            Transform::ColorIndexing {
                table,
                width_bits,
                width,
            } => {
                let packed_width = div_round_up(*width, *width_bits);
                let per_pixel = 8 >> width_bits;
                let mask = (1u32 << per_pixel) - 1;
                let mut expanded = vec![0u32; width * height];
                for y in 0..height {
                    for x in 0..*width {
                        let packed = pixels[y * packed_width + (x >> width_bits)];
                        let shift = (x & ((1 << width_bits) - 1)) as u32 * per_pixel;
                        let index = ((packed >> 8) >> shift) & mask;
                        expanded[y * width + x] = table.get(index as usize).copied().unwrap_or(0);
                    }
                }
                pixels = expanded;
            }
        }
    }
    Ok(pixels)
}

/// A whole VP8L chunk: the header, then the image stream. Returns the
/// dimensions, whether alpha is used, and the ARGB pixels.
pub(crate) fn decode(bytes: &[u8]) -> Result<(u32, u32, bool, Vec<u32>), WebpError> {
    if bytes.first() != Some(&0x2f) {
        return fail("bad lossless signature");
    }
    let mut reader = BitReader::new(&bytes[1..]);
    let width = reader.bits(14)? + 1;
    let height = reader.bits(14)? + 1;
    let alpha = reader.bits(1)? == 1;
    if reader.bits(3)? != 0 {
        return fail("unknown lossless version");
    }
    let pixels = decode_stream(&mut reader, width as usize, height as usize)?;
    Ok((width, height, alpha, pixels))
}
