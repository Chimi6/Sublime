//! Decodes a baseline JPEG into the image hub the way libjpeg does, so the
//! pixels match what browsers and Pillow produce: the IJG integer IDCT
//! (`jidctint`), triangle ("fancy") chroma upsampling, and fixed-point
//! YCbCr conversion. An interleaved scan streams one MCU row at a time
//! into a row sink; a file with one scan per component is decoded into
//! coefficients first, as is a progressive file. Metadata
//! segments (Exif, ICC, comments) are skipped and reported by name.

use std::io::Read;

use crate::image::{ColorType, Image};
use crate::io::png::{RowSink, RowsError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JpegError(pub String);

impl std::fmt::Display for JpegError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl std::error::Error for JpegError {}

fn fail<T>(message: impl Into<String>) -> Result<T, JpegError> {
    Err(JpegError(message.into()))
}

/// What the reader skipped or noticed, for the converter to report.
#[derive(Debug, Default)]
pub struct JpegNotes {
    /// Metadata segments left out, one name per kind (Exif, ICC, XMP,
    /// comment, and so on).
    pub dropped: Vec<String>,
    /// The Exif orientation tag when it is not 1: the pixels are as
    /// stored, and a viewer would rotate them.
    pub orientation: Option<u16>,
    pub progressive: bool,
}

pub fn read_jpeg(bytes: &[u8]) -> Result<(Image, JpegNotes), JpegError> {
    let mut source: &[u8] = bytes;
    read_jpeg_from(&mut source)
}

pub fn read_jpeg_from(reader: &mut dyn Read) -> Result<(Image, JpegNotes), JpegError> {
    let mut sink = Collect::default();
    let notes = match read_jpeg_rows(reader, &mut sink) {
        Ok(notes) => notes,
        Err(RowsError::Png(error)) => return Err(JpegError(error.0)),
        Err(RowsError::Io(error)) => return Err(JpegError(format!("row sink failed: {error}"))),
    };
    Ok((sink.image, notes))
}

/// A sink that keeps the rows as an image.
#[derive(Default)]
struct Collect {
    image: Image,
}

impl RowSink for Collect {
    fn start(&mut self, width: u32, height: u32, color: ColorType) -> std::io::Result<()> {
        self.image = Image::new(width, height, color);
        self.image.pixels.clear();
        Ok(())
    }

    fn row(&mut self, pixels: &[u8]) -> std::io::Result<()> {
        self.image.pixels.extend_from_slice(pixels);
        Ok(())
    }
}

/// Reads a JPEG from a stream straight into `sink`, one row at a time.
/// The file is read whole (its entropy-coded data is a bit stream that
/// cannot be split at row boundaries) and the pixels are never held.
pub fn read_jpeg_rows(
    reader: &mut dyn Read,
    sink: &mut dyn RowSink,
) -> Result<JpegNotes, RowsError> {
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).map_err(RowsError::Io)?;
    let mut decoder = Decoder::new(&bytes);
    decoder.run(sink).map_err(|error| match error {
        Failure::Jpeg(error) => RowsError::Png(crate::io::png::PngError(error.0)),
        Failure::Io(error) => RowsError::Io(error),
    })?;
    Ok(decoder.notes)
}

enum Failure {
    Jpeg(JpegError),
    Io(std::io::Error),
}

impl From<JpegError> for Failure {
    fn from(error: JpegError) -> Self {
        Failure::Jpeg(error)
    }
}

impl From<std::io::Error> for Failure {
    fn from(error: std::io::Error) -> Self {
        Failure::Io(error)
    }
}

// ---------------------------------------------------------------- tables

/// Zigzag position to natural (row-major) position.
const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

const LOOKAHEAD: u32 = 9;

/// A Huffman table as libjpeg keeps it: a lookahead table for codes up
/// to nine bits, and the canonical `maxcode`/`valoffset` walk for the
/// rest.
#[derive(Clone)]
struct Huffman {
    /// `length << 8 | symbol` for the nine-bit prefix, zero when the code
    /// is longer.
    lookup: Vec<u16>,
    maxcode: [i32; 18],
    valoffset: [i32; 17],
    values: Vec<u8>,
}

impl Huffman {
    fn build(counts: &[u8; 16], values: &[u8]) -> Result<Huffman, JpegError> {
        let mut sizes: Vec<u8> = Vec::with_capacity(values.len());
        for (length, count) in counts.iter().enumerate() {
            for _ in 0..*count {
                sizes.push(length as u8 + 1);
            }
        }
        if sizes.len() != values.len() {
            return fail("Huffman table counts do not match its values");
        }
        let mut codes: Vec<u32> = Vec::with_capacity(sizes.len());
        let mut code = 0u32;
        let mut size = sizes.first().copied().unwrap_or(1);
        let mut index = 0;
        while index < sizes.len() {
            while index < sizes.len() && sizes[index] == size {
                codes.push(code);
                code += 1;
                index += 1;
            }
            if code >= 1u32 << size {
                return fail("over-subscribed Huffman table");
            }
            code <<= 1;
            size += 1;
        }
        let mut maxcode = [-1i32; 18];
        let mut valoffset = [0i32; 17];
        let mut position = 0usize;
        for length in 1..=16usize {
            let count = usize::from(counts[length - 1]);
            if count > 0 {
                valoffset[length] = position as i32 - codes[position] as i32;
                position += count;
                maxcode[length] = codes[position - 1] as i32;
            } else {
                maxcode[length] = -1;
            }
        }
        maxcode[17] = i32::MAX;
        let mut lookup = vec![0u16; 1 << LOOKAHEAD];
        for (index, size) in sizes.iter().enumerate() {
            let size = u32::from(*size);
            if size <= LOOKAHEAD {
                let first = (codes[index] << (LOOKAHEAD - size)) as usize;
                let span = 1usize << (LOOKAHEAD - size);
                let entry = ((size as u16) << 8) | u16::from(values[index]);
                for slot in &mut lookup[first..first + span] {
                    *slot = entry;
                }
            }
        }
        Ok(Huffman {
            lookup,
            maxcode,
            valoffset,
            values: values.to_vec(),
        })
    }
}

/// The entropy-coded bit stream: bytes with `FF 00` stuffing, ending at
/// any other marker, after which zeros are fed.
struct Bits<'a> {
    bytes: &'a [u8],
    position: usize,
    buffer: u64,
    count: u32,
    /// A marker met while filling; decoding past it reads zeros.
    marker: Option<u8>,
    /// Where the current entropy-coded interval began; the marker that
    /// ends a scan is never before it.
    floor: usize,
}

