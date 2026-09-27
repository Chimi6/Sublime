//! Truevision TGA: an 18-byte header, an optional ID field and color map,
//! then pixels raw or run-length encoded, bottom-up unless the header
//! says otherwise. Read into the image hub from every common layout and
//! written from it as top-down RLE with the TGA 2.0 footer.

use std::io::{self, Read, Write};

use crate::image::{ColorType, Image, MAX_PIXELS};
use crate::io::png::{Collect, PngError, RowSink, RowsError};

/// Input read and output written in pieces this large.
const PIECE: usize = 64 * 1024;
/// The TGA 2.0 footer with no extension or developer area.
const FOOTER: &[u8; 26] = b"\0\0\0\0\0\0\0\0TRUEVISION-XFILE.\0";
const TOP_DOWN: u8 = 0x20;
const RIGHT_TO_LEFT: u8 = 0x10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TgaError(pub String);

impl std::fmt::Display for TgaError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl std::error::Error for TgaError {}

fn fail<T>(message: &str) -> Result<T, RowsError> {
    Err(RowsError::Png(PngError(message.to_string())))
}

/// The input, read a piece at a time.
struct Source<'a> {
    reader: &'a mut dyn Read,
    buffer: Vec<u8>,
    at: usize,
    end: usize,
}

impl Source<'_> {
    /// Fills `target` from the input: buffered bytes first, then whole
    /// pieces.
    fn fill(&mut self, target: &mut [u8]) -> Result<(), RowsError> {
        let mut done = 0;
        while done < target.len() {
            if self.at == self.end {
                match self.reader.read(&mut self.buffer) {
                    Ok(0) => return fail("TGA data cut short"),
                    Ok(count) => {
                        self.at = 0;
                        self.end = count;
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(RowsError::Io(error)),
                }
            }
            let take = (self.end - self.at).min(target.len() - done);
            target[done..done + take].copy_from_slice(&self.buffer[self.at..self.at + take]);
            self.at += take;
            done += take;
        }
        Ok(())
    }

    fn byte(&mut self) -> Result<u8, RowsError> {
        let mut one = [0u8];
        self.fill(&mut one)?;
        Ok(one[0])
    }
}

/// How a stored pixel becomes the hub's.
#[derive(Clone, Copy)]
enum Pixel {
    /// 8-bit gray.
    Gray,
    /// 16-bit gray and alpha.
    GrayAlpha,
    /// An index into the color map.
    Index,
    /// 15 or 16 bits, five a channel; with alpha, bit 15 set is opaque.
    Packed { alpha: bool },
    /// Blue, green, red.
    Bgr,
    /// Blue, green, red, and a fourth byte that is alpha only when the
    /// header gives alpha bits.
    Bgra { alpha: bool },
}

#[inline]
fn expand(five: u16) -> u8 {
    let five = (five & 31) as u8;
    (five << 3) | (five >> 2)
}

/// The hub's channels for a packed 16-bit value.
fn unpack(value: u16, alpha: bool, target: &mut [u8]) {
    target[0] = expand(value >> 10);
    target[1] = expand(value >> 5);
    target[2] = expand(value);
    if alpha {
        target[3] = if value & 0x8000 != 0 { 255 } else { 0 };
    }
}

struct Layout {
    width: u32,
    height: u32,
    stored: usize,
    pixel: Pixel,
    color: ColorType,
    rle: bool,
    top_down: bool,
    right_to_left: bool,
    /// The color map as the hub's pixels, and the index of its first.
    map: Vec<u8>,
    map_first: usize,
}

