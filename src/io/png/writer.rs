//! Encodes an image as PNG: eight bits per channel, the hub's color type
//! as is, each row under the filter with the smallest absolute sum (the
//! heuristic every encoder uses), the rows deflated in 256 KiB parts
//! that each become an IDAT chunk, so the output streams.

use std::io::{self, Write};

use crate::image::{ColorType, Image};
use crate::io::deflate::Level;
use crate::io::deflate::compress::deflate_part;
use crate::io::png::RowSink;
use crate::io::png::adler32_update;
use crate::io::zip::crc32::crc32;

const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
const PART_SIZE: usize = 256 * 1024;

pub fn write_png(image: &Image, sink: &mut dyn Write) -> io::Result<()> {
    let mut rows = PngRows::new(sink);
    rows.start(image.width, image.height, image.color)?;
    for y in 0..image.height {
        rows.row(image.row(y))?;
    }
    Ok(())
}

/// Writes a PNG row by row as a `RowSink`: each row is filtered against
/// the one before and deflated into IDAT chunks of about 256 KiB, so a
/// decoder can hand rows over as it produces them and no image is held.
pub struct PngRows<'a> {
    sink: &'a mut dyn Write,
    stride: usize,
    unit: usize,
    rows_left: u32,
    filtered: Vec<u8>,
    scratch: Vec<u8>,
    deflated: Vec<u8>,
    previous: Vec<u8>,
    adler: u32,
    first_part: bool,
}

impl<'a> PngRows<'a> {
    pub fn new(sink: &'a mut dyn Write) -> PngRows<'a> {
        PngRows {
            sink,
            stride: 0,
            unit: 1,
            rows_left: 0,
            filtered: Vec::new(),
            scratch: Vec::new(),
            deflated: Vec::new(),
            previous: Vec::new(),
            adler: 1,
            first_part: true,
        }
    }

    fn part(&mut self, is_final: bool) -> io::Result<()> {
        self.adler = adler32_update(self.adler, &self.filtered);
        self.deflated.clear();
        if self.first_part {
            self.deflated.extend_from_slice(&[0x78, 0x9c]);
            self.first_part = false;
        }
        deflate_part(&self.filtered, &mut self.deflated, Level::Default, is_final);
        if is_final {
            self.deflated.extend_from_slice(&self.adler.to_be_bytes());
        }
        write_chunk(self.sink, b"IDAT", &self.deflated)?;
        self.filtered.clear();
        Ok(())
    }
}

impl RowSink for PngRows<'_> {
    fn start(&mut self, width: u32, height: u32, color: ColorType) -> io::Result<()> {
        self.sink.write_all(&SIGNATURE)?;
        let mut header = Vec::with_capacity(13);
        header.extend_from_slice(&width.to_be_bytes());
        header.extend_from_slice(&height.to_be_bytes());
        header.push(8);
        header.push(match color {
            ColorType::Gray => 0,
            ColorType::GrayAlpha => 4,
            ColorType::Rgb => 2,
            ColorType::Rgba => 6,
        });
        header.extend_from_slice(&[0, 0, 0]);
        write_chunk(self.sink, b"IHDR", &header)?;
        self.stride = width as usize * color.channels();
        self.unit = color.channels();
        self.rows_left = height;
        self.filtered = Vec::with_capacity(PART_SIZE + self.stride + 1);
        self.scratch = vec![0; self.stride];
        // The row above the first is zeros, as the filters define it.
        self.previous = vec![0; self.stride];
        if height == 0 {
            self.part(true)?;
            write_chunk(self.sink, b"IEND", &[])?;
            self.sink.flush()?;
        }
        Ok(())
    }

    fn row(&mut self, pixels: &[u8]) -> io::Result<()> {
        filter_row(
            pixels,
            &self.previous,
            self.unit,
            &mut self.scratch,
            &mut self.filtered,
        );
        self.previous.copy_from_slice(pixels);
        self.rows_left = self.rows_left.saturating_sub(1);
        if self.rows_left == 0 {
            self.part(true)?;
            write_chunk(self.sink, b"IEND", &[])?;
            return self.sink.flush();
        }
        if self.filtered.len() >= PART_SIZE {
            self.part(false)?;
        }
        Ok(())
    }
}

