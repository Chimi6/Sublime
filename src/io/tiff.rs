//! TIFF: a header, then image file directories (IFDs) of tagged values
//! pointing at the pixel data in strips or tiles. Read: the first page,
//! in the layouts in common use (strips and tiles, chunky and planar;
//! none, LZW, deflate, and PackBits; the horizontal predictor; 1 to 16
//! bits; gray, min-is-white, palette, RGB, CMYK, with or without alpha).
//! Written: 8-bit strips, deflate with the horizontal predictor.

use std::io::{self, Read, Write};

use crate::image::{ColorType, Image, MAX_PIXELS};
use crate::io::deflate::{deflate, inflate};
use crate::io::png::{Collect, PngError, RowSink, RowsError, adler32};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TiffError(pub String);

impl std::fmt::Display for TiffError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl std::error::Error for TiffError {}

/// What reading left out.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TiffNotes {
    /// Pages after the first, not read.
    pub other_pages: usize,
    pub sixteen_bit: bool,
    pub cmyk: bool,
}

fn error(message: impl Into<String>) -> TiffError {
    TiffError(message.into())
}

// ------------------------------------------------------------- the file

/// The file with its byte order.
struct File<'a> {
    bytes: &'a [u8],
    big: bool,
}

impl File<'_> {
    fn u16(&self, at: usize) -> Result<u16, TiffError> {
        let pair: [u8; 2] = self
            .bytes
            .get(at..at + 2)
            .and_then(|slice| slice.try_into().ok())
            .ok_or_else(|| error("TIFF cut short"))?;
        Ok(if self.big {
            u16::from_be_bytes(pair)
        } else {
            u16::from_le_bytes(pair)
        })
    }

    fn u32(&self, at: usize) -> Result<u32, TiffError> {
        let quad: [u8; 4] = self
            .bytes
            .get(at..at + 4)
            .and_then(|slice| slice.try_into().ok())
            .ok_or_else(|| error("TIFF cut short"))?;
        Ok(if self.big {
            u32::from_be_bytes(quad)
        } else {
            u32::from_le_bytes(quad)
        })
    }

    /// An entry's integer values (BYTE, SHORT, or LONG).
    fn values(&self, entry: usize) -> Result<Vec<u32>, TiffError> {
        let kind = self.u16(entry + 2)?;
        let count = self.u32(entry + 4)? as usize;
        let size = match kind {
            1 => 1,
            3 => 2,
            4 => 4,
            _ => {
                return Err(error(format!(
                    "TIFF tag type {kind} where a number is expected"
                )));
            }
        };
        let total = count
            .checked_mul(size)
            .ok_or_else(|| error("TIFF tag too large"))?;
        let at = if total <= 4 {
            entry + 8
        } else {
            self.u32(entry + 8)? as usize
        };
        if at
            .checked_add(total)
            .is_none_or(|end| end > self.bytes.len())
        {
            return Err(error("TIFF tag values lie outside the file"));
        }
        (0..count)
            .map(|index| match size {
                1 => Ok(u32::from(self.bytes[at + index])),
                2 => self.u16(at + index * 2).map(u32::from),
                _ => self.u32(at + index * 4),
            })
            .collect()
    }
}

/// The tags of the first page that the pixels need.
#[derive(Default)]
struct Tags {
    width: u32,
    height: u32,
    bits: Vec<u32>,
    compression: u32,
    photometric: Option<u32>,
    offsets: Vec<u32>,
    samples: u32,
    rows_per_strip: u32,
    counts: Vec<u32>,
    planar: u32,
    predictor: u32,
    color_map: Vec<u32>,
    tile_width: u32,
    tile_height: u32,
    extra: Vec<u32>,
    sample_format: u32,
    ink_set: u32,
}

