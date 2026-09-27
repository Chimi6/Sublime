//! Windows icons (ICO) and cursors (CUR): a directory of entries, each a
//! PNG or a headerless BMP (a DIB with an AND mask for transparency).
//! Read: the largest, deepest entry into the image hub. Written: an icon
//! of PNG entries at the standard sizes that fit the source, downscaled
//! by area averaging, with the source itself as the largest when it is
//! 256 pixels or less on a side.

use std::io::{self, Read, Write};

use crate::image::resize::Downscale;
use crate::image::{ColorType, Image};
use crate::io::png::{PngError, RowSink, RowsError, read_png, write_png};

const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
/// The sizes an icon holds, as Windows and Pillow use them.
const SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IcoError(pub String);

impl std::fmt::Display for IcoError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl std::error::Error for IcoError {}

/// What reading left out: the other entries, and a cursor's hotspot.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct IcoNotes {
    pub others: usize,
    pub cursor: bool,
}

fn error(message: &str) -> IcoError {
    IcoError(message.to_string())
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// One directory entry.
struct Entry {
    width: u32,
    height: u32,
    bits: u16,
    data: std::ops::Range<usize>,
}

/// The directory: whether it is a cursor, and its entries in order.
fn directory(bytes: &[u8]) -> Result<(bool, Vec<Entry>), IcoError> {
    if bytes.len() < 6 || u16_at(bytes, 0) != 0 {
        return Err(error("not an icon"));
    }
    let cursor = match u16_at(bytes, 2) {
        1 => false,
        2 => true,
        _ => return Err(error("not an icon")),
    };
    let count = usize::from(u16_at(bytes, 4));
    if count == 0 {
        return Err(error("icon has no images"));
    }
    if bytes.len() < 6 + 16 * count {
        return Err(error("icon directory cut short"));
    }
    let mut entries = Vec::with_capacity(count);
    for index in 0..count {
        let at = 6 + 16 * index;
        let size = u32_at(bytes, at + 8) as usize;
        let offset = u32_at(bytes, at + 12) as usize;
        let end = offset
            .checked_add(size)
            .filter(|end| size > 0 && *end <= bytes.len())
            .ok_or_else(|| error("icon entry lies outside the file"))?;
        let side = |byte: u8| if byte == 0 { 256 } else { u32::from(byte) };
        entries.push(Entry {
            width: side(bytes[at]),
            height: side(bytes[at + 1]),
            // A cursor keeps its hotspot here; its depth is in the data.
            bits: if cursor { 0 } else { u16_at(bytes, at + 6) },
            data: offset..end,
        });
    }
    Ok((cursor, entries))
}

/// The directory's entry sizes, in order.
pub fn entries(bytes: &[u8]) -> Result<Vec<(u32, u32)>, IcoError> {
    Ok(directory(bytes)?
        .1
        .iter()
        .map(|entry| (entry.width, entry.height))
        .collect())
}

/// Decodes a headerless BMP entry: 1, 4, 8, 16, 24, or 32 bits, rows
/// bottom-up, then the AND mask (1 transparent). A 32-bit entry carries
/// alpha unless every alpha byte is zero, when the mask decides.
fn read_dib(data: &[u8]) -> Result<Image, IcoError> {
    let short = || error("icon bitmap cut short");
    if data.len() < 40 {
        return Err(short());
    }
    let header_size = u32_at(data, 0) as usize;
    let width = u32_at(data, 4) as i32;
    let doubled = u32_at(data, 8) as i32;
    let bits = u16_at(data, 14);
    let compression = u32_at(data, 16);
    let colors_used = u32_at(data, 32) as usize;
    if header_size < 40 || width <= 0 || width > 1024 || doubled.unsigned_abs() < 2 {
        return Err(error("icon bitmap header is not valid"));
    }
    if compression != 0 {
        return Err(error("compressed icon bitmaps are not supported"));
    }
    let (width, height) = (width as usize, (doubled.unsigned_abs() / 2) as usize);
    let palette_entries = match bits {
        1 | 4 | 8 if colors_used > 0 => colors_used.min(1 << bits),
        1 | 4 | 8 => 1 << bits,
        16 | 24 | 32 => 0,
        _ => return Err(error("icon bitmap depth is not supported")),
    };
    let palette_at = header_size;
    let pixels_at = palette_at + palette_entries * 4;
    let stride = (width * usize::from(bits)).div_ceil(32) * 4;
    let mask_stride = width.div_ceil(32) * 4;
    let mask_at = pixels_at + stride * height;
    if data.len() < mask_at {
        return Err(short());
    }
    let palette: Vec<[u8; 3]> = (0..palette_entries)
        .map(|index| {
            let at = palette_at + index * 4;
            [data[at + 2], data[at + 1], data[at]]
        })
        .collect();
    let has_mask = data.len() >= mask_at + mask_stride * height;
    let mut pixels = vec![0u8; width * height * 4];
    let mut any_alpha = false;
    for y in 0..height {
        // Bottom-up unless the height is negative.
        let stored = if doubled > 0 { height - 1 - y } else { y };
        let row = &data[pixels_at + stored * stride..pixels_at + (stored + 1) * stride];
        for x in 0..width {
            let target = &mut pixels[(y * width + x) * 4..(y * width + x + 1) * 4];
            let color = match bits {
                1 | 4 | 8 => {
                    let bit = x * usize::from(bits);
                    let byte = row[bit / 8];
                    let index = usize::from(
                        (byte >> (8 - usize::from(bits) - bit % 8)) & ((1 << bits) - 1) as u8,
                    );
                    *palette
                        .get(index)
                        .ok_or_else(|| error("icon palette index out of range"))?
                }
                16 => {
                    let value = u16_at(row, x * 2);
                    let five = |shift: u16| {
                        let v = ((value >> shift) & 31) as u8;
                        (v << 3) | (v >> 2)
                    };
                    [five(10), five(5), five(0)]
                }
                24 => [row[x * 3 + 2], row[x * 3 + 1], row[x * 3]],
                _ => {
                    target[3] = row[x * 4 + 3];
                    any_alpha |= target[3] != 0;
                    [row[x * 4 + 2], row[x * 4 + 1], row[x * 4]]
                }
            };
            target[..3].copy_from_slice(&color);
        }
    }
    if bits != 32 || !any_alpha {
        for y in 0..height {
            let stored = if doubled > 0 { height - 1 - y } else { y };
            for x in 0..width {
                let transparent = has_mask && {
                    let byte = data[mask_at + stored * mask_stride + x / 8];
                    byte & (0x80 >> (x % 8)) != 0
                };
                pixels[(y * width + x) * 4 + 3] = if transparent { 0 } else { 255 };
            }
        }
    }
    Ok(Image {
        width: width as u32,
        height: height as u32,
        color: ColorType::Rgba,
        pixels,
    })
}

/// Opens an icon or cursor to its largest entry, the deepest of those.
fn open(bytes: &[u8]) -> Result<(Image, IcoNotes), IcoError> {
    let (cursor, entries) = directory(bytes)?;
    let chosen = entries
        .iter()
        .enumerate()
        .max_by_key(|(index, entry)| {
            (
                u64::from(entry.width) * u64::from(entry.height),
                entry.bits,
                *index,
            )
        })
        .map(|(_, entry)| entry)
        .ok_or_else(|| error("icon has no images"))?;
    let data = &bytes[chosen.data.clone()];
    let image = if data.starts_with(PNG_SIGNATURE) {
        read_png(data).map_err(|failure| IcoError(failure.0))?.0
    } else {
        read_dib(data)?
    };
    Ok((
        image,
        IcoNotes {
            others: entries.len() - 1,
            cursor,
        },
    ))
}

/// Reads an icon or cursor into `sink` (the file is held: icons are
/// small, and the entry to read is found from the directory).
pub fn read_ico_rows(reader: &mut dyn Read, sink: &mut dyn RowSink) -> Result<IcoNotes, RowsError> {
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).map_err(RowsError::Io)?;
    let (image, notes) = open(&bytes).map_err(|failure| RowsError::Png(PngError(failure.0)))?;
    sink.start(image.width, image.height, image.color)
        .map_err(RowsError::Io)?;
    for y in 0..image.height {
        sink.row(image.row(y)).map_err(RowsError::Io)?;
    }
    Ok(notes)
}