/// Appends the row under the filter with the smallest sum of absolute
/// residuals, filter byte first. Each filter is its own tight loop that
/// writes the residuals and sums them in one pass; the best row is
/// kept and the others thrown away.
fn filter_row(row: &[u8], above: &[u8], unit: usize, scratch: &mut [u8], out: &mut Vec<u8>) {
    let mut best_filter = 0u8;
    let mut best_sum = u64::MAX;
    let start = out.len();
    out.push(0);
    out.extend_from_slice(row);
    for filter in 0..=4u8 {
        let sum = match filter {
            0 => residual_cost(row),
            1 => filter_sub(row, unit, scratch),
            2 => filter_up(row, above, scratch),
            3 => filter_average(row, above, unit, scratch),
            _ => filter_paeth(row, above, unit, scratch),
        };
        if sum < best_sum {
            best_sum = sum;
            best_filter = filter;
            if filter > 0 {
                out[start] = filter;
                out[start + 1..].copy_from_slice(scratch);
            }
            // Nothing beats a row of zeros (a flat area or a repeated
            // row); the remaining filters are skipped.
            if sum == 0 {
                break;
            }
        }
    }
    let _ = best_filter;
}

/// The sum of the residuals' magnitudes (as signed bytes), the standard
/// guide to which filter compresses best.
fn residual_cost(residuals: &[u8]) -> u64 {
    residuals
        .iter()
        .map(|byte| u64::from((*byte as i8).unsigned_abs()))
        .sum()
}

/// The filters run over zipped slices rather than indices, so each is
/// one pass the compiler vectorizes; the first pixel, with nothing to
/// its left, is handled apart.
fn filter_sub(row: &[u8], unit: usize, out: &mut [u8]) -> u64 {
    let head = unit.min(row.len());
    out[..head].copy_from_slice(&row[..head]);
    for ((target, byte), left) in out[head..].iter_mut().zip(&row[head..]).zip(row) {
        *target = byte.wrapping_sub(*left);
    }
    residual_cost(out)
}

fn filter_up(row: &[u8], above: &[u8], out: &mut [u8]) -> u64 {
    for ((target, byte), up) in out.iter_mut().zip(row).zip(above) {
        *target = byte.wrapping_sub(*up);
    }
    residual_cost(out)
}

fn filter_average(row: &[u8], above: &[u8], unit: usize, out: &mut [u8]) -> u64 {
    let head = unit.min(row.len());
    for ((target, byte), up) in out[..head].iter_mut().zip(row).zip(above) {
        *target = byte.wrapping_sub(*up / 2);
    }
    let rest = out[head..]
        .iter_mut()
        .zip(&row[head..])
        .zip(row)
        .zip(&above[head..]);
    for (((target, byte), left), up) in rest {
        let predicted = ((u16::from(*left) + u16::from(*up)) / 2) as u8;
        *target = byte.wrapping_sub(predicted);
    }
    residual_cost(out)
}

fn filter_paeth(row: &[u8], above: &[u8], unit: usize, out: &mut [u8]) -> u64 {
    let head = unit.min(row.len());
    for ((target, byte), up) in out[..head].iter_mut().zip(row).zip(above) {
        *target = byte.wrapping_sub(*up);
    }
    let rest = out[head..]
        .iter_mut()
        .zip(&row[head..])
        .zip(row)
        .zip(&above[head..])
        .zip(above);
    for ((((target, byte), left), up), up_left) in rest {
        *target = byte.wrapping_sub(paeth(*left, *up, *up_left));
    }
    residual_cost(out)
}

fn paeth(left: u8, up: u8, up_left: u8) -> u8 {
    let (a, b, c) = (i16::from(left), i16::from(up), i16::from(up_left));
    let p = a + b - c;
    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
    if pa <= pb && pa <= pc {
        left
    } else if pb <= pc {
        up
    } else {
        up_left
    }
}

fn write_chunk(sink: &mut dyn Write, kind: &[u8; 4], data: &[u8]) -> io::Result<()> {
    sink.write_all(&(data.len() as u32).to_be_bytes())?;
    sink.write_all(kind)?;
    sink.write_all(data)?;
    let mut checked = Vec::with_capacity(4 + data.len());
    checked.extend_from_slice(kind);
    checked.extend_from_slice(data);
    sink.write_all(&crc32(&checked).to_be_bytes())
}
