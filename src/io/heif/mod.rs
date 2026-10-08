//! HEIF and HEIC images (ISO/IEC 23008-12): the container's primary image,
//! an HEVC picture or a grid of them (as phone cameras tile their photos),
//! decoded by `io::hevc`, with its alpha, clean aperture, rotation, and
//! mirroring.

pub mod boxes;
pub mod rgb;

use boxes::{Colour, Item, Meta, Property};

use crate::image::ColorType;
use crate::io::hevc::{self, Picture};
use crate::io::png::RowSink;

/// Why a HEIF file does not read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeifError(pub String);

impl HeifError {
    pub fn new(message: &str) -> HeifError {
        HeifError(message.to_string())
    }
}

impl std::fmt::Display for HeifError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "HEIF: {}", self.0)
    }
}

impl std::error::Error for HeifError {}

impl From<hevc::HevcError> for HeifError {
    fn from(error: hevc::HevcError) -> HeifError {
        HeifError(error.0)
    }
}

/// A plane's samples: bytes at 8 bits, which halves a large photo's
/// memory, and 16-bit words above.
pub enum Samples {
    Eight(Vec<u8>),
    Deep(Vec<u16>),
}

impl Samples {
    fn new(depth: u32, count: usize) -> Samples {
        if depth <= 8 {
            Samples::Eight(vec![0; count])
        } else {
            Samples::Deep(vec![0; count])
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Samples::Eight(samples) => samples.len(),
            Samples::Deep(samples) => samples.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get(&self, index: usize) -> u16 {
        match self {
            Samples::Eight(samples) => u16::from(samples[index]),
            Samples::Deep(samples) => samples[index],
        }
    }

    /// `count` samples from `start`, widened into `out`.
    pub fn widen(&self, start: usize, count: usize, out: &mut [i64]) {
        match self {
            Samples::Eight(samples) => {
                for (value, &sample) in out.iter_mut().zip(&samples[start..start + count]) {
                    *value = i64::from(sample);
                }
            }
            Samples::Deep(samples) => {
                for (value, &sample) in out.iter_mut().zip(&samples[start..start + count]) {
                    *value = i64::from(sample);
                }
            }
        }
    }

    /// Copies `count` samples from `from` at `source` to `at`.
    fn copy_from(&mut self, at: usize, from: &Samples, source: usize, count: usize) {
        match (self, from) {
            (Samples::Eight(to), Samples::Eight(from)) => {
                to[at..at + count].copy_from_slice(&from[source..source + count]);
            }
            (Samples::Deep(to), Samples::Deep(from)) => {
                to[at..at + count].copy_from_slice(&from[source..source + count]);
            }
            (to, from) => {
                for offset in 0..count {
                    let sample = from.get(source + offset);
                    match to {
                        Samples::Eight(to) => to[at + offset] = sample as u8,
                        Samples::Deep(to) => to[at + offset] = sample,
                    }
                }
            }
        }
    }
}

/// Decoded planes: luma, then Cb and Cr unless monochrome, at their bit
/// depth; the coded picture's size, its tiles assembled.
pub struct Planes {
    pub width: usize,
    pub height: usize,
    pub chroma_format: u32,
    pub bit_depth: u32,
    pub bit_depth_chroma: u32,
    pub planes: Vec<Samples>,
    pub sizes: Vec<(usize, usize)>,
    pub vui_colour: Option<hevc::VuiColour>,
}

/// One HEVC item's picture, its conformance window applied.
fn decode_hevc_item(meta: &Meta<'_>, file: &[u8], item: &Item) -> Result<Planes, HeifError> {
    let (nals, length_size) = meta
        .properties_of(item)
        .find_map(|property| match property {
            Property::HevcConfig { nals, length_size } => Some((nals.clone(), *length_size)),
            _ => None,
        })
        .ok_or_else(|| HeifError::new("an HEVC item without its decoder configuration"))?;
    let data = meta.data(file, item)?;
    let mut units: Vec<&[u8]> = nals.iter().map(Vec::as_slice).collect();
    let mut at = 0usize;
    while at + length_size <= data.len() {
        let length = data[at..at + length_size]
            .iter()
            .fold(0usize, |value, &byte| (value << 8) | usize::from(byte));
        at += length_size;
        let end = at
            .checked_add(length)
            .filter(|&end| end <= data.len())
            .ok_or_else(|| HeifError::new("a NAL unit running past its item"))?;
        units.push(&data[at..end]);
        at = end;
    }
    let picture = hevc::decode_picture(units)?;
    Ok(cropped(picture))
}

/// A picture with its conformance window applied.
fn cropped(picture: Picture) -> Planes {
    let [left, right, top, bottom] = picture.crop.map(|value| value as usize);
    let width = picture.width.saturating_sub(left + right);
    let height = picture.height.saturating_sub(top + bottom);
    let (sub_x, sub_y) = match picture.chroma_format {
        1 => (1, 1),
        2 => (1, 0),
        _ => (0, 0),
    };
    let mut planes = Vec::with_capacity(picture.planes.len());
    let mut sizes = Vec::with_capacity(picture.planes.len());
    for (index, plane) in picture.planes.into_iter().enumerate() {
        let (shift_x, shift_y) = if index == 0 { (0, 0) } else { (sub_x, sub_y) };
        let depth = if index == 0 {
            picture.bit_depth_luma
        } else {
            picture.bit_depth_chroma
        };
        let stride = picture.sizes[index].0;
        let (x0, y0) = (left >> shift_x, top >> shift_y);
        let (w, h) = (width >> shift_x, height >> shift_y);
        let rows = (0..h).map(|row| (y0 + row) * stride + x0);
        let samples = if depth <= 8 {
            let mut out = Vec::with_capacity(w * h);
            for start in rows {
                out.extend(plane[start..start + w].iter().map(|&sample| sample as u8));
            }
            Samples::Eight(out)
        } else if w == stride && y0 == 0 {
            // Uncropped: the decoder's plane as it is.
            let mut plane = plane;
            plane.truncate(w * h);
            Samples::Deep(plane)
        } else {
            let mut out = Vec::with_capacity(w * h);
            for start in rows {
                out.extend_from_slice(&plane[start..start + w]);
            }
            Samples::Deep(out)
        };
        planes.push(samples);
        sizes.push((w, h));
    }
    Planes {
        width,
        height,
        chroma_format: picture.chroma_format,
        bit_depth: picture.bit_depth_luma,
        bit_depth_chroma: picture.bit_depth_chroma,
        planes,
        sizes,
        vui_colour: picture.vui_colour,
    }
}

/// An image item's planes: one picture, or a grid's tiles assembled.
fn decode_image_item(meta: &Meta<'_>, file: &[u8], id: u32) -> Result<Planes, HeifError> {
    let item = meta
        .item(id)
        .ok_or_else(|| HeifError::new("a missing image item"))?;
    match &item.kind {
        b"hvc1" => decode_hevc_item(meta, file, item),
        b"grid" => {
            let data = meta.data(file, item)?;
            let mut reader = boxes::Reader::new(&data);
            reader.u8()?;
            let flags = reader.u8()?;
            let rows = usize::from(reader.u8()?) + 1;
            let columns = usize::from(reader.u8()?) + 1;
            let (width, height) = if flags & 1 == 1 {
                (reader.u32()? as usize, reader.u32()? as usize)
            } else {
                (usize::from(reader.u16()?), usize::from(reader.u16()?))
            };
            // A gigapixel and more is refused before anything is decoded.
            if width == 0 || height == 0 || width * height > 1 << 30 {
                return Err(HeifError::new("a grid too large to read"));
            }
            let tiles = meta.referenced(b"dimg", id);
            if tiles.len() != rows * columns {
                return Err(HeifError::new("a grid without one tile per cell"));
            }
            let mut canvas: Option<Planes> = None;
            let (mut tile_width, mut tile_height) = (0, 0);
            decode_tiles(meta, file, &tiles, &mut |index, decoded| {
                let canvas = canvas.get_or_insert_with(|| {
                    tile_width = decoded.width;
                    tile_height = decoded.height;
                    let (sub_x, sub_y) = match decoded.chroma_format {
                        1 => (1, 1),
                        2 => (1, 0),
                        _ => (0, 0),
                    };
                    let sizes: Vec<(usize, usize)> = (0..decoded.planes.len())
                        .map(|plane| {
                            if plane == 0 {
                                (width, height)
                            } else {
                                (width.div_ceil(1 << sub_x), height.div_ceil(1 << sub_y))
                            }
                        })
                        .collect();
                    Planes {
                        width,
                        height,
                        chroma_format: decoded.chroma_format,
                        bit_depth: decoded.bit_depth,
                        bit_depth_chroma: decoded.bit_depth_chroma,
                        planes: sizes
                            .iter()
                            .enumerate()
                            .map(|(plane, &(w, h))| {
                                let depth = if plane == 0 {
                                    decoded.bit_depth
                                } else {
                                    decoded.bit_depth_chroma
                                };
                                Samples::new(depth, w * h)
                            })
                            .collect(),
                        sizes,
                        vui_colour: decoded.vui_colour,
                    }
                });
                if decoded.chroma_format != canvas.chroma_format
                    || decoded.planes.len() != canvas.planes.len()
                    || decoded.width != tile_width
                    || decoded.height != tile_height
                {
                    return Err(HeifError::new("a grid whose tiles differ in format"));
                }
                let (column, row) = (index % columns, index / columns);
                for plane in 0..canvas.planes.len() {
                    let (canvas_width, canvas_height) = canvas.sizes[plane];
                    let (tile_plane_width, tile_plane_height) = decoded.sizes[plane];
                    let (x0, y0) = (column * tile_plane_width, row * tile_plane_height);
                    if x0 >= canvas_width {
                        continue;
                    }
                    let span = tile_plane_width.min(canvas_width - x0);
                    for y in 0..tile_plane_height.min(canvas_height.saturating_sub(y0)) {
                        let at = (y0 + y) * canvas_width + x0;
                        canvas.planes[plane].copy_from(
                            at,
                            &decoded.planes[plane],
                            y * tile_plane_width,
                            span,
                        );
                    }
                }
                Ok(())
            })?;
            canvas.ok_or_else(|| HeifError::new("a grid without tiles"))
        }
        kind => Err(HeifError(format!(
            "an image item of type {} (only HEVC and grids are read)",
            String::from_utf8_lossy(kind)
        ))),
    }
}

/// Decodes each of `tiles`, on as many threads as the machine has (one
/// where threads are not available), handing each picture to `place`
/// with its index as it is done, so only the tiles in flight are held.
fn decode_tiles(
    meta: &Meta<'_>,
    file: &[u8],
    tiles: &[u32],
    place: &mut dyn FnMut(usize, Planes) -> Result<(), HeifError>,
) -> Result<(), HeifError> {
    let threads = std::thread::available_parallelism()
        .map_or(1, |count| count.get())
        .min(tiles.len());
    if threads <= 1 {
        for (index, &tile) in tiles.iter().enumerate() {
            place(index, decode_image_item(meta, file, tile)?)?;
        }
        return Ok(());
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let failed = std::sync::atomic::AtomicBool::new(false);
    // A finished tile waits for the one before it to be placed, no more.
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::scope(|scope| {
        for _ in 0..threads {
            let sender = sender.clone();
            let (next, failed) = (&next, &failed);
            scope.spawn(move || {
                use std::sync::atomic::Ordering;
                while !failed.load(Ordering::Relaxed) {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&tile) = tiles.get(index) else {
                        break;
                    };
                    let decoded = decode_image_item(meta, file, tile);
                    if decoded.is_err() {
                        failed.store(true, Ordering::Relaxed);
                    }
                    if sender.send((index, decoded)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(sender);
        let mut outcome = Ok(());
        for (index, decoded) in receiver {
            if outcome.is_ok() {
                outcome = decoded.and_then(|planes| place(index, planes));
                if outcome.is_err() {
                    failed.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }
        outcome
    })
}

/// The primary image's alpha item, when it has one.
fn alpha_item(meta: &Meta<'_>) -> Option<u32> {
    meta.referring(b"auxl", meta.primary)
        .into_iter()
        .find(|&id| {
            meta.item(id).is_some_and(|item| {
                meta.properties_of(item).any(|property| {
                    matches!(property, Property::Auxiliary(kind)
                    if kind == "urn:mpeg:hevc:2015:auxid:1"
                        || kind == "urn:mpeg:mpegB:cicp:systems:auxiliary:alpha")
                })
            })
        })
}

/// The primary image's decoded planes, before its clean aperture,
/// rotation, and mirroring; and its alpha plane, when it has one.
pub fn decode_planes(file: &[u8]) -> Result<(Planes, Option<Planes>), HeifError> {
    let meta = boxes::read_meta(file)?;
    let planes = decode_image_item(&meta, file, meta.primary)?;
    let alpha = alpha_item(&meta)
        .map(|id| decode_image_item(&meta, file, id))
        .transpose()?;
    Ok((planes, alpha))
}

/// The primary image's colour: what its `colr` property says, if anything.
pub fn colour(file: &[u8]) -> Result<Option<Colour>, HeifError> {
    let meta = boxes::read_meta(file)?;
    let item = meta
        .item(meta.primary)
        .ok_or_else(|| HeifError::new("no primary image"))?;
    Ok(meta
        .properties_of(item)
        .find_map(|property| match property {
            Property::Colour(colour) => Some(colour.clone()),
            _ => None,
        }))
}

/// What reading a HEIF image into rows dropped or changed.
#[derive(Debug, Default)]
pub struct HeifNotes {
    /// The samples' bit depth, when above 8 (rows carry 8 bits).
    pub deep: Option<u32>,
    /// A colour profile the sink could not keep.
    pub profile_dropped: bool,
    /// Exif the sink could not keep.
    pub exif_dropped: bool,
    /// Other images in the file (only the primary is read).
    pub other_images: usize,
}

/// What can go wrong reading a HEIF image into a sink.
#[derive(Debug)]
pub enum HeifRowsError {
    Heif(HeifError),
    Io(std::io::Error),
}

impl From<HeifError> for HeifRowsError {
    fn from(error: HeifError) -> Self {
        HeifRowsError::Heif(error)
    }
}

impl From<std::io::Error> for HeifRowsError {
    fn from(error: std::io::Error) -> Self {
        HeifRowsError::Io(error)
    }
}

/// Where an output pixel comes from in the decoded picture: source x and
/// y are `x_from` and `y_from` applied to the output's (x, y), each a
/// sum of the two coordinates' multiples (-1, 0, or 1) and an offset.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Placement {
    x_from: [i64; 3],
    y_from: [i64; 3],
    width: usize,
    height: usize,
}

impl Placement {
    fn identity(width: usize, height: usize) -> Placement {
        Placement {
            x_from: [1, 0, 0],
            y_from: [0, 1, 0],
            width,
            height,
        }
    }

    /// Applies an operation whose output pixel (x, y) is its input's
    /// (`x_from` . (x, y, 1), `y_from` . (x, y, 1)), of the given size.
    fn then(&self, x_from: [i64; 3], y_from: [i64; 3], width: usize, height: usize) -> Placement {
        let compose = |row: [i64; 3]| -> [i64; 3] {
            [
                row[0] * x_from[0] + row[1] * y_from[0],
                row[0] * x_from[1] + row[1] * y_from[1],
                row[0] * x_from[2] + row[1] * y_from[2] + row[2],
            ]
        };
        Placement {
            x_from: compose(self.x_from),
            y_from: compose(self.y_from),
            width,
            height,
        }
    }

    fn source(&self, x: usize, y: usize) -> (usize, usize) {
        let (x, y) = (x as i64, y as i64);
        let sx = self.x_from[0] * x + self.x_from[1] * y + self.x_from[2];
        let sy = self.y_from[0] * x + self.y_from[1] * y + self.y_from[2];
        (sx as usize, sy as usize)
    }

    /// True when each output row comes from one source row.
    fn rows_stay_rows(&self) -> bool {
        self.x_from[1] == 0 && self.y_from[0] == 0
    }
}

/// The primary image's transformative properties (clean aperture,
/// rotation, mirroring), in the order they are associated, applied to a
/// picture of the given size.
fn placement(meta: &Meta<'_>, item: &Item, width: usize, height: usize) -> Placement {
    let mut placed = Placement::identity(width, height);
    for property in meta.properties_of(item) {
        let (w, h) = (placed.width as i64, placed.height as i64);
        match property {
            Property::CleanAperture(fractions) => {
                let value = |(numerator, denominator): (i64, i64)| -> Option<f64> {
                    (denominator != 0).then(|| numerator as f64 / denominator as f64)
                };
                let (Some(clean_width), Some(clean_height), Some(horizontal), Some(vertical)) = (
                    value(fractions[0]),
                    value(fractions[1]),
                    value(fractions[2]),
                    value(fractions[3]),
                ) else {
                    continue;
                };
                // The window centred `horizontal`, `vertical` from the
                // picture's centre (ISO/IEC 14496-12 12.1.4).
                let clean_width = (clean_width.round() as i64).clamp(1, w);
                let clean_height = (clean_height.round() as i64).clamp(1, h);
                let left = (horizontal + (w - 1) as f64 / 2.0 - (clean_width - 1) as f64 / 2.0)
                    .floor() as i64;
                let top = (vertical + (h - 1) as f64 / 2.0 - (clean_height - 1) as f64 / 2.0)
                    .floor() as i64;
                let left = left.clamp(0, w - clean_width);
                let top = top.clamp(0, h - clean_height);
                placed = placed.then(
                    [1, 0, left],
                    [0, 1, top],
                    clean_width as usize,
                    clean_height as usize,
                );
            }
            // Quarter turns anticlockwise: the right column becomes the
            // top row.
            Property::Rotation(turns) => {
                for _ in 0..*turns {
                    let (w, h) = (placed.width as i64, placed.height as i64);
                    placed = placed.then([0, -1, w - 1], [1, 0, 0], h as usize, w as usize);
                }
            }
            // Axis 0 flips top to bottom and 1 left to right, as libheif
            // (the reference implementation) reads it; Apple ignores it.
            Property::Mirror(0) => {
                placed = placed.then([1, 0, 0], [0, -1, h - 1], placed.width, placed.height);
            }
            Property::Mirror(_) => {
                placed = placed.then([-1, 0, w - 1], [0, 1, 0], placed.width, placed.height);
            }
            _ => {}
        }
    }
    placed
}

/// The colour coefficients to convert with: the `nclx` property's, else
/// the HEVC stream's, else BT.601 full range (as phone files without
/// either mean).
fn coefficients(meta: &Meta<'_>, item: &Item, planes: &Planes) -> (u16, bool) {
    let nclx = meta
        .properties_of(item)
        .find_map(|property| match property {
            Property::Colour(Colour::Nclx {
                matrix, full_range, ..
            }) => Some((*matrix, *full_range)),
            _ => None,
        });
    let stream = planes
        .vui_colour
        .map(|colour| (u16::from(colour.matrix), colour.full_range));
    // Matrix 2 is "unspecified".
    nclx.filter(|&(matrix, _)| matrix != 2)
        .or(stream.filter(|&(matrix, _)| matrix != 2))
        .unwrap_or((6, true))
}

/// The primary image's Exif, as TIFF, with any orientation set to 1:
/// the rows come out turned already, as HEIF's own properties say.
fn exif(meta: &Meta<'_>, file: &[u8]) -> Option<Vec<u8>> {
    let id = meta
        .referring(b"cdsc", meta.primary)
        .into_iter()
        .find(|&id| meta.item(id).is_some_and(|item| &item.kind == b"Exif"))?;
    let data = meta.data(file, meta.item(id)?).ok()?;
    let offset = u32::from_be_bytes(data.get(..4)?.try_into().ok()?) as usize;
    let mut tiff = data.get(4 + offset..)?.to_vec();
    crate::io::orient::reset_exif_orientation(&mut tiff);
    Some(tiff)
}

/// Reads the primary image of a HEIF file into `sink` as 8-bit rows:
/// RGB (or gray), with alpha when the file has it, its crop, rotation,
/// and mirroring applied, and its colour profile and Exif handed over.
pub fn read_heif_rows(file: &[u8], sink: &mut dyn RowSink) -> Result<HeifNotes, HeifRowsError> {
    let meta = boxes::read_meta(file)?;
    let item = meta
        .item(meta.primary)
        .ok_or_else(|| HeifError::new("no primary image"))?;
    let planes = decode_image_item(&meta, file, meta.primary)?;
    let alpha = alpha_item(&meta)
        .map(|id| decode_image_item(&meta, file, id))
        .transpose()?;
    let mut notes = HeifNotes {
        deep: (planes.bit_depth > 8).then_some(planes.bit_depth),
        ..HeifNotes::default()
    };
    if let Some(profile) = meta
        .properties_of(item)
        .find_map(|property| match property {
            Property::Colour(Colour::Icc(profile)) => Some(profile),
            _ => None,
        })
    {
        notes.profile_dropped = !sink.icc_profile(profile);
    }
    if let Some(tiff) = exif(&meta, file) {
        notes.exif_dropped = !sink.exif(&tiff);
    }
    let (matrix, full_range) = coefficients(&meta, item, &planes);
    let mut converter = rgb::Converter::new(&planes, matrix, full_range);
    let colour_channels = converter.channels();
    let channels = colour_channels + usize::from(alpha.is_some());
    let color = match channels {
        1 => ColorType::Gray,
        2 => ColorType::GrayAlpha,
        3 => ColorType::Rgb,
        _ => ColorType::Rgba,
    };
    let placed = placement(&meta, item, planes.width, planes.height);
    let (width, height) = (placed.width, placed.height);
    // Alpha at 8 bits, scaled to the picture when its size differs.
    let alpha_at = |x: usize, y: usize| -> u8 {
        let Some(alpha) = &alpha else { return 255 };
        let ax = x * alpha.width / planes.width.max(1);
        let ay = y * alpha.height / planes.height.max(1);
        let max = (1u32 << alpha.bit_depth) - 1;
        let sample = u32::from(alpha.planes[0].get(ay * alpha.width + ax));
        ((sample * 255 + max / 2) / max) as u8
    };
    sink.start(width as u32, height as u32, color)?;
    let mut source = vec![0u8; planes.width * colour_channels];
    let mut out = vec![0u8; width * channels];
    if placed.rows_stay_rows() {
        for y in 0..height {
            let (_, sy) = placed.source(0, y);
            converter.row(sy, &mut source);
            for (x, pixel) in out.chunks_exact_mut(channels).enumerate() {
                let (sx, _) = placed.source(x, y);
                pixel[..colour_channels]
                    .copy_from_slice(&source[sx * colour_channels..(sx + 1) * colour_channels]);
                if channels > colour_channels {
                    pixel[colour_channels] = alpha_at(sx, sy);
                }
            }
            sink.row(&out)?;
        }
    } else {
        // Turned a quarter: convert the whole picture, then read it down
        // its columns.
        let mut whole = vec![0u8; planes.width * planes.height * colour_channels];
        for (y, row) in whole
            .chunks_exact_mut(planes.width * colour_channels)
            .enumerate()
        {
            converter.row(y, row);
        }
        for y in 0..height {
            for (x, pixel) in out.chunks_exact_mut(channels).enumerate() {
                let (sx, sy) = placed.source(x, y);
                let at = (sy * planes.width + sx) * colour_channels;
                pixel[..colour_channels].copy_from_slice(&whole[at..at + colour_channels]);
                if channels > colour_channels {
                    pixel[colour_channels] = alpha_at(sx, sy);
                }
            }
            sink.row(&out)?;
        }
    }
    notes.other_images = meta
        .items
        .iter()
        .filter(|other| {
            other.id != meta.primary
                && matches!(
                    &other.kind,
                    b"hvc1" | b"grid" | b"av01" | b"jpeg" | b"iden" | b"iovl"
                )
                && meta
                    .referenced(b"dimg", meta.primary)
                    .iter()
                    .all(|&tile| tile != other.id)
                && meta
                    .referring(b"auxl", meta.primary)
                    .iter()
                    .all(|&aux| aux != other.id)
                && meta
                    .referring(b"thmb", meta.primary)
                    .iter()
                    .all(|&thumb| thumb != other.id)
                && !meta.items.iter().any(|grid| {
                    &grid.kind == b"grid" && meta.referenced(b"dimg", grid.id).contains(&other.id)
                })
        })
        .count();
    Ok(notes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quarter_turn_puts_the_right_column_on_top() {
        // 3 wide, 2 high, turned anticlockwise: 2 wide, 3 high.
        let placed = Placement::identity(3, 2).then([0, -1, 2], [1, 0, 0], 2, 3);
        assert_eq!(placed.source(0, 0), (2, 0));
        assert_eq!(placed.source(1, 0), (2, 1));
        assert_eq!(placed.source(0, 2), (0, 0));
        assert!(!placed.rows_stay_rows());
    }

    #[test]
    fn a_crop_then_mirror_reads_the_window_backwards() {
        let placed = Placement::identity(10, 10)
            .then([1, 0, 2], [0, 1, 3], 4, 4)
            .then([-1, 0, 3], [0, 1, 0], 4, 4);
        assert_eq!(placed.source(0, 0), (5, 3));
        assert_eq!(placed.source(3, 1), (2, 4));
        assert!(placed.rows_stay_rows());
    }
}