/// Reads a whole icon or cursor into an image.
pub fn read_ico(bytes: &[u8]) -> Result<Image, IcoError> {
    open(bytes).map(|(image, _)| image)
}

/// The entries an icon of a `width` by `height` source holds: each
/// standard size that fits within the source's longer side (the source
/// fitted into it, aspect kept), and the source itself when it fits in
/// 256; smallest first.
fn plan(width: u32, height: u32) -> Vec<(u32, u32)> {
    let longer = width.max(height);
    let mut sizes: Vec<(u32, u32)> = SIZES
        .iter()
        .filter(|size| **size <= longer)
        .map(|size| {
            let fit = |side: u32| {
                ((f64::from(side) * f64::from(*size) / f64::from(longer)).round() as u32).max(1)
            };
            (fit(width), fit(height))
        })
        .collect();
    if width <= 256 && height <= 256 && !sizes.contains(&(width, height)) {
        sizes.push((width, height));
    }
    sizes.sort_by_key(|(w, h)| (u64::from(*w) * u64::from(*h), *w));
    sizes.dedup();
    sizes
}

/// One entry being built: the source's own rows, or a downscale.
enum Building {
    Source(Image),
    Scaled(Downscale, Image),
}

/// Writes an icon from rows as a `RowSink`. Only the entries (256 on a
/// side at most) are held, never a larger source.
pub struct IcoRows<'a> {
    sink: &'a mut dyn Write,
    entries: Vec<Building>,
    rows_left: u32,
}