/// Reads the header and the first IFD; counts the pages after it.
fn read_tags(bytes: &[u8]) -> Result<(File<'_>, Tags, usize), TiffError> {
    let big = match bytes.get(..2) {
        Some(b"II") => false,
        Some(b"MM") => true,
        _ => return Err(error("not a TIFF file")),
    };
    let file = File { bytes, big };
    if file.u16(2)? != 42 {
        return Err(error("not a TIFF file (or a BigTIFF, which is not read)"));
    }
    let first = file.u32(4)? as usize;
    let mut tags = Tags {
        compression: 1,
        samples: 1,
        rows_per_strip: u32::MAX,
        planar: 1,
        predictor: 1,
        sample_format: 1,
        ink_set: 1,
        ..Tags::default()
    };
    let count = usize::from(file.u16(first)?);
    for index in 0..count {
        let entry = first + 2 + 12 * index;
        let tag = file.u16(entry)?;
        let first_value = || -> Result<u32, TiffError> {
            file.values(entry)?
                .first()
                .copied()
                .ok_or_else(|| error("TIFF tag has no value"))
        };
        match tag {
            256 => tags.width = first_value()?,
            257 => tags.height = first_value()?,
            258 => tags.bits = file.values(entry)?,
            259 => tags.compression = first_value()?,
            262 => tags.photometric = Some(first_value()?),
            273 => tags.offsets = file.values(entry)?,
            277 => tags.samples = first_value()?,
            278 => tags.rows_per_strip = first_value()?,
            279 => tags.counts = file.values(entry)?,
            284 => tags.planar = first_value()?,
            317 => tags.predictor = first_value()?,
            320 => tags.color_map = file.values(entry)?,
            322 => tags.tile_width = first_value()?,
            323 => tags.tile_height = first_value()?,
            324 => tags.offsets = file.values(entry)?,
            325 => tags.counts = file.values(entry)?,
            332 => tags.ink_set = first_value()?,
            338 => tags.extra = file.values(entry)?,
            339 => tags.sample_format = first_value()?,
            _ => {}
        }
    }
    // The rest of the page chain, counted (with a guard against loops).
    let mut others = 0usize;
    let mut next = file.u32(first + 2 + 12 * count)? as usize;
    while next != 0 && others < 65_536 {
        others += 1;
        let entries = usize::from(file.u16(next)?);
        next = file.u32(next + 2 + 12 * entries)? as usize;
    }
    Ok((file, tags, others))
}

// ------------------------------------------------------- decompression

/// TIFF LZW: codes from 9 to 12 bits, most significant bit first, with
/// the code width growing one code early (libtiff's compatible form).
fn lzw(input: &[u8], expected: usize) -> Result<Vec<u8>, TiffError> {
    const CLEAR: usize = 256;
    const END: usize = 257;
    if input.starts_with(&[0, 1]) {
        return Err(error("old-style TIFF LZW is not supported"));
    }
    let mut out = Vec::with_capacity(expected);
    // Each code: its prefix code, last byte, first byte, and length.
    let mut prefix = vec![0u16; 4096];
    let mut last = vec![0u8; 4096];
    let mut first = vec![0u8; 4096];
    let mut length = vec![0u16; 4096];
    for code in 0..256 {
        last[code] = code as u8;
        first[code] = code as u8;
        length[code] = 1;
    }
    let (mut next, mut width) = (258usize, 9u32);
    let mut previous: Option<usize> = None;
    let (mut buffer, mut count, mut at) = (0u32, 0u32, 0usize);
    let emit = |out: &mut Vec<u8>, code: usize, prefix: &[u16], last: &[u8], length: &[u16]| {
        let size = usize::from(length[code]);
        let start = out.len();
        out.resize(start + size, 0);
        let mut cursor = code;
        for slot in out[start..].iter_mut().rev() {
            *slot = last[cursor];
            cursor = usize::from(prefix[cursor]);
        }
    };
    while out.len() < expected {
        while count < width {
            let Some(byte) = input.get(at) else {
                return Err(error("TIFF LZW data cut short"));
            };
            buffer = (buffer << 8) | u32::from(*byte);
            count += 8;
            at += 1;
        }
        let code = ((buffer >> (count - width)) & ((1 << width) - 1)) as usize;
        count -= width;
        if code == END {
            break;
        }
        if code == CLEAR {
            next = 258;
            width = 9;
            previous = None;
            continue;
        }
        match previous {
            None => {
                if code >= 256 {
                    return Err(error("bad TIFF LZW code"));
                }
                out.push(code as u8);
            }
            Some(before) => {
                let added = if code < next {
                    emit(&mut out, code, &prefix, &last, &length);
                    first[code]
                } else if code == next {
                    let head = first[before];
                    emit(&mut out, before, &prefix, &last, &length);
                    out.push(head);
                    head
                } else {
                    return Err(error("bad TIFF LZW code"));
                };
                if next < 4096 {
                    prefix[next] = before as u16;
                    last[next] = added;
                    first[next] = first[before];
                    length[next] = length[before] + 1;
                    next += 1;
                }
            }
        }
        previous = Some(code);
        if next + 1 >= (1 << width) && width < 12 {
            width += 1;
        }
    }
    out.truncate(expected);
    Ok(out)
}

