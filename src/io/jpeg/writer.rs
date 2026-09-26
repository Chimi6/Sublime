//! Encodes baseline JPEG the way libjpeg does: the IJG quality scaling of
//! the standard quantization tables (so a quality of 75 means what every
//! tool's 75 means), the accurate integer FDCT (`jfdctint`), 4:2:0 chroma
//! below quality 90 and 4:4:4 from 90 up, and the standard Huffman
//! tables, which let the encoder stream: rows arrive through a
//! `RowSink`, each band of eight or sixteen rows is transformed and
//! written, and nothing else is held. Alpha is flattened onto white.

use std::io::{self, Write};

use crate::image::{ColorType, Image};
use crate::io::png::RowSink;

/// The quality the converter uses when none is given.
pub const DEFAULT_QUALITY: u8 = 85;

const STD_LUMA_QUANT: [u8; 64] = [
    16, 11, 10, 16, 24, 40, 51, 61, 12, 12, 14, 19, 26, 58, 60, 55, 14, 13, 16, 24, 40, 57, 69, 56,
    14, 17, 22, 29, 51, 87, 80, 62, 18, 22, 37, 56, 68, 109, 103, 77, 24, 35, 55, 64, 81, 104, 113,
    92, 49, 64, 78, 87, 103, 121, 120, 101, 72, 92, 95, 98, 112, 100, 103, 99,
];
const STD_CHROMA_QUANT: [u8; 64] = [
    17, 18, 24, 47, 99, 99, 99, 99, 18, 21, 26, 66, 99, 99, 99, 99, 24, 26, 56, 99, 99, 99, 99, 99,
    47, 66, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
];
const DC_LUMA_BITS: [u8; 16] = [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0];
const DC_LUMA_VALUES: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
const DC_CHROMA_BITS: [u8; 16] = [0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0];
const DC_CHROMA_VALUES: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
const AC_LUMA_BITS: [u8; 16] = [0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 0x7d];
const AC_LUMA_VALUES: [u8; 162] = [
    0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07,
    0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xa1, 0x08, 0x23, 0x42, 0xb1, 0xc1, 0x15, 0x52, 0xd1, 0xf0,
    0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0a, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x25, 0x26, 0x27, 0x28,
    0x29, 0x2a, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49,
    0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69,
    0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89,
    0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7,
    0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5,
    0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xe1, 0xe2,
    0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8,
    0xf9, 0xfa,
];
const AC_CHROMA_BITS: [u8; 16] = [0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 0x77];
const AC_CHROMA_VALUES: [u8; 162] = [
    0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71,
    0x13, 0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91, 0xa1, 0xb1, 0xc1, 0x09, 0x23, 0x33, 0x52, 0xf0,
    0x15, 0x62, 0x72, 0xd1, 0x0a, 0x16, 0x24, 0x34, 0xe1, 0x25, 0xf1, 0x17, 0x18, 0x19, 0x1a, 0x26,
    0x27, 0x28, 0x29, 0x2a, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48,
    0x49, 0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68,
    0x69, 0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87,
    0x88, 0x89, 0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5,
    0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3,
    0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda,
    0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8,
    0xf9, 0xfa,
];
/// Zigzag position to natural position.
const ZIGZAG_TO_NATURAL: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];
/// Natural position to zigzag position.
const NATURAL_TO_ZIGZAG: [usize; 64] = [
    0, 1, 5, 6, 14, 15, 27, 28, 2, 4, 7, 13, 16, 26, 29, 42, 3, 8, 12, 17, 25, 30, 41, 43, 9, 11,
    18, 24, 31, 40, 44, 53, 10, 19, 23, 32, 39, 45, 52, 54, 20, 22, 33, 38, 46, 51, 55, 60, 21, 34,
    37, 47, 50, 56, 59, 61, 35, 36, 48, 49, 57, 58, 62, 63,
];

/// A Huffman code per symbol: `(length, code)`.
struct Codes {
    table: [(u8, u16); 256],
}

impl Codes {
    fn build(bits: &[u8; 16], values: &[u8]) -> Codes {
        let mut table = [(0u8, 0u16); 256];
        let mut code = 0u16;
        let mut index = 0usize;
        for (length, count) in bits.iter().enumerate() {
            for _ in 0..*count {
                table[usize::from(values[index])] = (length as u8 + 1, code);
                code += 1;
                index += 1;
            }
            code <<= 1;
        }
        Codes { table }
    }
}

