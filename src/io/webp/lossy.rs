//! The WebP lossy bitstream: VP8 key frames (RFC 6386) decoded the way
//! libwebp decodes them, so the pixels match what browsers and Pillow
//! produce. Macroblock rows stream through a window of two rows: each row
//! is predicted and reconstructed, loop-filtered, and the row above it is
//! then final and goes out through libwebp's fancy chroma upsampler and
//! fixed-point color conversion. An alpha plane (the `ALPH` chunk) is
//! decoded first and joined to the rows as they go out.

use super::tables::{AC_QUANT, BMODE_PROBS, COEFF_PROBS, COEFF_UPDATE_PROBS, DC_QUANT};
use super::{WebpError, lossless, to_rows};
use crate::image::ColorType;
use crate::io::png::{RowSink, RowsError};

fn fail<T>(message: &str) -> Result<T, WebpError> {
    Err(WebpError(message.to_string()))
}

// ---------------------------------------------------------- bool decoder

/// The VP8 boolean entropy decoder, in libwebp's form: `range` holds the
/// range minus one, `value` a window of up to 64 bits of which `bits`
/// plus eight are valid.
struct BoolDecoder<'a> {
    data: &'a [u8],
    position: usize,
    value: u64,
    bits: i32,
    range: u32,
    eof: bool,
}

impl<'a> BoolDecoder<'a> {
    fn new(data: &'a [u8]) -> BoolDecoder<'a> {
        let mut decoder = BoolDecoder {
            data,
            position: 0,
            value: 0,
            bits: -8,
            range: 254,
            eof: false,
        };
        decoder.load();
        decoder
    }

    fn load(&mut self) {
        if self.position + 8 <= self.data.len() {
            let bytes = &self.data[self.position..self.position + 8];
            let word = u64::from_be_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
            ]);
            self.value = (self.value << 56) | (word >> 8);
            self.bits += 56;
            self.position += 7;
        } else if self.position < self.data.len() {
            self.value = (self.value << 8) | u64::from(self.data[self.position]);
            self.bits += 8;
            self.position += 1;
        } else if !self.eof {
            self.value <<= 8;
            self.bits += 8;
            self.eof = true;
        } else {
            self.bits = 0;
        }
    }

    #[inline]
    fn bit(&mut self, probability: u8) -> bool {
        if self.bits < 0 {
            self.load();
        }
        let position = self.bits;
        let split = (self.range * u32::from(probability)) >> 8;
        let window = (self.value >> position) as u32;
        let bit = window > split;
        let mut range = if bit {
            self.value -= u64::from(split + 1) << position;
            self.range - split
        } else {
            split + 1
        };
        let shift = 7 ^ (31 - range.leading_zeros());
        range <<= shift;
        self.bits -= shift as i32;
        self.range = range - 1;
        bit
    }

    /// `n` bits, most significant first, each at even odds.
    fn value(&mut self, n: u32) -> i32 {
        let mut value = 0i32;
        for index in (0..n).rev() {
            value |= i32::from(self.bit(128)) << index;
        }
        value
    }

    fn flag(&mut self) -> bool {
        self.bit(128)
    }

    fn signed(&mut self, n: u32) -> i32 {
        let value = self.value(n);
        if self.flag() { -value } else { value }
    }

    fn optional_signed(&mut self, n: u32) -> i32 {
        if self.flag() { self.signed(n) } else { 0 }
    }
}

// --------------------------------------------------------------- header

/// Luma 16x16 and chroma modes, and the 4x4 modes, in RFC numbering.
const DC_PRED: u8 = 0;
const V_PRED: u8 = 1;
const H_PRED: u8 = 2;
const TM_PRED: u8 = 3;

const B_DC_PRED: u8 = 0;
const B_TM_PRED: u8 = 1;
const B_VE_PRED: u8 = 2;
const B_HE_PRED: u8 = 3;
const B_LD_PRED: u8 = 4;
const B_RD_PRED: u8 = 5;
const B_VR_PRED: u8 = 6;
const B_VL_PRED: u8 = 7;
const B_HD_PRED: u8 = 8;
const B_HU_PRED: u8 = 9;

/// RFC 6386's `bmode_tree`: pairs of branches, leaves as negated modes.
const BMODE_TREE: [i8; 18] = [
    -(B_DC_PRED as i8),
    2,
    -(B_TM_PRED as i8),
    4,
    -(B_VE_PRED as i8),
    6,
    8,
    12,
    -(B_HE_PRED as i8),
    10,
    -(B_RD_PRED as i8),
    -(B_VR_PRED as i8),
    -(B_LD_PRED as i8),
    14,
    -(B_VL_PRED as i8),
    16,
    -(B_HD_PRED as i8),
    -(B_HU_PRED as i8),
];

/// A 16x16 or chroma mode's equivalent 4x4 mode, for the contexts of
/// neighboring 4x4 blocks.
fn as_bmode(mode: u8) -> u8 {
    match mode {
        V_PRED => B_VE_PRED,
        H_PRED => B_HE_PRED,
        TM_PRED => B_TM_PRED,
        _ => B_DC_PRED,
    }
}

const ZIGZAG: [usize; 16] = [0, 1, 4, 8, 5, 2, 3, 6, 9, 12, 13, 10, 7, 11, 14, 15];
const BANDS: [usize; 17] = [0, 1, 2, 3, 6, 4, 5, 6, 6, 6, 6, 6, 6, 6, 6, 7, 0];
const CAT3: [u8; 3] = [173, 148, 140];
const CAT4: [u8; 4] = [176, 155, 140, 135];
const CAT5: [u8; 5] = [180, 157, 141, 134, 130];
const CAT6: [u8; 11] = [254, 254, 243, 230, 196, 177, 153, 140, 133, 130, 129];

/// Dequantization factors of one segment: `[dc, ac]` for luma, the
/// second-order luma block, and chroma.
#[derive(Clone, Copy, Default)]
struct Quant {
    y1: [i32; 2],
    y2: [i32; 2],
    uv: [i32; 2],
}

/// The loop filter's settings for a macroblock.
#[derive(Clone, Copy, Default)]
struct FilterInfo {
    limit: i32,
    interior: i32,
    hev_threshold: i32,
    inner: bool,
}

struct Header {
    mb_w: usize,
    mb_h: usize,
    update_map: bool,
    segment_probs: [u8; 3],
    quant: [Quant; 4],
    /// `[segment][is_i4x4]`.
    filters: [[FilterInfo; 2]; 4],
    /// 0 none, 1 simple, 2 normal.
    filter_type: u8,
    probs: Vec<u8>,
    skip_prob: Option<u8>,
    partition_count: usize,
}

fn clip(value: i32, max: i32) -> usize {
    value.clamp(0, max) as usize
}