impl<'a> Bits<'a> {
    fn new(bytes: &'a [u8], position: usize) -> Bits<'a> {
        Bits {
            bytes,
            position,
            buffer: 0,
            count: 0,
            marker: None,
            floor: position,
        }
    }

    /// Where the marker that ends the scan begins: the filler read at
    /// most eight bytes ahead of what was decoded, so it is the first
    /// marker at or after that point.
    fn end_of_scan(&self) -> usize {
        let mut at = self.position.saturating_sub(8).max(self.floor);
        while at + 1 < self.bytes.len() {
            if self.bytes[at] == 0xFF && self.bytes[at + 1] != 0 && self.bytes[at + 1] != 0xFF {
                return at;
            }
            at += 1;
        }
        self.bytes.len()
    }

    #[inline]
    fn fill(&mut self) {
        while self.count <= 56 {
            let byte = if self.marker.is_some() || self.position >= self.bytes.len() {
                0
            } else {
                let byte = self.bytes[self.position];
                if byte == 0xFF {
                    let next = self.bytes.get(self.position + 1).copied().unwrap_or(0xD9);
                    if next == 0 {
                        self.position += 2;
                        0xFF
                    } else {
                        // A marker: the stream ends here.
                        self.marker = Some(next);
                        0
                    }
                } else {
                    self.position += 1;
                    byte
                }
            };
            self.buffer |= u64::from(byte) << (56 - self.count);
            self.count += 8;
        }
    }

    #[inline]
    fn peek(&mut self, n: u32) -> u32 {
        if self.count < n {
            self.fill();
        }
        (self.buffer >> (64 - n)) as u32
    }

    #[inline]
    fn skip(&mut self, n: u32) {
        self.buffer <<= n;
        self.count -= n;
    }

    #[inline]
    fn bits(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        let value = self.peek(n);
        self.skip(n);
        value
    }

    #[inline]
    fn decode(&mut self, table: &Huffman) -> Result<u8, JpegError> {
        let look = self.peek(LOOKAHEAD) as usize;
        let entry = table.lookup[look];
        if entry != 0 {
            self.skip(u32::from(entry >> 8));
            return Ok(entry as u8);
        }
        // Longer than the lookahead: walk the lengths.
        let mut length = LOOKAHEAD + 1;
        let mut code = self.peek(16) as i32;
        while length <= 16 {
            let candidate = code >> (16 - length);
            if candidate <= table.maxcode[length as usize] {
                self.skip(length);
                let index = candidate + table.valoffset[length as usize];
                return table
                    .values
                    .get(index as usize)
                    .copied()
                    .ok_or_else(|| JpegError("bad Huffman code".to_string()));
            }
            length += 1;
        }
        code = 0;
        let _ = code;
        fail("bad Huffman code")
    }

    /// `n` extra bits as a signed magnitude.
    #[inline]
    fn extend(&mut self, n: u32) -> i32 {
        if n == 0 {
            return 0;
        }
        let value = self.bits(n) as i32;
        if value < (1 << (n - 1)) {
            value - (1 << n) + 1
        } else {
            value
        }
    }

    /// Drops buffered bits and stands at the next marker (for restarts).
    fn resync(&mut self) -> Option<u8> {
        self.buffer = 0;
        self.count = 0;
        if let Some(marker) = self.marker.take() {
            // Skip the FF xx pair.
            self.position += 2;
            self.floor = self.position;
            return Some(marker);
        }
        while self.position + 1 < self.bytes.len() {
            if self.bytes[self.position] == 0xFF
                && self.bytes[self.position + 1] != 0
                && self.bytes[self.position + 1] != 0xFF
            {
                let marker = self.bytes[self.position + 1];
                self.position += 2;
                self.floor = self.position;
                return Some(marker);
            }
            self.position += 1;
        }
        None
    }
}

// ------------------------------------------------------------- the frame

#[derive(Clone)]
struct Component {
    id: u8,
    h: usize,
    v: usize,
    quant: usize,
    dc_table: usize,
    ac_table: usize,
    /// Blocks per line and lines of blocks over the whole image (padded
    /// to whole MCUs).
    blocks_wide: usize,
    blocks_high: usize,
}

struct Frame {
    width: u32,
    height: u32,
    components: Vec<Component>,
    h_max: usize,
    v_max: usize,
    mcus_wide: usize,
    mcus_high: usize,
    progressive: bool,
}

/// One component's samples for one MCU row, with one context row above
/// and one below for the triangle upsampler.
struct Plane {
    width: usize,
    height: usize,
    /// `(height + 2) * width`: row 0 is the context row above.
    samples: Vec<u8>,
}

impl Plane {
    fn new(width: usize, height: usize) -> Plane {
        Plane {
            width,
            height,
            samples: vec![0; (height + 2) * width],
        }
    }

    fn row(&self, y: usize) -> &[u8] {
        &self.samples[(y + 1) * self.width..(y + 2) * self.width]
    }

    fn row_mut(&mut self, y: usize) -> &mut [u8] {
        &mut self.samples[(y + 1) * self.width..(y + 2) * self.width]
    }

    fn above_mut(&mut self) -> &mut [u8] {
        &mut self.samples[..self.width]
    }

    fn below_mut(&mut self) -> &mut [u8] {
        let start = (self.height + 1) * self.width;
        &mut self.samples[start..start + self.width]
    }
}

struct Decoder<'a> {
    bytes: &'a [u8],
    at: usize,
    quant: [Option<[i32; 64]>; 4],
    dc_tables: [Option<Huffman>; 4],
    ac_tables: [Option<Huffman>; 4],
    frame: Option<Frame>,
    restart_interval: usize,
    adobe_transform: Option<u8>,
    jfif: bool,
    notes: JpegNotes,
    /// Coefficients per component (blocks in the padded grid, 64 each),
    /// for progressive files and baseline files with a scan per
    /// component; empty on the streaming path.
    coefficients: Vec<Vec<i16>>,
    /// The end-of-band run carried between blocks of a progressive AC
    /// scan.
    eob_run: u32,
}

