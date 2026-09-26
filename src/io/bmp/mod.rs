//! Windows bitmaps: a reader into the image hub and a writer from it.
//! Read: uncompressed 1, 4, 8 (palette), 16 (555 or bit fields), 24, and
//! 32 bit, bottom-up or top-down, with BITMAPINFOHEADER and the V4 and V5
//! headers (whose alpha mask is honored). Write: 24-bit BGR for opaque
//! images, 32-bit BGRA with a V4 header and alpha mask otherwise.

use std::io::{self, Write};

use crate::image::{ColorType, Image};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BmpError(pub String);

impl std::fmt::Display for BmpError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl std::error::Error for BmpError {}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// A channel mask: where the bits sit and how many there are.
#[derive(Clone, Copy)]
struct Mask {
    shift: u32,
    bits: u32,
}

impl Mask {
    fn from_bits(mask: u32) -> Option<Mask> {
        if mask == 0 {
            return None;
        }
        let shift = mask.trailing_zeros();
        let bits = (mask >> shift).trailing_ones();
        Some(Mask { shift, bits })
    }

    /// The channel's value scaled to eight bits.
    fn extract(self, pixel: u32) -> u8 {
        let raw = (pixel >> self.shift) & ((1u32 << self.bits) - 1);
        if self.bits == 8 {
            raw as u8
        } else if self.bits == 0 {
            0
        } else {
            ((raw * 255 + (1 << (self.bits - 1))) / ((1 << self.bits) - 1)) as u8
        }
    }
}

/// Everything the pixel rows need, read from the headers and palette.
struct Layout {
    width: u32,
    height: u32,
    top_down: bool,
    bits: u16,
    pixel_offset: usize,
    row_bytes: usize,
    color: ColorType,
    has_alpha: bool,
    plain_bgr: bool,
    plain_alpha: bool,
    masks: [Option<Mask>; 4],
    palette: Vec<[u8; 3]>,
}

/// The byte count the headers and palette can take at most: the file
/// header, a V5 info header, four masks, and a 256-entry palette.
const MAX_PREAMBLE: usize = 14 + 124 + 16 + 1024;

/// Parses the headers and palette at the front of `bytes`, which must
/// hold at least the pixel offset's worth of the file.
fn layout(bytes: &[u8]) -> Result<Layout, BmpError> {
    if bytes.len() < 54 || &bytes[..2] != b"BM" {
        return Err(BmpError(
            "not a BMP: bad signature or too short".to_string(),
        ));
    }
    let pixel_offset = u32_at(bytes, 10) as usize;
    let header_size = u32_at(bytes, 14) as usize;
    if header_size < 40 || bytes.len() < 14 + header_size {
        return Err(BmpError(format!("unsupported header size {header_size}")));
    }
    let width_raw = u32_at(bytes, 18) as i32;
    let height_raw = u32_at(bytes, 22) as i32;
    let planes = u16_at(bytes, 26);
    let bits = u16_at(bytes, 28);
    let compression = u32_at(bytes, 30);
    let top_down = height_raw < 0;
    let width = width_raw.unsigned_abs();
    let height = height_raw.unsigned_abs();
    if planes != 1 || !Image::dimensions_fit(width, height, 4) {
        return Err(BmpError("bad dimensions or plane count".to_string()));
    }
    let bit_fields = compression == 3 || compression == 6;
    if compression != 0 && !bit_fields {
        return Err(BmpError(format!(
            "compression {compression} (RLE or JPEG) is not supported"
        )));
    }
    // Channel masks: from the header (V4+ or bit fields), else the defaults.
    let (mut red, mut green, mut blue, mut alpha) = match bits {
        16 => (0x7c00u32, 0x03e0u32, 0x001fu32, 0u32),
        32 => (0x00ff_0000, 0x0000_ff00, 0x0000_00ff, 0),
        _ => (0, 0, 0, 0),
    };
    if bit_fields || header_size >= 108 {
        let masks_at = 14 + 40;
        if bytes.len() >= masks_at + 12 {
            red = u32_at(bytes, masks_at);
            green = u32_at(bytes, masks_at + 4);
            blue = u32_at(bytes, masks_at + 8);
        }
        if header_size >= 108 && bytes.len() >= masks_at + 16 {
            alpha = u32_at(bytes, masks_at + 12);
        } else if bit_fields && header_size == 40 && bytes.len() >= masks_at + 16 && bits == 32 {
            // Some writers put four masks after a 40-byte header.
            alpha = u32_at(bytes, masks_at + 12);
        }
    }
    let palette_entries = match bits {
        1 | 4 | 8 => {
            let used = u32_at(bytes, 46) as usize;
            if used == 0 { 1usize << bits } else { used }
        }
        _ => 0,
    };
    let palette_at = 14
        + header_size
        + if bit_fields && header_size == 40 {
            12
        } else {
            0
        };
    let mut palette: Vec<[u8; 3]> = Vec::with_capacity(palette_entries);
    for index in 0..palette_entries {
        let at = palette_at + index * 4;
        if bytes.len() < at + 4 {
            return Err(BmpError("palette cut short".to_string()));
        }
        palette.push([bytes[at + 2], bytes[at + 1], bytes[at]]);
    }
    let row_bytes = (width as usize * bits as usize).div_ceil(32) * 4;
    let has_alpha = bits == 32 && alpha != 0;
    let color = match bits {
        1 | 4 | 8 | 16 | 24 => ColorType::Rgb,
        32 => {
            if has_alpha {
                ColorType::Rgba
            } else {
                ColorType::Rgb
            }
        }
        other => return Err(BmpError(format!("{other} bits per pixel is not supported"))),
    };
    let plain_bgr = bits == 32 && red == 0x00ff_0000 && green == 0x0000_ff00 && blue == 0x0000_00ff;
    Ok(Layout {
        width,
        height,
        top_down,
        bits,
        pixel_offset,
        row_bytes,
        color,
        has_alpha,
        plain_bgr,
        plain_alpha: plain_bgr && has_alpha && alpha == 0xff00_0000,
        masks: [
            Mask::from_bits(red),
            Mask::from_bits(green),
            Mask::from_bits(blue),
            Mask::from_bits(alpha),
        ],
        palette,
    })
}