fn read_header(
    first: &mut BoolDecoder<'_>,
    width: usize,
    height: usize,
) -> Result<Header, WebpError> {
    let _color_space = first.flag();
    let _clamping = first.flag();
    let use_segment = first.flag();
    let mut update_map = false;
    let mut absolute = false;
    let mut segment_quant = [0i32; 4];
    let mut segment_filter = [0i32; 4];
    let mut segment_probs = [255u8; 3];
    if use_segment {
        update_map = first.flag();
        let update_data = first.flag();
        if update_data {
            absolute = first.flag();
            for value in segment_quant.iter_mut() {
                *value = first.optional_signed(7);
            }
            for value in segment_filter.iter_mut() {
                *value = first.optional_signed(6);
            }
        }
        if update_map {
            for probability in segment_probs.iter_mut() {
                *probability = if first.flag() {
                    first.value(8) as u8
                } else {
                    255
                };
            }
        }
    }
    let simple = first.flag();
    let level = first.value(6);
    let sharpness = first.value(3);
    let use_lf_delta = first.flag();
    let mut ref_delta = [0i32; 4];
    let mut mode_delta = [0i32; 4];
    if use_lf_delta && first.flag() {
        for value in ref_delta.iter_mut() {
            if first.flag() {
                *value = first.signed(6);
            }
        }
        for value in mode_delta.iter_mut() {
            if first.flag() {
                *value = first.signed(6);
            }
        }
    }
    let partition_count = 1usize << first.value(2);
    let filter_type = if level == 0 {
        0
    } else if simple {
        1
    } else {
        2
    };
    let base_q = first.value(7);
    let y1_dc = first.optional_signed(4);
    let y2_dc = first.optional_signed(4);
    let y2_ac = first.optional_signed(4);
    let uv_dc = first.optional_signed(4);
    let uv_ac = first.optional_signed(4);
    let mut quant = [Quant::default(); 4];
    for (segment, slot) in quant.iter_mut().enumerate() {
        let q = if use_segment {
            segment_quant[segment] + if absolute { 0 } else { base_q }
        } else {
            base_q
        };
        let mut y2_ac_step = (AC_QUANT[clip(q + y2_ac, 127)] * 101_581) >> 16;
        if y2_ac_step < 8 {
            y2_ac_step = 8;
        }
        *slot = Quant {
            y1: [DC_QUANT[clip(q + y1_dc, 127)], AC_QUANT[clip(q, 127)]],
            y2: [DC_QUANT[clip(q + y2_dc, 127)] * 2, y2_ac_step],
            uv: [
                DC_QUANT[clip(q + uv_dc, 117)],
                AC_QUANT[clip(q + uv_ac, 127)],
            ],
        };
    }
    let mut filters = [[FilterInfo::default(); 2]; 4];
    if filter_type > 0 {
        for segment in 0..4 {
            let mut base = level;
            if use_segment {
                base = segment_filter[segment];
                if !absolute {
                    base += level;
                }
            }
            for (i4x4, info) in filters[segment].iter_mut().enumerate() {
                let mut value = base;
                if use_lf_delta {
                    value += ref_delta[0];
                    if i4x4 == 1 {
                        value += mode_delta[0];
                    }
                }
                let value = value.clamp(0, 63);
                info.inner = i4x4 == 1;
                if value > 0 {
                    let mut interior = value;
                    if sharpness > 0 {
                        if sharpness > 4 {
                            interior >>= 2;
                        } else {
                            interior >>= 1;
                        }
                        if interior > 9 - sharpness {
                            interior = 9 - sharpness;
                        }
                    }
                    if interior < 1 {
                        interior = 1;
                    }
                    info.interior = interior;
                    info.limit = 2 * value + interior;
                    info.hev_threshold = if value >= 40 {
                        2
                    } else if value >= 15 {
                        1
                    } else {
                        0
                    };
                } else {
                    info.limit = 0;
                }
            }
        }
    }
    let _refresh = first.flag();
    let mut probs = COEFF_PROBS.to_vec();
    for (index, probability) in probs.iter_mut().enumerate() {
        if first.bit(COEFF_UPDATE_PROBS[index]) {
            *probability = first.value(8) as u8;
        }
    }
    let skip_prob = if first.flag() {
        Some(first.value(8) as u8)
    } else {
        None
    };
    Ok(Header {
        mb_w: width.div_ceil(16),
        mb_h: height.div_ceil(16),
        update_map,
        segment_probs,
        quant,
        filters,
        filter_type,
        probs,
        skip_prob,
        partition_count,
    })
}

// --------------------------------------------------------------- tokens

/// Decodes one block's coefficients from position `first`, dequantized,
/// into `out` in natural order. Returns the position after the last
/// nonzero coefficient (libwebp's `GetCoeffs`).
fn read_coefficients(
    tokens: &mut BoolDecoder<'_>,
    probs: &[u8],
    kind: usize,
    context: usize,
    quant: [i32; 2],
    first: usize,
    out: &mut [i16],
) -> usize {
    let at = |position: usize, context: usize| (kind * 8 + BANDS[position]) * 33 + context * 11;
    let mut n = first;
    let mut p = at(n, context);
    while n < 16 {
        if !tokens.bit(probs[p]) {
            return n;
        }
        while !tokens.bit(probs[p + 1]) {
            n += 1;
            if n == 16 {
                return 16;
            }
            p = at(n, 0);
        }
        let value;
        if !tokens.bit(probs[p + 2]) {
            value = 1;
            p = at(n + 1, 1);
        } else {
            value = large_value(tokens, &probs[p..p + 11]);
            p = at(n + 1, 2);
        }
        let signed = if tokens.bit(128) { -value } else { value };
        let factor = if n > 0 { quant[1] } else { quant[0] };
        out[ZIGZAG[n]] = (signed * factor) as i16;
        n += 1;
    }
    16
}

fn large_value(tokens: &mut BoolDecoder<'_>, p: &[u8]) -> i32 {
    if !tokens.bit(p[3]) {
        if !tokens.bit(p[4]) {
            2
        } else {
            3 + i32::from(tokens.bit(p[5]))
        }
    } else if !tokens.bit(p[6]) {
        if !tokens.bit(p[7]) {
            5 + i32::from(tokens.bit(159))
        } else {
            let mut value = 7 + 2 * i32::from(tokens.bit(165));
            value += i32::from(tokens.bit(145));
            value
        }
    } else {
        let bit1 = usize::from(tokens.bit(p[8]));
        let bit0 = usize::from(tokens.bit(p[9 + bit1]));
        let category = 2 * bit1 + bit0;
        let table: &[u8] = match category {
            0 => &CAT3,
            1 => &CAT4,
            2 => &CAT5,
            _ => &CAT6,
        };
        let mut value = 0i32;
        for probability in table {
            value = value + value + i32::from(tokens.bit(*probability));
        }
        value + 3 + (8 << category)
    }
}

// ---------------------------------------------------------- transforms

fn inverse_wht(input: &[i16; 16], out: &mut [i16]) {
    let mut tmp = [0i32; 16];
    for i in 0..4 {
        let a0 = i32::from(input[i]) + i32::from(input[12 + i]);
        let a1 = i32::from(input[4 + i]) + i32::from(input[8 + i]);
        let a2 = i32::from(input[4 + i]) - i32::from(input[8 + i]);
        let a3 = i32::from(input[i]) - i32::from(input[12 + i]);
        tmp[i] = a0 + a1;
        tmp[8 + i] = a0 - a1;
        tmp[4 + i] = a3 + a2;
        tmp[12 + i] = a3 - a2;
    }
    for i in 0..4 {
        let dc = tmp[i * 4] + 3;
        let a0 = dc + tmp[3 + i * 4];
        let a1 = tmp[1 + i * 4] + tmp[2 + i * 4];
        let a2 = tmp[1 + i * 4] - tmp[2 + i * 4];
        let a3 = dc - tmp[3 + i * 4];
        let base = i * 64;
        out[base] = ((a0 + a1) >> 3) as i16;
        out[base + 16] = ((a3 + a2) >> 3) as i16;
        out[base + 32] = ((a0 - a1) >> 3) as i16;
        out[base + 48] = ((a3 - a2) >> 3) as i16;
    }
}

#[inline(always)]
fn mul1(a: i32) -> i32 {
    ((a * 20_091) >> 16) + a
}

#[inline(always)]
fn mul2(a: i32) -> i32 {
    (a * 35_468) >> 16
}

/// Adds the inverse DCT of `input` to the 4x4 block at `at` in `buffer`
/// (libwebp's `TransformOne`, with its DC-only shortcut).
fn add_idct(input: &[i16], buffer: &mut [u8], at: usize, stride: usize) {
    if input[1..16].iter().all(|coefficient| *coefficient == 0) {
        let dc = (i32::from(input[0]) + 4) >> 3;
        if dc != 0 {
            for row in 0..4 {
                for column in 0..4 {
                    let slot = &mut buffer[at + row * stride + column];
                    *slot = (i32::from(*slot) + dc).clamp(0, 255) as u8;
                }
            }
        }
        return;
    }
    let mut c = [0i32; 16];
    for i in 0..4 {
        let a = i32::from(input[i]) + i32::from(input[8 + i]);
        let b = i32::from(input[i]) - i32::from(input[8 + i]);
        let cc = mul2(i32::from(input[4 + i])) - mul1(i32::from(input[12 + i]));
        let d = mul1(i32::from(input[4 + i])) + mul2(i32::from(input[12 + i]));
        c[i * 4] = a + d;
        c[i * 4 + 1] = b + cc;
        c[i * 4 + 2] = b - cc;
        c[i * 4 + 3] = a - d;
    }
    for i in 0..4 {
        let dc = c[i] + 4;
        let a = dc + c[8 + i];
        let b = dc - c[8 + i];
        let cc = mul2(c[4 + i]) - mul1(c[12 + i]);
        let d = mul1(c[4 + i]) + mul2(c[12 + i]);
        let row = at + i * stride;
        for (column, value) in [a + d, b + cc, b - cc, a - d].into_iter().enumerate() {
            let slot = &mut buffer[row + column];
            *slot = (i32::from(*slot) + (value >> 3)).clamp(0, 255) as u8;
        }
    }
}

