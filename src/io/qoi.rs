//! QOI, the Quite OK Image format: a 14-byte header and a stream of
//! chunks, each a run, an index into the last 64 colors seen, a small
//! difference from the previous pixel, or a literal. Read and written
//! pixel by pixel as rows stream, exactly as the reference `qoi.h`.

use std::io::{self, Read, Write};

use crate::image::{ColorType, Image, MAX_PIXELS};
use crate::io::png::{Collect, PngError, RowSink, RowsError};

const MAGIC: &[u8; 4] = b"qoif";
const END: [u8; 8] = [0, 0, 0, 0, 0, 0, 0, 1];
const OP_RGB: u8 = 0xfe;
const OP_RGBA: u8 = 0xff;
const OP_INDEX: u8 = 0x00;
const OP_DIFF: u8 = 0x40;
const OP_LUMA: u8 = 0x80;
const OP_RUN: u8 = 0xc0;
const MASK: u8 = 0xc0;
/// Input read and output written in pieces this large.
const PIECE: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QoiError(pub String);

impl std::fmt::Display for QoiError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl std::error::Error for QoiError {}

fn fail<T>(message: &str) -> Result<T, RowsError> {
    Err(RowsError::Png(PngError(message.to_string())))
}

/// A pixel as `[r, g, b, a]`.
type Pixel = [u8; 4];

/// `(r * 3 + g * 5 + b * 7 + a * 11) % 64` in one multiply: the four
/// bytes spread into 16-bit lanes of a word whose product with the
/// weights sums them in the top byte (the qoi crate's form).
#[inline]
fn hash(pixel: Pixel) -> usize {
    let value = u64::from(u32::from_le_bytes(pixel));
    let spread = ((value & 0xff00_ff00) << 32) | (value & 0x00ff_00ff);
    (spread.wrapping_mul(0x0300_0700_0005_000b) >> 56) as usize & 63
}

/// The input, read a piece at a time into `buffer[..end]`.
struct Pieces<'a> {
    reader: &'a mut dyn Read,
    buffer: Vec<u8>,
    end: usize,
}

impl Pieces<'_> {
    /// Moves the unread bytes from `at` to the front and reads until at
    /// least `wanted` are held or the input ends. Returns the new `at`
    /// (zero).
    #[cold]
    fn top_up(&mut self, at: usize, wanted: usize) -> Result<usize, RowsError> {
        self.buffer.copy_within(at..self.end, 0);
        self.end -= at;
        while self.end < wanted {
            match self.reader.read(&mut self.buffer[self.end..]) {
                Ok(0) => break,
                Ok(count) => self.end += count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(RowsError::Io(error)),
            }
        }
        Ok(0)
    }
}

/// Reads a QOI stream into `sink` one row at a time; nothing but a row
/// is held.
pub fn read_qoi_rows(reader: &mut dyn Read, sink: &mut dyn RowSink) -> Result<(), RowsError> {
    let mut header = [0u8; 14];
    if let Err(error) = reader.read_exact(&mut header) {
        return match error.kind() {
            io::ErrorKind::UnexpectedEof => fail("QOI header cut short"),
            _ => Err(RowsError::Io(error)),
        };
    }
    if &header[..4] != MAGIC {
        return fail("not a QOI file");
    }
    let width = u32::from_be_bytes([header[4], header[5], header[6], header[7]]);
    let height = u32::from_be_bytes([header[8], header[9], header[10], header[11]]);
    let channels = header[12];
    if width == 0 || height == 0 {
        return fail("QOI image has no pixels");
    }
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return fail("QOI image too large");
    }
    let color = match channels {
        3 => ColorType::Rgb,
        4 => ColorType::Rgba,
        _ => return fail("QOI channel count is not 3 or 4"),
    };
    // The colorspace byte (header[13]) is informative only.
    sink.start(width, height, color).map_err(RowsError::Io)?;
    let mut input = Pieces {
        reader,
        buffer: vec![0; PIECE],
        end: 0,
    };
    match channels {
        3 => decode_rows::<3>(&mut input, sink, width, height),
        _ => decode_rows::<4>(&mut input, sink, width, height),
    }
}