/// Turns one file row into hub pixels.
fn decode_row(layout: &Layout, source: &[u8], target: &mut [u8]) {
    let bits = layout.bits;
    let channels = layout.color.channels();
    // The common depths as row loops with no per-pixel dispatch.
    if bits == 24 {
        for (cell, bgr) in target.chunks_exact_mut(3).zip(source.chunks_exact(3)) {
            cell[0] = bgr[2];
            cell[1] = bgr[1];
            cell[2] = bgr[0];
        }
        return;
    }
    if layout.plain_alpha {
        for (cell, bgra) in target.chunks_exact_mut(4).zip(source.chunks_exact(4)) {
            cell[0] = bgra[2];
            cell[1] = bgra[1];
            cell[2] = bgra[0];
            cell[3] = bgra[3];
        }
        return;
    }
    if layout.plain_bgr && !layout.has_alpha {
        for (cell, bgra) in target.chunks_exact_mut(3).zip(source.chunks_exact(4)) {
            cell[0] = bgra[2];
            cell[1] = bgra[1];
            cell[2] = bgra[0];
        }
        return;
    }
    let masks = &layout.masks;
    for x in 0..layout.width as usize {
        let cell = &mut target[x * channels..(x + 1) * channels];
        match bits {
            32 => {
                let pixel = u32_at(source, x * 4);
                cell[0] = masks[0].map_or(0, |mask| mask.extract(pixel));
                cell[1] = masks[1].map_or(0, |mask| mask.extract(pixel));
                cell[2] = masks[2].map_or(0, |mask| mask.extract(pixel));
                if layout.has_alpha {
                    cell[3] = masks[3].map_or(255, |mask| mask.extract(pixel));
                }
            }
            16 => {
                let pixel = u32::from(u16_at(source, x * 2));
                cell[0] = masks[0].map_or(0, |mask| mask.extract(pixel));
                cell[1] = masks[1].map_or(0, |mask| mask.extract(pixel));
                cell[2] = masks[2].map_or(0, |mask| mask.extract(pixel));
            }
            _ => {
                let per_byte = 8 / bits as usize;
                let byte = source[x / per_byte];
                let shift = 8 - bits as usize * (x % per_byte + 1);
                let index = ((byte >> shift) & ((1u16 << bits) - 1) as u8) as usize;
                let rgb = layout.palette.get(index).copied().unwrap_or([0, 0, 0]);
                cell[..3].copy_from_slice(&rgb);
            }
        }
    }
}