// ----------------------------------------------------------- prediction

/// Work buffers laid out as libwebp's: a row of top samples (with the
/// top-left corner before it and, for luma, four top-right samples
/// after), then the block rows, each with its left sample first.
const LUMA_STRIDE: usize = 32;
const CHROMA_STRIDE: usize = 16;
/// Where the block's first sample sits in its work buffer.
const ORIGIN_LUMA: usize = LUMA_STRIDE + 1;
const ORIGIN_CHROMA: usize = CHROMA_STRIDE + 1;

#[inline(always)]
fn avg3(a: u8, b: u8, c: u8) -> u8 {
    ((u32::from(a) + 2 * u32::from(b) + u32::from(c) + 2) >> 2) as u8
}

#[inline(always)]
fn avg2(a: u8, b: u8) -> u8 {
    ((u32::from(a) + u32::from(b) + 1) >> 1) as u8
}

fn true_motion(buffer: &mut [u8], at: usize, stride: usize, size: usize) {
    let corner = i32::from(buffer[at - stride - 1]);
    for row in 0..size {
        let left = i32::from(buffer[at + row * stride - 1]);
        for column in 0..size {
            let top = i32::from(buffer[at - stride + column]);
            buffer[at + row * stride + column] = (top + left - corner).clamp(0, 255) as u8;
        }
    }
}

fn fill(buffer: &mut [u8], at: usize, stride: usize, size: usize, value: u8) {
    for row in 0..size {
        buffer[at + row * stride..at + row * stride + size].fill(value);
    }
}

/// A 16x16 luma or 8x8 chroma prediction; DC takes the frame edges into
/// account (libwebp's `CheckMode`).
fn predict_block(
    buffer: &mut [u8],
    at: usize,
    stride: usize,
    size: usize,
    mode: u8,
    has_top: bool,
    has_left: bool,
) {
    match mode {
        V_PRED => {
            for row in 0..size {
                buffer.copy_within(at - stride..at - stride + size, at + row * stride);
            }
        }
        H_PRED => {
            for row in 0..size {
                let left = buffer[at + row * stride - 1];
                buffer[at + row * stride..at + row * stride + size].fill(left);
            }
        }
        TM_PRED => true_motion(buffer, at, stride, size),
        _ => {
            let shift = size.trailing_zeros();
            let value = match (has_top, has_left) {
                (true, true) => {
                    let mut sum = size as u32;
                    for index in 0..size {
                        sum += u32::from(buffer[at - stride + index])
                            + u32::from(buffer[at + index * stride - 1]);
                    }
                    (sum >> (shift + 1)) as u8
                }
                (false, true) => {
                    let mut sum = (size >> 1) as u32;
                    for index in 0..size {
                        sum += u32::from(buffer[at + index * stride - 1]);
                    }
                    (sum >> shift) as u8
                }
                (true, false) => {
                    let mut sum = (size >> 1) as u32;
                    for index in 0..size {
                        sum += u32::from(buffer[at - stride + index]);
                    }
                    (sum >> shift) as u8
                }
                (false, false) => 0x80,
            };
            fill(buffer, at, stride, size, value);
        }
    }
}

/// One 4x4 luma prediction at `at` in the luma work buffer.
fn predict_4x4(buffer: &mut [u8], at: usize, mode: u8) {
    let s = LUMA_STRIDE;
    let top = |i: usize| buffer[at - s + i];
    let left = |i: usize| buffer[at + i * s - 1];
    let corner = buffer[at - s - 1];
    let mut out = [[0u8; 4]; 4];
    match mode {
        B_DC_PRED => {
            let mut sum = 4u32;
            for i in 0..4 {
                sum += u32::from(top(i)) + u32::from(left(i));
            }
            out = [[(sum >> 3) as u8; 4]; 4];
        }
        B_TM_PRED => {
            for (y, row) in out.iter_mut().enumerate() {
                for (x, cell) in row.iter_mut().enumerate() {
                    *cell = (i32::from(top(x)) + i32::from(left(y)) - i32::from(corner))
                        .clamp(0, 255) as u8;
                }
            }
        }
        B_VE_PRED => {
            let values = [
                avg3(corner, top(0), top(1)),
                avg3(top(0), top(1), top(2)),
                avg3(top(1), top(2), top(3)),
                avg3(top(2), top(3), top(4)),
            ];
            out = [values; 4];
        }
        B_HE_PRED => {
            let (a, b, c, d, e) = (corner, left(0), left(1), left(2), left(3));
            out[0] = [avg3(a, b, c); 4];
            out[1] = [avg3(b, c, d); 4];
            out[2] = [avg3(c, d, e); 4];
            out[3] = [avg3(d, e, e); 4];
        }
        B_RD_PRED => {
            let (i, j, k, l, x) = (left(0), left(1), left(2), left(3), corner);
            let (a, b, c, d) = (top(0), top(1), top(2), top(3));
            let mut set = |px: usize, py: usize, v: u8| out[py][px] = v;
            set(0, 3, avg3(j, k, l));
            let v = avg3(i, j, k);
            set(1, 3, v);
            set(0, 2, v);
            let v = avg3(x, i, j);
            set(2, 3, v);
            set(1, 2, v);
            set(0, 1, v);
            let v = avg3(a, x, i);
            set(3, 3, v);
            set(2, 2, v);
            set(1, 1, v);
            set(0, 0, v);
            let v = avg3(b, a, x);
            set(3, 2, v);
            set(2, 1, v);
            set(1, 0, v);
            let v = avg3(c, b, a);
            set(3, 1, v);
            set(2, 0, v);
            set(3, 0, avg3(d, c, b));
        }
        B_LD_PRED => {
            let (a, b, c, d) = (top(0), top(1), top(2), top(3));
            let (e, f, g, h) = (top(4), top(5), top(6), top(7));
            let mut set = |px: usize, py: usize, v: u8| out[py][px] = v;
            set(0, 0, avg3(a, b, c));
            let v = avg3(b, c, d);
            set(1, 0, v);
            set(0, 1, v);
            let v = avg3(c, d, e);
            set(2, 0, v);
            set(1, 1, v);
            set(0, 2, v);
            let v = avg3(d, e, f);
            set(3, 0, v);
            set(2, 1, v);
            set(1, 2, v);
            set(0, 3, v);
            let v = avg3(e, f, g);
            set(3, 1, v);
            set(2, 2, v);
            set(1, 3, v);
            let v = avg3(f, g, h);
            set(3, 2, v);
            set(2, 3, v);
            set(3, 3, avg3(g, h, h));
        }
        B_VR_PRED => {
            let (i, j, k, x) = (left(0), left(1), left(2), corner);
            let (a, b, c, d) = (top(0), top(1), top(2), top(3));
            let mut set = |px: usize, py: usize, v: u8| out[py][px] = v;
            let v = avg2(x, a);
            set(0, 0, v);
            set(1, 2, v);
            let v = avg2(a, b);
            set(1, 0, v);
            set(2, 2, v);
            let v = avg2(b, c);
            set(2, 0, v);
            set(3, 2, v);
            set(3, 0, avg2(c, d));
            set(0, 3, avg3(k, j, i));
            set(0, 2, avg3(j, i, x));
            let v = avg3(i, x, a);
            set(0, 1, v);
            set(1, 3, v);
            let v = avg3(x, a, b);
            set(1, 1, v);
            set(2, 3, v);
            let v = avg3(a, b, c);
            set(2, 1, v);
            set(3, 3, v);
            set(3, 1, avg3(b, c, d));
        }
        B_VL_PRED => {
            let (a, b, c, d) = (top(0), top(1), top(2), top(3));
            let (e, f, g, h) = (top(4), top(5), top(6), top(7));
            let mut set = |px: usize, py: usize, v: u8| out[py][px] = v;
            set(0, 0, avg2(a, b));
            let v = avg2(b, c);
            set(1, 0, v);
            set(0, 2, v);
            let v = avg2(c, d);
            set(2, 0, v);
            set(1, 2, v);
            let v = avg2(d, e);
            set(3, 0, v);
            set(2, 2, v);
            set(0, 1, avg3(a, b, c));
            let v = avg3(b, c, d);
            set(1, 1, v);
            set(0, 3, v);
            let v = avg3(c, d, e);
            set(2, 1, v);
            set(1, 3, v);
            let v = avg3(d, e, f);
            set(3, 1, v);
            set(2, 3, v);
            set(3, 2, avg3(e, f, g));
            set(3, 3, avg3(f, g, h));
        }
        B_HD_PRED => {
            let (i, j, k, l, x) = (left(0), left(1), left(2), left(3), corner);
            let (a, b, c) = (top(0), top(1), top(2));
            let mut set = |px: usize, py: usize, v: u8| out[py][px] = v;
            let v = avg2(i, x);
            set(0, 0, v);
            set(2, 1, v);
            let v = avg2(j, i);
            set(0, 1, v);
            set(2, 2, v);
            let v = avg2(k, j);
            set(0, 2, v);
            set(2, 3, v);
            set(0, 3, avg2(l, k));
            set(3, 0, avg3(a, b, c));
            set(2, 0, avg3(x, a, b));
            let v = avg3(i, x, a);
            set(1, 0, v);
            set(3, 1, v);
            let v = avg3(j, i, x);
            set(1, 1, v);
            set(3, 2, v);
            let v = avg3(k, j, i);
            set(1, 2, v);
            set(3, 3, v);
            set(1, 3, avg3(l, k, j));
        }
        _ => {
            // B_HU_PRED
            let (i, j, k, l) = (left(0), left(1), left(2), left(3));
            let mut set = |px: usize, py: usize, v: u8| out[py][px] = v;
            set(0, 0, avg2(i, j));
            let v = avg2(j, k);
            set(2, 0, v);
            set(0, 1, v);
            let v = avg2(k, l);
            set(2, 1, v);
            set(0, 2, v);
            set(1, 0, avg3(i, j, k));
            let v = avg3(j, k, l);
            set(3, 0, v);
            set(1, 1, v);
            let v = avg3(k, l, l);
            set(3, 1, v);
            set(1, 2, v);
            for (px, py) in [(3, 2), (2, 2), (0, 3), (1, 3), (2, 3), (3, 3)] {
                set(px, py, l);
            }
        }
    }
    for (row, values) in out.iter().enumerate() {
        buffer[at + row * s..at + row * s + 4].copy_from_slice(values);
    }
}