/// PackBits: a count byte, then that many literal bytes or one repeated.
fn packbits(input: &[u8], expected: usize) -> Result<Vec<u8>, TiffError> {
    let mut out = Vec::with_capacity(expected);
    let mut at = 0;
    while out.len() < expected && at < input.len() {
        let head = input[at] as i8;
        at += 1;
        if head >= 0 {
            let count = head as usize + 1;
            let literal = input
                .get(at..at + count)
                .ok_or_else(|| error("TIFF PackBits data cut short"))?;
            out.extend_from_slice(literal);
            at += count;
        } else if head != -128 {
            let byte = *input
                .get(at)
                .ok_or_else(|| error("TIFF PackBits data cut short"))?;
            at += 1;
            out.extend(std::iter::repeat_n(byte, 1 - head as isize as usize));
        }
    }
    out.truncate(expected);
    Ok(out)
}

/// A zlib stream's payload inflated.
fn zlib(input: &[u8], expected: usize) -> Result<Vec<u8>, TiffError> {
    if input.len() < 2 || input[0] & 0x0f != 8 {
        return Err(error("bad TIFF deflate data"));
    }
    let mut out = Vec::new();
    inflate(&input[2..], &mut out, expected)
        .map_err(|failure| error(format!("bad TIFF deflate data: {failure:?}")))?;
    Ok(out)
}

/// A chunk (strip or tile) decompressed to `expected` bytes.
fn decompress(tags: &Tags, input: &[u8], expected: usize) -> Result<Vec<u8>, TiffError> {
    let mut out = match tags.compression {
        1 => input.to_vec(),
        5 => lzw(input, expected)?,
        8 | 32946 => zlib(input, expected)?,
        32773 => packbits(input, expected)?,
        other => {
            let name = match other {
                2..=4 => "CCITT fax",
                6 | 7 => "JPEG",
                34712 => "JPEG 2000",
                50000 => "Zstandard",
                _ => "this",
            };
            return Err(error(format!(
                "TIFF {name} compression ({other}) is not supported"
            )));
        }
    };
    if out.len() < expected {
        return Err(error("TIFF image data cut short"));
    }
    out.truncate(expected);
    Ok(out)
}

/// Undoes the horizontal predictor on one row of `samples` interleaved
/// samples per pixel at 8 or 16 bits.
fn unpredict(row: &mut [u8], samples: usize, bits: u32, big: bool) {
    if bits == 8 {
        for at in samples..row.len() {
            row[at] = row[at].wrapping_add(row[at - samples]);
        }
    } else {
        let read = |row: &[u8], at: usize| {
            let pair = [row[at * 2], row[at * 2 + 1]];
            if big {
                u16::from_be_bytes(pair)
            } else {
                u16::from_le_bytes(pair)
            }
        };
        for at in samples..row.len() / 2 {
            let value = read(row, at).wrapping_add(read(row, at - samples));
            let bytes = if big {
                value.to_be_bytes()
            } else {
                value.to_le_bytes()
            };
            row[at * 2..at * 2 + 2].copy_from_slice(&bytes);
        }
    }
}

// ------------------------------------------------------------ the pixels

/// How stored samples become the hub's pixels.
struct Layout {
    width: usize,
    height: usize,
    bits: u32,
    samples: usize,
    /// Samples before the alpha (1 gray or palette, 3 RGB, 4 CMYK).
    color_samples: usize,
    alpha: Option<usize>,
    associated: bool,
    photometric: u32,
    palette: Vec<[u8; 3]>,
    color: ColorType,
    big: bool,
}