/// The chunk loop for `N` channels (a constant, so each pixel's copy into
/// the row is a fixed-size store rather than a call).
fn decode_rows<const N: usize>(
    input: &mut Pieces<'_>,
    sink: &mut dyn RowSink,
    width: u32,
    height: u32,
) -> Result<(), RowsError> {
    // The read position and the chunk bytes in locals: through the piece
    // struct every byte reloaded its position and checked for a refill.
    let mut at = 0usize;
    let mut row = vec![0u8; width as usize * N];
    let mut index = [[0u8; 4]; 64];
    let mut pixel: Pixel = [0, 0, 0, 255];
    let mut run = 0u32;
    for _ in 0..height {
        for cell in row.chunks_exact_mut(N) {
            if run > 0 {
                run -= 1;
            } else {
                // The longest chunk is five bytes.
                if input.end - at < 5 {
                    at = input.top_up(at, 5)?;
                }
                let bytes = &input.buffer[at..input.end];
                let Some(&first) = bytes.first() else {
                    return fail("QOI data ends before the last pixel");
                };
                let length = match first {
                    OP_RGB => 4,
                    OP_RGBA => 5,
                    _ if first & MASK == OP_LUMA => 2,
                    _ => 1,
                };
                if bytes.len() < length {
                    return fail("QOI data ends before the last pixel");
                }
                match first {
                    OP_RGB => {
                        pixel[0] = bytes[1];
                        pixel[1] = bytes[2];
                        pixel[2] = bytes[3];
                    }
                    OP_RGBA => {
                        pixel = [bytes[1], bytes[2], bytes[3], bytes[4]];
                    }
                    _ => match first & MASK {
                        OP_INDEX => pixel = index[usize::from(first)],
                        OP_DIFF => {
                            pixel[0] = pixel[0].wrapping_add(((first >> 4) & 3).wrapping_sub(2));
                            pixel[1] = pixel[1].wrapping_add(((first >> 2) & 3).wrapping_sub(2));
                            pixel[2] = pixel[2].wrapping_add((first & 3).wrapping_sub(2));
                        }
                        OP_LUMA => {
                            let second = bytes[1];
                            let green = (first & 0x3f).wrapping_sub(32);
                            pixel[0] = pixel[0]
                                .wrapping_add(green.wrapping_sub(8).wrapping_add(second >> 4));
                            pixel[1] = pixel[1].wrapping_add(green);
                            pixel[2] = pixel[2]
                                .wrapping_add(green.wrapping_sub(8).wrapping_add(second & 0x0f));
                        }
                        _ => run = u32::from(first & 0x3f),
                    },
                }
                at += length;
                index[hash(pixel)] = pixel;
            }
            cell.copy_from_slice(&pixel[..N]);
        }
        sink.row(&row).map_err(RowsError::Io)?;
    }
    Ok(())
}

/// Reads a whole QOI file into an image.
pub fn read_qoi(bytes: &[u8]) -> Result<Image, QoiError> {
    let mut sink = Collect::default();
    match read_qoi_rows(&mut &bytes[..], &mut sink) {
        Ok(()) => Ok(sink.image),
        Err(RowsError::Png(error)) => Err(QoiError(error.0)),
        Err(RowsError::Io(error)) => Err(QoiError(format!("row sink failed: {error}"))),
    }
}

/// Writes QOI from rows as a `RowSink`, a pixel at a time, as `qoi.h`
/// encodes: the same chunks for the same pixels.
pub struct QoiRows<'a> {
    sink: &'a mut dyn Write,
    out: Vec<u8>,
    /// One row's worst case (five bytes a pixel), zeroed once.
    scratch: Vec<u8>,
    color: ColorType,
    remaining: u64,
    index: [Pixel; 64],
    previous: Pixel,
    run: u8,
}

impl<'a> QoiRows<'a> {
    pub fn new(sink: &'a mut dyn Write) -> QoiRows<'a> {
        QoiRows {
            sink,
            out: Vec::with_capacity(PIECE + 16),
            scratch: Vec::new(),
            color: ColorType::Rgb,
            remaining: 0,
            index: [[0; 4]; 64],
            previous: [0, 0, 0, 255],
            run: 0,
        }
    }

    fn flush_piece(&mut self) -> io::Result<()> {
        self.sink.write_all(&self.out)?;
        self.out.clear();
        Ok(())
    }
}

impl RowSink for QoiRows<'_> {
    fn start(&mut self, width: u32, height: u32, color: ColorType) -> io::Result<()> {
        self.color = color;
        self.remaining = u64::from(width) * u64::from(height);
        self.scratch = vec![0; width as usize * 5];
        let channels = match color {
            ColorType::Rgba | ColorType::GrayAlpha => 4,
            ColorType::Rgb | ColorType::Gray => 3,
        };
        self.out.extend_from_slice(MAGIC);
        self.out.extend_from_slice(&width.to_be_bytes());
        self.out.extend_from_slice(&height.to_be_bytes());
        // sRGB color with linear alpha: what the hub's pixels are.
        self.out.extend_from_slice(&[channels, 0]);
        Ok(())
    }