// ---------------------------------------------------------- loop filter

#[inline(always)]
fn sclip1(value: i32) -> i32 {
    value.clamp(-128, 127)
}

#[inline(always)]
fn sclip2(value: i32) -> i32 {
    value.clamp(-16, 15)
}

#[inline(always)]
fn clip255(value: i32) -> u8 {
    value.clamp(0, 255) as u8
}

/// The samples across an edge at `at`, `step` apart.
#[inline(always)]
fn sample(p: &[u8], at: usize, step: isize, offset: isize) -> i32 {
    i32::from(p[(at as isize + step * offset) as usize])
}

#[inline(always)]
fn set_sample(p: &mut [u8], at: usize, step: isize, offset: isize, value: u8) {
    p[(at as isize + step * offset) as usize] = value;
}

fn do_filter2(p: &mut [u8], at: usize, step: isize) {
    let (p1, p0, q0, q1) = (
        sample(p, at, step, -2),
        sample(p, at, step, -1),
        sample(p, at, step, 0),
        sample(p, at, step, 1),
    );
    let a = 3 * (q0 - p0) + sclip1(p1 - q1);
    let a1 = sclip2((a + 4) >> 3);
    let a2 = sclip2((a + 3) >> 3);
    set_sample(p, at, step, -1, clip255(p0 + a2));
    set_sample(p, at, step, 0, clip255(q0 - a1));
}

fn needs_filter(p: &[u8], at: usize, step: isize, threshold: i32) -> bool {
    let (p1, p0, q0, q1) = (
        sample(p, at, step, -2),
        sample(p, at, step, -1),
        sample(p, at, step, 0),
        sample(p, at, step, 1),
    );
    4 * (p0 - q0).abs() + (p1 - q1).abs() <= threshold
}

/// Filters `count` positions along an edge: `across` steps over the
/// edge, `along` moves to the next position.
fn simple_edge(p: &mut [u8], at: usize, across: isize, along: isize, count: usize, limit: i32) {
    let threshold = 2 * limit + 1;
    for index in 0..count {
        let position = (at as isize + along * index as isize) as usize;
        if needs_filter(p, position, across, threshold) {
            do_filter2(p, position, across);
        }
    }
}

/// The normal filter at one position, on the eight samples across the
/// edge as locals (libwebp's `FilterLoop26`/`FilterLoop24` arithmetic).
/// Returns whether the samples changed.
#[inline(always)]
fn normal_filter(
    samples: &mut [u8; 8],
    threshold: i32,
    interior: i32,
    hev: i32,
    macroblock_edge: bool,
) -> bool {
    let s = |k: usize| i32::from(samples[k]);
    let (p3, p2, p1, p0, q0, q1, q2, q3) = (s(0), s(1), s(2), s(3), s(4), s(5), s(6), s(7));
    if 4 * (p0 - q0).abs() + (p1 - q1).abs() > threshold {
        return false;
    }
    if (p3 - p2).abs() > interior
        || (p2 - p1).abs() > interior
        || (p1 - p0).abs() > interior
        || (q3 - q2).abs() > interior
        || (q2 - q1).abs() > interior
        || (q1 - q0).abs() > interior
    {
        return false;
    }
    if (p1 - p0).abs() > hev || (q1 - q0).abs() > hev {
        // High edge variance: the two-sample filter.
        let a = 3 * (q0 - p0) + sclip1(p1 - q1);
        let a1 = sclip2((a + 4) >> 3);
        let a2 = sclip2((a + 3) >> 3);
        samples[3] = clip255(p0 + a2);
        samples[4] = clip255(q0 - a1);
    } else if macroblock_edge {
        let a = sclip1(3 * (q0 - p0) + sclip1(p1 - q1));
        let a1 = (27 * a + 63) >> 7;
        let a2 = (18 * a + 63) >> 7;
        let a3 = (9 * a + 63) >> 7;
        samples[1] = clip255(p2 + a3);
        samples[2] = clip255(p1 + a2);
        samples[3] = clip255(p0 + a1);
        samples[4] = clip255(q0 - a1);
        samples[5] = clip255(q1 - a2);
        samples[6] = clip255(q2 - a3);
    } else {
        let a = 3 * (q0 - p0);
        let a1 = sclip2((a + 4) >> 3);
        let a2 = sclip2((a + 3) >> 3);
        let a3 = (a1 + 1) >> 1;
        samples[2] = clip255(p1 + a3);
        samples[3] = clip255(p0 + a2);
        samples[4] = clip255(q0 - a1);
        samples[5] = clip255(q1 - a3);
    }
    true
}

