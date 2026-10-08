//! Encodes baseline JPEG the way libjpeg does: the IJG quality scaling of
//! the standard quantization tables (so a quality of 75 means what every
//! tool's 75 means), the accurate integer FDCT (`jfdctint`), 4:2:0 chroma
//! below quality 90 and 4:4:4 from 90 up, and the standard Huffman
//! tables, which let the encoder stream: rows arrive through a
//! `RowSink`, each band of eight or sixteen rows is transformed and
//! written, and nothing else is held. Alpha is flattened onto white.

use std::io::{self, Write};

use crate::image::{ColorType, Image};
use crate::image::{flatten, luma_of};
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

    /// Pads the last byte with ones (a restart interval's end), the bytes
    /// left in `out`.
    fn pad(&mut self) {
        self.drain_bytes();
        if self.count > 0 {
            let pad = 8 - self.count;
            self.put((1 << pad) - 1, pad);
        }
        self.drain_bytes();
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

/// Bands encoded at once with restart intervals: enough for every thread
/// to have a few.
const RESTART_BATCH: usize = 16;

/// What encoding a band needs, apart from where it goes: shared by the
/// threads that encode bands at once.
struct Encoder {
    quality: u8,
    width: usize,
    height: usize,
    color: ColorType,
    subsampled: bool,
    /// Rows arrive as YCbCr 4:2:0 planes (`ycbcr_row`), not RGB.
    planar: bool,
    /// The band holds 4:2:0 chroma at half resolution: from the planes
    /// given, or from RGB row pairs as they come (padding included).
    halved: bool,
    band_rows: usize,
    luma_quant: [u16; 64],
    chroma_quant: [u16; 64],
    /// Reciprocals of the quantizers times eight (the FDCT's scale), so
    /// quantizing is a multiply and a shift, not a division.
    luma_reciprocal: [u32; 64],
    chroma_reciprocal: [u32; 64],
    padded_width: usize,
    codes: [Codes; 4],
}

impl Encoder {
    fn is_gray(&self) -> bool {
        matches!(self.color, ColorType::Gray | ColorType::GrayAlpha)
    }
}

/// A band of rows taken: its planes, how many rows are in, and the
/// buffer its bytes encode into.
struct TakenBand {
    planes: Vec<Vec<u8>>,
    rows: usize,
    out: Vec<u8>,
}

/// Writes a JPEG from rows as a `RowSink`.
pub struct JpegRows<'a> {
    sink: &'a mut dyn Write,
    encoder: Encoder,
    /// Rows of the band in hand, as Y, Cb, Cr (or Y alone) samples,
    /// each row the image width rounded up to the MCU.
    band: Vec<Vec<u8>>,
    rows_in_band: usize,
    rows_taken: usize,
    predictions: [i32; 3],
    bits: BitWriter,
    started: bool,
    icc_profile: Option<Vec<u8>>,
    exif: Option<Vec<u8>>,
    /// A restart interval a band (a megapixel and up): bands then encode
    /// on several threads at once, each from fresh predictions.
    restart: bool,
    /// Bands taken and waiting to encode, and spare buffers for more.
    taken: Vec<TakenBand>,
    spare: Vec<Vec<Vec<u8>>>,
    /// Encoded bands' buffers, written and kept for the next batch.
    spare_out: Vec<Vec<u8>>,
    /// Bands written, for the restart markers' numbers.
    bands_written: usize,
    /// The last two RGB rows, padded, for 4:2:0 chroma from RGB: each
    /// chroma row from a pair's 2x2 sums.
    pair: Vec<u8>,
}