fn read_layout(source: &mut Source<'_>) -> Result<Layout, RowsError> {
    let mut header = [0u8; 18];
    source.fill(&mut header)?;
    let id_length = usize::from(header[0]);
    let map_type = header[1];
    let image_type = header[2];
    let map_first = usize::from(u16::from_le_bytes([header[3], header[4]]));
    let map_length = usize::from(u16::from_le_bytes([header[5], header[6]]));
    let map_depth = header[7];
    let width = u32::from(u16::from_le_bytes([header[12], header[13]]));
    let height = u32::from(u16::from_le_bytes([header[14], header[15]]));
    let depth = header[16];
    let descriptor = header[17];
    let alpha_bits = descriptor & 0x0f;
    if width == 0 || height == 0 {
        return fail("TGA image has no pixels");
    }
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return fail("TGA image too large");
    }
    if map_type > 1 {
        return fail("not a TGA file");
    }
    let mut skipped = vec![0u8; id_length];
    source.fill(&mut skipped)?;
    // The color map, as the hub's pixels, whatever the image type.
    let (mut map, mut map_color) = (Vec::new(), ColorType::Rgb);
    if map_type == 1 {
        let (entry, color) = match map_depth {
            15 | 16 => (2, ColorType::Rgb),
            24 => (3, ColorType::Rgb),
            32 => (4, ColorType::Rgba),
            _ => return fail("TGA color map depth not supported"),
        };
        let mut raw = vec![0u8; map_length * entry];
        source.fill(&mut raw)?;
        let channels = color.channels();
        map = vec![0u8; map_length * channels];
        for (target, cell) in map.chunks_exact_mut(channels).zip(raw.chunks_exact(entry)) {
            match entry {
                2 => unpack(u16::from_le_bytes([cell[0], cell[1]]), false, target),
                3 => target.copy_from_slice(&[cell[2], cell[1], cell[0]]),
                _ => target.copy_from_slice(&[cell[2], cell[1], cell[0], cell[3]]),
            }
        }
        map_color = color;
    }
    let (pixel, color, stored) = match (image_type & 7, depth) {
        (1, 8) if map_type == 1 => (Pixel::Index, map_color, 1),
        (3, 8) => (Pixel::Gray, ColorType::Gray, 1),
        (3, 16) => (Pixel::GrayAlpha, ColorType::GrayAlpha, 2),
        (2, 15) => (Pixel::Packed { alpha: false }, ColorType::Rgb, 2),
        (2, 16) if alpha_bits > 0 => (Pixel::Packed { alpha: true }, ColorType::Rgba, 2),
        (2, 16) => (Pixel::Packed { alpha: false }, ColorType::Rgb, 2),
        (2, 24) => (Pixel::Bgr, ColorType::Rgb, 3),
        (2, 32) if alpha_bits > 0 => (Pixel::Bgra { alpha: true }, ColorType::Rgba, 4),
        (2, 32) => (Pixel::Bgra { alpha: false }, ColorType::Rgb, 4),
        (1 | 2 | 3, _) => return fail("TGA pixel depth not supported"),
        _ => return fail("not a TGA file"),
    };
    if image_type & !0x0b != 0 {
        return fail("not a TGA file");
    }
    Ok(Layout {
        width,
        height,
        stored,
        pixel,
        color,
        rle: image_type & 8 != 0,
        top_down: descriptor & TOP_DOWN != 0,
        right_to_left: descriptor & RIGHT_TO_LEFT != 0,
        map,
        map_first,
    })
}

/// Converts one row of stored pixels into the hub's.
fn convert(layout: &Layout, stored: &[u8], row: &mut [u8]) -> Result<(), RowsError> {
    let channels = layout.color.channels();
    let cells = row
        .chunks_exact_mut(channels)
        .zip(stored.chunks_exact(layout.stored));
    match layout.pixel {
        Pixel::Gray | Pixel::GrayAlpha => row.copy_from_slice(stored),
        Pixel::Bgr => {
            for (target, cell) in cells {
                target.copy_from_slice(&[cell[2], cell[1], cell[0]]);
            }
        }
        Pixel::Bgra { alpha } => {
            for (target, cell) in cells {
                target[..3].copy_from_slice(&[cell[2], cell[1], cell[0]]);
                if alpha {
                    target[3] = cell[3];
                }
            }
        }
        Pixel::Packed { alpha } => {
            for (target, cell) in cells {
                unpack(u16::from_le_bytes([cell[0], cell[1]]), alpha, target);
            }
        }
        Pixel::Index => {
            for (target, cell) in cells {
                let index = usize::from(cell[0])
                    .checked_sub(layout.map_first)
                    .filter(|index| (index + 1) * channels <= layout.map.len())
                    .ok_or_else(|| {
                        RowsError::Png(PngError("TGA color index outside the map".into()))
                    })?;
                target.copy_from_slice(&layout.map[index * channels..(index + 1) * channels]);
            }
        }
    }
    if layout.right_to_left {
        let pixels: Vec<u8> = row
            .chunks_exact(channels)
            .rev()
            .flatten()
            .copied()
            .collect();
        row.copy_from_slice(&pixels);
    }
    Ok(())
}

