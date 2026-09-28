//! The WebP container (RIFF): the simple forms (one `VP8 ` or `VP8L`
//! chunk) and the extended form (`VP8X` with alpha, metadata, and
//! animation chunks). Metadata is skipped and reported by name; of an
//! animation, the first frame is decoded and the rest reported.

use std::io::Read;

use super::{WebpError, lossless, lossy, to_rows};
use crate::image::{ColorType, Image};
use crate::io::png::{Collect, RowSink, RowsError};

fn fail<T>(message: &str) -> Result<T, WebpError> {
    Err(WebpError(message.to_string()))
}

/// What the reader skipped or noticed, for the converter to report.
#[derive(Debug, Default)]
pub struct WebpNotes {
    /// Metadata chunks left out (ICC profile, Exif, XMP), one name each.
    pub dropped: Vec<String>,
    /// Frames of an animation after the first, which are not decoded.
    pub frames_dropped: usize,
    pub lossy: bool,
}

pub fn read_webp(bytes: &[u8]) -> Result<(Image, WebpNotes), WebpError> {
    let mut sink = Collect::default();
    let notes = match decode(bytes, &mut sink) {
        Ok(notes) => notes,
        Err(RowsError::Png(error)) => return Err(WebpError(error.0)),
        Err(RowsError::Io(error)) => return Err(WebpError(format!("row sink failed: {error}"))),
    };
    Ok((sink.image, notes))
}

pub fn read_webp_from(reader: &mut dyn Read) -> Result<(Image, WebpNotes), WebpError> {
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|error| WebpError(format!("read failed: {error}")))?;
    read_webp(&bytes)
}

/// Reads a WebP from a stream into `sink`, one row at a time. The file
/// is read whole (its bitstreams are not row-aligned).
pub fn read_webp_rows(
    reader: &mut dyn Read,
    sink: &mut dyn RowSink,
) -> Result<WebpNotes, RowsError> {
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).map_err(RowsError::Io)?;
    decode(&bytes, sink)
}

/// One chunk: its type and payload.
struct Chunk<'a> {
    kind: [u8; 4],
    data: &'a [u8],
}

fn chunks(mut body: &[u8]) -> Result<Vec<Chunk<'_>>, WebpError> {
    let mut out = Vec::new();
    while body.len() >= 8 {
        let kind = [body[0], body[1], body[2], body[3]];
        let size = u32::from_le_bytes([body[4], body[5], body[6], body[7]]) as usize;
        let padded = size + (size & 1);
        if body.len() < 8 + size {
            return fail("chunk runs past the file");
        }
        out.push(Chunk {
            kind,
            data: &body[8..8 + size],
        });
        body = &body[(8 + padded).min(body.len())..];
    }
    Ok(out)
}

fn decode(bytes: &[u8], sink: &mut dyn RowSink) -> Result<WebpNotes, RowsError> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return Err(to_rows(WebpError(
            "not a WebP: bad RIFF header".to_string(),
        )));
    }
    let declared = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    let end = (8 + declared).min(bytes.len());
    let list = chunks(&bytes[12..end]).map_err(to_rows)?;
    let mut notes = WebpNotes::default();
    let first = list
        .first()
        .ok_or_else(|| to_rows(WebpError("an empty WebP".to_string())))?;
    match &first.kind {
        b"VP8L" => {
            decode_lossless(first.data, sink)?;
        }
        b"VP8 " => {
            notes.lossy = true;
            decode_lossy(first.data, None, sink)?;
        }
        b"VP8X" => {
            let mut alpha: Option<&[u8]> = None;
            let mut image: Option<&Chunk<'_>> = None;
            let mut frames = 0usize;
            let mut first_frame: Option<&[u8]> = None;
            for chunk in &list[1..] {
                match &chunk.kind {
                    b"ICCP" => note(&mut notes, "ICC profile"),
                    b"EXIF" => note(&mut notes, "Exif"),
                    b"XMP " => note(&mut notes, "XMP"),
                    b"ALPH" => alpha = Some(chunk.data),
                    b"VP8 " | b"VP8L" => {
                        if image.is_none() {
                            image = Some(chunk);
                        }
                    }
                    b"ANMF" => {
                        frames += 1;
                        if first_frame.is_none() {
                            first_frame = Some(chunk.data);
                        }
                    }
                    _ => {}
                }
            }
            if let Some(frame) = first_frame {
                notes.frames_dropped = frames - 1;
                decode_frame(frame, first.data, sink, &mut notes)?;
            } else {
                let image = image.ok_or_else(|| {
                    to_rows(WebpError("no image data in an extended WebP".to_string()))
                })?;
                if &image.kind == b"VP8L" {
                    decode_lossless(image.data, sink)?;
                } else {
                    notes.lossy = true;
                    decode_lossy(image.data, alpha, sink)?;
                }
            }
        }
        _ => {
            return Err(to_rows(WebpError(
                "no image chunk where a WebP starts".to_string(),
            )));
        }
    }
    Ok(notes)
}