pub fn read_bmp(bytes: &[u8]) -> Result<Image, BmpError> {
    let layout = layout(bytes)?;
    let needed = layout.pixel_offset + layout.row_bytes * layout.height as usize;
    if bytes.len() < needed {
        return Err(BmpError("pixel data cut short".to_string()));
    }
    let mut image = Image::new(layout.width, layout.height, layout.color);
    let stride = image.stride();
    for file_row in 0..layout.height as usize {
        let y = if layout.top_down {
            file_row
        } else {
            layout.height as usize - 1 - file_row
        };
        let start = layout.pixel_offset + file_row * layout.row_bytes;
        let source = &bytes[start..start + layout.row_bytes];
        let target = &mut image.pixels[y * stride..(y + 1) * stride];
        decode_row(&layout, source, target);
    }
    Ok(image)
}

/// What can go wrong reading rows into a sink: the file, or the sink.
#[derive(Debug)]
pub enum BmpRowsError {
    Bmp(BmpError),
    Io(io::Error),
}

impl From<BmpError> for BmpRowsError {
    fn from(error: BmpError) -> Self {
        BmpRowsError::Bmp(error)
    }
}

impl From<io::Error> for BmpRowsError {
    fn from(error: io::Error) -> Self {
        BmpRowsError::Io(error)
    }
}

/// Reads a BMP from a stream into a row sink, top to bottom. A top-down
/// file streams a row at a time and nothing is held; a bottom-up file
/// (the common kind) stores its last row first, so its pixel data is
/// read once into memory and handed over from the end, one copy where
/// an image would be a second.
pub fn read_bmp_rows(
    reader: &mut dyn std::io::Read,
    sink: &mut dyn crate::io::png::RowSink,
) -> Result<(), BmpRowsError> {
    let mut preamble = vec![0u8; 54];
    reader.read_exact(&mut preamble)?;
    let pixel_offset = u32_at(&preamble, 10) as usize;
    if !(54..=MAX_PREAMBLE).contains(&pixel_offset) {
        return Err(BmpError("bad pixel offset".to_string()).into());
    }
    preamble.resize(pixel_offset, 0);
    reader.read_exact(&mut preamble[54..])?;
    let layout = layout(&preamble)?;
    let stride = layout.width as usize * layout.color.channels();
    sink.start(layout.width, layout.height, layout.color)?;
    let mut pixels = vec![0u8; stride];
    if layout.top_down {
        let mut file_row = vec![0u8; layout.row_bytes];
        for _ in 0..layout.height {
            reader.read_exact(&mut file_row)?;
            decode_row(&layout, &file_row, &mut pixels);
            sink.row(&pixels)?;
        }
        return Ok(());
    }
    let total = layout.row_bytes * layout.height as usize;
    let mut data = vec![0u8; total];
    reader.read_exact(&mut data)?;
    for file_row in (0..layout.height as usize).rev() {
        let start = file_row * layout.row_bytes;
        decode_row(&layout, &data[start..start + layout.row_bytes], &mut pixels);
        sink.row(&pixels)?;
    }
    Ok(())
}

/// Rows are written in pieces of about this size.
const WRITE_CHUNK: usize = 1 << 20;

/// Writes the image: 24-bit for opaque images, 32-bit BGRA with a V4
/// header for images with alpha. Gray becomes RGB, as BMP has no gray.
/// The file and info headers. A negative `height_field` declares a
/// top-down file, the form the row writer uses.
fn headers(width: u32, height_field: i32, rows: u32, with_alpha: bool) -> (Vec<u8>, usize) {
    let bits: u32 = if with_alpha { 32 } else { 24 };
    let header_size: u32 = if with_alpha { 108 } else { 40 };
    let row_bytes = (width as usize * bits as usize).div_ceil(32) * 4;
    let pixel_bytes = row_bytes * rows as usize;
    let pixel_offset = 14 + header_size;
    let file_size = pixel_offset as usize + pixel_bytes;
    let mut header = Vec::with_capacity(pixel_offset as usize);
    header.extend_from_slice(b"BM");
    header.extend_from_slice(&(file_size as u32).to_le_bytes());
    header.extend_from_slice(&[0, 0, 0, 0]);
    header.extend_from_slice(&pixel_offset.to_le_bytes());
    header.extend_from_slice(&header_size.to_le_bytes());
    header.extend_from_slice(&(width as i32).to_le_bytes());
    header.extend_from_slice(&height_field.to_le_bytes());
    header.extend_from_slice(&1u16.to_le_bytes());
    header.extend_from_slice(&(bits as u16).to_le_bytes());
    let compression: u32 = if with_alpha { 3 } else { 0 };
    header.extend_from_slice(&compression.to_le_bytes());
    header.extend_from_slice(&(pixel_bytes as u32).to_le_bytes());
    header.extend_from_slice(&2835u32.to_le_bytes());
    header.extend_from_slice(&2835u32.to_le_bytes());
    header.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]);
    if with_alpha {
        for mask in [0x00ff_0000u32, 0x0000_ff00, 0x0000_00ff, 0xff00_0000] {
            header.extend_from_slice(&mask.to_le_bytes());
        }
        header.extend_from_slice(b"sRGB");
        header.extend_from_slice(&[0; 48]);
    }
    (header, row_bytes)
}