    fn row(&mut self, pixels: &[u8]) -> io::Result<()> {
        // The encoder state in locals for the row and the chunks written
        // into a scratch slice sized once for the worst case (five bytes
        // a pixel): through `self` every pixel reloaded the state and each
        // push checked capacity.
        let mut encoder = Encoder {
            out: &mut self.scratch,
            at: 0,
            index: &mut self.index,
            previous: self.previous,
            run: self.run,
        };
        match self.color {
            ColorType::Rgb => {
                for cell in pixels.chunks_exact(3) {
                    encoder.pixel([cell[0], cell[1], cell[2], 255]);
                }
            }
            ColorType::Rgba => {
                for cell in pixels.chunks_exact(4) {
                    encoder.pixel(cell.try_into().unwrap_or([0; 4]));
                }
            }
            ColorType::Gray => {
                for value in pixels {
                    encoder.pixel([*value, *value, *value, 255]);
                }
            }
            ColorType::GrayAlpha => {
                for cell in pixels.chunks_exact(2) {
                    encoder.pixel([cell[0], cell[0], cell[0], cell[1]]);
                }
            }
        }
        self.remaining -= (pixels.len() / self.color.channels()) as u64;
        // A run open at the image's end closes there (qoi.h's last-pixel
        // check, taken once per row instead of once per pixel).
        if self.remaining == 0 && encoder.run > 0 {
            encoder.put(OP_RUN | (encoder.run - 1));
            encoder.run = 0;
        }
        let written = encoder.at;
        self.previous = encoder.previous;
        self.run = encoder.run;
        self.out.extend_from_slice(&self.scratch[..written]);
        if self.remaining == 0 {
            self.out.extend_from_slice(&END);
            self.flush_piece()?;
            return self.sink.flush();
        }
        if self.out.len() >= PIECE {
            self.flush_piece()?;
        }
        Ok(())
    }
}

/// One row's encoding state, held in locals.
struct Encoder<'a> {
    out: &'a mut [u8],
    at: usize,
    index: &'a mut [Pixel; 64],
    previous: Pixel,
    run: u8,
}

impl Encoder<'_> {
    #[inline(always)]
    fn put(&mut self, byte: u8) {
        self.out[self.at] = byte;
        self.at += 1;
    }

    #[inline(always)]
    fn pixel(&mut self, pixel: Pixel) {
        if u32::from_ne_bytes(pixel) == u32::from_ne_bytes(self.previous) {
            self.run += 1;
            if self.run == 62 {
                self.put(OP_RUN | 61);
                self.run = 0;
            }
            return;
        }
        if self.run > 0 {
            self.put(OP_RUN | (self.run - 1));
            self.run = 0;
        }
        let slot = hash(pixel);
        if u32::from_ne_bytes(self.index[slot]) == u32::from_ne_bytes(pixel) {
            self.put(OP_INDEX | slot as u8);
        } else {
            self.index[slot] = pixel;
            let previous = self.previous;
            if pixel[3] == previous[3] {
                let red = pixel[0].wrapping_sub(previous[0]) as i8;
                let green = pixel[1].wrapping_sub(previous[1]) as i8;
                let blue = pixel[2].wrapping_sub(previous[2]) as i8;
                let red_green = red.wrapping_sub(green);
                let blue_green = blue.wrapping_sub(green);
                if (-2..=1).contains(&red) && (-2..=1).contains(&green) && (-2..=1).contains(&blue)
                {
                    self.put(
                        OP_DIFF
                            | (((red + 2) as u8) << 4)
                            | (((green + 2) as u8) << 2)
                            | ((blue + 2) as u8),
                    );
                } else if (-32..=31).contains(&green)
                    && (-8..=7).contains(&red_green)
                    && (-8..=7).contains(&blue_green)
                {
                    self.put(OP_LUMA | ((green + 32) as u8));
                    self.put((((red_green + 8) as u8) << 4) | ((blue_green + 8) as u8));
                } else {
                    self.out[self.at..self.at + 4]
                        .copy_from_slice(&[OP_RGB, pixel[0], pixel[1], pixel[2]]);
                    self.at += 4;
                }
            } else {
                self.out[self.at..self.at + 5]
                    .copy_from_slice(&[OP_RGBA, pixel[0], pixel[1], pixel[2], pixel[3]]);
                self.at += 5;
            }
        }
        self.previous = pixel;
    }
}

/// Writes a whole image as QOI.
pub fn write_qoi(image: &Image, sink: &mut dyn Write) -> io::Result<()> {
    let mut rows = QoiRows::new(sink);
    rows.start(image.width, image.height, image.color)?;
    for y in 0..image.height {
        rows.row(image.row(y))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_one_multiply_hash_is_the_spec_hash() {
        for sample in 0..100_000u32 {
            let pixel = (sample.wrapping_mul(2_654_435_761)).to_le_bytes();
            let [r, g, b, a] = pixel.map(usize::from);
            assert_eq!(
                hash(pixel),
                (r * 3 + g * 5 + b * 7 + a * 11) % 64,
                "{pixel:?}"
            );
        }
        assert_eq!(hash([255, 255, 255, 255]), (255 * 26) % 64);
    }
}