fn layout(tags: &Tags, big: bool) -> Result<Layout, TiffError> {
    if tags.width == 0 || tags.height == 0 {
        return Err(error("TIFF image has no width or height"));
    }
    if u64::from(tags.width) * u64::from(tags.height) > MAX_PIXELS {
        return Err(error("TIFF image too large"));
    }
    if tags.sample_format != 1 {
        return Err(error(
            "TIFF floating-point and signed samples are not supported",
        ));
    }
    let bits = tags.bits.first().copied().unwrap_or(1);
    if !matches!(bits, 1 | 2 | 4 | 8 | 16) || tags.bits.iter().any(|value| *value != bits) {
        return Err(error(format!(
            "TIFF samples of {bits} bits are not supported"
        )));
    }
    let samples = tags.samples as usize;
    let photometric = tags
        .photometric
        .ok_or_else(|| error("TIFF image has no photometric interpretation"))?;
    let color_samples = match photometric {
        0 | 1 | 3 => 1,
        2 => 3,
        5 if tags.ink_set == 1 => 4,
        6 => return Err(error("TIFF YCbCr images are not supported")),
        other => {
            return Err(error(format!(
                "TIFF photometric interpretation {other} is not supported"
            )));
        }
    };
    if samples < color_samples {
        return Err(error("TIFF has fewer samples than its colors need"));
    }
    let alpha = match tags.extra.first() {
        Some(1 | 2) if samples > color_samples && photometric != 3 => Some(color_samples),
        _ => None,
    };
    let palette = if photometric == 3 {
        let entries = 1usize << bits;
        if tags.color_map.len() < entries * 3 {
            return Err(error("TIFF palette image has no color map"));
        }
        (0..entries)
            .map(|index| {
                let channel = |plane: usize| (tags.color_map[plane * entries + index] >> 8) as u8;
                [channel(0), channel(1), channel(2)]
            })
            .collect()
    } else {
        Vec::new()
    };
    let color = match (photometric, alpha.is_some()) {
        (0 | 1, false) => ColorType::Gray,
        (0 | 1, true) => ColorType::GrayAlpha,
        (_, false) => ColorType::Rgb,
        (_, true) => ColorType::Rgba,
    };
    if tags.predictor == 2 && bits < 8 {
        return Err(error(
            "TIFF predictor on samples under 8 bits is not supported",
        ));
    }
    if !matches!(tags.predictor, 1 | 2) {
        return Err(error("TIFF floating-point predictor is not supported"));
    }
    Ok(Layout {
        width: tags.width as usize,
        height: tags.height as usize,
        bits,
        samples,
        color_samples,
        alpha,
        associated: tags.extra.first() == Some(&1),
        photometric,
        palette,
        color,
        big,
    })
}

/// A stored row (chunky, at the file's depth) as the hub's pixels.
fn convert(layout: &Layout, stored: &[u8], row: &mut [u8]) {
    let bits = layout.bits as usize;
    let levels = (1u32 << layout.bits) - 1;
    let sample = |index: usize| -> u32 {
        match bits {
            16 => {
                let pair = [stored[index * 2], stored[index * 2 + 1]];
                u32::from(if layout.big {
                    u16::from_be_bytes(pair)
                } else {
                    u16::from_le_bytes(pair)
                })
            }
            8 => u32::from(stored[index]),
            _ => {
                let bit = index * bits;
                u32::from(stored[bit / 8] >> (8 - bits - bit % 8)) & levels
            }
        }
    };
    // A sample scaled to 8 bits: the high byte of 16, levels spread below 8.
    let eight = |value: u32| -> u8 {
        match bits {
            16 => (value >> 8) as u8,
            8 => value as u8,
            _ => (value * 255 / levels) as u8,
        }
    };
    let channels = layout.color.channels();
    for (x, target) in row.chunks_exact_mut(channels).enumerate() {
        let base = x * layout.samples;
        let alpha = layout.alpha.map(|at| eight(sample(base + at)));
        match layout.photometric {
            0 => target[0] = 255 - eight(sample(base)),
            1 => target[0] = eight(sample(base)),
            3 => {
                let index = (sample(base) as usize).min(layout.palette.len() - 1);
                target[..3].copy_from_slice(&layout.palette[index]);
            }
            5 => {
                let black = 255 - u32::from(eight(sample(base + 3)));
                for channel in 0..3 {
                    let ink = 255 - u32::from(eight(sample(base + channel)));
                    target[channel] = ((ink * black + 127) / 255) as u8;
                }
            }
            _ => {
                for channel in 0..3 {
                    target[channel] = eight(sample(base + channel));
                }
            }
        }
        if let Some(alpha) = alpha {
            let color = channels - 1;
            target[color] = alpha;
            if layout.associated {
                for value in &mut target[..color] {
                    *value = if alpha == 0 {
                        0
                    } else {
                        ((u32::from(*value) * 255 + u32::from(alpha) / 2) / u32::from(alpha))
                            .min(255) as u8
                    };
                }
            }
        }
    }
    let _ = layout.color_samples;
}