/// Turns one hub row into a BMP row (BGR or BGRA) at the end of `out`.
fn append_row(
    color: ColorType,
    with_alpha: bool,
    source: &[u8],
    row_bytes: usize,
    out: &mut Vec<u8>,
) {
    let start = out.len();
    out.resize(start + row_bytes, 0);
    let target = &mut out[start..];
    match color {
        ColorType::Rgb if !with_alpha => {
            for (bgr, rgb) in target.chunks_exact_mut(3).zip(source.chunks_exact(3)) {
                bgr[0] = rgb[2];
                bgr[1] = rgb[1];
                bgr[2] = rgb[0];
            }
        }
        ColorType::Rgba if with_alpha => {
            for (bgra, rgba) in target.chunks_exact_mut(4).zip(source.chunks_exact(4)) {
                bgra[0] = rgba[2];
                bgra[1] = rgba[1];
                bgra[2] = rgba[0];
                bgra[3] = rgba[3];
            }
        }
        _ => {
            let channels = color.channels();
            let width = source.len() / channels;
            for x in 0..width {
                let cell = &source[x * channels..(x + 1) * channels];
                let (r, g, b, a) = match color {
                    ColorType::Gray => (cell[0], cell[0], cell[0], 255),
                    ColorType::GrayAlpha => (cell[0], cell[0], cell[0], cell[1]),
                    ColorType::Rgb => (cell[0], cell[1], cell[2], 255),
                    ColorType::Rgba => (cell[0], cell[1], cell[2], cell[3]),
                };
                if with_alpha {
                    target[x * 4..x * 4 + 4].copy_from_slice(&[b, g, r, a]);
                } else {
                    target[x * 3..x * 3 + 3].copy_from_slice(&[b, g, r]);
                }
            }
        }
    }
}

/// Writes a BMP row by row as a `RowSink`, top-down (a negative height
/// in the header, which every reader since Windows 95 takes), so a
/// decoder can hand rows over as it produces them and no image is held.
pub struct BmpRows<'a> {
    sink: &'a mut dyn Write,
    color: ColorType,
    with_alpha: bool,
    row_bytes: usize,
    rows_left: u32,
    pending: Vec<u8>,
}

impl<'a> BmpRows<'a> {
    pub fn new(sink: &'a mut dyn Write) -> BmpRows<'a> {
        BmpRows {
            sink,
            color: ColorType::Rgb,
            with_alpha: false,
            row_bytes: 0,
            rows_left: 0,
            pending: Vec::new(),
        }
    }
}

impl crate::io::png::RowSink for BmpRows<'_> {
    fn start(&mut self, width: u32, height: u32, color: ColorType) -> io::Result<()> {
        self.color = color;
        self.with_alpha = color.has_alpha();
        let (header, row_bytes) = headers(width, -(height as i32), height, self.with_alpha);
        self.row_bytes = row_bytes;
        self.rows_left = height;
        self.pending = Vec::with_capacity(WRITE_CHUNK + row_bytes);
        self.sink.write_all(&header)
    }

    fn row(&mut self, pixels: &[u8]) -> io::Result<()> {
        append_row(
            self.color,
            self.with_alpha,
            pixels,
            self.row_bytes,
            &mut self.pending,
        );
        self.rows_left = self.rows_left.saturating_sub(1);
        if self.pending.len() >= WRITE_CHUNK || self.rows_left == 0 {
            self.sink.write_all(&self.pending)?;
            self.pending.clear();
            if self.rows_left == 0 {
                self.sink.flush()?;
            }
        }
        Ok(())
    }
}