/// Repeats an `N`-byte pixel over `span` (a constant width, so each copy
/// is a store rather than a call).
fn repeat<const N: usize>(span: &mut [u8], pixel: &[u8; 4]) {
    let value: [u8; N] = pixel[..N].try_into().unwrap_or([0; N]);
    for cell in span.chunks_exact_mut(N) {
        cell.copy_from_slice(&value);
    }
}

/// RLE state carried across rows: a packet may continue into the next.
struct Packets {
    /// Pixels left in the current packet.
    left: usize,
    /// The current packet repeats one pixel.
    run: bool,
    pixel: [u8; 4],
}

impl Packets {
    /// Fills one row of stored pixels, a packet's span at a time: raw
    /// pixels are read in one piece, a run is repeated in place.
    fn row(
        &mut self,
        source: &mut Source<'_>,
        stored: usize,
        target: &mut [u8],
    ) -> Result<(), RowsError> {
        let mut at = 0;
        while at < target.len() {
            if self.left == 0 {
                let head = source.byte()?;
                self.left = usize::from(head & 0x7f) + 1;
                self.run = head & 0x80 != 0;
                if self.run {
                    source.fill(&mut self.pixel[..stored])?;
                }
            }
            let span = self.left.min((target.len() - at) / stored);
            let bytes = span * stored;
            if self.run {
                let span = &mut target[at..at + bytes];
                match stored {
                    1 => span.fill(self.pixel[0]),
                    2 => repeat::<2>(span, &self.pixel),
                    3 => repeat::<3>(span, &self.pixel),
                    _ => repeat::<4>(span, &self.pixel),
                }
            } else {
                source.fill(&mut target[at..at + bytes])?;
            }
            self.left -= span;
            at += bytes;
        }
        Ok(())
    }
}

/// Reads a TGA into `sink` one row at a time. A top-down file streams;
/// a bottom-up one (the default) is held once, its first row being the
/// image's last.
pub fn read_tga_rows(reader: &mut dyn Read, sink: &mut dyn RowSink) -> Result<(), RowsError> {
    let mut source = Source {
        reader,
        buffer: vec![0; PIECE],
        at: 0,
        end: 0,
    };
    let layout = read_layout(&mut source)?;
    sink.start(layout.width, layout.height, layout.color)
        .map_err(RowsError::Io)?;
    let width = layout.width as usize;
    let row_length = width * layout.color.channels();
    let mut row = vec![0u8; row_length];
    if !layout.top_down {
        return read_bottom_up(source, &layout, sink, &mut row);
    }
    let mut stored = vec![0u8; width * layout.stored];
    let mut packets = Packets {
        left: 0,
        run: false,
        pixel: [0; 4],
    };
    for _ in 0..layout.height {
        if layout.rle {
            packets.row(&mut source, layout.stored, &mut stored)?;
        } else {
            source.fill(&mut stored)?;
        }
        convert(&layout, &stored, &mut row)?;
        sink.row(&row).map_err(RowsError::Io)?;
    }
    if packets.left > 0 {
        return fail("TGA run-length packet runs past the image");
    }
    Ok(())
}

