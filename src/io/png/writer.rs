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
    rows_seen: u32,
    last_filter: u8,
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
            rows_seen: 0,
            last_filter: 1,
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
        // The filter is chosen by trial on every fourth row and kept for
        // the rows between: image statistics change slowly down the
        // rows, and the trials cost four passes a row.
        let keep = if self.rows_seen % 4 == 0 {
            None
        } else {
            Some(self.last_filter)
        };
        self.rows_seen += 1;
        self.last_filter = filter_row(
            keep,
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
fn filter_row(
    keep: Option<u8>,
    row: &[u8],
    above: &[u8],
    unit: usize,
    scratch: &mut [u8],
    out: &mut Vec<u8>,
) -> u8 {
    let mut best_filter = 0u8;
    let mut best_sum = u64::MAX;
    let start = out.len();
    out.push(0);
    out.extend_from_slice(row);
    if let Some(filter) = keep {
        // No trial: the filter chosen on a recent row, written in one pass.
        match filter {
            0 => {}
            1 => {
                filter_sub(row, unit, scratch, u64::MAX);
            }
            2 => {
                filter_up(row, above, scratch, u64::MAX);
            }
            3 => {
                filter_average(row, above, unit, scratch, u64::MAX);
            }
            _ => {
                filter_paeth(row, above, unit, scratch, u64::MAX);
            }
        }
        if filter > 0 {
            out[start] = filter;
            out[start + 1..].copy_from_slice(scratch);
        }
        return filter;
    }
    // Sub first: it wins most rows, and a low early bar lets the other
    // trials stop after a few hundred bytes. None goes last for the same
    // reason (it rarely wins and its sum is the largest). Scoring all
    // five in one pass was tried and lost: the combined loop does not
    // vectorize, the separate ones do.
    for filter in [1u8, 2, 3, 4, 0] {
        // Each trial stops as soon as its running sum passes the best
        // (libpng's rule), so only the winner is computed in full.
        let sum = match filter {
            0 => residual_cost(row, best_sum),
            1 => filter_sub(row, unit, scratch, best_sum),
            2 => filter_up(row, above, scratch, best_sum),
            3 => filter_average(row, above, unit, scratch, best_sum),
            _ => filter_paeth(row, above, unit, scratch, best_sum),
        };
        if sum < best_sum {
            best_sum = sum;
            best_filter = filter;
            if filter > 0 {
                out[start] = filter;
                out[start + 1..].copy_from_slice(scratch);
            }
            // A filter whose residuals average under a sixteenth leaves
            // nothing worth another pass (a flat area or a repeated row);
            // the remaining filters are skipped.
            if sum <= row.len() as u64 / 16 {
                break;
            }
        }
    }
    best_filter
}

/// The sum of the residuals' magnitudes (as signed bytes), the standard
/// guide to which filter compresses best; `u64::MAX` as soon as the
/// running sum passes `limit`.
fn residual_cost(residuals: &[u8], limit: u64) -> u64 {
    let mut sum = 0u64;
    for piece in residuals.chunks(1024) {
        sum += piece
            .iter()
            .map(|byte| u64::from((*byte as i8).unsigned_abs()))
            .sum::<u64>();
        if sum > limit {
            return u64::MAX;
        }
    }
    sum
}

/// The filters run over zipped slices rather than indices, so each is
/// one pass the compiler vectorizes; the first pixel, with nothing to
/// its left, is handled apart.
fn filter_sub(row: &[u8], unit: usize, out: &mut [u8], limit: u64) -> u64 {
    let head = unit.min(row.len());
    out[..head].copy_from_slice(&row[..head]);
    for ((target, byte), left) in out[head..].iter_mut().zip(&row[head..]).zip(row) {
        *target = byte.wrapping_sub(*left);
    }
    residual_cost(out, limit)
}

fn filter_up(row: &[u8], above: &[u8], out: &mut [u8], limit: u64) -> u64 {
    for ((target, byte), up) in out.iter_mut().zip(row).zip(above) {
        *target = byte.wrapping_sub(*up);
    }
    residual_cost(out, limit)
}

fn filter_average(row: &[u8], above: &[u8], unit: usize, out: &mut [u8], limit: u64) -> u64 {
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
    residual_cost(out, limit)
}

fn filter_paeth(row: &[u8], above: &[u8], unit: usize, out: &mut [u8], limit: u64) -> u64 {
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
    residual_cost(out, limit)
}

fn paeth(left: u8, up: u8, up_left: u8) -> u8 {
    // stb_image's formulation of the predictor: the same choice as the
    // specification's, written as a threshold against three times the
    // corner and two selects, which the compiler makes branch-free (a
    // noisy row mispredicts the textbook form once per pixel).
    let threshold = i16::from(up_left) * 3 - (i16::from(left) + i16::from(up));
    let low = left.min(up);
    let high = left.max(up);
    let first = if i16::from(high) <= threshold {
        low
    } else {
        up_left
    };
    if threshold <= i16::from(low) {
        high
    } else {
        first
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