/// `ceil(2^32 / (8 q))` per quantizer: with coefficients under 2^15 in
/// magnitude the product's error stays under one part in 2^17, below
/// 1/2040, so the shifted product is the exact rounded quotient.
fn reciprocals(quant: &[u16; 64]) -> [u32; 64] {
    let mut table = [0u32; 64];
    for (slot, q) in table.iter_mut().zip(quant) {
        let divisor = u64::from(*q) << 3;
        // At most 2^29, so a 32-bit operand: the compiler multiplies
        // two zero-extended 32-bit lanes with one vector instruction.
        *slot = (1u64 << 32).div_ceil(divisor) as u32;
    }
    table
}

/// The IJG scaling of a base table by quality.
fn scaled(base: &[u8; 64], quality: u8) -> [u16; 64] {
    let quality = u32::from(quality.clamp(1, 100));
    let scale = if quality < 50 {
        5000 / quality
    } else {
        200 - quality * 2
    };
    let mut table = [0u16; 64];
    for (slot, value) in table.iter_mut().zip(base) {
        *slot = ((u32::from(*value) * scale + 50) / 100).clamp(1, 255) as u16;
    }
    table
}

struct BitWriter {
    out: Vec<u8>,
    /// Bits gather here, most recent lowest; flushed a byte at a time
    /// only when more than 32 have gathered, so a symbol and its value
    /// bits go in as one shift.
    buffer: u64,
    count: u32,
}

impl BitWriter {
    fn new() -> BitWriter {
        BitWriter {
            out: Vec::with_capacity(1 << 16),
            buffer: 0,
            count: 0,
        }
    }

    /// Appends `length` bits (at most 32) of `code`.
    #[inline]
    fn put(&mut self, code: u32, length: u32) {
        self.buffer =
            (self.buffer << length) | u64::from(code & ((1u32 << length).wrapping_sub(1)));
        self.count += length;
        if self.count > 32 {
            self.drain_bytes();
        }
    }

    /// Moves whole bytes out of the buffer, stuffing a zero after 0xFF:
    /// four at a time while none of them is 0xFF, one at a time otherwise.
    #[inline]
    fn drain_bytes(&mut self) {
        while self.count >= 32 {
            let word = (self.buffer >> (self.count - 32)) as u32;
            // A byte is 0xFF exactly when its complement is zero, and a
            // zero byte is what the borrow trick finds.
            let inverted = !word;
            let has_ff = (inverted.wrapping_sub(0x0101_0101) & !inverted & 0x8080_8080) != 0;
            if has_ff {
                break;
            }
            self.out.extend_from_slice(&word.to_be_bytes());
            self.count -= 32;
        }
        while self.count >= 8 {
            let byte = (self.buffer >> (self.count - 8)) as u8;
            self.out.push(byte);
            if byte == 0xFF {
                self.out.push(0);
            }
            self.count -= 8;
        }
        self.buffer &= (1u64 << self.count) - 1;
    }

    fn flush_bytes(&mut self, sink: &mut dyn Write) -> io::Result<()> {
        self.drain_bytes();
        sink.write_all(&self.out)?;
        self.out.clear();
        Ok(())
    }

    /// Pads the last byte with ones, as the standard requires.
    fn finish(&mut self, sink: &mut dyn Write) -> io::Result<()> {
        self.drain_bytes();
        if self.count > 0 {
            let pad = 8 - self.count;
            self.put((1 << pad) - 1, pad);
        }
        self.flush_bytes(sink)
    }
}

/// Writes a JPEG from rows as a `RowSink`.
pub struct JpegRows<'a> {
    sink: &'a mut dyn Write,
    quality: u8,
    width: usize,
    height: usize,
    color: ColorType,
    subsampled: bool,
    band_rows: usize,
    /// Rows of the band in hand, as Y, Cb, Cr (or Y alone) samples,
    /// each row the image width rounded up to the MCU.
    band: Vec<Vec<u8>>,
    rows_in_band: usize,
    rows_taken: usize,
    luma_quant: [u16; 64],
    chroma_quant: [u16; 64],
    /// Reciprocals of the quantizers times eight (the FDCT's scale), so
    /// quantizing is a multiply and a shift, not a division.
    luma_reciprocal: [u32; 64],
    chroma_reciprocal: [u32; 64],
    predictions: [i32; 3],
    padded_width: usize,
    codes: [Codes; 4],
    bits: BitWriter,
    started: bool,
}