impl<'a> Decoder<'a> {
    fn new(bytes: &'a [u8]) -> Decoder<'a> {
        Decoder {
            bytes,
            at: 0,
            quant: [None, None, None, None],
            dc_tables: [None, None, None, None],
            ac_tables: [None, None, None, None],
            frame: None,
            restart_interval: 0,
            adobe_transform: None,
            jfif: false,
            notes: JpegNotes::default(),
            coefficients: Vec::new(),
            eob_run: 0,
        }
    }

    fn u16_at(&self, at: usize) -> Result<u16, JpegError> {
        if at + 2 > self.bytes.len() {
            return fail("file cut short");
        }
        Ok(u16::from_be_bytes([self.bytes[at], self.bytes[at + 1]]))
    }

    /// The next segment: marker and payload (after its length).
    fn segment(&mut self) -> Result<(u8, &'a [u8]), JpegError> {
        // Skip fill bytes to the marker.
        while self.at < self.bytes.len() && self.bytes[self.at] == 0xFF {
            self.at += 1;
        }
        if self.at >= self.bytes.len() {
            return fail("file cut short before the end marker");
        }
        let marker = self.bytes[self.at];
        self.at += 1;
        if matches!(marker, 0xD8 | 0xD9 | 0x01 | 0xD0..=0xD7) {
            return Ok((marker, &[]));
        }
        let length = usize::from(self.u16_at(self.at)?);
        if length < 2 || self.at + length > self.bytes.len() {
            return fail("segment length runs past the file");
        }
        let payload = &self.bytes[self.at + 2..self.at + length];
        self.at += length;
        Ok((marker, payload))
    }

    fn run(&mut self, sink: &mut dyn RowSink) -> Result<(), Failure> {
        if self.bytes.len() < 4 || self.bytes[0] != 0xFF || self.bytes[1] != 0xD8 {
            return Err(JpegError("not a JPEG: bad signature".to_string()).into());
        }
        self.at = 2;
        let mut started = false;
        loop {
            let (marker, payload) = self.segment()?;
            match marker {
                0xD8 => {}
                0xD9 => {
                    if !started {
                        return Err(
                            JpegError("no image data before the end marker".to_string()).into()
                        );
                    }
                    if !self.coefficients.is_empty() {
                        self.finish_buffered(sink)?;
                    }
                    return Ok(());
                }
                0xDB => self.quantization(payload)?,
                0xC4 => self.huffman(payload)?,
                0xDD => {
                    if payload.len() < 2 {
                        return Err(JpegError("bad restart interval".to_string()).into());
                    }
                    self.restart_interval =
                        usize::from(u16::from_be_bytes([payload[0], payload[1]]));
                }
                0xC0..=0xC2 => {
                    if self.frame.is_some() {
                        return Err(JpegError("two frame headers".to_string()).into());
                    }
                    self.frame = Some(self.frame_header(payload, marker == 0xC2)?);
                }
                0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => {
                    return Err(JpegError(
                        "lossless, hierarchical, and arithmetic-coded JPEG are not supported"
                            .to_string(),
                    )
                    .into());
                }
                0xDA => {
                    started = true;
                    self.scan(payload, sink)?;
                }
                0xE0 => {
                    if payload.starts_with(b"JFIF\0") {
                        self.jfif = true;
                    } else {
                        self.drop("APP0");
                    }
                }
                0xE1 => {
                    if payload.starts_with(b"Exif\0\0") {
                        self.exif(&payload[6..]);
                        self.drop("Exif");
                    } else if payload.starts_with(b"http://ns.adobe.com/xap/1.0/\0") {
                        self.drop("XMP");
                    } else {
                        self.drop("APP1");
                    }
                }
                0xE2 => {
                    if payload.starts_with(b"ICC_PROFILE\0") {
                        self.drop("ICC profile");
                    } else {
                        self.drop("APP2");
                    }
                }
                0xEE => {
                    if payload.starts_with(b"Adobe") && payload.len() >= 12 {
                        self.adobe_transform = Some(payload[11]);
                    }
                }
                0xE3..=0xED | 0xEF => self.drop(&format!("APP{}", marker - 0xE0)),
                0xFE => self.drop("comment"),
                0xDC => {}
                _ => {}
            }
        }
    }

    fn drop(&mut self, name: &str) {
        if !self.notes.dropped.iter().any(|seen| seen == name) {
            self.notes.dropped.push(name.to_string());
        }
    }

    /// The orientation tag out of an Exif TIFF header, when present.
    fn exif(&mut self, tiff: &[u8]) {
        if tiff.len() < 8 {
            return;
        }
        let little = &tiff[..2] == b"II";
        let read16 = |at: usize| -> Option<u16> {
            let pair = tiff.get(at..at + 2)?;
            Some(if little {
                u16::from_le_bytes([pair[0], pair[1]])
            } else {
                u16::from_be_bytes([pair[0], pair[1]])
            })
        };
        let read32 = |at: usize| -> Option<u32> {
            let quad = tiff.get(at..at + 4)?;
            Some(if little {
                u32::from_le_bytes([quad[0], quad[1], quad[2], quad[3]])
            } else {
                u32::from_be_bytes([quad[0], quad[1], quad[2], quad[3]])
            })
        };
        let Some(ifd) = read32(4) else { return };
        let ifd = ifd as usize;
        let Some(count) = read16(ifd) else { return };
        for index in 0..usize::from(count) {
            let entry = ifd + 2 + index * 12;
            if read16(entry) == Some(0x0112) {
                if let Some(value) = read16(entry + 8) {
                    if value != 1 && (1..=8).contains(&value) {
                        self.notes.orientation = Some(value);
                    }
                }
                return;
            }
        }
    }

    fn quantization(&mut self, mut payload: &[u8]) -> Result<(), JpegError> {
        while !payload.is_empty() {
            let precision = payload[0] >> 4;
            let id = usize::from(payload[0] & 15);
            if id > 3 {
                return fail("bad quantization table id");
            }
            let entry = if precision == 0 { 1 } else { 2 };
            if payload.len() < 1 + 64 * entry {
                return fail("quantization table cut short");
            }
            let mut table = [0i32; 64];
            for (zig, slot) in (0..64).map(|zig| ZIGZAG[zig]).enumerate() {
                let at = 1 + zig * entry;
                table[slot] = if entry == 1 {
                    i32::from(payload[at])
                } else {
                    i32::from(u16::from_be_bytes([payload[at], payload[at + 1]]))
                };
            }
            self.quant[id] = Some(table);
            payload = &payload[1 + 64 * entry..];
        }
        Ok(())
    }

    fn huffman(&mut self, mut payload: &[u8]) -> Result<(), JpegError> {
        while !payload.is_empty() {
            if payload.len() < 17 {
                return fail("Huffman table cut short");
            }
            let class = payload[0] >> 4;
            let id = usize::from(payload[0] & 15);
            if class > 1 || id > 3 {
                return fail("bad Huffman table header");
            }
            let mut counts = [0u8; 16];
            counts.copy_from_slice(&payload[1..17]);
            let total: usize = counts.iter().map(|count| usize::from(*count)).sum();
            if total > 256 || payload.len() < 17 + total {
                return fail("Huffman table cut short");
            }
            let table = Huffman::build(&counts, &payload[17..17 + total])?;
            if class == 0 {
                self.dc_tables[id] = Some(table);
            } else {
                self.ac_tables[id] = Some(table);
            }
            payload = &payload[17 + total..];
        }
        Ok(())
    }

    fn frame_header(&mut self, payload: &[u8], progressive: bool) -> Result<Frame, JpegError> {
        if payload.len() < 6 {
            return fail("frame header cut short");
        }
        if payload[0] != 8 {
            return fail(format!("{}-bit samples are not supported", payload[0]));
        }
        let height = u32::from(u16::from_be_bytes([payload[1], payload[2]]));
        let width = u32::from(u16::from_be_bytes([payload[3], payload[4]]));
        let count = usize::from(payload[5]);
        if width == 0 || height == 0 {
            return fail(
                "image dimensions are zero (a height given by a DNL marker is not supported)",
            );
        }
        if !(count == 1 || count == 3) {
            return fail(format!(
                "{count}-component images (CMYK, YCCK) are not supported"
            ));
        }
        if payload.len() < 6 + 3 * count {
            return fail("frame header cut short");
        }
        let mut components = Vec::with_capacity(count);
        for index in 0..count {
            let at = 6 + index * 3;
            let h = usize::from(payload[at + 1] >> 4);
            let v = usize::from(payload[at + 1] & 15);
            let quant = usize::from(payload[at + 2]);
            if !(1..=4).contains(&h) || !(1..=4).contains(&v) || quant > 3 {
                return fail("bad component sampling or table id");
            }
            components.push(Component {
                id: payload[at],
                h,
                v,
                quant,
                dc_table: 0,
                ac_table: 0,
                blocks_wide: 0,
                blocks_high: 0,
            });
        }
        let h_max = components.iter().map(|c| c.h).max().unwrap_or(1);
        let v_max = components.iter().map(|c| c.v).max().unwrap_or(1);
        if count == 1 {
            // A single component is not subsampled whatever its factors.
            components[0].h = 1;
            components[0].v = 1;
        }
        let (h_max, v_max) = if count == 1 { (1, 1) } else { (h_max, v_max) };
        let mcus_wide = (width as usize).div_ceil(8 * h_max);
        let mcus_high = (height as usize).div_ceil(8 * v_max);
        for component in &mut components {
            component.blocks_wide = mcus_wide * component.h;
            component.blocks_high = mcus_high * component.v;
        }
        if !Image::dimensions_fit(width, height, 3) {
            return fail("image is too large");
        }
        self.notes.progressive = progressive;
        Ok(Frame {
            width,
            height,
            components,
            h_max,
            v_max,
            mcus_wide,
            mcus_high,
            progressive,
        })
    }

    fn scan(&mut self, payload: &[u8], sink: &mut dyn RowSink) -> Result<(), Failure> {
        let frame = self
            .frame
            .as_ref()
            .ok_or_else(|| JpegError("scan before the frame header".to_string()))?;
        if payload.is_empty() {
            return Err(JpegError("scan header cut short".to_string()).into());
        }
        let count = usize::from(payload[0]);
        if payload.len() < 1 + 2 * count + 3 {
            return Err(JpegError("scan header cut short".to_string()).into());
        }
        let mut members: Vec<usize> = Vec::with_capacity(count);
        let mut components = frame.components.clone();
        for index in 0..count {
            let id = payload[1 + index * 2];
            let tables = payload[2 + index * 2];
            let position = components
                .iter()
                .position(|c| c.id == id)
                .ok_or_else(|| JpegError("scan names an unknown component".to_string()))?;
            components[position].dc_table = usize::from(tables >> 4);
            components[position].ac_table = usize::from(tables & 15);
            if components[position].dc_table > 3 || components[position].ac_table > 3 {
                return Err(JpegError("bad Huffman table id in scan".to_string()).into());
            }
            members.push(position);
        }
        let spectral_start = usize::from(payload[1 + 2 * count]);
        let spectral_end = usize::from(payload[2 + 2 * count]);
        let approximation = payload[3 + 2 * count];
        let (high, low) = (u32::from(approximation >> 4), u32::from(approximation & 15));
        for position in &members {
            let component = &components[*position];
            if self.quant[component.quant].is_none() {
                return Err(JpegError(
                    "scan uses a quantization table that was never defined".to_string(),
                )
                .into());
            }
        }
        if frame.progressive || count != frame.components.len() || !self.coefficients.is_empty() {
            let frame = Frame {
                width: frame.width,
                height: frame.height,
                components: frame.components.clone(),
                h_max: frame.h_max,
                v_max: frame.v_max,
                mcus_wide: frame.mcus_wide,
                mcus_high: frame.mcus_high,
                progressive: frame.progressive,
            };
            let scan = Scan {
                members,
                components,
                spectral_start,
                spectral_end,
                high,
                low,
            };
            return self.buffered_scan(&frame, &scan).map_err(Failure::from);
        }
        for position in &members {
            let component = &components[*position];
            if self.dc_tables[component.dc_table].is_none()
                || self.ac_tables[component.ac_table].is_none()
            {
                return Err(JpegError(
                    "scan uses a Huffman table that was never defined".to_string(),
                )
                .into());
            }
        }
        let frame = Frame {
            width: frame.width,
            height: frame.height,
            components,
            h_max: frame.h_max,
            v_max: frame.v_max,
            mcus_wide: frame.mcus_wide,
            mcus_high: frame.mcus_high,
            progressive: false,
        };
        let color = if frame.components.len() == 1 {
            ColorType::Gray
        } else {
            ColorType::Rgb
        };
        sink.start(frame.width, frame.height, color)?;
        let mut bits = Bits::new(self.bytes, self.at);
        let mut predictions = [0i32; 4];
        let mut coefficients = [0i32; 64];
        let mut block = [0u8; 64];
        // Planes for the previous and the current MCU row: rows go out one
        // MCU row late so the row below is known for the upsampler.
        let plane_height = |c: &Component| c.v * 8;
        let mut current: Vec<Plane> = frame
            .components
            .iter()
            .map(|c| Plane::new(c.blocks_wide * 8, plane_height(c)))
            .collect();
        let mut previous: Option<Vec<Plane>> = None;
        let mut emitted = 0u32;
        let mut mcus_done = 0usize;
        let rows_per_mcu = frame.v_max * 8;
        let stride = frame.width as usize * color.channels();
        let mut out_row = vec![0u8; frame.width as usize * color.channels() + 16];
        let mut upsampled: Vec<Vec<u8>> = frame
            .components
            .iter()
            .map(|_| vec![0u8; frame.mcus_wide * frame.h_max * 8])
            .collect();
        for _mcu_y in 0..frame.mcus_high {
            for mcu_x in 0..frame.mcus_wide {
                if self.restart_interval > 0
                    && mcus_done > 0
                    && mcus_done % self.restart_interval == 0
                {
                    match bits.resync() {
                        Some(marker) if (0xD0..=0xD7).contains(&marker) => {}
                        _ => return Err(JpegError("missing restart marker".to_string()).into()),
                    }
                    predictions = [0; 4];
                }
                for (index, component) in frame.components.iter().enumerate() {
                    let quant = self.quant[component.quant].as_ref().expect("checked");
                    let dc = self.dc_tables[component.dc_table]
                        .as_ref()
                        .expect("checked");
                    let ac = self.ac_tables[component.ac_table]
                        .as_ref()
                        .expect("checked");
                    for by in 0..component.v {
                        for bx in 0..component.h {
                            decode_block(
                                &mut bits,
                                dc,
                                ac,
                                quant,
                                &mut predictions[index],
                                &mut coefficients,
                            )?;
                            idct(&coefficients, &mut block);
                            let plane = &mut current[index];
                            let x0 = (mcu_x * component.h + bx) * 8;
                            let y0 = by * 8;
                            for row in 0..8 {
                                let target = plane.row_mut(y0 + row);
                                target[x0..x0 + 8].copy_from_slice(&block[row * 8..row * 8 + 8]);
                            }
                        }
                    }
                }
                mcus_done += 1;
                if bits.marker.is_some()
                    && bits.marker != Some(0xD9)
                    && !(0xD0..=0xD7).contains(&bits.marker.unwrap_or(0))
                {
                    // Unexpected marker inside the scan: the file is cut.
                }
            }
            // Context rows: the current row's first line goes below the
            // previous row's plane; the previous row's last line goes above
            // the current one.
            if let Some(previous_planes) = previous.as_mut() {
                for (index, plane) in previous_planes.iter_mut().enumerate() {
                    let first = current[index].row(0).to_vec();
                    plane.below_mut().copy_from_slice(&first);
                }
                for (index, plane) in current.iter_mut().enumerate() {
                    let last = previous_planes[index]
                        .row(previous_planes[index].height - 1)
                        .to_vec();
                    plane.above_mut().copy_from_slice(&last);
                }
                emit_mcu_row(
                    &frame,
                    previous_planes,
                    &mut upsampled,
                    &mut out_row,
                    stride,
                    rows_per_mcu,
                    &mut emitted,
                    self.adobe_transform,
                    sink,
                )?;
            } else {
                for plane in current.iter_mut() {
                    let first = plane.row(0).to_vec();
                    plane.above_mut().copy_from_slice(&first);
                }
            }
            let finished = std::mem::replace(
                &mut current,
                frame
                    .components
                    .iter()
                    .map(|c| Plane::new(c.blocks_wide * 8, plane_height(c)))
                    .collect(),
            );
            previous = Some(finished);
        }
        if let Some(previous_planes) = previous.as_mut() {
            replicate_bottom(&frame, previous_planes);
            emit_mcu_row(
                &frame,
                previous_planes,
                &mut upsampled,
                &mut out_row,
                stride,
                rows_per_mcu,
                &mut emitted,
                self.adobe_transform,
                sink,
            )?;
        }
        // Stand at the marker that ended the scan.
        self.at = bits.end_of_scan();
        if emitted != frame.height {
            return Err(JpegError("scan ended before the last row".to_string()).into());
        }
        Ok(())
    }
}

/// Decodes one block's coefficients, dequantized, in natural order.
#[inline]
fn decode_block(
    bits: &mut Bits<'_>,
    dc: &Huffman,
    ac: &Huffman,
    quant: &[i32; 64],
    prediction: &mut i32,
    out: &mut [i32; 64],
) -> Result<(), JpegError> {
    out.fill(0);
    let size = u32::from(bits.decode(dc)?);
    if size > 11 {
        return fail("bad DC magnitude");
    }
    *prediction += bits.extend(size);
    out[0] = *prediction * quant[0];
    let mut k = 1usize;
    while k < 64 {
        let symbol = bits.decode(ac)?;
        let run = usize::from(symbol >> 4);
        let size = u32::from(symbol & 15);
        if size == 0 {
            if run == 15 {
                k += 16;
                continue;
            }
            break;
        }
        k += run;
        if k > 63 {
            return fail("AC run past the block");
        }
        let position = ZIGZAG[k];
        out[position] = bits.extend(size) * quant[position];
        k += 1;
    }
    Ok(())
}

const CONST_BITS: i32 = 13;
const PASS1_BITS: i32 = 2;
const FIX_0_298631336: i32 = 2446;
const FIX_0_390180644: i32 = 3196;
const FIX_0_541196100: i32 = 4433;
const FIX_0_765366865: i32 = 6270;
const FIX_0_899976223: i32 = 7373;
const FIX_1_175875602: i32 = 9633;
const FIX_1_501321110: i32 = 12299;
const FIX_1_847759065: i32 = 15137;
const FIX_1_961570560: i32 = 16069;
const FIX_2_053119869: i32 = 16819;
const FIX_2_562915447: i32 = 20995;
const FIX_3_072711026: i32 = 25172;

#[inline(always)]
fn descale(value: i32, n: i32) -> i32 {
    (value + (1 << (n - 1))) >> n
}

/// The IJG accurate integer IDCT (`jpeg_idct_islow`): columns into a
/// workspace at two extra bits of precision, then rows to samples. A
/// block with no AC energy is one flat value, which is what the full
/// transform would compute for every sample.
fn idct(input: &[i32; 64], out: &mut [u8; 64]) {
    if input[1..].iter().all(|coefficient| *coefficient == 0) {
        out.fill(limit((input[0] + 4) >> 3));
        return;
    }
    let mut workspace = [0i32; 64];
    for column in 0..8 {
        let c = |row: usize| input[row * 8 + column];
        if c(1) == 0 && c(2) == 0 && c(3) == 0 && c(4) == 0 && c(5) == 0 && c(6) == 0 && c(7) == 0 {
            let dc = c(0) << PASS1_BITS;
            for row in 0..8 {
                workspace[row * 8 + column] = dc;
            }
            continue;
        }
        let z2 = c(2);
        let z3 = c(6);
        let z1 = (z2 + z3) * FIX_0_541196100;
        let tmp2 = z1 + z3 * (-FIX_1_847759065);
        let tmp3 = z1 + z2 * FIX_0_765366865;
        let z2 = c(0);
        let z3 = c(4);
        let tmp0 = (z2 + z3) << CONST_BITS;
        let tmp1 = (z2 - z3) << CONST_BITS;
        let tmp10 = tmp0 + tmp3;
        let tmp13 = tmp0 - tmp3;
        let tmp11 = tmp1 + tmp2;
        let tmp12 = tmp1 - tmp2;
        let (mut tmp0, mut tmp1, mut tmp2, mut tmp3) = (c(7), c(5), c(3), c(1));
        let z1 = tmp0 + tmp3;
        let z2 = tmp1 + tmp2;
        let z3 = tmp0 + tmp2;
        let z4 = tmp1 + tmp3;
        let z5 = (z3 + z4) * FIX_1_175875602;
        tmp0 *= FIX_0_298631336;
        tmp1 *= FIX_2_053119869;
        tmp2 *= FIX_3_072711026;
        tmp3 *= FIX_1_501321110;
        let z1 = z1 * (-FIX_0_899976223);
        let z2 = z2 * (-FIX_2_562915447);
        let z3 = z3 * (-FIX_1_961570560) + z5;
        let z4 = z4 * (-FIX_0_390180644) + z5;
        tmp0 += z1 + z3;
        tmp1 += z2 + z4;
        tmp2 += z2 + z3;
        tmp3 += z1 + z4;
        let shift = CONST_BITS - PASS1_BITS;
        workspace[column] = descale(tmp10 + tmp3, shift);
        workspace[7 * 8 + column] = descale(tmp10 - tmp3, shift);
        workspace[8 + column] = descale(tmp11 + tmp2, shift);
        workspace[6 * 8 + column] = descale(tmp11 - tmp2, shift);
        workspace[2 * 8 + column] = descale(tmp12 + tmp1, shift);
        workspace[5 * 8 + column] = descale(tmp12 - tmp1, shift);
        workspace[3 * 8 + column] = descale(tmp13 + tmp0, shift);
        workspace[4 * 8 + column] = descale(tmp13 - tmp0, shift);
    }
    let shift = CONST_BITS + PASS1_BITS + 3;
    for row in 0..8 {
        let w = &workspace[row * 8..row * 8 + 8];
        let z2 = w[2];
        let z3 = w[6];
        let z1 = (z2 + z3) * FIX_0_541196100;
        let tmp2 = z1 + z3 * (-FIX_1_847759065);
        let tmp3 = z1 + z2 * FIX_0_765366865;
        let tmp0 = (w[0] + w[4]) << CONST_BITS;
        let tmp1 = (w[0] - w[4]) << CONST_BITS;
        let tmp10 = tmp0 + tmp3;
        let tmp13 = tmp0 - tmp3;
        let tmp11 = tmp1 + tmp2;
        let tmp12 = tmp1 - tmp2;
        let (mut tmp0, mut tmp1, mut tmp2, mut tmp3) = (w[7], w[5], w[3], w[1]);
        let z1 = tmp0 + tmp3;
        let z2 = tmp1 + tmp2;
        let z3 = tmp0 + tmp2;
        let z4 = tmp1 + tmp3;
        let z5 = (z3 + z4) * FIX_1_175875602;
        tmp0 *= FIX_0_298631336;
        tmp1 *= FIX_2_053119869;
        tmp2 *= FIX_3_072711026;
        tmp3 *= FIX_1_501321110;
        let z1 = z1 * (-FIX_0_899976223);
        let z2 = z2 * (-FIX_2_562915447);
        let z3 = z3 * (-FIX_1_961570560) + z5;
        let z4 = z4 * (-FIX_0_390180644) + z5;
        tmp0 += z1 + z3;
        tmp1 += z2 + z4;
        tmp2 += z2 + z3;
        tmp3 += z1 + z4;
        let o = &mut out[row * 8..row * 8 + 8];
        o[0] = limit(descale(tmp10 + tmp3, shift));
        o[7] = limit(descale(tmp10 - tmp3, shift));
        o[1] = limit(descale(tmp11 + tmp2, shift));
        o[6] = limit(descale(tmp11 - tmp2, shift));
        o[2] = limit(descale(tmp12 + tmp1, shift));
        o[5] = limit(descale(tmp12 - tmp1, shift));
        o[3] = limit(descale(tmp13 + tmp0, shift));
        o[4] = limit(descale(tmp13 - tmp0, shift));
    }
}

#[inline(always)]
fn limit(value: i32) -> u8 {
    (value + 128).clamp(0, 255) as u8
}

/// Upsamples every component of one MCU row and converts it to the
/// output color, handing the rows to the sink.
#[allow(clippy::too_many_arguments)]
fn emit_mcu_row(
    frame: &Frame,
    planes: &mut [Plane],
    upsampled: &mut [Vec<u8>],
    out_row: &mut [u8],
    stride: usize,
    rows_per_mcu: usize,
    emitted: &mut u32,
    adobe_transform: Option<u8>,
    sink: &mut dyn RowSink,
) -> Result<(), Failure> {
    let width = frame.width as usize;
    let full_width = frame.mcus_wide * frame.h_max * 8;
    for row in 0..rows_per_mcu {
        if *emitted >= frame.height {
            break;
        }
        for (index, component) in frame.components.iter().enumerate() {
            let plane = &planes[index];
            let hs = frame.h_max / component.h;
            let vs = frame.v_max / component.v;
            // The component's real width: the upsampler treats the last
            // real column as the edge, not the block padding past it.
            let real = (width * component.h).div_ceil(frame.h_max);
            let target = &mut upsampled[index][..(real * hs).min(full_width)];
            match (hs, vs) {
                (1, 1) => target.copy_from_slice(&plane.row(row)[..real]),
                (2, 1) => h2v1_fancy(&plane.row(row)[..real], target),
                (2, 2) => {
                    let source_row = row / 2;
                    let (this, other) = if row % 2 == 0 {
                        (plane.row(source_row), neighbor(plane, source_row, -1))
                    } else {
                        (plane.row(source_row), neighbor(plane, source_row, 1))
                    };
                    h2v2_fancy(&this[..real], &other[..real], target);
                }
                (1, 2) => {
                    let source_row = row / 2;
                    let (this, other, bias) = if row % 2 == 0 {
                        (plane.row(source_row), neighbor(plane, source_row, -1), 1)
                    } else {
                        (plane.row(source_row), neighbor(plane, source_row, 1), 2)
                    };
                    for ((out, a), b) in target.iter_mut().zip(&this[..real]).zip(&other[..real]) {
                        *out = ((i32::from(*a) * 3 + i32::from(*b) + bias) >> 2) as u8;
                    }
                }
                _ => {
                    let source = plane.row(row / vs);
                    for (x, out) in target.iter_mut().enumerate() {
                        *out = source[(x / hs).min(real - 1)];
                    }
                }
            }
        }
        let rgb = frame.components.len() == 3 && adobe_transform == Some(0);
        match frame.components.len() {
            1 => out_row[..width].copy_from_slice(&upsampled[0][..width]),
            _ if rgb => {
                for x in 0..width {
                    out_row[x * 3] = upsampled[0][x];
                    out_row[x * 3 + 1] = upsampled[1][x];
                    out_row[x * 3 + 2] = upsampled[2][x];
                }
            }
            _ => ycbcr_to_rgb(
                &upsampled[0][..width],
                &upsampled[1][..width],
                &upsampled[2][..width],
                &mut out_row[..width * 3],
            ),
        }
        sink.row(&out_row[..stride])?;
        *emitted += 1;
    }
    Ok(())
}

/// The row above or below `y` in the plane, its context rows included.
fn neighbor(plane: &Plane, y: usize, direction: i32) -> &[u8] {
    let index = (y as i32 + 1 + direction) as usize;
    &plane.samples[index * plane.width..(index + 1) * plane.width]
}

/// libjpeg's `h2v1_fancy_upsample`: each output pixel is three quarters
/// its own sample and a quarter of the nearer neighbor.
fn h2v1_fancy(input: &[u8], output: &mut [u8]) {
    let width = input.len();
    if width == 1 {
        output[0] = input[0];
        output[1] = input[0];
        return;
    }
    let value = i32::from(input[0]);
    output[0] = value as u8;
    output[1] = ((value * 3 + i32::from(input[1]) + 2) >> 2) as u8;
    for column in 1..width - 1 {
        let value = i32::from(input[column]) * 3;
        output[column * 2] = ((value + i32::from(input[column - 1]) + 1) >> 2) as u8;
        output[column * 2 + 1] = ((value + i32::from(input[column + 1]) + 2) >> 2) as u8;
    }
    let value = i32::from(input[width - 1]);
    output[(width - 1) * 2] = ((value * 3 + i32::from(input[width - 2]) + 1) >> 2) as u8;
    output[(width - 1) * 2 + 1] = value as u8;
}

/// libjpeg's `h2v2_fancy_upsample` for one output row: a 3:1 blend of
/// this input row and its nearer neighbor vertically, then the same
/// blend horizontally, with the rounding libjpeg uses.
fn h2v2_fancy(this: &[u8], other: &[u8], output: &mut [u8]) {
    let width = this.len();
    let column_sum = |x: usize| i32::from(this[x]) * 3 + i32::from(other[x]);
    if width == 1 {
        let sum = column_sum(0);
        output[0] = ((sum * 4 + 8) >> 4) as u8;
        output[1] = ((sum * 4 + 7) >> 4) as u8;
        return;
    }
    let mut this_sum = column_sum(0);
    let mut next_sum = column_sum(1);
    output[0] = ((this_sum * 4 + 8) >> 4) as u8;
    output[1] = ((this_sum * 3 + next_sum + 7) >> 4) as u8;
    let mut last_sum = this_sum;
    this_sum = next_sum;
    for column in 2..width {
        next_sum = column_sum(column);
        output[(column - 1) * 2] = ((this_sum * 3 + last_sum + 8) >> 4) as u8;
        output[(column - 1) * 2 + 1] = ((this_sum * 3 + next_sum + 7) >> 4) as u8;
        last_sum = this_sum;
        this_sum = next_sum;
    }
    output[(width - 1) * 2] = ((this_sum * 3 + last_sum + 8) >> 4) as u8;
    output[(width - 1) * 2 + 1] = ((this_sum * 4 + 7) >> 4) as u8;
}

/// libjpeg's fixed-point YCbCr to RGB (`jdcolor.c`).
fn ycbcr_to_rgb(y: &[u8], cb: &[u8], cr: &[u8], out: &mut [u8]) {
    const ONE_HALF: i32 = 1 << 15;
    for (((y, cb), cr), pixel) in y.iter().zip(cb).zip(cr).zip(out.chunks_exact_mut(3)) {
        let y = i32::from(*y);
        let cb = i32::from(*cb) - 128;
        let cr = i32::from(*cr) - 128;
        let r = y + ((91_881 * cr + ONE_HALF) >> 16);
        let g = y + ((-22_554 * cb - 46_802 * cr + ONE_HALF) >> 16);
        let b = y + ((116_130 * cb + ONE_HALF) >> 16);
        pixel[0] = r.clamp(0, 255) as u8;
        pixel[1] = g.clamp(0, 255) as u8;
        pixel[2] = b.clamp(0, 255) as u8;
    }
}

/// One scan's header: which components, which band, which bit position.
struct Scan {
    members: Vec<usize>,
    components: Vec<Component>,
    spectral_start: usize,
    spectral_end: usize,
    high: u32,
    low: u32,
}

impl Decoder<'_> {
    /// Decodes one scan into the coefficient buffers: a whole baseline
    /// block per component for sequential files, one band and bit
    /// position of it for progressive ones (libjpeg's `jdphuff`).
    fn buffered_scan(&mut self, frame: &Frame, scan: &Scan) -> Result<(), JpegError> {
        if self.coefficients.is_empty() {
            self.coefficients = frame
                .components
                .iter()
                .map(|c| vec![0i16; c.blocks_wide * c.blocks_high * 64])
                .collect();
        }
        let progressive = frame.progressive;
        if progressive {
            if scan.spectral_end > 63 || scan.spectral_start > scan.spectral_end || scan.low > 13 {
                return fail("bad progressive scan band");
            }
            if scan.spectral_start == 0 && scan.spectral_end != 0 {
                return fail("a progressive DC scan may not carry AC coefficients");
            }
            if scan.spectral_start > 0 && scan.members.len() != 1 {
                return fail("a progressive AC scan covers one component");
            }
        }
        for position in &scan.members {
            let component = &scan.components[*position];
            let needs_dc = !progressive || scan.spectral_start == 0;
            let needs_ac = !progressive || scan.spectral_start > 0;
            if (needs_dc && scan.high == 0 && self.dc_tables[component.dc_table].is_none())
                || (needs_ac && self.ac_tables[component.ac_table].is_none())
            {
                return fail("scan uses a Huffman table that was never defined");
            }
        }
        let mut bits = Bits::new(self.bytes, self.at);
        let mut predictions = [0i32; 4];
        self.eob_run = 0;
        let interleaved = scan.members.len() > 1;
        let mut units_done = 0usize;
        // A non-interleaved scan walks the component's own block grid,
        // which is its real size in blocks, not the MCU-padded one.
        let (units_wide, units_high) = if interleaved {
            (frame.mcus_wide, frame.mcus_high)
        } else {
            let component = &scan.components[scan.members[0]];
            let samples_wide = (frame.width as usize * component.h).div_ceil(frame.h_max);
            let samples_high = (frame.height as usize * component.v).div_ceil(frame.v_max);
            (samples_wide.div_ceil(8), samples_high.div_ceil(8))
        };
        for unit_y in 0..units_high {
            for unit_x in 0..units_wide {
                if self.restart_interval > 0
                    && units_done > 0
                    && units_done % self.restart_interval == 0
                {
                    match bits.resync() {
                        Some(marker) if (0xD0..=0xD7).contains(&marker) => {}
                        _ => return fail("missing restart marker"),
                    }
                    predictions = [0; 4];
                    self.eob_run = 0;
                }
                for position in &scan.members {
                    let component = scan.components[*position].clone();
                    let (blocks_x, blocks_y) = if interleaved {
                        (component.h, component.v)
                    } else {
                        (1, 1)
                    };
                    for by in 0..blocks_y {
                        for bx in 0..blocks_x {
                            let (block_x, block_y) = if interleaved {
                                (unit_x * component.h + bx, unit_y * component.v + by)
                            } else {
                                (unit_x, unit_y)
                            };
                            let start = (block_y * component.blocks_wide + block_x) * 64;
                            let coefficients = &mut self.coefficients[*position];
                            let block: &mut [i16; 64] = (&mut coefficients[start..start + 64])
                                .try_into()
                                .expect("64 coefficients");
                            if !progressive {
                                let dc = self.dc_tables[component.dc_table]
                                    .as_ref()
                                    .expect("checked");
                                let ac = self.ac_tables[component.ac_table]
                                    .as_ref()
                                    .expect("checked");
                                decode_block_raw(
                                    &mut bits,
                                    dc,
                                    ac,
                                    &mut predictions[*position],
                                    block,
                                )?;
                            } else if scan.spectral_start == 0 {
                                if scan.high == 0 {
                                    let dc = self.dc_tables[component.dc_table]
                                        .as_ref()
                                        .expect("checked");
                                    let size = u32::from(bits.decode(dc)?);
                                    if size > 11 {
                                        return fail("bad DC magnitude");
                                    }
                                    predictions[*position] += bits.extend(size);
                                    block[0] = (predictions[*position] << scan.low) as i16;
                                } else if bits.bits(1) == 1 {
                                    block[0] |= (1 << scan.low) as i16;
                                }
                            } else {
                                let ac = self.ac_tables[component.ac_table]
                                    .as_ref()
                                    .expect("checked");
                                if scan.high == 0 {
                                    self.eob_run =
                                        ac_first(&mut bits, ac, scan, self.eob_run, block)?;
                                } else {
                                    self.eob_run =
                                        ac_refine(&mut bits, ac, scan, self.eob_run, block)?;
                                }
                            }
                        }
                    }
                }
                units_done += 1;
            }
        }
        self.at = bits.end_of_scan();
        Ok(())
    }