impl<'a> JpegRows<'a> {
    pub fn new(sink: &'a mut dyn Write, quality: u8) -> JpegRows<'a> {
        JpegRows {
            sink,
            encoder: Encoder {
                quality: quality.clamp(1, 100),
                width: 0,
                height: 0,
                color: ColorType::Rgb,
                subsampled: false,
                planar: false,
                halved: false,
                band_rows: 8,
                luma_quant: [1; 64],
                chroma_quant: [1; 64],
                luma_reciprocal: [0; 64],
                chroma_reciprocal: [0; 64],
                padded_width: 0,
                codes: [
                    Codes::build(&DC_LUMA_BITS, &DC_LUMA_VALUES),
                    Codes::build(&AC_LUMA_BITS, &AC_LUMA_VALUES),
                    Codes::build(&DC_CHROMA_BITS, &DC_CHROMA_VALUES),
                    Codes::build(&AC_CHROMA_BITS, &AC_CHROMA_VALUES),
                ],
            },
            band: Vec::new(),
            rows_in_band: 0,
            rows_taken: 0,
            predictions: [0; 3],
            bits: BitWriter::new(),
            started: false,
            icc_profile: None,
            exif: None,
            restart: false,
            taken: Vec::new(),
            spare: Vec::new(),
            spare_out: Vec::new(),
            bands_written: 0,
            pair: Vec::new(),
        }
    }

    fn gray(&self) -> bool {
        self.encoder.is_gray()
    }

    fn headers(&mut self) -> io::Result<()> {
        let mut head: Vec<u8> = Vec::with_capacity(1024);
        head.extend_from_slice(&[0xFF, 0xD8]);
        // JFIF APP0.
        head.extend_from_slice(&[0xFF, 0xE0, 0, 16]);
        head.extend_from_slice(b"JFIF\0");
        head.extend_from_slice(&[1, 1, 0, 0, 1, 0, 1, 0, 0]);
        // Exif in APP1, when it fits one segment.
        if let Some(exif) = self.exif.take() {
            head.extend_from_slice(&[0xFF, 0xE1]);
            head.extend_from_slice(&((exif.len() + 8) as u16).to_be_bytes());
            head.extend_from_slice(b"Exif\0\0");
            head.extend_from_slice(&exif);
        }
        // The ICC profile in APP2 segments, numbered from one.
        if let Some(profile) = self.icc_profile.take() {
            let parts: Vec<&[u8]> = profile.chunks(65_519).collect();
            for (index, part) in parts.iter().enumerate() {
                head.extend_from_slice(&[0xFF, 0xE2]);
                head.extend_from_slice(&((part.len() + 16) as u16).to_be_bytes());
                head.extend_from_slice(b"ICC_PROFILE\0");
                head.extend_from_slice(&[index as u8 + 1, parts.len() as u8]);
                head.extend_from_slice(part);
            }
        }
        // Quantization tables in zigzag order.
        let tables: Vec<(u8, [u16; 64])> = if self.gray() {
            vec![(0, self.encoder.luma_quant)]
        } else {
            vec![(0, self.encoder.luma_quant), (1, self.encoder.chroma_quant)]
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
        head.extend_from_slice(&(self.encoder.height as u16).to_be_bytes());
        head.extend_from_slice(&(self.encoder.width as u16).to_be_bytes());
        head.push(components);
        let luma_sampling = if self.encoder.subsampled { 0x22 } else { 0x11 };
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
        if self.restart {
            let interval = (self.encoder.padded_width / self.encoder.band_rows) as u16;
            head.extend_from_slice(&[0xFF, 0xDD, 0, 4]);
            head.extend_from_slice(&interval.to_be_bytes());
        }
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
        let channels = self.encoder.color.channels();
        let y_index = self.rows_in_band;
        let width = self.encoder.width;
        if self.encoder.subsampled {
            let padded = self.encoder.padded_width;
            let luma = &mut self.band[0][y_index * padded..(y_index + 1) * padded];
            let (upper, lower) = self.pair.split_at_mut(padded * 3);
            let rgb = if y_index % 2 == 0 { upper } else { lower };
            if self.encoder.color == ColorType::Rgb {
                rgb[..width * 3].copy_from_slice(&pixels[..width * 3]);
            } else {
                for (target, cell) in rgb.chunks_exact_mut(3).zip(pixels.chunks_exact(channels)) {
                    target[0] = flatten(cell[0], cell[3]);
                    target[1] = flatten(cell[1], cell[3]);
                    target[2] = flatten(cell[2], cell[3]);
                }
            }
            for (y, pixel) in luma[..width].iter_mut().zip(rgb.chunks_exact(3)) {
                *y = luma_of(pixel[0], pixel[1], pixel[2]);
            }
            // Pad the right edge by replication, as libjpeg does.
            let last = luma[width - 1];
            for value in &mut luma[width..] {
                *value = last;
            }
            let last_pixel = [rgb[width * 3 - 3], rgb[width * 3 - 2], rgb[width * 3 - 1]];
            for pixel in rgb[width * 3..].chunks_exact_mut(3) {
                pixel.copy_from_slice(&last_pixel);
            }
            if y_index % 2 == 1 {
                self.chroma_row(y_index / 2, false);
            }
            self.rows_in_band += 1;
            return;
        }
        if self.encoder.color == ColorType::Rgb {
            // The common case as one pass over the row.
            let start = y_index * self.encoder.padded_width;
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
                let row = &mut plane[start..start + self.encoder.padded_width];
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
            let (r, g, b) = match self.encoder.color {
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
                self.band[0][y_index * self.encoder.padded_width + x] = r;
            } else {
                let (yy, cb, cr) = rgb_to_ycbcr(r, g, b);
                self.band[0][y_index * self.encoder.padded_width + x] = yy;
                self.band[1][y_index * self.encoder.padded_width + x] = cb;
                self.band[2][y_index * self.encoder.padded_width + x] = cr;
            }
        }
        // Pad the right edge by replication, as libjpeg does.
        for plane in self.band.iter_mut() {
            let row = &mut plane
                [y_index * self.encoder.padded_width..(y_index + 1) * self.encoder.padded_width];
            let last = row[width - 1];
            for value in &mut row[width..] {
                *value = last;
            }
        }
        self.rows_in_band += 1;
    }

    /// Chroma row `row` of the band from the RGB row pair in hand, or
    /// (`alone`) from the last row taken twice, as the rows past the
    /// image repeat it.
    fn chroma_row(&mut self, row: usize, alone: bool) {
        let half = self.encoder.padded_width / 2;
        let stride = self.encoder.padded_width * 3;
        let last = (self.rows_in_band % 2) * stride;
        let (upper, lower) = if alone {
            (
                &self.pair[last..last + stride],
                &self.pair[last..last + stride],
            )
        } else {
            (&self.pair[..stride], &self.pair[stride..])
        };
        let (blue_plane, red_plane) = self.band[1..].split_at_mut(1);
        let blue = &mut blue_plane[0][row * half..(row + 1) * half];
        let red = &mut red_plane[0][row * half..(row + 1) * half];
        for (((blue, red), a), b) in blue
            .iter_mut()
            .zip(red.iter_mut())
            .zip(upper.chunks_exact(6))
            .zip(lower.chunks_exact(6))
        {
            // Each chroma sample from the sum of a 2x2 block's RGB: the
            // conversion is linear, so this is the average of the four
            // pixels' chroma, rounded once (and held to a byte).
            let red_sum = i32::from(a[0]) + i32::from(a[3]) + i32::from(b[0]) + i32::from(b[3]);
            let green = i32::from(a[1]) + i32::from(a[4]) + i32::from(b[1]) + i32::from(b[4]);
            let blue_sum = i32::from(a[2]) + i32::from(a[5]) + i32::from(b[2]) + i32::from(b[5]);
            let cb = (-11_059 * red_sum - 21_709 * green + 32_768 * blue_sum + (1 << 17)) >> 18;
            let cr = (32_768 * red_sum - 27_439 * green - 5_329 * blue_sum + (1 << 17)) >> 18;
            *blue = (cb + 128).clamp(0, 255) as u8;
            *red = (cr + 128).clamp(0, 255) as u8;
        }
    }

    /// Encodes the band in hand: written at once, or with restart
    /// intervals kept to encode a batch of bands on several threads.
    fn encode_band(&mut self) -> io::Result<()> {
        if self.encoder.subsampled && !self.encoder.planar {
            // The chroma rows below the last pair: from the last row
            // repeated, as the padded rows below it are.
            let first = self.rows_in_band / 2;
            if first < self.encoder.band_rows / 2 {
                self.rows_in_band -= 1;
                self.chroma_row(first, true);
                for row in first + 1..self.encoder.band_rows / 2 {
                    let half = self.encoder.padded_width / 2;
                    for plane in &mut self.band[1..] {
                        plane.copy_within(first * half..(first + 1) * half, row * half);
                    }
                }
                self.rows_in_band += 1;
            }
        }
        if self.restart {
            let spare = self.spare.pop().unwrap_or_else(|| {
                self.band
                    .iter()
                    .map(|plane| vec![0u8; plane.len()])
                    .collect()
            });
            let planes = std::mem::replace(&mut self.band, spare);
            let mut out = self.spare_out.pop().unwrap_or_default();
            out.clear();
            self.taken.push(TakenBand {
                planes,
                rows: self.rows_in_band,
                out,
            });
            self.rows_in_band = 0;
            let last = self.rows_taken == self.encoder.height;
            if self.taken.len() >= RESTART_BATCH || last {
                self.encode_taken(last)?;
            }
            return Ok(());
        }
        let mut bits = std::mem::replace(&mut self.bits, BitWriter::new());
        encode_band(
            &self.encoder,
            &mut self.band,
            self.rows_in_band,
            &mut bits,
            &mut self.predictions,
        );
        bits.flush_bytes(self.sink)?;
        self.bits = bits;
        self.rows_in_band = 0;
        Ok(())
    }

    /// Encodes the bands taken, each its own restart interval, on as many
    /// threads as there are (one where none), and writes them in order
    /// with their restart markers (none after the image's last band).
    fn encode_taken(&mut self, last: bool) -> io::Result<()> {
        let mut taken = std::mem::take(&mut self.taken);
        let encoder = &self.encoder;
        let encode = |band: &mut TakenBand| {
            let mut bits = BitWriter {
                out: std::mem::take(&mut band.out),
                buffer: 0,
                count: 0,
            };
            let mut predictions = [0i32; 3];
            encode_band(
                encoder,
                &mut band.planes,
                band.rows,
                &mut bits,
                &mut predictions,
            );
            bits.pad();
            band.out = bits.out;
        };
        let threads = std::thread::available_parallelism()
            .map_or(1, |count| count.get())
            .min(taken.len());
        if threads <= 1 {
            taken.iter_mut().for_each(encode);
        } else {
            let chunk = taken.len().div_ceil(threads);
            std::thread::scope(|scope| {
                for bands in taken.chunks_mut(chunk) {
                    scope.spawn(|| bands.iter_mut().for_each(&encode));
                }
            });
        }
        let count = taken.len();
        for (index, band) in taken.iter().enumerate() {
            self.sink.write_all(&band.out)?;
            if !(last && index + 1 == count) {
                self.sink
                    .write_all(&[0xFF, 0xD0 + (self.bands_written % 8) as u8])?;
            }
            self.bands_written += 1;
        }
        for band in taken {
            self.spare.push(band.planes);
            self.spare_out.push(band.out);
        }
        Ok(())
    }
}

/// Encodes a band of rows (its last rows padded first by repeating the
/// last row taken) into `bits`, each block's DC from `predictions`.
fn encode_band(
    encoder: &Encoder,
    band: &mut [Vec<u8>],
    rows_in_band: usize,
    bits: &mut BitWriter,
    predictions: &mut [i32; 3],
) {
    // Pad the bottom by replicating the last row taken (chroma from RGB
    // comes padded).
    let rows = encoder.band_rows;
    for (index, plane) in band.iter_mut().enumerate() {
        if index > 0 && encoder.halved && !encoder.planar {
            break;
        }
        // Planar chroma holds half the band's rows.
        let (rows, taken) = if encoder.halved && index > 0 {
            (rows / 2, rows_in_band.div_ceil(2))
        } else {
            (rows, rows_in_band)
        };
        let stride = plane.len() / rows;
        let last = taken - 1;
        for y in taken..rows {
            let (before, after) = plane.split_at_mut(y * stride);
            after[..stride].copy_from_slice(&before[last * stride..(last + 1) * stride]);
        }
    }
    let mcu = if encoder.subsampled { 16 } else { 8 };
    let mcus_wide = encoder.padded_width / mcu;
    let mut samples = [0i32; 64];
    let mut coefficients = [0i32; 64];
    for mcu_x in 0..mcus_wide {
        // Luma blocks.
        let luma_blocks = if encoder.subsampled { 2 } else { 1 };
        for by in 0..luma_blocks {
            for bx in 0..luma_blocks {
                let x0 = mcu_x * mcu + bx * 8;
                let y0 = by * 8;
                for row in 0..8 {
                    let start = (y0 + row) * encoder.padded_width + x0;
                    let source = &band[0][start..start + 8];
                    for (sample, byte) in samples[row * 8..row * 8 + 8].iter_mut().zip(source) {
                        *sample = i32::from(*byte) - 128;
                    }
                }
                fdct(&samples, &mut coefficients);
                let codes = (&encoder.codes[0], &encoder.codes[1]);
                encode_block(
                    bits,
                    &coefficients,
                    &encoder.luma_quant,
                    &encoder.luma_reciprocal,
                    &mut predictions[0],
                    codes,
                );
            }
        }
        if !encoder.is_gray() {
            let x0 = mcu_x * mcu;
            let mut chroma = [[0i32; 64]; 2];
            if encoder.halved {
                let half = encoder.padded_width / 2;
                for (component, samples) in chroma.iter_mut().enumerate() {
                    let plane = &band[component + 1];
                    for row in 0..8 {
                        let start = row * half + x0 / 2;
                        for (sample, byte) in samples[row * 8..row * 8 + 8]
                            .iter_mut()
                            .zip(&plane[start..start + 8])
                        {
                            *sample = i32::from(*byte) - 128;
                        }
                    }
                }
            } else {
                for (component, samples) in chroma.iter_mut().enumerate() {
                    let plane = &band[component + 1];
                    for row in 0..8 {
                        let start = row * encoder.padded_width + x0;
                        let source = &plane[start..start + 8];
                        for (sample, byte) in samples[row * 8..row * 8 + 8].iter_mut().zip(source) {
                            *sample = i32::from(*byte) - 128;
                        }
                    }
                }
            }
            for (index, samples) in chroma.iter().enumerate() {
                fdct(samples, &mut coefficients);
                let codes = (&encoder.codes[2], &encoder.codes[3]);
                encode_block(
                    bits,
                    &coefficients,
                    &encoder.chroma_quant,
                    &encoder.chroma_reciprocal,
                    &mut predictions[index + 1],
                    codes,
                );
            }
        }
    }
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
    // The AC coefficients in zigzag order, and a mask with bit k set when
    // coefficient k is nonzero: the coding loop jumps from one nonzero
    // coefficient to the next by counting zeros in the mask (libjpeg-
    // turbo's approach) instead of testing all 63.
    let mut zigzag = [0i32; 64];
    let mut nonzero: u64 = 0;
    for (k, position) in ZIGZAG_TO_NATURAL.iter().enumerate().skip(1) {
        let value = quantized[*position];
        zigzag[k] = value;
        nonzero |= u64::from(value != 0) << k;
    }
    let mut last = 0u32;
    while nonzero != 0 {
        let k = nonzero.trailing_zeros();
        nonzero &= nonzero - 1;
        let mut run = k - last - 1;
        last = k;
        while run > 15 {
            let (length, code) = ac_codes.table[0xF0];
            bits.put(u32::from(code), u32::from(length));
            run -= 16;
        }
        let (size, value_bits) = magnitude(zigzag[k as usize]);
        let (length, code) = ac_codes.table[((run << 4) | size) as usize];
        bits.put(
            (u32::from(code) << size) | value_bits,
            u32::from(length) + size,
        );
    }
    if last < 63 {
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
    fn icc_profile(&mut self, profile: &[u8]) -> bool {
        // At most 255 segments of a profile.
        let fits = profile.len() <= 255 * 65_519;
        if fits {
            self.icc_profile = Some(profile.to_vec());
        }
        fits
    }

    fn exif(&mut self, exif: &[u8]) -> bool {
        // One APP1 segment.
        let fits = exif.len() <= 65_527;
        if fits {
            self.exif = Some(exif.to_vec());
        }
        fits
    }

    fn start(&mut self, width: u32, height: u32, color: ColorType) -> io::Result<()> {
        if width > 65_535 || height > 65_535 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "JPEG holds at most 65535 by 65535 pixels",
            ));
        }
        self.encoder.width = width as usize;
        self.encoder.height = height as usize;
        self.encoder.color = color;
        // A 4:2:0 source has no more chroma than 4:2:0 keeps.
        self.encoder.subsampled =
            self.encoder.planar || (!self.gray() && self.encoder.quality < 90);
        self.encoder.band_rows = if self.encoder.subsampled { 16 } else { 8 };
        let mcu = self.encoder.band_rows;
        self.encoder.padded_width = self.encoder.width.div_ceil(mcu) * mcu;
        let planes = if self.gray() { 1 } else { 3 };
        self.encoder.halved = self.encoder.subsampled;
        self.band = if self.encoder.subsampled {
            // Luma at full resolution, chroma at half: given, or from
            // each pair of RGB rows as it comes.
            if !self.encoder.planar {
                self.pair = vec![0u8; self.encoder.padded_width * 3 * 2];
            }
            let chroma = self.encoder.band_rows / 2 * self.encoder.padded_width / 2;
            vec![
                vec![0u8; self.encoder.band_rows * self.encoder.padded_width],
                vec![0u8; chroma],
                vec![0u8; chroma],
            ]
        } else {
            (0..planes)
                .map(|_| vec![0u8; self.encoder.band_rows * self.encoder.padded_width])
                .collect()
        };
        self.encoder.luma_quant = scaled(&STD_LUMA_QUANT, self.encoder.quality);
        self.encoder.chroma_quant = scaled(&STD_CHROMA_QUANT, self.encoder.quality);
        self.encoder.luma_reciprocal = reciprocals(&self.encoder.luma_quant);
        self.encoder.chroma_reciprocal = reciprocals(&self.encoder.chroma_quant);
        // A megapixel and up: a restart interval a band, so bands encode
        // on several threads (decided by size alone, so the bytes do not
        // depend on the machine).
        let mcus_wide = self.encoder.padded_width / self.encoder.band_rows;
        self.restart = self.encoder.width * self.encoder.height >= 1 << 20 && mcus_wide <= 65_535;
        self.headers()?;
        self.bits = BitWriter::new();
        self.started = true;
        Ok(())
    }