fn note(notes: &mut WebpNotes, name: &str) {
    if !notes.dropped.iter().any(|seen| seen == name) {
        notes.dropped.push(name.to_string());
    }
}

/// The first frame of an animation, placed on the canvas (transparent
/// elsewhere).
fn decode_frame(
    frame: &[u8],
    vp8x: &[u8],
    sink: &mut dyn RowSink,
    notes: &mut WebpNotes,
) -> Result<(), RowsError> {
    if frame.len() < 16 || vp8x.len() < 10 {
        return Err(to_rows(WebpError("animation frame cut short".to_string())));
    }
    let read24 = |b: &[u8]| u32::from(b[0]) | (u32::from(b[1]) << 8) | (u32::from(b[2]) << 16);
    let canvas_width = read24(&vp8x[4..7]) + 1;
    let canvas_height = read24(&vp8x[7..10]) + 1;
    let x0 = read24(&frame[0..3]) * 2;
    let y0 = read24(&frame[3..6]) * 2;
    let list = chunks(&frame[16..]).map_err(to_rows)?;
    let mut alpha = None;
    let mut collect = Collect::default();
    for chunk in &list {
        match &chunk.kind {
            b"ALPH" => alpha = Some(chunk.data),
            b"VP8L" => {
                decode_lossless(chunk.data, &mut collect)?;
                break;
            }
            b"VP8 " => {
                notes.lossy = true;
                decode_lossy(chunk.data, alpha, &mut collect)?;
                break;
            }
            _ => {}
        }
    }
    let piece = collect.image;
    if piece.width == 0 {
        return Err(to_rows(WebpError(
            "no image in the first animation frame".to_string(),
        )));
    }
    sink.start(canvas_width, canvas_height, ColorType::Rgba)?;
    let channels = piece.color.channels();
    let mut row = vec![0u8; canvas_width as usize * 4];
    for y in 0..canvas_height {
        row.fill(0);
        if y >= y0 && y < y0 + piece.height {
            let source = piece.row(y - y0);
            for x in 0..piece.width.min(canvas_width.saturating_sub(x0)) {
                let cell = &source[x as usize * channels..(x as usize + 1) * channels];
                let target = &mut row[(x0 + x) as usize * 4..(x0 + x) as usize * 4 + 4];
                target[..3].copy_from_slice(&cell[..3]);
                target[3] = if channels == 4 { cell[3] } else { 255 };
            }
        }
        sink.row(&row)?;
    }
    Ok(())
}

fn decode_lossless(data: &[u8], sink: &mut dyn RowSink) -> Result<(), RowsError> {
    let (width, height, alpha, pixels) = lossless::decode(data).map_err(to_rows)?;
    emit_argb(width, height, alpha, &pixels, sink)
}

/// Hands an ARGB buffer to the sink as RGB or RGBA rows.
fn emit_argb(
    width: u32,
    height: u32,
    alpha: bool,
    pixels: &[u32],
    sink: &mut dyn RowSink,
) -> Result<(), RowsError> {
    let color = if alpha {
        ColorType::Rgba
    } else {
        ColorType::Rgb
    };
    sink.start(width, height, color)?;
    let channels = color.channels();
    let mut row = vec![0u8; width as usize * channels];
    for line in pixels.chunks_exact(width as usize) {
        for (cell, argb) in row.chunks_exact_mut(channels).zip(line) {
            cell[0] = (argb >> 16) as u8;
            cell[1] = (argb >> 8) as u8;
            cell[2] = *argb as u8;
            if alpha {
                cell[3] = (argb >> 24) as u8;
            }
        }
        sink.row(&row)?;
    }
    Ok(())
}

fn decode_lossy(
    data: &[u8],
    alpha: Option<&[u8]>,
    sink: &mut dyn RowSink,
) -> Result<(), RowsError> {
    lossy::decode(data, alpha, sink)
}