/// Where a row's pixels start in held RLE data: the byte, and the packet
/// in progress there.
#[derive(Clone, Copy)]
struct RowStart {
    at: usize,
    left: usize,
    run: bool,
    pixel: [u8; 4],
}

/// A bottom-up file, whose first row is the image's last: the raster's
/// bytes are held (for RLE, with where each row starts, found in one
/// pass over the packet headers) and the rows decoded last first. The
/// file is held, not the pixels: a flat image's RLE is a fraction of
/// them.
fn read_bottom_up(
    source: Source<'_>,
    layout: &Layout,
    sink: &mut dyn RowSink,
    row: &mut [u8],
) -> Result<(), RowsError> {
    let mut bytes = source.buffer[source.at..source.end].to_vec();
    source
        .reader
        .read_to_end(&mut bytes)
        .map_err(RowsError::Io)?;
    let stored = layout.stored;
    let stored_row = layout.width as usize * stored;
    let height = layout.height as usize;
    let cut = || RowsError::Png(PngError("TGA data cut short".into()));
    if !layout.rle {
        if bytes.len() < stored_row * height {
            return Err(cut());
        }
        for index in (0..height).rev() {
            convert(
                layout,
                &bytes[index * stored_row..(index + 1) * stored_row],
                row,
            )?;
            sink.row(row).map_err(RowsError::Io)?;
        }
        return Ok(());
    }
    let mut starts = Vec::with_capacity(height);
    let mut state = RowStart {
        at: 0,
        left: 0,
        run: false,
        pixel: [0; 4],
    };
    for _ in 0..height {
        starts.push(state);
        let mut remaining = layout.width as usize;
        while remaining > 0 {
            if state.left == 0 {
                let head = *bytes.get(state.at).ok_or_else(cut)?;
                state.at += 1;
                state.left = usize::from(head & 0x7f) + 1;
                state.run = head & 0x80 != 0;
                if state.run {
                    let pixel = bytes.get(state.at..state.at + stored).ok_or_else(cut)?;
                    state.pixel[..stored].copy_from_slice(pixel);
                    state.at += stored;
                }
            }
            let span = state.left.min(remaining);
            if !state.run {
                state.at += span * stored;
            }
            state.left -= span;
            remaining -= span;
        }
    }
    if state.at > bytes.len() {
        return Err(cut());
    }
    if state.left > 0 {
        return fail("TGA run-length packet runs past the image");
    }
    let mut decoded = vec![0u8; stored_row];
    for start in starts.iter().rev() {
        let mut state = *start;
        let mut at = 0;
        while at < stored_row {
            if state.left == 0 {
                let head = bytes[state.at];
                state.at += 1;
                state.left = usize::from(head & 0x7f) + 1;
                state.run = head & 0x80 != 0;
                if state.run {
                    state.pixel[..stored].copy_from_slice(&bytes[state.at..state.at + stored]);
                    state.at += stored;
                }
            }
            let span = state.left.min((stored_row - at) / stored);
            let length = span * stored;
            let target = &mut decoded[at..at + length];
            if state.run {
                match stored {
                    1 => target.fill(state.pixel[0]),
                    2 => repeat::<2>(target, &state.pixel),
                    3 => repeat::<3>(target, &state.pixel),
                    _ => repeat::<4>(target, &state.pixel),
                }
            } else {
                target.copy_from_slice(&bytes[state.at..state.at + length]);
                state.at += length;
            }
            state.left -= span;
            at += length;
        }
        convert(layout, &decoded, row)?;
        sink.row(row).map_err(RowsError::Io)?;
    }
    Ok(())
}

/// Reads a whole TGA into an image.
pub fn read_tga(bytes: &[u8]) -> Result<Image, TgaError> {
    let mut sink = Collect::default();
    match read_tga_rows(&mut &bytes[..], &mut sink) {
        Ok(()) => Ok(sink.image),
        Err(RowsError::Png(error)) => Err(TgaError(error.0)),
        Err(RowsError::Io(error)) => Err(TgaError(format!("row sink failed: {error}"))),
    }
}

