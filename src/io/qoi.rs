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

#[inline]
fn hash(pixel: Pixel) -> usize {
    let [r, g, b, a] = pixel.map(usize::from);
    (r * 3 + g * 5 + b * 7 + a * 11) % 64
}

/// The input, read a piece at a time.
struct Pieces<'a> {
    reader: &'a mut dyn Read,
    buffer: Vec<u8>,
    at: usize,
    end: usize,
}

impl Pieces<'_> {
    #[inline]
    fn byte(&mut self) -> Result<u8, RowsError> {
        if self.at == self.end {
            self.refill()?;
        }
        let byte = self.buffer[self.at];
        self.at += 1;
        Ok(byte)
    }

    #[cold]
    fn refill(&mut self) -> Result<(), RowsError> {
        loop {
            match self.reader.read(&mut self.buffer) {
                Ok(0) => return fail("QOI data ends before the last pixel"),
                Ok(count) => {
                    self.at = 0;
                    self.end = count;
                    return Ok(());
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(RowsError::Io(error)),
            }
        }
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
    let channels = usize::from(channels);
    let mut input = Pieces {
        reader,
        buffer: vec![0; PIECE],
        at: 0,
        end: 0,
    };
    let mut row = vec![0u8; width as usize * channels];
    let mut index = [[0u8; 4]; 64];
    let mut pixel: Pixel = [0, 0, 0, 255];
    let mut run = 0u32;
    for _ in 0..height {
        for cell in row.chunks_exact_mut(channels) {
            if run > 0 {
                run -= 1;
            } else {
                let first = input.byte()?;
                match first {
                    OP_RGB => {
                        pixel[0] = input.byte()?;
                        pixel[1] = input.byte()?;
                        pixel[2] = input.byte()?;
                    }
                    OP_RGBA => {
                        pixel[0] = input.byte()?;
                        pixel[1] = input.byte()?;
                        pixel[2] = input.byte()?;
                        pixel[3] = input.byte()?;
                    }
                    _ => match first & MASK {
                        OP_INDEX => pixel = index[usize::from(first)],
                        OP_DIFF => {
                            pixel[0] = pixel[0].wrapping_add(((first >> 4) & 3).wrapping_sub(2));
                            pixel[1] = pixel[1].wrapping_add(((first >> 2) & 3).wrapping_sub(2));
                            pixel[2] = pixel[2].wrapping_add((first & 3).wrapping_sub(2));
                        }
                        OP_LUMA => {
                            let second = input.byte()?;
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
                index[hash(pixel)] = pixel;
            }
            cell.copy_from_slice(&pixel[..channels]);
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
            remaining: self.remaining,
        };
        match self.color {
            ColorType::Rgb => {
                for cell in pixels.chunks_exact(3) {
                    encoder.pixel([cell[0], cell[1], cell[2], 255]);
                }
            }
            ColorType::Rgba => {
                for cell in pixels.chunks_exact(4) {
                    encoder.pixel([cell[0], cell[1], cell[2], cell[3]]);
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
        let written = encoder.at;
        self.previous = encoder.previous;
        self.run = encoder.run;
        self.remaining = encoder.remaining;
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
    remaining: u64,
}

impl Encoder<'_> {
    #[inline(always)]
    fn put(&mut self, byte: u8) {
        self.out[self.at] = byte;
        self.at += 1;
    }

    #[inline(always)]
    fn pixel(&mut self, pixel: Pixel) {
        self.remaining -= 1;
        if u32::from_ne_bytes(pixel) == u32::from_ne_bytes(self.previous) {
            self.run += 1;
            if self.run == 62 || self.remaining == 0 {
                self.put(OP_RUN | (self.run - 1));
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
