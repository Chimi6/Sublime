//! Netpbm: PBM, PGM, and PPM in their plain (ASCII, P1 to P3) and raw
//! (P4 to P6) forms, and PAM (P7), read into the image hub and written
//! from it. One reader takes every form whatever the extension; each
//! writer writes its own kind. Rows stream both ways.

use std::io::{self, Read, Write};

use crate::image::{ColorType, Image, MAX_PIXELS, flatten, luma_of};
use crate::io::png::{Collect, PngError, RowSink, RowsError};

/// Input read and output written in pieces this large.
const PIECE: usize = 64 * 1024;

/// Which Netpbm file a writer writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// P4: black and white, one bit a pixel.
    Pbm,
    /// P5: gray.
    Pgm,
    /// P6: color.
    Ppm,
    /// P7: gray or color, with or without alpha.
    Pam,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetpbmError(pub String);

impl std::fmt::Display for NetpbmError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl std::error::Error for NetpbmError {}

/// What reading dropped: samples wider than 8 bits were scaled down.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NetpbmNotes {
    pub maxval: u32,
    /// Samples over 8 bits went on at 16 bits (the sink took them).
    pub deep: bool,
}

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
    /// The next byte, or `None` at the end of the input.
    fn next(&mut self) -> Result<Option<u8>, RowsError> {
        if self.at == self.end {
            loop {
                match self.reader.read(&mut self.buffer) {
                    Ok(0) => return Ok(None),
                    Ok(count) => {
                        self.at = 0;
                        self.end = count;
                        break;
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(RowsError::Io(error)),
                }
            }
        }
        let byte = self.buffer[self.at];
        self.at += 1;
        Ok(Some(byte))
    }

    /// Fills `target` from the input: buffered bytes first, the rest read
    /// straight into it.
    fn fill(&mut self, target: &mut [u8]) -> Result<(), RowsError> {
        let held = (self.end - self.at).min(target.len());
        target[..held].copy_from_slice(&self.buffer[self.at..self.at + held]);
        self.at += held;
        if held < target.len() {
            match self.reader.read_exact(&mut target[held..]) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
                    return fail("Netpbm image data cut short");
                }
                Err(error) => return Err(RowsError::Io(error)),
            }
        }
        Ok(())
    }

    /// The next byte that is not whitespace or inside a `#` comment.
    fn token_start(&mut self) -> Result<Option<u8>, RowsError> {
        loop {
            match self.next()? {
                None => return Ok(None),
                Some(b'#') => loop {
                    match self.next()? {
                        None => return Ok(None),
                        Some(b'\n' | b'\r') => break,
                        Some(_) => {}
                    }
                },
                Some(byte) if byte.is_ascii_whitespace() => {}
                Some(byte) => return Ok(Some(byte)),
            }
        }
    }

    /// A decimal number and the one whitespace byte after it.
    fn number(&mut self) -> Result<u32, RowsError> {
        let Some(first) = self.token_start()? else {
            return fail("Netpbm header cut short");
        };
        if !first.is_ascii_digit() {
            return fail("Netpbm header holds a non-number");
        }
        let mut value = u32::from(first - b'0');
        loop {
            match self.next()? {
                Some(byte) if byte.is_ascii_digit() => {
                    value = value
                        .checked_mul(10)
                        .and_then(|value| value.checked_add(u32::from(byte - b'0')))
                        .ok_or_else(|| {
                            RowsError::Png(PngError("Netpbm number too large".into()))
                        })?;
                }
                Some(byte) if byte.is_ascii_whitespace() => return Ok(value),
                None => return Ok(value),
                Some(b'#') => {
                    // A comment right after a number ends it too.
                    self.at -= 1;
                    return Ok(value);
                }
                Some(_) => return fail("Netpbm header holds a non-number"),
            }
        }
    }

    /// One PAM header line, without its newline; `None` at the input's end.
    fn line(&mut self) -> Result<Option<String>, RowsError> {
        let mut line = Vec::new();
        loop {
            match self.next()? {
                None if line.is_empty() => return Ok(None),
                None | Some(b'\n') => return Ok(Some(String::from_utf8_lossy(&line).into_owned())),
                Some(byte) => line.push(byte),
            }
        }
    }
}

/// The raster's layout after the header.
struct Layout {
    width: u32,
    height: u32,
    depth: usize,
    maxval: u32,
    plain: bool,
    bits: bool,
}