/// Filters `count` positions along an edge with the normal filter:
/// `across` steps over the edge, `along` moves to the next position.
#[allow(clippy::too_many_arguments)]
fn normal_edge(
    p: &mut [u8],
    at: usize,
    across: isize,
    along: isize,
    count: usize,
    limit: i32,
    interior: i32,
    hev: i32,
    macroblock_edge: bool,
) {
    let threshold = 2 * limit + 1;
    let step = across as usize;
    for index in 0..count {
        let position = (at as isize + along * index as isize) as usize;
        let base = position - 4 * step;
        if step == 1 {
            // Across a vertical edge the eight samples are adjacent: one
            // bounds check for the window (image-webp's form).
            let window: &mut [u8; 8] = (&mut p[base..base + 8]).try_into().unwrap();
            normal_filter(window, threshold, interior, hev, macroblock_edge);
        } else {
            let mut samples = [0u8; 8];
            for (k, sample) in samples.iter_mut().enumerate() {
                *sample = p[base + k * step];
            }
            if normal_filter(&mut samples, threshold, interior, hev, macroblock_edge) {
                for (k, sample) in samples.iter().enumerate().skip(1).take(6) {
                    p[base + k * step] = *sample;
                }
            }
        }
    }
}

// ---------------------------------------------------------- the decoder

/// Per macroblock column: the nonzero flags of the blocks along the
/// bottom of the macroblock above (4 luma, 2 + 2 chroma, the Y2 block),
/// and the 4x4 modes along its bottom.
#[derive(Clone, Copy, Default)]
struct TopContext {
    nz_y: [bool; 4],
    nz_u: [bool; 2],
    nz_v: [bool; 2],
    nz_dc: bool,
    modes: [u8; 4],
}

pub(crate) fn decode(
    data: &[u8],
    alpha_chunk: Option<&[u8]>,
    sink: &mut dyn RowSink,
) -> Result<(), RowsError> {
    decode_inner(data, alpha_chunk, sink).map_err(|failure| match failure {
        Failure::Webp(error) => to_rows(error),
        Failure::Rows(error) => error,
    })
}

enum Failure {
    Webp(WebpError),
    Rows(RowsError),
}

impl From<WebpError> for Failure {
    fn from(error: WebpError) -> Self {
        Failure::Webp(error)
    }
}

impl From<RowsError> for Failure {
    fn from(error: RowsError) -> Self {
        Failure::Rows(error)
    }
}

impl From<std::io::Error> for Failure {
    fn from(error: std::io::Error) -> Self {
        Failure::Rows(RowsError::Io(error))
    }
}

fn decode_inner(
    data: &[u8],
    alpha_chunk: Option<&[u8]>,
    sink: &mut dyn RowSink,
) -> Result<(), Failure> {
    if data.len() < 10 {
        return Err(WebpError("lossy data cut short".to_string()).into());
    }
    let tag = u32::from(data[0]) | (u32::from(data[1]) << 8) | (u32::from(data[2]) << 16);
    if tag & 1 != 0 {
        return Err(WebpError("a lossy WebP must hold a key frame".to_string()).into());
    }
    if (tag >> 1) & 7 > 3 {
        return Err(WebpError("unknown VP8 version".to_string()).into());
    }
    let first_size = (tag >> 5) as usize;
    if data[3..6] != [0x9d, 0x01, 0x2a] {
        return Err(WebpError("bad VP8 start code".to_string()).into());
    }
    let width = usize::from(u16::from_le_bytes([data[6], data[7]]) & 0x3fff);
    let height = usize::from(u16::from_le_bytes([data[8], data[9]]) & 0x3fff);
    if width == 0 || height == 0 {
        return Err(WebpError("lossy image dimensions are zero".to_string()).into());
    }
    let body = &data[10..];
    if first_size > body.len() {
        return Err(WebpError("first partition runs past the data".to_string()).into());
    }
    let mut first = BoolDecoder::new(&body[..first_size]);
    let header = read_header(&mut first, width, height)?;
    let partitions = split_partitions(&body[first_size..], header.partition_count)?;
    let mut tokens: Vec<BoolDecoder<'_>> = partitions.into_iter().map(BoolDecoder::new).collect();
    let alpha = match alpha_chunk {
        Some(chunk) => Some(decode_alpha(chunk, width, height)?),
        None => None,
    };
    let color = if alpha.is_some() {
        ColorType::Rgba
    } else {
        ColorType::Rgb
    };
    sink.start(width as u32, height as u32, color)?;
    let mut frame = Frame::new(&header);
    let mut output = Output::new(width, height, alpha);
    for mb_y in 0..header.mb_h {
        frame.begin_row();
        for mb_x in 0..header.mb_w {
            let modes = read_modes(&mut first, &header, &mut frame, mb_x);
            let partition = &mut tokens[mb_y % header.partition_count];
            frame.decode_macroblock(&header, partition, &modes, mb_x, mb_y);
            if partition.eof {
                return Err(WebpError("lossy data cut short".to_string()).into());
            }
        }
        if first.eof {
            return Err(WebpError("lossy header data cut short".to_string()).into());
        }
        frame.save_top_samples();
        if header.filter_type > 0 {
            frame.filter_row(&header, mb_y);
        }
        if mb_y > 0 {
            output.emit(&frame, mb_y - 1, sink)?;
        }
        if mb_y + 1 == header.mb_h {
            output.emit_last(&frame, mb_y, sink)?;
        }
        frame.shift();
    }
    Ok(())
}

fn split_partitions(rest: &[u8], count: usize) -> Result<Vec<&[u8]>, WebpError> {
    let sizes_length = 3 * (count - 1);
    if rest.len() < sizes_length {
        return fail("partition sizes cut short");
    }
    let mut start = sizes_length;
    let mut left = rest.len() - sizes_length;
    let mut out = Vec::with_capacity(count);
    for index in 0..count - 1 {
        let at = index * 3;
        let mut size = usize::from(rest[at])
            | (usize::from(rest[at + 1]) << 8)
            | (usize::from(rest[at + 2]) << 16);
        if size > left {
            size = left;
        }
        out.push(&rest[start..start + size]);
        start += size;
        left -= size;
    }
    if start >= rest.len() {
        return fail("token partition missing");
    }
    out.push(&rest[start..]);
    Ok(out)
}

/// A macroblock's prediction modes and segment.
struct Modes {
    segment: usize,
    skip: bool,
    is_i4x4: bool,
    luma: u8,
    sub: [u8; 16],
    chroma: u8,
}

fn read_modes(
    first: &mut BoolDecoder<'_>,
    header: &Header,
    frame: &mut Frame,
    mb_x: usize,
) -> Modes {
    let segment = if header.update_map {
        if !first.bit(header.segment_probs[0]) {
            usize::from(first.bit(header.segment_probs[1]))
        } else {
            2 + usize::from(first.bit(header.segment_probs[2]))
        }
    } else {
        0
    };
    let skip = match header.skip_prob {
        Some(probability) => first.bit(probability),
        None => false,
    };
    let is_i4x4 = !first.bit(145);
    let mut luma = DC_PRED;
    let mut sub = [B_DC_PRED; 16];
    if !is_i4x4 {
        luma = if first.bit(156) {
            if first.bit(128) { TM_PRED } else { H_PRED }
        } else if first.bit(163) {
            V_PRED
        } else {
            DC_PRED
        };
        let context = as_bmode(luma);
        frame.top[mb_x].modes = [context; 4];
        frame.left_modes = [context; 4];
    } else {
        for y in 0..4 {
            let mut left = frame.left_modes[y];
            for x in 0..4 {
                let above = frame.top[mb_x].modes[x];
                let base = (usize::from(above) * 10 + usize::from(left)) * 9;
                let probs = &BMODE_PROBS[base..base + 9];
                let mut node = 0usize;
                let mode = loop {
                    let branch = BMODE_TREE[node + usize::from(first.bit(probs[node >> 1]))];
                    if branch <= 0 {
                        break (-branch) as u8;
                    }
                    node = branch as usize;
                };
                sub[y * 4 + x] = mode;
                frame.top[mb_x].modes[x] = mode;
                left = mode;
            }
            frame.left_modes[y] = left;
        }
    }
    let chroma = if !first.bit(142) {
        DC_PRED
    } else if !first.bit(114) {
        V_PRED
    } else if first.bit(183) {
        TM_PRED
    } else {
        H_PRED
    };
    Modes {
        segment,
        skip,
        is_i4x4,
        luma,
        sub,
        chroma,
    }
}