/// Decodes the first page into `sink`, a band of rows (a strip, or a row
/// of tiles) at a time. The file itself is held: its data can lie
/// anywhere in it.
fn decode(bytes: &[u8], sink: &mut dyn RowSink) -> Result<TiffNotes, RowsError> {
    let fail = |failure: TiffError| RowsError::Png(PngError(failure.0));
    let (file, tags, other_pages) = read_tags(bytes).map_err(fail)?;
    let layout = layout(&tags, file.big).map_err(fail)?;
    sink.start(layout.width as u32, layout.height as u32, layout.color)
        .map_err(RowsError::Io)?;
    let planar = tags.planar == 2 && layout.samples > 1;
    let plane_samples = if planar { 1 } else { layout.samples };
    let planes = if planar { layout.samples } else { 1 };
    let bits = layout.bits as usize;
    let tiled = tags.tile_width > 0 && tags.tile_height > 0;
    let (chunk_width, chunk_height) = if tiled {
        (tags.tile_width as usize, tags.tile_height as usize)
    } else {
        (
            layout.width,
            (tags.rows_per_strip as usize).min(layout.height).max(1),
        )
    };
    let across = layout.width.div_ceil(chunk_width);
    let down = layout.height.div_ceil(chunk_height);
    let per_plane = across * down;
    if tags.offsets.len() < per_plane * planes || tags.counts.len() < tags.offsets.len() {
        return Err(fail(error(
            "TIFF lists fewer strips or tiles than the image needs",
        )));
    }
    let chunk_row_bytes = (chunk_width * plane_samples * bits).div_ceil(8);
    let chunk = |index: usize| -> Result<Vec<u8>, TiffError> {
        let start = tags.offsets[index] as usize;
        let end = start
            .checked_add(tags.counts[index] as usize)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| error("TIFF strip or tile lies outside the file"))?;
        // The last strip may be short.
        let rows = if tiled {
            chunk_height
        } else {
            chunk_height.min(layout.height - (index % per_plane) * chunk_height)
        };
        let mut data = decompress(&tags, &bytes[start..end], chunk_row_bytes * rows)?;
        if tags.predictor == 2 {
            for row in data.chunks_exact_mut(chunk_row_bytes) {
                unpredict(row, plane_samples, layout.bits, file.big);
            }
        }
        Ok(data)
    };
    let stored_row_bytes = (layout.width * layout.samples * bits).div_ceil(8);
    let mut stored = vec![0u8; stored_row_bytes];
    let mut row = vec![0u8; layout.width * layout.color.channels()];
    let sample_bytes = bits / 8;
    for band in 0..down {
        // Each chunk of the band, for each plane: [plane][column].
        let mut chunks = Vec::with_capacity(planes * across);
        for plane in 0..planes {
            for column in 0..across {
                chunks.push(chunk(plane * per_plane + band * across + column).map_err(fail)?);
            }
        }
        let band_rows = chunk_height.min(layout.height - band * chunk_height);
        for line in 0..band_rows {
            if !planar && across == 1 {
                stored.copy_from_slice(
                    &chunks[0][line * chunk_row_bytes..line * chunk_row_bytes + stored_row_bytes],
                );
            } else if !planar {
                // Tiles side by side: whole bytes at 8 and 16 bits.
                for column in 0..across {
                    let x0 = column * chunk_width;
                    let pixels = chunk_width.min(layout.width - x0);
                    let size = pixels * layout.samples * sample_bytes.max(1);
                    let from = &chunks[column][line * chunk_row_bytes..][..size];
                    let at = x0 * layout.samples * sample_bytes.max(1);
                    if bits < 8 {
                        return Err(fail(error("tiled TIFF under 8 bits is not supported")));
                    }
                    stored[at..at + size].copy_from_slice(from);
                }
            } else {
                if bits < 8 {
                    return Err(fail(error("planar TIFF under 8 bits is not supported")));
                }
                for plane in 0..planes {
                    for column in 0..across {
                        let x0 = column * chunk_width;
                        let pixels = chunk_width.min(layout.width - x0);
                        let from = &chunks[plane * across + column][line * chunk_row_bytes..];
                        for x in 0..pixels {
                            let target = ((x0 + x) * layout.samples + plane) * sample_bytes;
                            stored[target..target + sample_bytes]
                                .copy_from_slice(&from[x * sample_bytes..(x + 1) * sample_bytes]);
                        }
                    }
                }
            }
            convert(&layout, &stored, &mut row);
            sink.row(&row).map_err(RowsError::Io)?;
        }
    }
    Ok(TiffNotes {
        other_pages,
        sixteen_bit: layout.bits == 16,
        cmyk: layout.photometric == 5,
    })
}