fn read_header(source: &mut Source<'_>) -> Result<Layout, RowsError> {
    let (Some(b'P'), Some(kind)) = (source.next()?, source.next()?) else {
        return fail("not a Netpbm file");
    };
    let layout = match kind {
        b'1'..=b'6' => {
            let width = source.number()?;
            let height = source.number()?;
            let bits = kind == b'1' || kind == b'4';
            let maxval = if bits { 1 } else { source.number()? };
            let depth = if kind == b'3' || kind == b'6' { 3 } else { 1 };
            Layout {
                width,
                height,
                depth,
                maxval,
                plain: kind <= b'3',
                bits,
            }
        }
        b'7' => read_pam_header(source)?,
        _ => return fail("not a Netpbm file"),
    };
    if layout.width == 0 || layout.height == 0 {
        return fail("Netpbm image has no pixels");
    }
    if u64::from(layout.width) * u64::from(layout.height) > MAX_PIXELS {
        return fail("Netpbm image too large");
    }
    if layout.maxval == 0 || layout.maxval > 65_535 {
        return fail("Netpbm maxval is not 1 to 65535");
    }
    Ok(layout)
}

fn read_pam_header(source: &mut Source<'_>) -> Result<Layout, RowsError> {
    let (mut width, mut height, mut depth, mut maxval) = (0u32, 0u32, 0u32, 0u32);
    loop {
        let Some(line) = source.line()? else {
            return fail("PAM header has no ENDHDR");
        };
        let mut words = line.split_ascii_whitespace();
        let Some(key) = words.next() else {
            continue;
        };
        if key.starts_with('#') {
            continue;
        }
        if key == "ENDHDR" {
            break;
        }
        let value = || -> Result<u32, RowsError> {
            line.split_ascii_whitespace()
                .nth(1)
                .and_then(|value| value.parse().ok())
                .ok_or_else(|| RowsError::Png(PngError(format!("PAM {key} is not a number"))))
        };
        match key {
            "WIDTH" => width = value()?,
            "HEIGHT" => height = value()?,
            "DEPTH" => depth = value()?,
            "MAXVAL" => maxval = value()?,
            // The tuple type only names what the depth already says.
            _ => {}
        }
    }
    if !(1..=4).contains(&depth) {
        return fail("PAM depth is not 1 to 4");
    }
    Ok(Layout {
        width,
        height,
        depth: depth as usize,
        maxval,
        plain: false,
        bits: false,
    })
}

/// Reads any Netpbm file into `sink` one row at a time.
pub fn read_netpbm_rows(
    reader: &mut dyn Read,
    sink: &mut dyn RowSink,
) -> Result<NetpbmNotes, RowsError> {
    let mut source = Source {
        reader,
        buffer: vec![0; PIECE],
        at: 0,
        end: 0,
    };
    let layout = read_header(&mut source)?;
    let color = match layout.depth {
        1 => ColorType::Gray,
        2 => ColorType::GrayAlpha,
        3 => ColorType::Rgb,
        _ => ColorType::Rgba,
    };
    // Samples over 8 bits go on at 16 bits to a sink that holds them.
    let deep = layout.maxval > 255 && !layout.bits && sink.accept_deep(color);
    sink.start(layout.width, layout.height, color)
        .map_err(RowsError::Io)?;
    let samples = layout.width as usize * layout.depth;
    if deep {
        read_deep_rows(&mut source, &layout, samples, sink)?;
        return Ok(NetpbmNotes {
            maxval: layout.maxval,
            deep,
        });
    }
    let mut row = vec![0u8; samples];
    let maxval = layout.maxval;
    // Each 8-bit sample's value, scaled as Netpbm's tools scale.
    let table: Vec<u8> = (0..256u32)
        .map(|value| scale(value.min(maxval), maxval))
        .collect();
    if layout.bits && !layout.plain {
        let mut packed = vec![0u8; (layout.width as usize).div_ceil(8)];
        for _ in 0..layout.height {
            source.fill(&mut packed)?;
            for (x, target) in row.iter_mut().enumerate() {
                let black = packed[x / 8] & (0x80 >> (x % 8)) != 0;
                *target = if black { 0 } else { 255 };
            }
            sink.row(&row).map_err(RowsError::Io)?;
        }
    } else if layout.plain {
        for _ in 0..layout.height {
            for target in row.iter_mut() {
                *target = if layout.bits {
                    // Plain PBM digits need no separators: 1 is black.
                    match source.token_start()? {
                        Some(b'0') => 255,
                        Some(b'1') => 0,
                        Some(_) => return fail("plain PBM holds a digit other than 0 and 1"),
                        None => return fail("Netpbm image data cut short"),
                    }
                } else {
                    let value = source.number().map_err(|_| {
                        RowsError::Png(PngError("Netpbm image data cut short".into()))
                    })?;
                    if value > maxval {
                        return fail("Netpbm sample above maxval");
                    }
                    scale(value, maxval)
                };
            }
            sink.row(&row).map_err(RowsError::Io)?;
        }
    } else if maxval < 256 {
        for _ in 0..layout.height {
            source.fill(&mut row)?;
            if maxval != 255 {
                for sample in row.iter_mut() {
                    *sample = table[usize::from(*sample)];
                }
            }
            sink.row(&row).map_err(RowsError::Io)?;
        }
    } else {
        let mut wide = vec![0u8; samples * 2];
        for _ in 0..layout.height {
            source.fill(&mut wide)?;
            for (target, pair) in row.iter_mut().zip(wide.chunks_exact(2)) {
                let value = u32::from(u16::from_be_bytes([pair[0], pair[1]]));
                *target = scale(value.min(maxval), maxval);
            }
            sink.row(&row).map_err(RowsError::Io)?;
        }
    }
    Ok(NetpbmNotes {
        maxval,
        deep: false,
    })
}