/// Writes TGA from rows as a `RowSink`: top-down, run-length encoded with
/// packets that end at each row (Pillow's layout), and the 2.0 footer.
pub struct TgaRows<'a> {
    sink: &'a mut dyn Write,
    color: ColorType,
    rows_left: u32,
    stored: Vec<u8>,
    out: Vec<u8>,
}

impl<'a> TgaRows<'a> {
    pub fn new(sink: &'a mut dyn Write) -> TgaRows<'a> {
        TgaRows {
            sink,
            color: ColorType::Rgb,
            rows_left: 0,
            stored: Vec::new(),
            out: Vec::with_capacity(PIECE + 1024),
        }
    }
}

impl RowSink for TgaRows<'_> {
    fn start(&mut self, width: u32, height: u32, color: ColorType) -> io::Result<()> {
        if width > 65_535 || height > 65_535 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "TGA holds at most 65535 pixels on a side",
            ));
        }
        self.color = color;
        self.rows_left = height;
        let (image_type, depth, alpha_bits) = match color {
            ColorType::Gray => (11, 8, 0),
            ColorType::GrayAlpha => (11, 16, 8),
            ColorType::Rgb => (10, 24, 0),
            ColorType::Rgba => (10, 32, 8),
        };
        let mut header = [0u8; 18];
        header[2] = image_type;
        header[12..14].copy_from_slice(&(width as u16).to_le_bytes());
        header[14..16].copy_from_slice(&(height as u16).to_le_bytes());
        header[16] = depth;
        header[17] = alpha_bits | TOP_DOWN;
        self.out.extend_from_slice(&header);
        self.stored = vec![0u8; width as usize * color.channels()];
        Ok(())
    }

    fn row(&mut self, pixels: &[u8]) -> io::Result<()> {
        let size = self.color.channels();
        match self.color {
            ColorType::Gray | ColorType::GrayAlpha => self.stored.copy_from_slice(pixels),
            ColorType::Rgb | ColorType::Rgba => {
                for (target, cell) in self
                    .stored
                    .chunks_exact_mut(size)
                    .zip(pixels.chunks_exact(size))
                {
                    target.copy_from_slice(cell);
                    target.swap(0, 2);
                }
            }
        }
        encode_row(&self.stored, size, &mut self.out);
        self.rows_left -= 1;
        if self.rows_left == 0 {
            self.out.extend_from_slice(FOOTER);
        }
        if self.out.len() >= PIECE || self.rows_left == 0 {
            self.sink.write_all(&self.out)?;
            self.out.clear();
        }
        if self.rows_left == 0 {
            self.sink.flush()?;
        }
        Ok(())
    }
}

/// Run-length encodes one row: a run packet for two or more equal
/// pixels, a raw packet up to the next such pair, 128 pixels at most.
fn encode_row(row: &[u8], size: usize, out: &mut Vec<u8>) {
    let count = row.len() / size;
    let pixel = |index: usize| &row[index * size..(index + 1) * size];
    let mut at = 0;
    while at < count {
        let mut run = 1;
        while at + run < count && run < 128 && pixel(at + run) == pixel(at) {
            run += 1;
        }
        if run >= 2 {
            out.push(0x80 | (run - 1) as u8);
            out.extend_from_slice(pixel(at));
            at += run;
            continue;
        }
        let mut end = at + 1;
        while end < count && end - at < 128 && !(end + 1 < count && pixel(end) == pixel(end + 1)) {
            end += 1;
        }
        out.push((end - at - 1) as u8);
        out.extend_from_slice(&row[at * size..end * size]);
        at = end;
    }
}

/// Writes a whole image as TGA.
pub fn write_tga(image: &Image, sink: &mut dyn Write) -> io::Result<()> {
    let mut rows = TgaRows::new(sink);
    rows.start(image.width, image.height, image.color)?;
    for y in 0..image.height {
        rows.row(image.row(y))?;
    }
    Ok(())
}