/// Reads the first page of a TIFF into `sink`.
pub fn read_tiff_rows(
    reader: &mut dyn Read,
    sink: &mut dyn RowSink,
) -> Result<TiffNotes, RowsError> {
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).map_err(RowsError::Io)?;
    decode(&bytes, sink)
}

/// Reads the first page of a TIFF into an image.
pub fn read_tiff(bytes: &[u8]) -> Result<Image, TiffError> {
    let mut sink = Collect::default();
    match decode(bytes, &mut sink) {
        Ok(_) => Ok(sink.image),
        Err(RowsError::Png(failure)) => Err(TiffError(failure.0)),
        Err(RowsError::Io(failure)) => Err(TiffError(format!("row sink failed: {failure}"))),
    }
}

// ------------------------------------------------------------ the writer

/// Strips hold about this many bytes of pixels before compression.
const STRIP_BYTES: usize = 256 * 1024;

/// Writes a TIFF from rows as a `RowSink`: 8-bit samples in strips,
/// deflate (zlib) with the horizontal predictor, little-endian, the IFD
/// ahead of the data. The compressed strips are held until the last row
/// (their offsets go in the IFD); the pixels are not.
pub struct TiffRows<'a> {
    sink: &'a mut dyn Write,
    width: u32,
    height: u32,
    color: ColorType,
    rows_per_strip: usize,
    pending: Vec<u8>,
    strips: Vec<Vec<u8>>,
    rows_seen: usize,
}