/// Rows of samples over 8 bits, as big-endian 16-bit samples scaled to
/// 65535 (unchanged when maxval is 65535).
fn read_deep_rows(
    source: &mut Source<'_>,
    layout: &Layout,
    samples: usize,
    sink: &mut dyn RowSink,
) -> Result<(), RowsError> {
    let maxval = layout.maxval;
    let widen = |value: u32| -> [u8; 2] {
        let scaled = (u64::from(value) * 65_535 + u64::from(maxval) / 2) / u64::from(maxval);
        (scaled as u16).to_be_bytes()
    };
    let mut row = vec![0u8; samples * 2];
    let mut wide = vec![0u8; samples * 2];
    for _ in 0..layout.height {
        if layout.plain {
            for target in row.chunks_exact_mut(2) {
                let value = source
                    .number()
                    .map_err(|_| RowsError::Png(PngError("Netpbm image data cut short".into())))?;
                if value > maxval {
                    return fail("Netpbm sample above maxval");
                }
                target.copy_from_slice(&widen(value));
            }
        } else {
            source.fill(&mut wide)?;
            if maxval == 65_535 {
                row.copy_from_slice(&wide);
            } else {
                for (target, pair) in row.chunks_exact_mut(2).zip(wide.chunks_exact(2)) {
                    let value = u32::from(u16::from_be_bytes([pair[0], pair[1]]));
                    target.copy_from_slice(&widen(value.min(maxval)));
                }
            }
        }
        sink.row(&row).map_err(RowsError::Io)?;
    }
    Ok(())
}

/// A sample in `0..=maxval` as 8 bits, rounded (Netpbm's own scaling).
#[inline]
fn scale(value: u32, maxval: u32) -> u8 {
    ((value * 255 + maxval / 2) / maxval) as u8
}

/// Reads a whole Netpbm file into an image.
pub fn read_netpbm(bytes: &[u8]) -> Result<Image, NetpbmError> {
    let mut sink = Collect::default();
    match read_netpbm_rows(&mut &bytes[..], &mut sink) {
        Ok(_) => Ok(sink.image),
        Err(RowsError::Png(error)) => Err(NetpbmError(error.0)),
        Err(RowsError::Io(error)) => Err(NetpbmError(format!("row sink failed: {error}"))),
    }
}

/// Writes one Netpbm kind from rows as a `RowSink`. PPM and PGM flatten
/// alpha onto white; PGM and PBM take luma; PBM is black below half.
pub struct NetpbmRows<'a> {
    sink: &'a mut dyn Write,
    kind: Kind,
    color: ColorType,
    rows_left: u32,
    out: Vec<u8>,
    /// 16-bit samples, written as they come (maxval 65535 is big-endian).
    deep: bool,
}