    fn accept_ycbcr(&mut self) -> bool {
        self.encoder.planar = true;
        true
    }

    fn ycbcr_row(&mut self, luma: &[u8], chroma: Option<(&[u8], &[u8])>) -> io::Result<()> {
        if !self.started || !self.encoder.planar {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a YCbCr row before start or without the offer",
            ));
        }
        let (padded, width) = (self.encoder.padded_width, self.encoder.width);
        let y_index = self.rows_in_band;
        // Each row padded on the right by replication, as libjpeg does.
        let target = &mut self.band[0][y_index * padded..(y_index + 1) * padded];
        target[..width].copy_from_slice(&luma[..width]);
        let last = target[width - 1];
        target[width..].fill(last);
        if let Some((blue, red)) = chroma {
            let (half, chroma_width) = (padded / 2, width.div_ceil(2));
            let row = y_index / 2;
            for (plane, source) in [(1, blue), (2, red)] {
                let target = &mut self.band[plane][row * half..(row + 1) * half];
                target[..chroma_width].copy_from_slice(&source[..chroma_width]);
                let last = target[chroma_width - 1];
                target[chroma_width..].fill(last);
            }
        }
        self.rows_in_band += 1;
        self.rows_taken += 1;
        if self.rows_in_band == self.encoder.band_rows || self.rows_taken == self.encoder.height {
            self.encode_band()?;
        }
        if self.rows_taken == self.encoder.height {
            let mut bits = std::mem::replace(&mut self.bits, BitWriter::new());
            bits.finish(self.sink)?;
            self.sink.write_all(&[0xFF, 0xD9])?;
            self.sink.flush()?;
        }
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
        if self.rows_in_band == self.encoder.band_rows || self.rows_taken == self.encoder.height {
            self.encode_band()?;
        }
        if self.rows_taken == self.encoder.height {
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