impl<'a> TiffRows<'a> {
    pub fn new(sink: &'a mut dyn Write) -> TiffRows<'a> {
        TiffRows {
            sink,
            width: 0,
            height: 0,
            color: ColorType::Rgb,
            rows_per_strip: 1,
            pending: Vec::new(),
            strips: Vec::new(),
            rows_seen: 0,
        }
    }

    fn close_strip(&mut self) {
        let mut out = vec![0x78, 0x9c];
        deflate(&self.pending, &mut out);
        out.extend_from_slice(&adler32(&self.pending).to_be_bytes());
        self.strips.push(out);
        self.pending.clear();
    }

    fn finish(&mut self) -> io::Result<()> {
        let samples = self.color.channels() as u32;
        let alpha = matches!(self.color, ColorType::GrayAlpha | ColorType::Rgba);
        let photometric = if matches!(self.color, ColorType::Gray | ColorType::GrayAlpha) {
            1
        } else {
            2
        };
        let strip_count = self.strips.len() as u32;
        // (tag, type, count, values): SHORT 3, LONG 4, RATIONAL 5.
        let mut entries: Vec<(u16, u16, Vec<u32>)> = vec![
            (256, 4, vec![self.width]),
            (257, 4, vec![self.height]),
            (258, 3, vec![8; samples as usize]),
            (259, 3, vec![8]),
            (262, 3, vec![photometric]),
            (273, 4, vec![0; strip_count as usize]),
            (277, 3, vec![samples]),
            (278, 4, vec![self.rows_per_strip as u32]),
            (
                279,
                4,
                self.strips.iter().map(|strip| strip.len() as u32).collect(),
            ),
            (282, 5, vec![72, 1]),
            (283, 5, vec![72, 1]),
            (284, 3, vec![1]),
            (296, 3, vec![2]),
            (317, 3, vec![2]),
        ];
        if alpha {
            entries.push((338, 3, vec![2]));
        }
        let ifd_size = 2 + 12 * entries.len() + 4;
        let size_of = |kind: u16, values: &[u32]| match kind {
            3 => values.len() * 2,
            5 => values.len() * 4,
            _ => values.len() * 4,
        };
        let mut extra_at = 8 + ifd_size;
        let mut extras: Vec<Option<usize>> = Vec::new();
        for (_, kind, values) in &entries {
            let size = size_of(*kind, values);
            if size > 4 {
                extras.push(Some(extra_at));
                extra_at += size + (size & 1);
            } else {
                extras.push(None);
            }
        }
        let mut data_at = extra_at;
        let strip_offsets: Vec<u32> = self
            .strips
            .iter()
            .map(|strip| {
                let at = data_at as u32;
                data_at += strip.len();
                at
            })
            .collect();
        entries[5].2 = strip_offsets;
        let put_values = |out: &mut Vec<u8>, kind: u16, values: &[u32]| {
            for value in values {
                match kind {
                    3 => out.extend_from_slice(&(*value as u16).to_le_bytes()),
                    _ => out.extend_from_slice(&value.to_le_bytes()),
                }
            }
        };
        let mut out = Vec::with_capacity(extra_at);
        out.extend_from_slice(b"II");
        out.extend_from_slice(&42u16.to_le_bytes());
        out.extend_from_slice(&8u32.to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        for ((tag, kind, values), extra) in entries.iter().zip(&extras) {
            out.extend_from_slice(&tag.to_le_bytes());
            out.extend_from_slice(&kind.to_le_bytes());
            let count = if *kind == 5 {
                values.len() / 2
            } else {
                values.len()
            };
            out.extend_from_slice(&(count as u32).to_le_bytes());
            match extra {
                Some(at) => out.extend_from_slice(&(*at as u32).to_le_bytes()),
                None => {
                    let start = out.len();
                    put_values(&mut out, *kind, values);
                    out.resize(start + 4, 0);
                }
            }
        }
        out.extend_from_slice(&0u32.to_le_bytes());
        for ((_, kind, values), extra) in entries.iter().zip(&extras) {
            if extra.is_some() {
                put_values(&mut out, *kind, values);
                if out.len() % 2 == 1 {
                    out.push(0);
                }
            }
        }
        self.sink.write_all(&out)?;
        for strip in &self.strips {
            self.sink.write_all(strip)?;
        }
        self.strips.clear();
        self.sink.flush()
    }
}

impl RowSink for TiffRows<'_> {
    fn start(&mut self, width: u32, height: u32, color: ColorType) -> io::Result<()> {
        self.width = width;
        self.height = height;
        self.color = color;
        let row_bytes = width as usize * color.channels();
        self.rows_per_strip = (STRIP_BYTES / row_bytes.max(1)).clamp(1, height as usize);
        self.pending = Vec::with_capacity(self.rows_per_strip * row_bytes);
        Ok(())
    }

    fn row(&mut self, pixels: &[u8]) -> io::Result<()> {
        let samples = self.color.channels();
        // The horizontal predictor, each sample less its left neighbor,
        // read from the source row so the loop runs forward (in place it
        // had to run backward and did not vectorize).
        let start = self.pending.len();
        self.pending.resize(start + pixels.len(), 0);
        let row = &mut self.pending[start..];
        let head = samples.min(pixels.len());
        row[..head].copy_from_slice(&pixels[..head]);
        for ((target, current), left) in row[head..].iter_mut().zip(&pixels[head..]).zip(pixels) {
            *target = current.wrapping_sub(*left);
        }
        self.rows_seen += 1;
        if self.rows_seen % self.rows_per_strip == 0 || self.rows_seen == self.height as usize {
            self.close_strip();
        }
        if self.rows_seen == self.height as usize {
            self.finish()?;
        }
        Ok(())
    }
}

/// Writes a whole image as TIFF.
pub fn write_tiff(image: &Image, sink: &mut dyn Write) -> io::Result<()> {
    let mut rows = TiffRows::new(sink);
    rows.start(image.width, image.height, image.color)?;
    for y in 0..image.height {
        rows.row(image.row(y))?;
    }
    Ok(())
}