impl<'a> NetpbmRows<'a> {
    pub fn new(sink: &'a mut dyn Write, kind: Kind) -> NetpbmRows<'a> {
        NetpbmRows {
            sink,
            kind,
            color: ColorType::Rgb,
            rows_left: 0,
            out: Vec::with_capacity(PIECE + 1024),
            deep: false,
        }
    }

    /// A pixel's gray level over white (PGM and PBM).
    fn gray(&self, cell: &[u8]) -> u8 {
        match self.color {
            ColorType::Gray => cell[0],
            ColorType::GrayAlpha => flatten(cell[0], cell[1]),
            ColorType::Rgb => luma_of(cell[0], cell[1], cell[2]),
            ColorType::Rgba => luma_of(
                flatten(cell[0], cell[3]),
                flatten(cell[1], cell[3]),
                flatten(cell[2], cell[3]),
            ),
        }
    }
}

impl RowSink for NetpbmRows<'_> {
    fn start(&mut self, width: u32, height: u32, color: ColorType) -> io::Result<()> {
        self.color = color;
        self.rows_left = height;
        let maxval = if self.deep { 65_535 } else { 255 };
        let header = match self.kind {
            Kind::Pbm => format!("P4\n{width} {height}\n"),
            Kind::Pgm => format!("P5\n{width} {height}\n{maxval}\n"),
            Kind::Ppm => format!("P6\n{width} {height}\n{maxval}\n"),
            Kind::Pam => {
                let (depth, tuple_type) = match color {
                    ColorType::Gray => (1, "GRAYSCALE"),
                    ColorType::GrayAlpha => (2, "GRAYSCALE_ALPHA"),
                    ColorType::Rgb => (3, "RGB"),
                    ColorType::Rgba => (4, "RGB_ALPHA"),
                };
                format!(
                    "P7\nWIDTH {width}\nHEIGHT {height}\nDEPTH {depth}\nMAXVAL {maxval}\nTUPLTYPE {tuple_type}\nENDHDR\n"
                )
            }
        };
        self.out.extend_from_slice(header.as_bytes());
        Ok(())
    }

    /// Deep rows go straight through where the format holds the color as
    /// it is: PAM always, PPM for RGB, PGM for gray.
    fn accept_deep(&mut self, color: ColorType) -> bool {
        self.deep = matches!(
            (self.kind, color),
            (Kind::Pam, _) | (Kind::Ppm, ColorType::Rgb) | (Kind::Pgm, ColorType::Gray)
        );
        self.deep
    }

    fn row(&mut self, pixels: &[u8]) -> io::Result<()> {
        let channels = self.color.channels();
        if self.deep {
            self.out.extend_from_slice(pixels);
            self.rows_left = self.rows_left.saturating_sub(1);
            if self.out.len() >= PIECE || self.rows_left == 0 {
                self.sink.write_all(&self.out)?;
                self.out.clear();
            }
            if self.rows_left == 0 {
                self.sink.flush()?;
            }
            return Ok(());
        }
        match self.kind {
            Kind::Pam => self.out.extend_from_slice(pixels),
            Kind::Ppm => match self.color {
                ColorType::Rgb => self.out.extend_from_slice(pixels),
                ColorType::Rgba => {
                    for cell in pixels.chunks_exact(4) {
                        self.out.extend_from_slice(&[
                            flatten(cell[0], cell[3]),
                            flatten(cell[1], cell[3]),
                            flatten(cell[2], cell[3]),
                        ]);
                    }
                }
                ColorType::Gray | ColorType::GrayAlpha => {
                    for cell in pixels.chunks_exact(channels) {
                        let value = self.gray(cell);
                        self.out.extend_from_slice(&[value, value, value]);
                    }
                }
            },
            Kind::Pgm => {
                if self.color == ColorType::Gray {
                    self.out.extend_from_slice(pixels);
                } else {
                    for cell in pixels.chunks_exact(channels) {
                        let value = self.gray(cell);
                        self.out.push(value);
                    }
                }
            }
            Kind::Pbm => {
                let width = pixels.len() / channels;
                let start = self.out.len();
                self.out.resize(start + width.div_ceil(8), 0);
                for (x, cell) in pixels.chunks_exact(channels).enumerate() {
                    if self.gray(cell) < 128 {
                        self.out[start + x / 8] |= 0x80 >> (x % 8);
                    }
                }
            }
        }
        self.rows_left -= 1;
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

/// Writes a whole image as one Netpbm kind.
pub fn write_netpbm(image: &Image, sink: &mut dyn Write, kind: Kind) -> io::Result<()> {
    let mut rows = NetpbmRows::new(sink, kind);
    rows.start(image.width, image.height, image.color)?;
    for y in 0..image.height {
        rows.row(image.row(y))?;
    }
    Ok(())
}