pub fn write_bmp(image: &Image, sink: &mut dyn Write) -> io::Result<()> {
    let with_alpha = image.color.has_alpha();
    let (header, row_bytes) = headers(image.width, image.height as i32, image.height, with_alpha);
    sink.write_all(&header)?;
    let channels = image.color.channels();
    let mut row = vec![0u8; row_bytes];
    // Rows gather into a buffer of about a megabyte before each write,
    // so a large image costs tens of writes rather than thousands.
    let mut pending: Vec<u8> = Vec::with_capacity(WRITE_CHUNK + row_bytes);
    for y in (0..image.height).rev() {
        if pending.len() >= WRITE_CHUNK {
            sink.write_all(&pending)?;
            pending.clear();
        }
        let source = image.row(y);
        // The common layouts as row loops.
        if image.color == ColorType::Rgb {
            let start = pending.len();
            pending.resize(start + row_bytes, 0);
            let target = &mut pending[start..];
            for (bgr, rgb) in target.chunks_exact_mut(3).zip(source.chunks_exact(3)) {
                bgr[0] = rgb[2];
                bgr[1] = rgb[1];
                bgr[2] = rgb[0];
            }
            continue;
        }
        if image.color == ColorType::Rgba {
            let start = pending.len();
            pending.resize(start + row_bytes, 0);
            let target = &mut pending[start..];
            for (bgra, rgba) in target.chunks_exact_mut(4).zip(source.chunks_exact(4)) {
                bgra[0] = rgba[2];
                bgra[1] = rgba[1];
                bgra[2] = rgba[0];
                bgra[3] = rgba[3];
            }
            continue;
        }
        for x in 0..image.width as usize {
            let cell = &source[x * channels..(x + 1) * channels];
            let (r, g, b, a) = match image.color {
                ColorType::Gray => (cell[0], cell[0], cell[0], 255),
                ColorType::GrayAlpha => (cell[0], cell[0], cell[0], cell[1]),
                ColorType::Rgb => (cell[0], cell[1], cell[2], 255),
                ColorType::Rgba => (cell[0], cell[1], cell[2], cell[3]),
            };
            if with_alpha {
                row[x * 4..x * 4 + 4].copy_from_slice(&[b, g, r, a]);
            } else {
                row[x * 3..x * 3 + 3].copy_from_slice(&[b, g, r]);
            }
        }
        pending.extend_from_slice(&row);
    }
    sink.write_all(&pending)?;
    sink.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_and_alpha_images_round_trip() {
        for color in [
            ColorType::Gray,
            ColorType::GrayAlpha,
            ColorType::Rgb,
            ColorType::Rgba,
        ] {
            let mut image = Image::new(3, 2, color);
            for (index, byte) in image.pixels.iter_mut().enumerate() {
                *byte = (index * 37 % 256) as u8;
            }
            let mut bytes = Vec::new();
            write_bmp(&image, &mut bytes).unwrap();
            let back = read_bmp(&bytes).unwrap();
            let channels = color.channels();
            for (index, cell) in image.pixels.chunks(channels).enumerate() {
                let got = &back.pixels
                    [index * back.color.channels()..(index + 1) * back.color.channels()];
                let (r, g, b, a) = match color {
                    ColorType::Gray => (cell[0], cell[0], cell[0], 255),
                    ColorType::GrayAlpha => (cell[0], cell[0], cell[0], cell[1]),
                    ColorType::Rgb => (cell[0], cell[1], cell[2], 255),
                    ColorType::Rgba => (cell[0], cell[1], cell[2], cell[3]),
                };
                assert_eq!(&got[..3], &[r, g, b], "{color:?} pixel {index}");
                if back.color == ColorType::Rgba {
                    assert_eq!(got[3], a, "{color:?} alpha {index}");
                }
            }
        }
    }

    #[test]
    fn masks_scale_to_eight_bits() {
        let five = Mask::from_bits(0x7c00).unwrap();
        assert_eq!(five.extract(0x7c00), 255);
        assert_eq!(five.extract(0x0400), 8);
        assert!(Mask::from_bits(0).is_none());
    }
}
