//! Decodes a PNG into the image hub: every bit depth and color type,
//! palettes with transparency, transparency keys, interlacing, and IDAT
//! data in any number of chunks. Sixteen-bit samples come down to their
//! high byte; bit depths under eight scale up to eight. Ancillary chunks
//! (gamma, color profiles, text, timing) are skipped. Chunk CRCs are
//! checked, so a damaged file is refused rather than misread.

use std::io::Read;

use crate::image::{ColorType, Image};
use crate::io::deflate::{InflateError, Inflater, Progress};
use crate::io::png::adler32_update;
use crate::io::zip::crc32::crc32_update;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PngError(pub String);

impl std::fmt::Display for PngError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl std::error::Error for PngError {}

const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];

/// What the reader dropped, one note per kind.
#[derive(Debug, Default)]
pub struct PngNotes {
    pub sixteen_bit: bool,
    pub dropped_chunks: Vec<[u8; 4]>,
}

#[derive(Clone)]
struct Header {
    width: u32,
    height: u32,
    depth: u8,
    color: u8,
    interlaced: bool,
}

impl Header {
    /// Samples per pixel in the file.
    fn samples(&self) -> usize {
        match self.color {
            0 | 3 => 1,
            4 => 2,
            2 => 3,
            _ => 4,
        }
    }

    /// Bytes per complete pixel, at least one (for the filters).
    fn filter_unit(&self) -> usize {
        ((self.samples() * self.depth as usize) / 8).max(1)
    }

    fn row_bytes(&self, width: u32) -> usize {
        (width as usize * self.samples() * self.depth as usize).div_ceil(8)
    }
}

/// Reads a PNG held whole in memory.
pub fn read_png(bytes: &[u8]) -> Result<(Image, PngNotes), PngError> {
    let mut source: &[u8] = bytes;
    read_png_from(&mut source)
}

/// Where rows go when a PNG is read row by row: `start` once with the
/// dimensions, then `row` with each row's pixels in the hub layout, top
/// to bottom.
pub trait RowSink {
    fn start(&mut self, width: u32, height: u32, color: ColorType) -> std::io::Result<()>;
    fn row(&mut self, pixels: &[u8]) -> std::io::Result<()>;
}

/// What can go wrong reading rows into a sink: the file, or the sink.
#[derive(Debug)]
pub enum RowsError {
    Png(PngError),
    Io(std::io::Error),
}

impl From<PngError> for RowsError {
    fn from(error: PngError) -> Self {
        RowsError::Png(error)
    }
}

impl From<std::io::Error> for RowsError {
    fn from(error: std::io::Error) -> Self {
        RowsError::Io(error)
    }
}

/// Reads a PNG from a stream straight into `sink`, one row at a time,
/// without holding the image: each row is unfiltered from the inflater's
/// output and handed over. An interlaced image is decoded whole first
/// (its rows arrive out of order) and then handed over row by row.
pub fn read_png_rows(reader: &mut dyn Read, sink: &mut dyn RowSink) -> Result<PngNotes, RowsError> {
    let (image, notes) = read_png_inner(reader, Some(sink))?;
    if image.height > 0 {
        sink.start(image.width, image.height, image.color)?;
        for y in 0..image.height {
            sink.row(image.row(y))?;
        }
    }
    Ok(notes)
}

/// Input is read in pieces of this size.
const PIECE: usize = 256 * 1024;
/// How much decoded data the inflater is asked for at a time; rows are
/// taken out of it as soon as they are complete, so this (plus the
/// inflater's history window) is all the raw data ever held.
const OUTPUT_STEP: usize = 256 * 1024;
/// The longest chunk read whole (IHDR, PLTE, tRNS); IDAT and ancillary
/// chunks stream through the piece buffer.
const MAX_WHOLE_CHUNK: usize = PIECE;

/// Reads a PNG from a stream: chunks are pulled in pieces, the image
/// data is inflated as it arrives, and each row is unfiltered into the
/// image the moment it is complete. Memory is the image plus a few
/// hundred kilobytes, whatever the file's size.
pub fn read_png_from(reader: &mut dyn Read) -> Result<(Image, PngNotes), PngError> {
    match read_png_inner(reader, None) {
        Ok(result) => Ok(result),
        Err(RowsError::Png(error)) => Err(error),
        Err(RowsError::Io(error)) => Err(PngError(format!("row sink failed: {error}"))),
    }
}