    /// Runs the output pipeline over the coefficient buffers: dequantize
    /// and transform each block into MCU-row planes, upsample, convert,
    /// and hand rows over, as the streaming path does as it decodes.
    fn finish_buffered(&mut self, sink: &mut dyn RowSink) -> Result<(), Failure> {
        let frame = self
            .frame
            .as_ref()
            .ok_or_else(|| JpegError("no frame".to_string()))?;
        let color = if frame.components.len() == 1 {
            ColorType::Gray
        } else {
            ColorType::Rgb
        };
        sink.start(frame.width, frame.height, color)?;
        let rows_per_mcu = frame.v_max * 8;
        let stride = frame.width as usize * color.channels();
        let mut out_row = vec![0u8; stride + 16];
        let mut upsampled: Vec<Vec<u8>> = frame
            .components
            .iter()
            .map(|_| vec![0u8; frame.mcus_wide * frame.h_max * 8])
            .collect();
        let mut emitted = 0u32;
        let mut previous: Option<Vec<Plane>> = None;
        let mut dequantized = [0i32; 64];
        let mut block = [0u8; 64];
        for mcu_y in 0..frame.mcus_high {
            let mut current: Vec<Plane> = frame
                .components
                .iter()
                .map(|c| Plane::new(c.blocks_wide * 8, c.v * 8))
                .collect();
            for (index, component) in frame.components.iter().enumerate() {
                let quant = self.quant[component.quant].as_ref().expect("checked");
                for by in 0..component.v {
                    let block_y = mcu_y * component.v + by;
                    for block_x in 0..component.blocks_wide {
                        let start = (block_y * component.blocks_wide + block_x) * 64;
                        let source = &self.coefficients[index][start..start + 64];
                        for k in 0..64 {
                            dequantized[k] = i32::from(source[k]) * quant[k];
                        }
                        idct(&dequantized, &mut block);
                        let plane = &mut current[index];
                        for row in 0..8 {
                            let target = plane.row_mut(by * 8 + row);
                            target[block_x * 8..block_x * 8 + 8]
                                .copy_from_slice(&block[row * 8..row * 8 + 8]);
                        }
                    }
                }
            }
            if let Some(previous_planes) = previous.as_mut() {
                for (index, plane) in previous_planes.iter_mut().enumerate() {
                    let first = current[index].row(0).to_vec();
                    plane.below_mut().copy_from_slice(&first);
                }
                for (index, plane) in current.iter_mut().enumerate() {
                    let last = previous_planes[index]
                        .row(previous_planes[index].height - 1)
                        .to_vec();
                    plane.above_mut().copy_from_slice(&last);
                }
                emit_mcu_row(
                    frame,
                    previous_planes,
                    &mut upsampled,
                    &mut out_row,
                    stride,
                    rows_per_mcu,
                    &mut emitted,
                    self.adobe_transform,
                    sink,
                )?;
            } else {
                for plane in current.iter_mut() {
                    let first = plane.row(0).to_vec();
                    plane.above_mut().copy_from_slice(&first);
                }
            }
            previous = Some(current);
        }
        if let Some(previous_planes) = previous.as_mut() {
            replicate_bottom(frame, previous_planes);
            emit_mcu_row(
                frame,
                previous_planes,
                &mut upsampled,
                &mut out_row,
                stride,
                rows_per_mcu,
                &mut emitted,
                self.adobe_transform,
                sink,
            )?;
        }
        if emitted != frame.height {
            return Err(JpegError("image data ended before the last row".to_string()).into());
        }
        Ok(())
    }
}