impl<'a> IcoRows<'a> {
    pub fn new(sink: &'a mut dyn Write) -> IcoRows<'a> {
        IcoRows {
            sink,
            entries: Vec::new(),
            rows_left: 0,
        }
    }

    fn finish(&mut self) -> io::Result<()> {
        let mut encoded = Vec::with_capacity(self.entries.len());
        for entry in &self.entries {
            let image = match entry {
                Building::Source(image) | Building::Scaled(_, image) => image,
            };
            let mut png = Vec::new();
            write_png(image, &mut png)?;
            encoded.push((image.width, image.height, png));
        }
        let mut out = Vec::new();
        out.extend_from_slice(&[0, 0, 1, 0]);
        out.extend_from_slice(&(encoded.len() as u16).to_le_bytes());
        let mut offset = 6 + 16 * encoded.len();
        for (width, height, png) in &encoded {
            out.push((*width % 256) as u8);
            out.push((*height % 256) as u8);
            out.extend_from_slice(&[0, 0, 1, 0, 32, 0]);
            out.extend_from_slice(&(png.len() as u32).to_le_bytes());
            out.extend_from_slice(&(offset as u32).to_le_bytes());
            offset += png.len();
        }
        self.sink.write_all(&out)?;
        for (_, _, png) in &encoded {
            self.sink.write_all(png)?;
        }
        self.sink.flush()
    }
}

impl RowSink for IcoRows<'_> {
    fn start(&mut self, width: u32, height: u32, color: ColorType) -> io::Result<()> {
        self.rows_left = height;
        self.entries = plan(width, height)
            .into_iter()
            .map(|(target_width, target_height)| {
                let mut image = Image::new(target_width, target_height, color);
                image.pixels.clear();
                if (target_width, target_height) == (width, height) {
                    Building::Source(image)
                } else {
                    let scale = Downscale::new(width, height, target_width, target_height, color);
                    Building::Scaled(scale, image)
                }
            })
            .collect();
        Ok(())
    }

    fn row(&mut self, pixels: &[u8]) -> io::Result<()> {
        for entry in &mut self.entries {
            match entry {
                Building::Source(image) => image.pixels.extend_from_slice(pixels),
                Building::Scaled(scale, image) => {
                    for done in scale.row(pixels) {
                        image.pixels.extend(done);
                    }
                }
            }
        }
        self.rows_left -= 1;
        if self.rows_left == 0 {
            self.finish()?;
        }
        Ok(())
    }
}

/// Writes a whole image as an icon.
pub fn write_ico(image: &Image, sink: &mut dyn Write) -> io::Result<()> {
    let mut rows = IcoRows::new(sink);
    rows.start(image.width, image.height, image.color)?;
    for y in 0..image.height {
        rows.row(image.row(y))?;
    }
    Ok(())
}