/// The reconstruction state: the window of two macroblock rows (the
/// previous, already filtered, and the current), the unfiltered bottom
/// samples of the row above for prediction, and the contexts.
struct Frame {
    mb_w: usize,
    luma_stride: usize,
    chroma_stride: usize,
    /// 32 luma rows: rows 0..16 the previous macroblock row, 16..32 the
    /// current.
    y: Vec<u8>,
    /// 17 chroma rows each: row 0 the last chroma row before the previous
    /// macroblock row (kept for the upsampler), 1..9 the previous row,
    /// 9..17 the current.
    u: Vec<u8>,
    v: Vec<u8>,
    /// Unfiltered bottom samples of the macroblock row above.
    top_y: Vec<u8>,
    top_u: Vec<u8>,
    top_v: Vec<u8>,
    top: Vec<TopContext>,
    left_nz_y: [bool; 4],
    left_nz_u: [bool; 2],
    left_nz_v: [bool; 2],
    left_nz_dc: bool,
    left_modes: [u8; 4],
    filters: Vec<FilterInfo>,
    has_top: bool,
}

impl Frame {
    fn new(header: &Header) -> Frame {
        let luma_stride = header.mb_w * 16;
        let chroma_stride = header.mb_w * 8;
        Frame {
            mb_w: header.mb_w,
            luma_stride,
            chroma_stride,
            y: vec![0; luma_stride * 32],
            u: vec![0; chroma_stride * 17],
            v: vec![0; chroma_stride * 17],
            top_y: vec![127; luma_stride + 4],
            top_u: vec![127; chroma_stride],
            top_v: vec![127; chroma_stride],
            top: vec![TopContext::default(); header.mb_w],
            left_nz_y: [false; 4],
            left_nz_u: [false; 2],
            left_nz_v: [false; 2],
            left_nz_dc: false,
            left_modes: [B_DC_PRED; 4],
            filters: vec![FilterInfo::default(); header.mb_w],
            has_top: false,
        }
    }

    fn begin_row(&mut self) {
        self.left_nz_y = [false; 4];
        self.left_nz_u = [false; 2];
        self.left_nz_v = [false; 2];
        self.left_nz_dc = false;
        self.left_modes = [B_DC_PRED; 4];
    }

    fn decode_macroblock(
        &mut self,
        header: &Header,
        tokens: &mut BoolDecoder<'_>,
        modes: &Modes,
        mb_x: usize,
        mb_y: usize,
    ) {
        let quant = header.quant[modes.segment];
        let mut coefficients = [0i16; 384];
        let mut any_nonzero = false;
        let top = &mut self.top[mb_x];
        if !modes.skip {
            let probs = &header.probs;
            let (first_position, luma_kind) = if modes.is_i4x4 {
                (0, 3)
            } else {
                let mut dc = [0i16; 16];
                let context = usize::from(top.nz_dc) + usize::from(self.left_nz_dc);
                let n = read_coefficients(tokens, probs, 1, context, quant.y2, 0, &mut dc);
                let nonzero = n > 0;
                top.nz_dc = nonzero;
                self.left_nz_dc = nonzero;
                if n > 1 {
                    inverse_wht(&dc, &mut coefficients);
                } else {
                    let dc0 = ((i32::from(dc[0]) + 3) >> 3) as i16;
                    for block in 0..16 {
                        coefficients[block * 16] = dc0;
                    }
                }
                (1, 0)
            };
            for y in 0..4 {
                let mut left = self.left_nz_y[y];
                for x in 0..4 {
                    let context = usize::from(left) + usize::from(top.nz_y[x]);
                    let block = &mut coefficients[(y * 4 + x) * 16..(y * 4 + x) * 16 + 16];
                    let n = read_coefficients(
                        tokens,
                        probs,
                        luma_kind,
                        context,
                        quant.y1,
                        first_position,
                        block,
                    );
                    left = n > first_position;
                    top.nz_y[x] = left;
                    // libwebp's rule: a coefficient past the DC, or a
                    // nonzero DC (from the Y2 block, for 16x16 modes).
                    if n > 1 || block[0] != 0 {
                        any_nonzero = true;
                    }
                }
                self.left_nz_y[y] = left;
            }
            for (plane, base) in [(0usize, 256usize), (1, 320)] {
                for y in 0..2 {
                    let mut left = if plane == 0 {
                        self.left_nz_u[y]
                    } else {
                        self.left_nz_v[y]
                    };
                    for x in 0..2 {
                        let above = if plane == 0 { top.nz_u[x] } else { top.nz_v[x] };
                        let context = usize::from(left) + usize::from(above);
                        let at = base + (y * 2 + x) * 16;
                        let n = read_coefficients(
                            tokens,
                            probs,
                            2,
                            context,
                            quant.uv,
                            0,
                            &mut coefficients[at..at + 16],
                        );
                        left = n > 0;
                        if plane == 0 {
                            top.nz_u[x] = left;
                        } else {
                            top.nz_v[x] = left;
                        }
                        if n > 1 || coefficients[at] != 0 {
                            any_nonzero = true;
                        }
                    }
                    if plane == 0 {
                        self.left_nz_u[y] = left;
                    } else {
                        self.left_nz_v[y] = left;
                    }
                }
            }
        } else {
            top.nz_y = [false; 4];
            top.nz_u = [false; 2];
            top.nz_v = [false; 2];
            self.left_nz_y = [false; 4];
            self.left_nz_u = [false; 2];
            self.left_nz_v = [false; 2];
            if !modes.is_i4x4 {
                top.nz_dc = false;
                self.left_nz_dc = false;
            }
        }
        if header.filter_type > 0 {
            let mut info = header.filters[modes.segment][usize::from(modes.is_i4x4)];
            info.inner |= any_nonzero;
            self.filters[mb_x] = info;
        }
        self.reconstruct(modes, &coefficients, mb_x, mb_y);
    }