/// Rows past a component's real height are block padding; libjpeg
/// upsamples against copies of the last real row instead.
fn replicate_bottom(frame: &Frame, planes: &mut [Plane]) {
    for (index, plane) in planes.iter_mut().enumerate() {
        let component = &frame.components[index];
        let real_height = (frame.height as usize * component.v).div_ceil(frame.v_max);
        let first_padding = real_height.saturating_sub((frame.mcus_high - 1) * plane.height);
        if first_padding >= 1 && first_padding < plane.height {
            let last = plane.row(first_padding - 1).to_vec();
            for y in first_padding..plane.height {
                plane.row_mut(y).copy_from_slice(&last);
            }
        }
        let last = plane.row(plane.height - 1).to_vec();
        plane.below_mut().copy_from_slice(&last);
    }
}

/// A whole baseline block, quantized, in natural order.
fn decode_block_raw(
    bits: &mut Bits<'_>,
    dc: &Huffman,
    ac: &Huffman,
    prediction: &mut i32,
    out: &mut [i16; 64],
) -> Result<(), JpegError> {
    out.fill(0);
    let size = u32::from(bits.decode(dc)?);
    if size > 11 {
        return fail("bad DC magnitude");
    }
    *prediction += bits.extend(size);
    out[0] = *prediction as i16;
    let mut k = 1usize;
    while k < 64 {
        let symbol = bits.decode(ac)?;
        let run = usize::from(symbol >> 4);
        let size = u32::from(symbol & 15);
        if size == 0 {
            if run == 15 {
                k += 16;
                continue;
            }
            break;
        }
        k += run;
        if k > 63 {
            return fail("AC run past the block");
        }
        out[ZIGZAG[k]] = bits.extend(size) as i16;
        k += 1;
    }
    Ok(())
}