/// The reader behind both entry points. With a sink, a non-interlaced
/// image streams to it and the returned image is empty; an interlaced
/// one comes back whole for the caller to hand over.
fn read_png_inner(
    reader: &mut dyn Read,
    mut sink: Option<&mut (dyn RowSink + '_)>,
) -> Result<(Image, PngNotes), RowsError> {
    let mut source = Source::new(reader);
    if source.exact(SIGNATURE.len())? != SIGNATURE {
        return Err(PngError("not a PNG: bad signature".to_string()).into());
    }
    let mut header: Option<Header> = None;
    let mut palette: Vec<[u8; 3]> = Vec::new();
    let mut transparency: Vec<u8> = Vec::new();
    let mut notes = PngNotes::default();
    let mut decoder: Option<Decoder> = None;
    loop {
        let head = source.exact(8)?;
        let length = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
        let kind: [u8; 4] = [head[4], head[5], head[6], head[7]];
        if !kind.iter().all(u8::is_ascii_alphabetic) {
            return Err(PngError("bad chunk type".to_string()).into());
        }
        let mut crc = crc32_update(0, &kind);
        match &kind {
            b"IHDR" => {
                if header.is_some() {
                    return Err(PngError("two IHDR chunks".to_string()).into());
                }
                let data = source.whole_chunk(length, &kind)?;
                crc = crc32_update(crc, data);
                header = Some(parse_header(data)?);
            }
            b"PLTE" => {
                let data = source.whole_chunk(length, &kind)?;
                crc = crc32_update(crc, data);
                if data.len() % 3 != 0 || data.is_empty() || data.len() > 768 {
                    return Err(PngError("bad palette length".to_string()).into());
                }
                palette = data.chunks(3).map(|rgb| [rgb[0], rgb[1], rgb[2]]).collect();
            }
            b"tRNS" => {
                let data = source.whole_chunk(length, &kind)?;
                crc = crc32_update(crc, data);
                transparency = data.to_vec();
            }
            b"IDAT" => {
                let decoder = match decoder.as_mut() {
                    Some(decoder) => decoder,
                    None => {
                        let header = header
                            .as_ref()
                            .ok_or_else(|| PngError("IDAT before IHDR".to_string()))?;
                        if header.color == 3 && palette.is_empty() {
                            return Err(PngError(
                                "a palette image without a PLTE chunk".to_string(),
                            )
                            .into());
                        }
                        let streams = sink.is_some() && !header.interlaced;
                        if streams {
                            let color = output_color(header, &transparency);
                            if let Some(sink) = sink.as_deref_mut() {
                                sink.start(header.width, header.height, color)?;
                            }
                        }
                        decoder.insert(Decoder::new(header, &palette, &transparency, streams))
                    }
                };
                let mut remaining = length;
                while remaining > 0 {
                    let piece = source.take(remaining)?;
                    if piece.is_empty() {
                        return Err(PngError("chunk cut short".to_string()).into());
                    }
                    crc = crc32_update(crc, piece);
                    remaining -= piece.len();
                    decoder.feed(piece, sink.as_deref_mut())?;
                }
            }
            b"IEND" => {
                if length != 0 {
                    return Err(PngError("IEND chunk with data".to_string()).into());
                }
                source.check_crc(crc, &kind)?;
                break;
            }
            other => {
                if other[0].is_ascii_uppercase() {
                    return Err(PngError(format!(
                        "unknown critical chunk {}",
                        String::from_utf8_lossy(other)
                    ))
                    .into());
                }
                if !notes.dropped_chunks.contains(other) {
                    notes.dropped_chunks.push(*other);
                }
                let mut remaining = length;
                while remaining > 0 {
                    let piece = source.take(remaining)?;
                    if piece.is_empty() {
                        return Err(PngError("chunk cut short".to_string()).into());
                    }
                    crc = crc32_update(crc, piece);
                    remaining -= piece.len();
                }
            }
        }
        source.check_crc(crc, &kind)?;
    }
    let header = header.ok_or_else(|| PngError("no IHDR chunk".to_string()))?;
    let decoder = decoder.ok_or_else(|| PngError("no IDAT chunk".to_string()))?;
    let image = decoder.finish()?;
    notes.sixteen_bit = header.depth == 16;
    Ok((image, notes))
}

/// Buffered input read in pieces; chunk headers and small chunks come
/// out whole, long chunks come out piece by piece.
struct Source<'a> {
    reader: &'a mut dyn Read,
    buffer: Vec<u8>,
    start: usize,
    end: usize,
}