    /// Predicts and adds the residuals of one macroblock in work buffers
    /// built from its neighbors, then places it in the window.
    fn reconstruct(&mut self, modes: &Modes, coefficients: &[i16; 384], mb_x: usize, mb_y: usize) {
        let mut luma = [0u8; LUMA_STRIDE * 17];
        let has_top = mb_y > 0;
        let has_left = mb_x > 0;
        // Top row, corner, and top-right.
        let corner = if !has_top {
            127
        } else if !has_left {
            129
        } else {
            self.top_y[mb_x * 16 - 1]
        };
        luma[0] = corner;
        luma[1..17].copy_from_slice(&self.top_y[mb_x * 16..mb_x * 16 + 16]);
        if has_top {
            if mb_x + 1 < self.mb_w {
                luma[17..21].copy_from_slice(&self.top_y[(mb_x + 1) * 16..(mb_x + 1) * 16 + 4]);
            } else {
                luma[17..21].fill(self.top_y[mb_x * 16 + 15]);
            }
        } else {
            luma[17..21].fill(127);
        }
        for row in 0..16 {
            luma[(row + 1) * LUMA_STRIDE] = if has_left {
                self.y[(16 + row) * self.luma_stride + mb_x * 16 - 1]
            } else {
                129
            };
        }
        if modes.is_i4x4 {
            // The macroblock's top-right samples stand in for the
            // right-hand column of 4x4 blocks below the first row.
            for row in [4, 8, 12] {
                let (head, tail) = luma.split_at_mut(row * LUMA_STRIDE + 17);
                tail[..4].copy_from_slice(&head[17..21]);
            }
            for block in 0..16 {
                let at = ORIGIN_LUMA + (block / 4) * 4 * LUMA_STRIDE + (block % 4) * 4;
                predict_4x4(&mut luma, at, modes.sub[block]);
                add_idct(
                    &coefficients[block * 16..block * 16 + 16],
                    &mut luma,
                    at,
                    LUMA_STRIDE,
                );
            }
        } else {
            predict_block(
                &mut luma,
                ORIGIN_LUMA,
                LUMA_STRIDE,
                16,
                modes.luma,
                has_top,
                has_left,
            );
            for block in 0..16 {
                let at = ORIGIN_LUMA + (block / 4) * 4 * LUMA_STRIDE + (block % 4) * 4;
                add_idct(
                    &coefficients[block * 16..block * 16 + 16],
                    &mut luma,
                    at,
                    LUMA_STRIDE,
                );
            }
        }
        for row in 0..16 {
            let target = (16 + row) * self.luma_stride + mb_x * 16;
            let source = ORIGIN_LUMA + row * LUMA_STRIDE;
            self.y[target..target + 16].copy_from_slice(&luma[source..source + 16]);
        }
        for (plane, base) in [(0usize, 256usize), (1, 320)] {
            let mut work = [0u8; CHROMA_STRIDE * 9];
            let (top_samples, window) = if plane == 0 {
                (&self.top_u, &self.u)
            } else {
                (&self.top_v, &self.v)
            };
            work[0] = if !has_top {
                127
            } else if !has_left {
                129
            } else {
                top_samples[mb_x * 8 - 1]
            };
            work[1..9].copy_from_slice(&top_samples[mb_x * 8..mb_x * 8 + 8]);
            for row in 0..8 {
                work[(row + 1) * CHROMA_STRIDE] = if has_left {
                    window[(9 + row) * self.chroma_stride + mb_x * 8 - 1]
                } else {
                    129
                };
            }
            predict_block(
                &mut work,
                ORIGIN_CHROMA,
                CHROMA_STRIDE,
                8,
                modes.chroma,
                has_top,
                has_left,
            );
            for block in 0..4 {
                let at = ORIGIN_CHROMA + (block / 2) * 4 * CHROMA_STRIDE + (block % 2) * 4;
                let start = base + block * 16;
                add_idct(
                    &coefficients[start..start + 16],
                    &mut work,
                    at,
                    CHROMA_STRIDE,
                );
            }
            let window = if plane == 0 { &mut self.u } else { &mut self.v };
            for row in 0..8 {
                let target = (9 + row) * self.chroma_stride + mb_x * 8;
                let source = ORIGIN_CHROMA + row * CHROMA_STRIDE;
                window[target..target + 8].copy_from_slice(&work[source..source + 8]);
            }
        }
    }

    /// Keeps the unfiltered bottom row of the current macroblock row for
    /// the next row's prediction.
    fn save_top_samples(&mut self) {
        let luma_start = 31 * self.luma_stride;
        self.top_y[..self.luma_stride]
            .copy_from_slice(&self.y[luma_start..luma_start + self.luma_stride]);
        let chroma_start = 16 * self.chroma_stride;
        self.top_u
            .copy_from_slice(&self.u[chroma_start..chroma_start + self.chroma_stride]);
        self.top_v
            .copy_from_slice(&self.v[chroma_start..chroma_start + self.chroma_stride]);
        self.has_top = true;
    }

    /// Filters the current macroblock row in raster order (libwebp's
    /// `DoFilter`): left edge, inner vertical edges, top edge, inner
    /// horizontal edges; luma, then chroma for the normal filter.
    fn filter_row(&mut self, header: &Header, mb_y: usize) {
        let ls = self.luma_stride;
        let cs = self.chroma_stride;
        for mb_x in 0..self.mb_w {
            let info = self.filters[mb_x];
            if info.limit == 0 {
                continue;
            }
            let luma_at = 16 * ls + mb_x * 16;
            let (step_row, step_col) = (ls as isize, 1isize);
            if header.filter_type == 1 {
                if mb_x > 0 {
                    simple_edge(&mut self.y, luma_at, step_col, step_row, 16, info.limit + 4);
                }
                if info.inner {
                    for column in [4, 8, 12] {
                        simple_edge(
                            &mut self.y,
                            luma_at + column,
                            step_col,
                            step_row,
                            16,
                            info.limit,
                        );
                    }
                }
                if mb_y > 0 {
                    simple_edge(&mut self.y, luma_at, step_row, step_col, 16, info.limit + 4);
                }
                if info.inner {
                    for row in [4, 8, 12] {
                        simple_edge(
                            &mut self.y,
                            luma_at + row * ls,
                            step_row,
                            step_col,
                            16,
                            info.limit,
                        );
                    }
                }
                continue;
            }
            let (limit, interior, hev) = (info.limit, info.interior, info.hev_threshold);
            if mb_x > 0 {
                normal_edge(
                    &mut self.y,
                    luma_at,
                    step_col,
                    step_row,
                    16,
                    limit + 4,
                    interior,
                    hev,
                    true,
                );
            }
            if info.inner {
                for column in [4, 8, 12] {
                    normal_edge(
                        &mut self.y,
                        luma_at + column,
                        step_col,
                        step_row,
                        16,
                        limit,
                        interior,
                        hev,
                        false,
                    );
                }
            }
            if mb_y > 0 {
                normal_edge(
                    &mut self.y,
                    luma_at,
                    step_row,
                    step_col,
                    16,
                    limit + 4,
                    interior,
                    hev,
                    true,
                );
            }
            if info.inner {
                for row in [4, 8, 12] {
                    normal_edge(
                        &mut self.y,
                        luma_at + row * ls,
                        step_row,
                        step_col,
                        16,
                        limit,
                        interior,
                        hev,
                        false,
                    );
                }
            }
            let chroma_at = 9 * cs + mb_x * 8;
            let (crow, ccol) = (cs as isize, 1isize);
            for plane in [&mut self.u, &mut self.v] {
                if mb_x > 0 {
                    normal_edge(
                        plane,
                        chroma_at,
                        ccol,
                        crow,
                        8,
                        limit + 4,
                        interior,
                        hev,
                        true,
                    );
                }
            }
            if info.inner {
                for plane in [&mut self.u, &mut self.v] {
                    normal_edge(
                        plane,
                        chroma_at + 4,
                        ccol,
                        crow,
                        8,
                        limit,
                        interior,
                        hev,
                        false,
                    );
                }
            }
            for plane in [&mut self.u, &mut self.v] {
                if mb_y > 0 {
                    normal_edge(
                        plane,
                        chroma_at,
                        crow,
                        ccol,
                        8,
                        limit + 4,
                        interior,
                        hev,
                        true,
                    );
                }
            }
            if info.inner {
                for plane in [&mut self.u, &mut self.v] {
                    normal_edge(
                        plane,
                        chroma_at + 4 * cs,
                        crow,
                        ccol,
                        8,
                        limit,
                        interior,
                        hev,
                        false,
                    );
                }
            }
        }
    }

    /// Moves the current row into the previous row's place.
    fn shift(&mut self) {
        let ls = self.luma_stride;
        self.y.copy_within(16 * ls..32 * ls, 0);
        let cs = self.chroma_stride;
        for plane in [&mut self.u, &mut self.v] {
            plane.copy_within(8 * cs..9 * cs, 0);
            plane.copy_within(9 * cs..17 * cs, cs);
        }
    }
}

/// Turns final macroblock rows into RGB or RGBA rows.
struct Output {
    width: usize,
    height: usize,
    chroma_height: usize,
    alpha: Option<Vec<u8>>,
    row: Vec<u8>,
    emitted: usize,
}

impl Output {
    fn new(width: usize, height: usize, alpha: Option<Vec<u8>>) -> Output {
        let channels = if alpha.is_some() { 4 } else { 3 };
        Output {
            width,
            height,
            chroma_height: height.div_ceil(2),
            alpha,
            row: vec![0; width * channels],
            emitted: 0,
        }
    }

    /// Emits the sixteen rows of macroblock row `mb_row`, which sits in
    /// the window's previous slot while the next row is being built.
    fn emit(
        &mut self,
        frame: &Frame,
        mb_row: usize,
        sink: &mut dyn RowSink,
    ) -> Result<(), Failure> {
        self.emit_rows(frame, mb_row, mb_row as isize, 0, sink)
    }