impl<'a> JpegRows<'a> {
    pub fn new(sink: &'a mut dyn Write, quality: u8) -> JpegRows<'a> {
        JpegRows {
            sink,
            quality: quality.clamp(1, 100),
            width: 0,
            height: 0,
            color: ColorType::Rgb,
            subsampled: false,
            band_rows: 8,
            band: Vec::new(),
            rows_in_band: 0,
            rows_taken: 0,
            luma_quant: [1; 64],
            chroma_quant: [1; 64],
            luma_reciprocal: [0; 64],
            chroma_reciprocal: [0; 64],
            predictions: [0; 3],
            padded_width: 0,
            codes: [
                Codes::build(&DC_LUMA_BITS, &DC_LUMA_VALUES),
                Codes::build(&AC_LUMA_BITS, &AC_LUMA_VALUES),
                Codes::build(&DC_CHROMA_BITS, &DC_CHROMA_VALUES),
                Codes::build(&AC_CHROMA_BITS, &AC_CHROMA_VALUES),
            ],
            bits: BitWriter::new(),
            started: false,
        }
    }

    fn gray(&self) -> bool {
        matches!(self.color, ColorType::Gray | ColorType::GrayAlpha)
    }

    fn headers(&mut self) -> io::Result<()> {
        let mut head: Vec<u8> = Vec::with_capacity(1024);
        head.extend_from_slice(&[0xFF, 0xD8]);
        // JFIF APP0.
        head.extend_from_slice(&[0xFF, 0xE0, 0, 16]);
        head.extend_from_slice(b"JFIF\0");
        head.extend_from_slice(&[1, 1, 0, 0, 1, 0, 1, 0, 0]);
        // Quantization tables in zigzag order.
        let tables: Vec<(u8, [u16; 64])> = if self.gray() {
            vec![(0, self.luma_quant)]
        } else {
            vec![(0, self.luma_quant), (1, self.chroma_quant)]
        };
        for (id, table) in &tables {
            head.extend_from_slice(&[0xFF, 0xDB, 0, 67, *id]);
            let mut zigzag = [0u8; 64];
            for natural in 0..64 {
                zigzag[NATURAL_TO_ZIGZAG[natural]] = table[natural] as u8;
            }
            head.extend_from_slice(&zigzag);
        }
        // Frame header.
        let components: u8 = if self.gray() { 1 } else { 3 };
        let length = 8 + 3 * u16::from(components);
        head.extend_from_slice(&[0xFF, 0xC0]);
        head.extend_from_slice(&length.to_be_bytes());
        head.push(8);
        head.extend_from_slice(&(self.height as u16).to_be_bytes());
        head.extend_from_slice(&(self.width as u16).to_be_bytes());
        head.push(components);
        let luma_sampling = if self.subsampled { 0x22 } else { 0x11 };
        head.extend_from_slice(&[1, luma_sampling, 0]);
        if !self.gray() {
            head.extend_from_slice(&[2, 0x11, 1, 3, 0x11, 1]);
        }
        // Huffman tables.
        let mut huffman: Vec<(u8, &[u8; 16], &[u8])> = vec![
            (0x00, &DC_LUMA_BITS, &DC_LUMA_VALUES),
            (0x10, &AC_LUMA_BITS, &AC_LUMA_VALUES),
        ];
        if !self.gray() {
            huffman.push((0x01, &DC_CHROMA_BITS, &DC_CHROMA_VALUES));
            huffman.push((0x11, &AC_CHROMA_BITS, &AC_CHROMA_VALUES));
        }
        for (id, bits, values) in huffman {
            let length = (3 + 16 + values.len()) as u16;
            head.extend_from_slice(&[0xFF, 0xC4]);
            head.extend_from_slice(&length.to_be_bytes());
            head.push(id);
            head.extend_from_slice(bits);
            head.extend_from_slice(values);
        }
        // Scan header.
        let length = 6 + 2 * u16::from(components);
        head.extend_from_slice(&[0xFF, 0xDA]);
        head.extend_from_slice(&length.to_be_bytes());
        head.push(components);
        head.extend_from_slice(&[1, 0x00]);
        if !self.gray() {
            head.extend_from_slice(&[2, 0x11, 3, 0x11]);
        }
        head.extend_from_slice(&[0, 63, 0]);
        self.sink.write_all(&head)
    }

    /// Converts one hub row into the band's Y, Cb, Cr rows (or Y alone),
    /// flattening alpha onto white.
    fn take_row(&mut self, pixels: &[u8]) {
        let channels = self.color.channels();
        let y_index = self.rows_in_band;
        let width = self.width;
        if self.color == ColorType::Rgb {
            // The common case as one pass over the row.
            let start = y_index * self.padded_width;
            let (luma, rest) = self.band.split_at_mut(1);
            let (blue_diff, red_diff) = rest.split_at_mut(1);
            let luma = &mut luma[0][start..start + width];
            let blue_diff = &mut blue_diff[0][start..start + width];
            let red_diff = &mut red_diff[0][start..start + width];
            for (((pixel, y), cb), cr) in pixels
                .chunks_exact(3)
                .zip(luma.iter_mut())
                .zip(blue_diff.iter_mut())
                .zip(red_diff.iter_mut())
            {
                let (yy, cbb, crr) = rgb_to_ycbcr(pixel[0], pixel[1], pixel[2]);
                *y = yy;
                *cb = cbb;
                *cr = crr;
            }
            for plane in self.band.iter_mut() {
                let row = &mut plane[start..start + self.padded_width];
                let last = row[width - 1];
                for value in &mut row[width..] {
                    *value = last;
                }
            }
            self.rows_in_band += 1;
            return;
        }
        for x in 0..width {
            let cell = &pixels[x * channels..(x + 1) * channels];
            let (r, g, b) = match self.color {
                ColorType::Gray => (cell[0], cell[0], cell[0]),
                ColorType::GrayAlpha => {
                    let v = flatten(cell[0], cell[1]);
                    (v, v, v)
                }
                ColorType::Rgb => (cell[0], cell[1], cell[2]),
                ColorType::Rgba => (
                    flatten(cell[0], cell[3]),
                    flatten(cell[1], cell[3]),
                    flatten(cell[2], cell[3]),
                ),
            };
            if self.gray() {
                self.band[0][y_index * self.padded_width + x] = r;
            } else {
                let (yy, cb, cr) = rgb_to_ycbcr(r, g, b);
                self.band[0][y_index * self.padded_width + x] = yy;
                self.band[1][y_index * self.padded_width + x] = cb;
                self.band[2][y_index * self.padded_width + x] = cr;
            }
        }
        // Pad the right edge by replication, as libjpeg does.
        for plane in self.band.iter_mut() {
            let row = &mut plane[y_index * self.padded_width..(y_index + 1) * self.padded_width];
            let last = row[width - 1];
            for value in &mut row[width..] {
                *value = last;
            }
        }
        self.rows_in_band += 1;
    }

    /// Encodes the band in hand: one MCU row.
    fn encode_band(&mut self) -> io::Result<()> {
        // Pad the bottom by replicating the last row taken.
        let rows = self.band_rows;
        for plane in self.band.iter_mut() {
            let stride = self.padded_width;
            let last = self.rows_in_band - 1;
            for y in self.rows_in_band..rows {
                let (before, after) = plane.split_at_mut(y * stride);
                after[..stride].copy_from_slice(&before[last * stride..(last + 1) * stride]);
            }
        }
        let mcu = if self.subsampled { 16 } else { 8 };
        let mcus_wide = self.padded_width / mcu;
        let mut samples = [0i32; 64];
        let mut coefficients = [0i32; 64];
        let mut bits = std::mem::replace(&mut self.bits, BitWriter::new());
        for mcu_x in 0..mcus_wide {
            // Luma blocks.
            let luma_blocks = if self.subsampled { 2 } else { 1 };
            for by in 0..luma_blocks {
                for bx in 0..luma_blocks {
                    let x0 = mcu_x * mcu + bx * 8;
                    let y0 = by * 8;
                    for row in 0..8 {
                        let start = (y0 + row) * self.padded_width + x0;
                        let source = &self.band[0][start..start + 8];
                        for (sample, byte) in samples[row * 8..row * 8 + 8].iter_mut().zip(source) {
                            *sample = i32::from(*byte) - 128;
                        }
                    }
                    fdct(&samples, &mut coefficients);
                    let codes = (&self.codes[0], &self.codes[1]);
                    encode_block(
                        &mut bits,
                        &coefficients,
                        &self.luma_quant,
                        &self.luma_reciprocal,
                        &mut self.predictions[0],
                        codes,
                    );
                }
            }
            if !self.gray() {
                for component in 1..3 {
                    let x0 = mcu_x * mcu;
                    let plane = &self.band[component];
                    if self.subsampled {
                        for row in 0..8 {
                            let upper_start = (row * 2) * self.padded_width + x0;
                            let upper = &plane[upper_start..upper_start + 16];
                            let lower_start = upper_start + self.padded_width;
                            let lower = &plane[lower_start..lower_start + 16];
                            let out = &mut samples[row * 8..row * 8 + 8];
                            for (column, ((a, b), target)) in upper
                                .chunks_exact(2)
                                .zip(lower.chunks_exact(2))
                                .zip(out.iter_mut())
                                .enumerate()
                            {
                                let sum = i32::from(a[0])
                                    + i32::from(a[1])
                                    + i32::from(b[0])
                                    + i32::from(b[1]);
                                // libjpeg's alternating bias keeps the average unbiased.
                                let bias = 1 + (column as i32 & 1);
                                *target = ((sum + bias) >> 2) - 128;
                            }
                        }
                    } else {
                        for row in 0..8 {
                            let start = row * self.padded_width + x0;
                            let source = &plane[start..start + 8];
                            for (sample, byte) in
                                samples[row * 8..row * 8 + 8].iter_mut().zip(source)
                            {
                                *sample = i32::from(*byte) - 128;
                            }
                        }
                    }
                    fdct(&samples, &mut coefficients);
                    let codes = (&self.codes[2], &self.codes[3]);
                    encode_block(
                        &mut bits,
                        &coefficients,
                        &self.chroma_quant,
                        &self.chroma_reciprocal,
                        &mut self.predictions[component],
                        codes,
                    );
                }
            }
        }
        bits.flush_bytes(self.sink)?;
        self.bits = bits;
        self.rows_in_band = 0;
        Ok(())
    }
}

/// A sample over alpha, onto white.
#[inline]
fn flatten(value: u8, alpha: u8) -> u8 {
    let (value, alpha) = (u32::from(value), u32::from(alpha));
    ((value * alpha + 255 * (255 - alpha) + 127) / 255) as u8
}

/// libjpeg's fixed-point RGB to YCbCr (`jccolor.c`).
#[inline]
fn rgb_to_ycbcr(r: u8, g: u8, b: u8) -> (u8, u8, u8) {
    const ONE_HALF: i32 = 1 << 15;
    const OFFSET: i32 = 128 << 16;
    let (r, g, b) = (i32::from(r), i32::from(g), i32::from(b));
    let y = (19_595 * r + 38_470 * g + 7_471 * b + ONE_HALF) >> 16;
    let cb = (-11_059 * r - 21_709 * g + 32_768 * b + OFFSET + ONE_HALF - 1) >> 16;
    let cr = (32_768 * r - 27_439 * g - 5_329 * b + OFFSET + ONE_HALF - 1) >> 16;
    (y as u8, cb as u8, cr as u8)
}

const CONST_BITS: i32 = 13;
const PASS1_BITS: i32 = 2;
const FIX_0_298631336: i32 = 2446;
const FIX_0_390180644: i32 = 3196;
const FIX_0_541196100: i32 = 4433;
const FIX_0_765366865: i32 = 6270;
const FIX_0_899976223: i32 = 7373;
const FIX_1_175875602: i32 = 9633;
const FIX_1_501321110: i32 = 12299;
const FIX_1_847759065: i32 = 15137;
const FIX_1_961570560: i32 = 16069;
const FIX_2_053119869: i32 = 16819;
const FIX_2_562915447: i32 = 20995;
const FIX_3_072711026: i32 = 25172;

#[inline(always)]
fn descale(value: i32, n: i32) -> i32 {
    (value + (1 << (n - 1))) >> n
}

/// The IJG accurate integer FDCT (`jpeg_fdct_islow`): the output is the
/// DCT scaled up by eight, which the quantizer allows for.
fn fdct(input: &[i32; 64], out: &mut [i32; 64]) {
    let mut data = *input;
    // Pass 1: rows.
    for row in 0..8 {
        let d = &mut data[row * 8..row * 8 + 8];
        let tmp0 = d[0] + d[7];
        let tmp7 = d[0] - d[7];
        let tmp1 = d[1] + d[6];
        let tmp6 = d[1] - d[6];
        let tmp2 = d[2] + d[5];
        let tmp5 = d[2] - d[5];
        let tmp3 = d[3] + d[4];
        let tmp4 = d[3] - d[4];
        let tmp10 = tmp0 + tmp3;
        let tmp13 = tmp0 - tmp3;
        let tmp11 = tmp1 + tmp2;
        let tmp12 = tmp1 - tmp2;
        d[0] = (tmp10 + tmp11) << PASS1_BITS;
        d[4] = (tmp10 - tmp11) << PASS1_BITS;
        let z1 = (tmp12 + tmp13) * FIX_0_541196100;
        d[2] = descale(z1 + tmp13 * FIX_0_765366865, CONST_BITS - PASS1_BITS);
        d[6] = descale(z1 + tmp12 * (-FIX_1_847759065), CONST_BITS - PASS1_BITS);
        let z1 = tmp4 + tmp7;
        let z2 = tmp5 + tmp6;
        let z3 = tmp4 + tmp6;
        let z4 = tmp5 + tmp7;
        let z5 = (z3 + z4) * FIX_1_175875602;
        let tmp4 = tmp4 * FIX_0_298631336;
        let tmp5 = tmp5 * FIX_2_053119869;
        let tmp6 = tmp6 * FIX_3_072711026;
        let tmp7 = tmp7 * FIX_1_501321110;
        let z1 = z1 * (-FIX_0_899976223);
        let z2 = z2 * (-FIX_2_562915447);
        let z3 = z3 * (-FIX_1_961570560) + z5;
        let z4 = z4 * (-FIX_0_390180644) + z5;
        d[7] = descale(tmp4 + z1 + z3, CONST_BITS - PASS1_BITS);
        d[5] = descale(tmp5 + z2 + z4, CONST_BITS - PASS1_BITS);
        d[3] = descale(tmp6 + z2 + z3, CONST_BITS - PASS1_BITS);
        d[1] = descale(tmp7 + z1 + z4, CONST_BITS - PASS1_BITS);
    }
    // Pass 2: columns.
    for column in 0..8 {
        let c = |row: usize| data[row * 8 + column];
        let tmp0 = c(0) + c(7);
        let tmp7 = c(0) - c(7);
        let tmp1 = c(1) + c(6);
        let tmp6 = c(1) - c(6);
        let tmp2 = c(2) + c(5);
        let tmp5 = c(2) - c(5);
        let tmp3 = c(3) + c(4);
        let tmp4 = c(3) - c(4);
        let tmp10 = tmp0 + tmp3;
        let tmp13 = tmp0 - tmp3;
        let tmp11 = tmp1 + tmp2;
        let tmp12 = tmp1 - tmp2;
        out[column] = descale(tmp10 + tmp11, PASS1_BITS);
        out[4 * 8 + column] = descale(tmp10 - tmp11, PASS1_BITS);
        let z1 = (tmp12 + tmp13) * FIX_0_541196100;
        out[2 * 8 + column] = descale(z1 + tmp13 * FIX_0_765366865, CONST_BITS + PASS1_BITS);
        out[6 * 8 + column] = descale(z1 + tmp12 * (-FIX_1_847759065), CONST_BITS + PASS1_BITS);
        let z1 = tmp4 + tmp7;
        let z2 = tmp5 + tmp6;
        let z3 = tmp4 + tmp6;
        let z4 = tmp5 + tmp7;
        let z5 = (z3 + z4) * FIX_1_175875602;
        let tmp4 = tmp4 * FIX_0_298631336;
        let tmp5 = tmp5 * FIX_2_053119869;
        let tmp6 = tmp6 * FIX_3_072711026;
        let tmp7 = tmp7 * FIX_1_501321110;
        let z1 = z1 * (-FIX_0_899976223);
        let z2 = z2 * (-FIX_2_562915447);
        let z3 = z3 * (-FIX_1_961570560) + z5;
        let z4 = z4 * (-FIX_0_390180644) + z5;
        out[7 * 8 + column] = descale(tmp4 + z1 + z3, CONST_BITS + PASS1_BITS);
        out[5 * 8 + column] = descale(tmp5 + z2 + z4, CONST_BITS + PASS1_BITS);
        out[3 * 8 + column] = descale(tmp6 + z2 + z3, CONST_BITS + PASS1_BITS);
        out[8 + column] = descale(tmp7 + z1 + z4, CONST_BITS + PASS1_BITS);
    }
}

/// Quantizes and Huffman-codes one block.
fn encode_block(
    bits: &mut BitWriter,
    coefficients: &[i32; 64],
    quant: &[u16; 64],
    reciprocal: &[u32; 64],
    prediction: &mut i32,
    codes: (&Codes, &Codes),
) {
    let (dc_codes, ac_codes) = codes;
    // Quantize in natural order (a loop the compiler vectorizes); the
    // FDCT output is scaled by eight, and the division is a multiply by
    // the reciprocal. The coding loop below gathers in zigzag order.
    let mut quantized = [0i32; 64];
    for k in 0..64 {
        let half = u32::from(quant[k]) << 2;
        let value = coefficients[k];
        let magnitude = (u64::from(value.unsigned_abs() + half) * u64::from(reciprocal[k])) >> 32;
        let signed = magnitude as i32;
        quantized[k] = if value < 0 { -signed } else { signed };
    }
    let diff = quantized[0] - *prediction;
    *prediction = quantized[0];
    let (size, value_bits) = magnitude(diff);
    let (length, code) = dc_codes.table[size as usize];
    // The code and its value bits go in as one write (at most 27 bits).
    bits.put(
        (u32::from(code) << size) | value_bits,
        u32::from(length) + size,
    );
    let mut run = 0u32;
    for position in &ZIGZAG_TO_NATURAL[1..] {
        let value = quantized[*position];
        if value == 0 {
            run += 1;
            continue;
        }
        while run > 15 {
            let (length, code) = ac_codes.table[0xF0];
            bits.put(u32::from(code), u32::from(length));
            run -= 16;
        }
        let (size, value_bits) = magnitude(value);
        let (length, code) = ac_codes.table[((run << 4) | size) as usize];
        bits.put(
            (u32::from(code) << size) | value_bits,
            u32::from(length) + size,
        );
        run = 0;
    }
    if run > 0 {
        let (length, code) = ac_codes.table[0x00];
        bits.put(u32::from(code), u32::from(length));
    }
}

/// A coefficient's size category and the bits that encode it.
#[inline]
fn magnitude(value: i32) -> (u32, u32) {
    if value == 0 {
        return (0, 0);
    }
    let size = 32 - value.unsigned_abs().leading_zeros();
    let bits = if value < 0 {
        (value - 1) as u32 & ((1 << size) - 1)
    } else {
        value as u32
    };
    (size, bits)
}

impl RowSink for JpegRows<'_> {
    fn start(&mut self, width: u32, height: u32, color: ColorType) -> io::Result<()> {
        if width > 65_535 || height > 65_535 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "JPEG holds at most 65535 by 65535 pixels",
            ));
        }
        self.width = width as usize;
        self.height = height as usize;
        self.color = color;
        self.subsampled = !self.gray() && self.quality < 90;
        self.band_rows = if self.subsampled { 16 } else { 8 };
        let mcu = self.band_rows;
        self.padded_width = self.width.div_ceil(mcu) * mcu;
        let planes = if self.gray() { 1 } else { 3 };
        self.band = (0..planes)
            .map(|_| vec![0u8; self.band_rows * self.padded_width])
            .collect();
        self.luma_quant = scaled(&STD_LUMA_QUANT, self.quality);
        self.chroma_quant = scaled(&STD_CHROMA_QUANT, self.quality);
        self.luma_reciprocal = reciprocals(&self.luma_quant);
        self.chroma_reciprocal = reciprocals(&self.chroma_quant);
        self.headers()?;
        self.bits = BitWriter::new();
        self.started = true;
        Ok(())
    }

    fn row(&mut self, pixels: &[u8]) -> io::Result<()> {
        if !self.started {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "row before start",
            ));
        }
        self.take_row(pixels);
        self.rows_taken += 1;
        if self.rows_in_band == self.band_rows || self.rows_taken == self.height {
            self.encode_band()?;
        }
        if self.rows_taken == self.height {
            let mut bits = std::mem::replace(&mut self.bits, BitWriter::new());
            bits.finish(self.sink)?;
            self.sink.write_all(&[0xFF, 0xD9])?;
            self.sink.flush()?;
        }
        Ok(())
    }
}

/// Writes a whole image.
pub fn write_jpeg(image: &Image, sink: &mut dyn Write, quality: u8) -> io::Result<()> {
    let mut rows = JpegRows::new(sink, quality);
    rows.start(image.width, image.height, image.color)?;
    for y in 0..image.height {
        rows.row(image.row(y))?;
    }
    Ok(())
}