impl<'a> Source<'a> {
    fn new(reader: &'a mut dyn Read) -> Source<'a> {
        Source {
            reader,
            buffer: vec![0; PIECE],
            start: 0,
            end: 0,
        }
    }

    /// Reads more input behind what is buffered; false at the end.
    fn fill(&mut self) -> Result<bool, PngError> {
        if self.start > 0 {
            self.buffer.copy_within(self.start..self.end, 0);
            self.end -= self.start;
            self.start = 0;
        }
        loop {
            match self.reader.read(&mut self.buffer[self.end..]) {
                Ok(0) => return Ok(false),
                Ok(count) => {
                    self.end += count;
                    return Ok(true);
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(PngError(format!("read failed: {error}"))),
            }
        }
    }

    /// Exactly `count` bytes, which must fit the buffer.
    fn exact(&mut self, count: usize) -> Result<&[u8], PngError> {
        while self.end - self.start < count {
            if !self.fill()? {
                return Err(PngError("chunk cut short".to_string()));
            }
        }
        let slice = &self.buffer[self.start..self.start + count];
        self.start += count;
        Ok(slice)
    }

    /// Up to `max` bytes, at least one unless the input has ended.
    fn take(&mut self, max: usize) -> Result<&[u8], PngError> {
        if self.start == self.end && !self.fill()? {
            return Ok(&[]);
        }
        let count = (self.end - self.start).min(max);
        let slice = &self.buffer[self.start..self.start + count];
        self.start += count;
        Ok(slice)
    }

    fn whole_chunk(&mut self, length: usize, kind: &[u8; 4]) -> Result<&[u8], PngError> {
        if length > MAX_WHOLE_CHUNK {
            return Err(PngError(format!(
                "{} chunk too long",
                String::from_utf8_lossy(kind)
            )));
        }
        self.exact(length)
    }

    fn check_crc(&mut self, computed: u32, kind: &[u8; 4]) -> Result<(), PngError> {
        let stored = self.exact(4)?;
        let stored = u32::from_be_bytes([stored[0], stored[1], stored[2], stored[3]]);
        if stored != computed {
            return Err(PngError(format!(
                "bad CRC in chunk {}",
                String::from_utf8_lossy(kind)
            )));
        }
        Ok(())
    }
}

/// The image data path: zlib header, the inflater, rows, the trailer.
struct Decoder {
    zlib_header: Vec<u8>,
    inflater: Inflater,
    rows: Rows,
    adler: u32,
    trailer: Vec<u8>,
}

impl Decoder {
    fn new(header: &Header, palette: &[[u8; 3]], transparency: &[u8], streams: bool) -> Decoder {
        Decoder {
            zlib_header: Vec::with_capacity(2),
            inflater: Inflater::new(),
            rows: Rows::new(header, palette, transparency, streams),
            adler: 1,
            trailer: Vec::with_capacity(4),
        }
    }

    fn feed(
        &mut self,
        mut piece: &[u8],
        mut sink: Option<&mut (dyn RowSink + '_)>,
    ) -> Result<(), RowsError> {
        while self.zlib_header.len() < 2 && !piece.is_empty() {
            self.zlib_header.push(piece[0]);
            piece = &piece[1..];
            if self.zlib_header.len() == 2 {
                let (cmf, flg) = (self.zlib_header[0], self.zlib_header[1]);
                let check = (u16::from(cmf) << 8) | u16::from(flg);
                if cmf & 0x0f != 8 || check % 31 != 0 || flg & 0x20 != 0 {
                    return Err(PngError("bad zlib header".to_string()).into());
                }
            }
        }
        let want = OUTPUT_STEP.max(self.rows.row_len());
        while !piece.is_empty() {
            if self.inflater.is_done() {
                let room = 4 - self.trailer.len();
                self.trailer
                    .extend_from_slice(&piece[..room.min(piece.len())]);
                return Ok(());
            }
            let (consumed, progress) = self
                .inflater
                .push(piece, want)
                .map_err(|error| PngError(format!("bad deflate data: {error}")))?;
            piece = &piece[consumed..];
            self.deliver(sink.as_deref_mut())?;
            if matches!(progress, Progress::Done) {
                let leftover = self.inflater.leftover();
                self.trailer
                    .extend_from_slice(&leftover[..leftover.len().min(4)]);
            }
        }
        Ok(())
    }

    /// Moves every complete row out of the inflater into the image.
    fn deliver(&mut self, sink: Option<&mut (dyn RowSink + '_)>) -> Result<(), RowsError> {
        let output = self.inflater.output();
        let used = self.rows.feed(output, sink)?;
        self.adler = adler32_update(self.adler, &output[..used]);
        if self.rows.done && used < output.len() {
            return Err(PngError("more image data than the image holds".to_string()).into());
        }
        self.inflater.drain(used);
        Ok(())
    }

    fn finish(self) -> Result<Image, PngError> {
        if !self.rows.done {
            return Err(PngError("image data cut short".to_string()));
        }
        if self.trailer.len() == 4 {
            let stored = u32::from_be_bytes([
                self.trailer[0],
                self.trailer[1],
                self.trailer[2],
                self.trailer[3],
            ]);
            if stored != self.adler {
                return Err(PngError("bad Adler-32 checksum".to_string()));
            }
        }
        Ok(self.rows.image)
    }
}

/// Where the next row lands: pass by pass for interlaced files, one pass
/// for the rest. Eight-bit gray, gray+alpha, RGB, and RGBA rows that need
/// no expansion unfilter straight into the image buffer.
struct Rows {
    header: Header,
    palette: Vec<[u8; 3]>,
    transparency: Vec<u8>,
    image: Image,
    unit: usize,
    direct: bool,
    pass: usize,
    row: u32,
    pass_width: u32,
    pass_height: u32,
    row_bytes: usize,
    previous: Vec<u8>,
    current: Vec<u8>,
    expanded: Vec<u8>,
    /// Rows go to a sink as they complete instead of into `image`.
    streams: bool,
    done: bool,
}

impl Rows {
    fn new(header: &Header, palette: &[[u8; 3]], transparency: &[u8], streams: bool) -> Rows {
        let color = output_color(header, transparency);
        let held_height = if streams { 0 } else { header.height };
        let image = Image::new(header.width, held_height, color);
        let direct =
            header.depth == 8 && header.color != 3 && transparency.is_empty() && !header.interlaced;
        let mut rows = Rows {
            header: header.clone(),
            palette: palette.to_vec(),
            transparency: transparency.to_vec(),
            image,
            unit: header.filter_unit(),
            direct,
            pass: 0,
            row: 0,
            pass_width: 0,
            pass_height: 0,
            row_bytes: 0,
            previous: Vec::new(),
            current: Vec::new(),
            expanded: Vec::new(),
            streams,
            done: false,
        };
        if header.interlaced {
            rows.start_pass(0);
        } else {
            rows.pass = 7;
            rows.pass_width = header.width;
            rows.pass_height = header.height;
            rows.row_bytes = header.row_bytes(header.width);
            rows.prepare_row_buffers();
        }
        rows
    }

    /// Filter byte plus the row's data.
    fn row_len(&self) -> usize {
        self.row_bytes + 1
    }

    /// Enters the first non-empty pass from `pass` on, or finishes.
    fn start_pass(&mut self, mut pass: usize) {
        while pass < 7 {
            let (width, height) = pass_size(&self.header, pass);
            if width > 0 && height > 0 {
                self.pass = pass;
                self.row = 0;
                self.pass_width = width;
                self.pass_height = height;
                self.row_bytes = self.header.row_bytes(width);
                self.prepare_row_buffers();
                return;
            }
            pass += 1;
        }
        self.done = true;
    }

    fn prepare_row_buffers(&mut self) {
        if self.direct && !self.streams {
            return;
        }
        self.previous.clear();
        self.previous.resize(self.row_bytes, 0);
        self.current.clear();
        self.current.resize(self.row_bytes, 0);
        if self.header.interlaced || self.streams {
            let channels = self.image.color.channels();
            self.expanded.clear();
            self.expanded.resize(self.pass_width as usize * channels, 0);
        }
    }

    /// Takes every complete row from `data`; returns the bytes used.
    fn feed(
        &mut self,
        data: &[u8],
        mut sink: Option<&mut (dyn RowSink + '_)>,
    ) -> Result<usize, RowsError> {
        let mut used = 0;
        // The row length changes between interlace passes.
        while !self.done && data.len() - used >= self.row_len() {
            let row_len = self.row_len();
            let line = &data[used..used + row_len];
            self.row(line[0], &line[1..], sink.as_deref_mut())?;
            used += row_len;
        }
        Ok(used)
    }

    fn row(
        &mut self,
        filter: u8,
        source: &[u8],
        sink: Option<&mut (dyn RowSink + '_)>,
    ) -> Result<(), RowsError> {
        if self.streams {
            // Straight to the sink: an 8-bit row in the hub's own layout
            // unfilters in place and goes; any other expands first.
            let previous: &[u8] = if self.row == 0 { &[] } else { &self.previous };
            unfilter_row(filter, source, previous, self.unit, &mut self.current)?;
            let pixels: &[u8] = if self.direct {
                &self.current
            } else {
                expand_row(
                    &self.header,
                    &self.current,
                    self.pass_width,
                    &self.palette,
                    &self.transparency,
                    &mut self.expanded,
                );
                &self.expanded
            };
            if let Some(sink) = sink {
                sink.row(pixels)?;
            }
            std::mem::swap(&mut self.previous, &mut self.current);
        } else if self.direct {
            let stride = self.image.stride();
            let y = self.row as usize;
            let (before, rest) = self.image.pixels.split_at_mut(y * stride);
            let current = &mut rest[..stride];
            let previous: &[u8] = if y == 0 {
                &[]
            } else {
                &before[(y - 1) * stride..]
            };
            unfilter_row(filter, source, previous, self.unit, current)?;
        } else {
            let previous: &[u8] = if self.row == 0 { &[] } else { &self.previous };
            unfilter_row(filter, source, previous, self.unit, &mut self.current)?;
            if self.header.interlaced {
                expand_row(
                    &self.header,
                    &self.current,
                    self.pass_width,
                    &self.palette,
                    &self.transparency,
                    &mut self.expanded,
                );
                self.scatter();
            } else {
                let target = row_mut(&mut self.image, self.row);
                expand_row(
                    &self.header,
                    &self.current,
                    self.pass_width,
                    &self.palette,
                    &self.transparency,
                    target,
                );
            }
            std::mem::swap(&mut self.previous, &mut self.current);
        }
        self.row += 1;
        if self.row == self.pass_height {
            if self.header.interlaced {
                self.start_pass(self.pass + 1);
            } else {
                self.done = true;
            }
        }
        Ok(())
    }

    /// Places one expanded interlace row into its pixels of the image.
    fn scatter(&mut self) {
        let channels = self.image.color.channels();
        let stride = self.image.stride();
        let y = PASS_START_Y[self.pass] + self.row * PASS_STEP_Y[self.pass];
        let row_start = y as usize * stride;
        for column in 0..self.pass_width as usize {
            let x = PASS_START_X[self.pass] as usize + column * PASS_STEP_X[self.pass] as usize;
            let start = row_start + x * channels;
            self.image.pixels[start..start + channels]
                .copy_from_slice(&self.expanded[column * channels..(column + 1) * channels]);
        }
    }
}

impl From<InflateError> for PngError {
    fn from(error: InflateError) -> Self {
        PngError(format!("bad deflate data: {error}"))
    }
}

fn parse_header(data: &[u8]) -> Result<Header, PngError> {
    if data.len() != 13 {
        return Err(PngError("bad IHDR length".to_string()));
    }
    let width = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
    let height = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
    let depth = data[8];
    let color = data[9];
    let (compression, filter, interlace) = (data[10], data[11], data[12]);
    let depth_ok = match color {
        0 => matches!(depth, 1 | 2 | 4 | 8 | 16),
        3 => matches!(depth, 1 | 2 | 4 | 8),
        2 | 4 | 6 => matches!(depth, 8 | 16),
        _ => return Err(PngError(format!("bad color type {color}"))),
    };
    if !depth_ok {
        return Err(PngError(format!(
            "bad bit depth {depth} for color type {color}"
        )));
    }
    if compression != 0 || filter != 0 || interlace > 1 {
        return Err(PngError(
            "unknown compression, filter, or interlace method".to_string(),
        ));
    }
    if !Image::dimensions_fit(width, height, 4) {
        return Err(PngError(
            "image dimensions are zero or too large".to_string(),
        ));
    }
    Ok(Header {
        width,
        height,
        depth,
        color,
        interlaced: interlace == 1,
    })
}

/// One row; `previous` is the row above, empty for the first. The
/// filters that look left run one lane per byte of the pixel, so each
/// lane is its own dependency chain and the bounds checks happen once
/// per pixel rather than once per byte.
fn unfilter_row(
    filter: u8,
    source: &[u8],
    previous: &[u8],
    unit: usize,
    current: &mut [u8],
) -> Result<(), PngError> {
    match filter {
        0 => current.copy_from_slice(source),
        2 => {
            if previous.is_empty() {
                current.copy_from_slice(source);
            } else {
                for ((target, byte), above) in current.iter_mut().zip(source).zip(previous) {
                    *target = byte.wrapping_add(*above);
                }
            }
        }
        1 => match unit {
            1 => unfilter_sub::<1>(source, current),
            2 => unfilter_sub::<2>(source, current),
            3 => unfilter_sub::<3>(source, current),
            4 => unfilter_sub::<4>(source, current),
            6 => unfilter_sub::<6>(source, current),
            _ => unfilter_sub::<8>(source, current),
        },
        3 | 4 => match unit {
            1 => unfilter_lanes::<1>(filter, source, previous, current),
            2 => unfilter_lanes::<2>(filter, source, previous, current),
            3 => unfilter_lanes::<3>(filter, source, previous, current),
            4 => unfilter_lanes::<4>(filter, source, previous, current),
            6 => unfilter_lanes::<6>(filter, source, previous, current),
            _ => unfilter_lanes::<8>(filter, source, previous, current),
        },
        other => return Err(PngError(format!("bad filter type {other}"))),
    }
    Ok(())
}

/// Sub over pixels of `N` bytes: the pixel's bytes sit in one word and
/// add lane by lane without carries between them. Pixels up to four
/// bytes go two per word: the second pixel takes the first one first,
/// off the chain, so the chain from one word to the next is one add.
fn unfilter_sub<const N: usize>(source: &[u8], current: &mut [u8]) {
    const LOW: u64 = 0x7f7f_7f7f_7f7f_7f7f;
    const HIGH: u64 = 0x8080_8080_8080_8080;
    fn add(a: u64, b: u64) -> u64 {
        ((a & LOW) + (b & LOW)) ^ ((a ^ b) & HIGH)
    }
    let mut left: u64 = 0;
    let mut done = 0usize;
    if N <= 4 {
        let pair = 2 * N;
        let shift = 8 * N;
        let pairs = current.len() / pair;
        for index in 0..pairs {
            let at = index * pair;
            // One load and one store per pair: the bytes go through a
            // zero-padded eight-byte array the compiler turns into a
            // single unaligned move when the pair is eight bytes.
            let mut bytes = [0u8; 8];
            bytes[..pair].copy_from_slice(&source[at..at + pair]);
            let word = u64::from_le_bytes(bytes);
            let stepped = add(word, word << shift);
            let sum = add(stepped, left | (left << shift));
            current[at..at + pair].copy_from_slice(&sum.to_le_bytes()[..pair]);
            left = (sum >> shift) & ((1u64 << shift) - 1);
        }
        done = pairs * pair;
    }
    let rest = current.len() - done;
    let whole = (rest / N) * N;
    let (source_rest, current_rest) = (
        &source[done..done + whole],
        &mut current[done..done + whole],
    );
    for (target, bytes) in current_rest
        .chunks_exact_mut(N)
        .zip(source_rest.chunks_exact(N))
    {
        let mut word: u64 = 0;
        for (lane, byte) in bytes.iter().enumerate() {
            word |= u64::from(*byte) << (lane * 8);
        }
        let sum = add(word, left);
        for (lane, byte) in target.iter_mut().enumerate() {
            *byte = (sum >> (lane * 8)) as u8;
        }
        left = sum;
    }
    let tail = done + whole;
    current[tail..].copy_from_slice(&source[tail..]);
}

/// Average and Paeth over pixels of `N` bytes. A missing row above
/// counts as zeros, as the specification says.
fn unfilter_lanes<const N: usize>(filter: u8, source: &[u8], previous: &[u8], current: &mut [u8]) {
    let zeros = [0u8; N];
    let mut left = [0u8; N];
    let mut above_left = [0u8; N];
    let mut above_pixels = previous.chunks_exact(N);
    for (target, bytes) in current.chunks_exact_mut(N).zip(source.chunks_exact(N)) {
        let above: [u8; N] = match above_pixels.next() {
            Some(pixel) => pixel.try_into().unwrap_or(zeros),
            None => zeros,
        };
        let mut pixel = [0u8; N];
        match filter {
            3 => {
                for lane in 0..N {
                    let mean = (u16::from(left[lane]) + u16::from(above[lane])) / 2;
                    pixel[lane] = bytes[lane].wrapping_add(mean as u8);
                }
            }
            _ => {
                for lane in 0..N {
                    let predicted = paeth(left[lane], above[lane], above_left[lane]);
                    pixel[lane] = bytes[lane].wrapping_add(predicted);
                }
            }
        }
        target.copy_from_slice(&pixel);
        left = pixel;
        above_left = above;
    }
    // A row whose length is not a whole number of pixels only happens
    // below eight bits per pixel, where the unit is one byte.
    let whole = (current.len() / N) * N;
    let length = current.len();
    current[whole..].copy_from_slice(&source[whole..length]);
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

/// The hub color type the file's pixels become.
fn output_color(header: &Header, transparency: &[u8]) -> ColorType {
    let keyed = !transparency.is_empty();
    match header.color {
        0 => {
            if keyed {
                ColorType::GrayAlpha
            } else {
                ColorType::Gray
            }
        }
        2 => {
            if keyed {
                ColorType::Rgba
            } else {
                ColorType::Rgb
            }
        }
        3 => {
            if keyed {
                ColorType::Rgba
            } else {
                ColorType::Rgb
            }
        }
        4 => ColorType::GrayAlpha,
        _ => ColorType::Rgba,
    }
}

fn row_mut(image: &mut Image, y: u32) -> &mut [u8] {
    let stride = image.stride();
    let start = y as usize * stride;
    &mut image.pixels[start..start + stride]
}

/// Turns one unfiltered file row of `width` pixels into hub pixels.
fn expand_row(
    header: &Header,
    source: &[u8],
    width: u32,
    palette: &[[u8; 3]],
    transparency: &[u8],
    target: &mut [u8],
) {
    let channels = target.len() / width as usize;
    match header.color {
        3 => {
            for x in 0..width as usize {
                let index = sample_bits(source, x, header.depth) as usize;
                let rgb = palette.get(index).copied().unwrap_or([0, 0, 0]);
                let cell = &mut target[x * channels..(x + 1) * channels];
                cell[..3].copy_from_slice(&rgb);
                if channels == 4 {
                    cell[3] = transparency.get(index).copied().unwrap_or(255);
                }
            }
        }
        0 => {
            let key = key_value(transparency, 0);
            for x in 0..width as usize {
                let raw = sample_bits(source, x, header.depth);
                let cell = &mut target[x * channels..(x + 1) * channels];
                cell[0] = scale_to_byte(raw, header.depth);
                if channels == 2 {
                    cell[1] = if Some(raw) == key { 0 } else { 255 };
                }
            }
        }
        2 => {
            let keys = [
                key_value(transparency, 0),
                key_value(transparency, 2),
                key_value(transparency, 4),
            ];
            for x in 0..width as usize {
                let cell = &mut target[x * channels..(x + 1) * channels];
                let mut raw = [0u16; 3];
                for (channel, value) in raw.iter_mut().enumerate() {
                    *value = sample_bits(source, x * 3 + channel, header.depth);
                    cell[channel] = scale_to_byte(*value, header.depth);
                }
                if channels == 4 {
                    let is_key = keys[0] == Some(raw[0])
                        && keys[1] == Some(raw[1])
                        && keys[2] == Some(raw[2]);
                    cell[3] = if is_key { 0 } else { 255 };
                }
            }
        }
        4 => {
            for x in 0..width as usize {
                let cell = &mut target[x * 2..(x + 1) * 2];
                cell[0] = scale_to_byte(sample_bits(source, x * 2, header.depth), header.depth);
                cell[1] = scale_to_byte(sample_bits(source, x * 2 + 1, header.depth), header.depth);
            }
        }
        _ => {
            for x in 0..width as usize {
                let cell = &mut target[x * 4..(x + 1) * 4];
                for (channel, value) in cell.iter_mut().enumerate() {
                    *value = scale_to_byte(
                        sample_bits(source, x * 4 + channel, header.depth),
                        header.depth,
                    );
                }
            }
        }
    }
}

/// The `index`th sample of a row at the file's bit depth.
fn sample_bits(row: &[u8], index: usize, depth: u8) -> u16 {
    match depth {
        8 => u16::from(row[index]),
        16 => u16::from_be_bytes([row[index * 2], row[index * 2 + 1]]),
        1 | 2 | 4 => {
            let per_byte = 8 / depth as usize;
            let byte = row[index / per_byte];
            let shift = 8 - depth as usize * (index % per_byte + 1);
            u16::from((byte >> shift) & ((1u8 << depth) - 1))
        }
        _ => 0,
    }
}

fn scale_to_byte(sample: u16, depth: u8) -> u8 {
    match depth {
        1 => (sample as u8) * 255,
        2 => (sample as u8) * 85,
        4 => (sample as u8) * 17,
        8 => sample as u8,
        _ => (sample >> 8) as u8,
    }
}

/// A tRNS key sample at `offset` (big-endian 16-bit), if present.
fn key_value(transparency: &[u8], offset: usize) -> Option<u16> {
    if transparency.len() >= offset + 2 {
        Some(u16::from_be_bytes([
            transparency[offset],
            transparency[offset + 1],
        ]))
    } else {
        None
    }
}

// ---- Adam7 ----

const PASS_START_X: [u32; 7] = [0, 4, 0, 2, 0, 1, 0];
const PASS_START_Y: [u32; 7] = [0, 0, 4, 0, 2, 0, 1];
const PASS_STEP_X: [u32; 7] = [8, 8, 4, 4, 2, 2, 1];
const PASS_STEP_Y: [u32; 7] = [8, 8, 8, 4, 4, 2, 2];

fn pass_size(header: &Header, pass: usize) -> (u32, u32) {
    let w = (header.width + PASS_STEP_X[pass] - 1 - PASS_START_X[pass]) / PASS_STEP_X[pass];
    let h = (header.height + PASS_STEP_Y[pass] - 1 - PASS_START_Y[pass]) / PASS_STEP_Y[pass];
    (w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_unpack_at_every_depth() {
        assert_eq!(sample_bits(&[0b1010_0000], 0, 1), 1);
        assert_eq!(sample_bits(&[0b1010_0000], 1, 1), 0);
        assert_eq!(sample_bits(&[0b1110_0100], 1, 2), 2);
        assert_eq!(sample_bits(&[0xAB], 1, 4), 0xB);
        assert_eq!(sample_bits(&[0x12, 0x34], 0, 16), 0x1234);
        assert_eq!(scale_to_byte(3, 2), 255);
        assert_eq!(scale_to_byte(0x1234, 16), 0x12);
    }

    #[test]
    fn paeth_picks_the_nearest_predictor() {
        assert_eq!(paeth(10, 20, 10), 20);
        assert_eq!(paeth(10, 20, 20), 10);
        assert_eq!(paeth(0, 0, 0), 0);
    }
}