    /// Emits the last macroblock row, which is in the current slot.
    fn emit_last(
        &mut self,
        frame: &Frame,
        mb_row: usize,
        sink: &mut dyn RowSink,
    ) -> Result<(), Failure> {
        self.emit_rows(frame, mb_row, mb_row as isize - 1, 16, sink)
    }

    /// `previous_mb` is the macroblock row held in the window's previous
    /// slot (-1 before the first); window chroma row `k` is then chroma
    /// row `8 * previous_mb - 1 + k`. `slot_offset` is 0 for rows in the
    /// previous slot and 16 for the current.
    fn emit_rows(
        &mut self,
        frame: &Frame,
        mb_row: usize,
        previous_mb: isize,
        slot_offset: usize,
        sink: &mut dyn RowSink,
    ) -> Result<(), Failure> {
        let chroma_base = 8 * previous_mb - 1;
        let first = mb_row * 16;
        let last = (first + 16).min(self.height);
        for r in first..last {
            let luma_row = slot_offset + (r - first);
            let luma =
                &frame.y[luma_row * frame.luma_stride..luma_row * frame.luma_stride + self.width];
            let (near, far) = if r == 0 {
                (0, 0)
            } else if r % 2 == 1 {
                ((r - 1) / 2, r.div_ceil(2).min(self.chroma_height - 1))
            } else {
                (r / 2, r / 2 - 1)
            };
            let window = |c: usize| (c as isize - chroma_base) as usize;
            let (near_at, far_at) = (
                window(near) * frame.chroma_stride,
                window(far) * frame.chroma_stride,
            );
            let chroma_width = self.width.div_ceil(2);
            let channels = if self.alpha.is_some() { 4 } else { 3 };
            upsample_convert(
                luma,
                [
                    &frame.u[near_at..near_at + chroma_width],
                    &frame.u[far_at..far_at + chroma_width],
                ],
                [
                    &frame.v[near_at..near_at + chroma_width],
                    &frame.v[far_at..far_at + chroma_width],
                ],
                channels,
                &mut self.row,
            );
            if let Some(alpha) = &self.alpha {
                let line = &alpha[r * self.width..(r + 1) * self.width];
                for (cell, a) in self.row.chunks_exact_mut(4).zip(line) {
                    cell[3] = *a;
                }
            }
            sink.row(&self.row)?;
            self.emitted += 1;
        }
        Ok(())
    }
}

/// Converts one pixel into `cell`.
#[inline(always)]
fn put_pixel(cell: &mut [u8], y: u8, u: u32, v: u32) {
    let (y, u, v) = (i32::from(y), u as i32, v as i32);
    cell[0] = yuv_to_r(y, v);
    cell[1] = yuv_to_g(y, u, v);
    cell[2] = yuv_to_b(y, u);
}

/// The fancy upsampler and color conversion in one pass over a row, a
/// pixel pair at a time: for each pair, the U and V of both pixels (as
/// `upsample_row` computes them) and the pixels themselves.
fn upsample_convert(luma: &[u8], u: [&[u8]; 2], v: [&[u8]; 2], channels: usize, out: &mut [u8]) {
    let length = luma.len();
    let (un, uf, vn, vf) = (u[0], u[1], v[0], v[1]);
    let at = |i: usize, s: &[u8]| u32::from(s[i]);
    let first_u = (3 * at(0, un) + at(0, uf) + 2) >> 2;
    let first_v = (3 * at(0, vn) + at(0, vf) + 2) >> 2;
    put_pixel(&mut out[..channels], luma[0], first_u, first_v);
    let last_pair = (length - 1) >> 1;
    let mut u_left = (at(0, un), at(0, uf));
    let mut v_left = (at(0, vn), at(0, vf));
    for x in 1..=last_pair {
        let u_here = (at(x, un), at(x, uf));
        let v_here = (at(x, vn), at(x, vf));
        let blend = |left: (u32, u32), here: (u32, u32)| {
            let average = left.0 + here.0 + left.1 + here.1 + 8;
            let diagonal_12 = (average + 2 * (here.0 + left.1)) >> 3;
            let diagonal_03 = (average + 2 * (left.0 + here.1)) >> 3;
            ((diagonal_12 + left.0) >> 1, (diagonal_03 + here.0) >> 1)
        };
        let (u_odd, u_even) = blend(u_left, u_here);
        let (v_odd, v_even) = blend(v_left, v_here);
        let odd = 2 * x - 1;
        put_pixel(
            &mut out[odd * channels..odd * channels + 3],
            luma[odd],
            u_odd,
            v_odd,
        );
        let even = 2 * x;
        put_pixel(
            &mut out[even * channels..even * channels + 3],
            luma[even],
            u_even,
            v_even,
        );
        u_left = u_here;
        v_left = v_here;
    }
    if length % 2 == 0 {
        let last = length - 1;
        let u_last = (3 * u_left.0 + u_left.1 + 2) >> 2;
        let v_last = (3 * v_left.0 + v_left.1 + 2) >> 2;
        put_pixel(
            &mut out[last * channels..last * channels + 3],
            luma[last],
            u_last,
            v_last,
        );
    }
}

#[inline(always)]
fn mult_hi(value: i32, coefficient: i32) -> i32 {
    (value * coefficient) >> 8
}

#[inline(always)]
fn clip8(value: i32) -> u8 {
    const MASK: i32 = (256 << 6) - 1;
    if value & !MASK == 0 {
        (value >> 6) as u8
    } else if value < 0 {
        0
    } else {
        255
    }
}

#[inline(always)]
fn yuv_to_r(y: i32, v: i32) -> u8 {
    clip8(mult_hi(y, 19_077) + mult_hi(v, 26_149) - 14_234)
}

#[inline(always)]
fn yuv_to_g(y: i32, u: i32, v: i32) -> u8 {
    clip8(mult_hi(y, 19_077) - mult_hi(u, 6_419) - mult_hi(v, 13_320) + 8_708)
}

#[inline(always)]
fn yuv_to_b(y: i32, u: i32) -> u8 {
    clip8(mult_hi(y, 19_077) + mult_hi(u, 33_050) - 17_685)
}

/// The `ALPH` chunk: a header byte, then raw or VP8L-compressed alpha,
/// unfiltered by its prediction method.
fn decode_alpha(chunk: &[u8], width: usize, height: usize) -> Result<Vec<u8>, WebpError> {
    let Some(&flags) = chunk.first() else {
        return fail("empty alpha chunk");
    };
    let compression = flags & 3;
    let filtering = (flags >> 2) & 3;
    let mut plane = match compression {
        0 => {
            if chunk.len() < 1 + width * height {
                return fail("alpha data cut short");
            }
            chunk[1..1 + width * height].to_vec()
        }
        1 => {
            let mut reader = lossless::BitReader::new(&chunk[1..]);
            let pixels = lossless::decode_stream(&mut reader, width, height)?;
            pixels.iter().map(|argb| (argb >> 8) as u8).collect()
        }
        _ => return fail("unknown alpha compression"),
    };
    for y in 0..height {
        let (before, rest) = plane.split_at_mut(y * width);
        let row = &mut rest[..width];
        let previous = if y == 0 {
            None
        } else {
            Some(&before[(y - 1) * width..])
        };
        match (filtering, previous) {
            (0, _) => {}
            (1, previous) | (_, previous @ None) => {
                let mut prediction = previous.map_or(0, |line| line[0]);
                for value in row.iter_mut() {
                    *value = value.wrapping_add(prediction);
                    prediction = *value;
                }
            }
            (2, Some(above)) => {
                for (value, top) in row.iter_mut().zip(above) {
                    *value = value.wrapping_add(*top);
                }
            }
            (_, Some(above)) => {
                let mut top_left = above[0];
                let mut left = above[0];
                for (x, value) in row.iter_mut().enumerate() {
                    let top = above[x];
                    let prediction = (i32::from(left) + i32::from(top) - i32::from(top_left))
                        .clamp(0, 255) as u8;
                    left = value.wrapping_add(prediction);
                    top_left = top;
                    *value = left;
                }
            }
        }
    }
    Ok(plane)
}