/// A progressive AC first scan over one block (`decode_mcu_AC_first`).
fn ac_first(
    bits: &mut Bits<'_>,
    ac: &Huffman,
    scan: &Scan,
    mut eob_run: u32,
    block: &mut [i16; 64],
) -> Result<u32, JpegError> {
    if eob_run > 0 {
        return Ok(eob_run - 1);
    }
    let mut k = scan.spectral_start;
    while k <= scan.spectral_end {
        let symbol = bits.decode(ac)?;
        let run = u32::from(symbol >> 4);
        let size = u32::from(symbol & 15);
        if size == 0 {
            if run < 15 {
                eob_run = 1 << run;
                if run > 0 {
                    eob_run += bits.bits(run);
                }
                eob_run -= 1;
                break;
            }
            k += 16;
            continue;
        }
        k += run as usize;
        if k > 63 {
            return fail("AC run past the block");
        }
        block[ZIGZAG[k]] = (bits.extend(size) << scan.low) as i16;
        k += 1;
    }
    Ok(eob_run)
}

/// A progressive AC refinement scan over one block
/// (`decode_mcu_AC_refine`): one correction bit for every coefficient
/// already nonzero, and new coefficients of one bit.
fn ac_refine(
    bits: &mut Bits<'_>,
    ac: &Huffman,
    scan: &Scan,
    mut eob_run: u32,
    block: &mut [i16; 64],
) -> Result<u32, JpegError> {
    let p1: i16 = 1 << scan.low;
    let m1: i16 = -1 << scan.low;
    let mut k = scan.spectral_start;
    if eob_run == 0 {
        while k <= scan.spectral_end {
            let symbol = bits.decode(ac)?;
            let mut run = i32::from(symbol >> 4);
            let size = symbol & 15;
            let mut value: i16 = 0;
            if size != 0 {
                if size != 1 {
                    return fail("bad refinement coefficient size");
                }
                value = if bits.bits(1) == 1 { p1 } else { m1 };
            } else if run != 15 {
                eob_run = 1 << run;
                if run > 0 {
                    eob_run += bits.bits(run as u32);
                }
                break;
            }
            // Skip `run` zero coefficients, correcting the nonzero ones
            // passed on the way, then place the new value.
            while k <= scan.spectral_end {
                let position = ZIGZAG[k];
                if block[position] != 0 {
                    if bits.bits(1) == 1 && (block[position] & p1) == 0 {
                        block[position] += if block[position] >= 0 { p1 } else { m1 };
                    }
                } else {
                    run -= 1;
                    if run < 0 {
                        break;
                    }
                }
                k += 1;
            }
            if value != 0 {
                if k > 63 {
                    return fail("AC run past the block");
                }
                block[ZIGZAG[k]] = value;
            }
            k += 1;
        }
    }
    if eob_run > 0 {
        while k <= scan.spectral_end {
            let position = ZIGZAG[k];
            if block[position] != 0 && bits.bits(1) == 1 && (block[position] & p1) == 0 {
                block[position] += if block[position] >= 0 { p1 } else { m1 };
            }
            k += 1;
        }
        eob_run -= 1;
    }
    Ok(eob_run)
}
