//! HEIF and HEIC images (ISO/IEC 23008-12): the container's primary image,
//! an HEVC picture or a grid of them (as phone cameras tile their photos),
//! decoded by `io::hevc`, with its alpha, clean aperture, rotation, and
//! mirroring.

pub mod boxes;
pub mod rgb;
mod stream;

use boxes::{Colour, Item, Meta, Property, Source};

use crate::image::ColorType;
pub use crate::io::hevc::Samples;
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
/// An HEVC item's units: its configuration's parameter sets, its data,
/// and where each of the data's NAL units lies in it.
type HevcUnits<'f> = (
    Vec<Vec<u8>>,
    std::borrow::Cow<'f, [u8]>,
    Vec<(usize, usize)>,
);

/// An HEVC item's NAL units: its configuration's parameter sets, then its
/// data's units.
pub(super) fn hevc_units<'f>(
    meta: &Meta<'_>,
    file: &Source<'f>,
    item: &Item,
) -> Result<HevcUnits<'f>, HeifError> {
    let (nals, length_size) = meta
        .properties_of(item)
        .find_map(|property| match property {
            Property::HevcConfig { nals, length_size } => Some((nals.clone(), *length_size)),
            _ => None,
        })
        .ok_or_else(|| HeifError::new("an HEVC item without its decoder configuration"))?;
    let data = meta.data(file, item)?;
    let mut ranges = Vec::new();
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
        ranges.push((at, end));
        at = end;
    }
    Ok((nals, data, ranges))
}

fn decode_hevc_item(
    meta: &Meta<'_>,
    file: &Source<'_>,
    item: &Item,
    workspace: &mut hevc::Workspace,
    threads: usize,
) -> Result<Planes, HeifError> {
    let (nals, data, ranges) = hevc_units(meta, file, item)?;
    let units = nals
        .iter()
        .map(Vec::as_slice)
        .chain(ranges.iter().map(|&(start, end)| &data[start..end]));
    let picture = hevc::decode_picture_in(units, workspace, threads)?;
    Ok(cropped(picture))
}

/// The primary picture (one HEVC item) decoded by bands as the streamed
/// path decodes it, put back together: for checking that path against
/// the whole one.
#[doc(hidden)]
pub fn decode_primary_by_bands(file: &[u8], threads: usize) -> Result<Planes, HeifError> {
    let meta = boxes::read_meta(file)?;
    let item = meta
        .item(meta.primary)
        .ok_or_else(|| HeifError::new("no primary image"))?;
    let (nals, data, ranges) = hevc_units(&meta, &Source::Bytes(file), item)?;
    let units = nals
        .iter()
        .map(Vec::as_slice)
        .chain(ranges.iter().map(|&(start, end)| &data[start..end]));
    Ok(cropped(hevc::decode_picture_assembled(units, threads)?))
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
    for (index, mut plane) in picture.planes.into_iter().enumerate() {
        let (shift_x, shift_y) = if index == 0 { (0, 0) } else { (sub_x, sub_y) };
        let stride = picture.sizes[index].0;
        let (x0, y0) = (left >> shift_x, top >> shift_y);
        let (w, h) = (width >> shift_x, height >> shift_y);
        // The window's rows moved up within the plane: no second copy.
        match &mut plane {
            Samples::Eight(samples) => crop_in_place(samples, stride, x0, y0, w, h),
            Samples::Deep(samples) => crop_in_place(samples, stride, x0, y0, w, h),
        }
        planes.push(plane);
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

/// Keeps the `w` by `h` window at (`x0`, `y0`) of a plane `stride` wide,
/// moving its rows to the front.
fn crop_in_place<T: Copy>(
    samples: &mut Vec<T>,
    stride: usize,
    x0: usize,
    y0: usize,
    w: usize,
    h: usize,
) {
    if w != stride || y0 != 0 {
        for row in 0..h {
            let start = (y0 + row) * stride + x0;
            samples.copy_within(start..start + w, row * w);
        }
    }
    samples.truncate(w * h);
}

/// An image item's planes: one picture, or a grid's tiles assembled.
fn decode_image_item(meta: &Meta<'_>, file: &Source<'_>, id: u32) -> Result<Planes, HeifError> {
    // One picture's wavefront rows decode on every thread; a grid's tiles
    // do instead, each on one.
    let threads = std::thread::available_parallelism().map_or(1, |count| count.get());
    decode_item_in(meta, file, id, &mut hevc::Workspace::default(), threads)
}

/// An image item's planes, an HEVC picture's buffers from `workspace`.
fn decode_item_in(
    meta: &Meta<'_>,
    file: &Source<'_>,
    id: u32,
    workspace: &mut hevc::Workspace,
    threads: usize,
) -> Result<Planes, HeifError> {
    let item = meta
        .item(id)
        .ok_or_else(|| HeifError::new("a missing image item"))?;
    match &item.kind {
        b"hvc1" => decode_hevc_item(meta, file, item, workspace, threads),
        b"grid" => {
            let grid = Grid::of(meta, file, item)?;
            let canvas: std::sync::Mutex<(Option<Planes>, Option<TileShape>)> =
                std::sync::Mutex::new((None, None));
            let place = |index: usize, tile: &Planes| -> Result<(), HeifError> {
                let mut guard = canvas.lock().unwrap_or_else(|poison| poison.into_inner());
                let (canvas, shape) = &mut *guard;
                grid.check(tile, shape)?;
                let canvas =
                    canvas.get_or_insert_with(|| Planes::blank(grid.width, grid.height, tile));
                let (x, y) = grid.origin(index, tile);
                canvas.place(tile, x, y);
                Ok(())
            };
            decode_tiles(meta, file, &grid.tiles, &place, None, &mut |_| Ok(())).map_err(
                |error| match error {
                    HeifRowsError::Heif(error) => error,
                    HeifRowsError::Io(error) => HeifError(error.to_string()),
                },
            )?;
            let (canvas, _) = canvas
                .into_inner()
                .unwrap_or_else(|poison| poison.into_inner());
            canvas.ok_or_else(|| HeifError::new("a grid without tiles"))
        }
        kind => Err(HeifError(format!(
            "an image item of type {} (only HEVC and grids are read)",
            String::from_utf8_lossy(kind)
        ))),
    }
}

/// A grid item: its tiles, row by row, and the canvas they cover.
pub(super) struct Grid {
    pub rows: usize,
    pub columns: usize,
    pub width: usize,
    pub height: usize,
    pub tiles: Vec<u32>,
}

impl Grid {
    pub(super) fn of(meta: &Meta<'_>, file: &Source<'_>, item: &Item) -> Result<Grid, HeifError> {
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
        let tiles = meta.referenced(b"dimg", item.id);
        if tiles.len() != rows * columns {
            return Err(HeifError::new("a grid without one tile per cell"));
        }
        Ok(Grid {
            rows,
            columns,
            width,
            height,
            tiles,
        })
    }

    /// Every tile of a grid is the first's size and format; the tiles
    /// cover the canvas.
    pub(super) fn check(
        &self,
        tile: &Planes,
        first: &mut Option<TileShape>,
    ) -> Result<(), HeifError> {
        let shape = TileShape::of(tile);
        if *first.get_or_insert(shape) != shape {
            return Err(HeifError::new("a grid whose tiles differ in format"));
        }
        if tile.width == 0
            || tile.height == 0
            || tile.width * self.columns < self.width
            || tile.height * self.rows < self.height
        {
            return Err(HeifError::new("a grid whose tiles do not cover it"));
        }
        Ok(())
    }

    /// Where tile `index` lies on the canvas, in luma samples.
    pub(super) fn origin(&self, index: usize, tile: &Planes) -> (usize, usize) {
        (
            (index % self.columns) * tile.width,
            (index / self.columns) * tile.height,
        )
    }
}

impl Planes {
    /// Planes `width` by `height` in the format of `like`, zeroed.
    pub(super) fn blank(width: usize, height: usize, like: &Planes) -> Planes {
        let (sub_x, sub_y) = subsampling(like.chroma_format);
        let sizes: Vec<(usize, usize)> = (0..like.planes.len())
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
            chroma_format: like.chroma_format,
            bit_depth: like.bit_depth,
            bit_depth_chroma: like.bit_depth_chroma,
            planes: sizes
                .iter()
                .enumerate()
                .map(|(plane, &(w, h))| {
                    let depth = if plane == 0 {
                        like.bit_depth
                    } else {
                        like.bit_depth_chroma
                    };
                    Samples::new(depth, w * h)
                })
                .collect(),
            sizes,
            vui_colour: like.vui_colour,
        }
    }

    /// Copies `tile` in with its top left at luma (`x`, `y`), cut at the
    /// edges.
    pub(super) fn place(&mut self, tile: &Planes, x: usize, y: usize) {
        let (sub_x, sub_y) = subsampling(self.chroma_format);
        for plane in 0..self.planes.len().min(tile.planes.len()) {
            let (shift_x, shift_y) = if plane == 0 { (0, 0) } else { (sub_x, sub_y) };
            let (width, height) = self.sizes[plane];
            let (tile_width, tile_height) = tile.sizes[plane];
            let (x0, y0) = (x >> shift_x, y >> shift_y);
            if x0 >= width || y0 >= height {
                continue;
            }
            let span = tile_width.min(width - x0);
            for row in 0..tile_height.min(height - y0) {
                self.planes[plane].copy_from(
                    (y0 + row) * width + x0,
                    &tile.planes[plane],
                    row * tile_width,
                    span,
                );
            }
        }
    }
}

/// What every tile of a grid shares.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct TileShape {
    width: usize,
    height: usize,
    chroma_format: u32,
    planes: usize,
    depths: (u32, u32),
}

impl TileShape {
    fn of(tile: &Planes) -> TileShape {
        TileShape {
            width: tile.width,
            height: tile.height,
            chroma_format: tile.chroma_format,
            planes: tile.planes.len(),
            depths: (tile.bit_depth, tile.bit_depth_chroma),
        }
    }
}

/// Chroma's horizontal and vertical subsampling shifts.
pub(super) fn subsampling(chroma_format: u32) -> (u32, u32) {
    match chroma_format {
        1 => (1, 1),
        2 => (1, 0),
        _ => (0, 0),
    }
}

/// Decodes each of `tiles` on as many threads as the machine has (one
/// where threads are not available), each worker into its own decoding
/// buffers, and `place`s each picture there on the worker as it is done.
/// Meanwhile the calling thread runs `drain` whenever tiles have been
/// placed, and last with `true` once all are: a consumer hands on what is
/// complete. Only the tiles being decoded are held beyond what `place`
/// keeps.
pub(super) fn decode_tiles(
    meta: &Meta<'_>,
    file: &Source<'_>,
    tiles: &[u32],
    place: &(dyn Fn(usize, &Planes) -> Result<(), HeifError> + Sync),
    until: Option<&std::sync::atomic::AtomicUsize>,
    drain: &mut dyn FnMut(bool) -> Result<(), HeifRowsError>,
) -> Result<(), HeifRowsError> {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Condvar, Mutex};
    let threads = std::thread::available_parallelism()
        .map_or(1, |count| count.get())
        .min(tiles.len());
    if threads <= 1 {
        let mut workspace = hevc::Workspace::default();
        for (index, &tile) in tiles.iter().enumerate() {
            let decoded = decode_item_in(meta, file, tile, &mut workspace, 1)?;
            place(index, &decoded)?;
            workspace.recycle(decoded.planes);
            drain(false)?;
        }
        return drain(true);
    }
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    // Tiles placed, and the first failure.
    let progress: Mutex<(usize, Option<HeifError>)> = Mutex::new((0, None));
    let changed = Condvar::new();
    let lock = || progress.lock().unwrap_or_else(|poison| poison.into_inner());
    std::thread::scope(|scope| {
        for _ in 0..threads {
            let (next, stop, changed, lock) = (&next, &stop, &changed, &lock);
            scope.spawn(move || {
                let mut workspace = hevc::Workspace::default();
                while !stop.load(Ordering::Relaxed) {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&tile) = tiles.get(index) else {
                        break;
                    };
                    // A tile past `until` waits for the tiles before it to
                    // be handed on, so decoding cannot run far ahead.
                    if let Some(until) = until {
                        let mut state = lock();
                        while index >= until.load(Ordering::Acquire)
                            && state.1.is_none()
                            && !stop.load(Ordering::Relaxed)
                        {
                            state = changed
                                .wait(state)
                                .unwrap_or_else(|poison| poison.into_inner());
                        }
                    }
                    let outcome = decode_item_in(meta, file, tile, &mut workspace, 1)
                        .and_then(|decoded| place(index, &decoded).map(|()| decoded));
                    let mut state = lock();
                    match outcome {
                        Ok(decoded) => {
                            state.0 += 1;
                            drop(state);
                            workspace.recycle(decoded.planes);
                        }
                        Err(error) => {
                            state.1.get_or_insert(error);
                            stop.store(true, Ordering::Relaxed);
                        }
                    }
                    changed.notify_all();
                }
            });
        }
        let mut seen = 0;
        loop {
            let mut state = lock();
            while state.0 == seen && state.0 < tiles.len() && state.1.is_none() {
                state = changed
                    .wait(state)
                    .unwrap_or_else(|poison| poison.into_inner());
            }
            if let Some(error) = state.1.take() {
                return Err(error.into());
            }
            seen = state.0;
            drop(state);
            let done = seen == tiles.len();
            let drained = drain(done);
            if drained.is_err() {
                stop.store(true, Ordering::Relaxed);
            }
            // The drain may have let more tiles start, or stopped them:
            // the waiting threads look again (under the lock, so none
            // misses the call).
            drop(lock());
            changed.notify_all();
            drained?;
            if done {
                return Ok(());
            }
        }
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
    let file = &Source::Bytes(file);
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
    /// The samples' bit depth, when above 8 and the sink took 8-bit rows.
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
pub(super) struct Placement {
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

/// The alpha plane as output samples, a row at a time, scaled to the
/// picture's size when it differs.
struct AlphaRows<'a> {
    alpha: &'a Planes,
    picture_width: usize,
    picture_height: usize,
    deep: bool,
}

impl<'a> AlphaRows<'a> {
    fn new(alpha: &'a Planes, picture: &Planes, deep: bool) -> AlphaRows<'a> {
        AlphaRows {
            alpha,
            picture_width: picture.width.max(1),
            picture_height: picture.height.max(1),
            deep,
        }
    }

    /// The alpha of picture row `y` from column `x0`, a sample (one or
    /// two bytes, big-endian) per pixel.
    fn row(&mut self, y: usize, x0: usize, out: &mut [u8]) {
        let alpha = self.alpha;
        let max = (1u32 << alpha.bit_depth) - 1;
        let target = if self.deep { 65_535 } else { 255 };
        let ay = y * alpha.height / self.picture_height;
        let same = alpha.width == self.picture_width;
        let bytes = if self.deep { 2 } else { 1 };
        for (index, sample) in out.chunks_exact_mut(bytes).enumerate() {
            let x = x0 + index;
            let ax = if same {
                x
            } else {
                x * alpha.width / self.picture_width
            };
            let value = u32::from(alpha.planes[0].get(ay * alpha.width + ax));
            let scaled = (value * target + max / 2) / max;
            if self.deep {
                sample.copy_from_slice(&(scaled as u16).to_be_bytes());
            } else {
                sample[0] = scaled as u8;
            }
        }
    }
}

/// Images in the file besides the primary, its tiles, alpha, and
/// thumbnails: what reading the primary leaves out.
fn other_images(meta: &Meta<'_>) -> usize {
    meta.items
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
        .count()
}

/// Whether the rows can go to a JPEG-like sink as the picture's own
/// YCbCr: 8-bit 4:2:0 by BT.601 at full range (JFIF's terms), read
/// forwards and down from an even row and column.
pub(super) fn ycbcr_exact(
    planes: &Planes,
    matrix: u16,
    full_range: bool,
    placed: &Placement,
) -> bool {
    planes.chroma_format == 1
        && planes.bit_depth == 8
        && planes.bit_depth_chroma == 8
        && matches!(matrix, 5 | 6)
        && full_range
        && placed.rows_stay_rows()
        && placed.x_from[0] == 1
        && placed.y_from[1] == 1
        && placed.x_from[2] % 2 == 0
        && placed.y_from[2] % 2 == 0
}

/// Hands output row `y` (source row `source_row`) to a sink that took
/// YCbCr: the luma row from column `x0`, and on even rows its chroma.
/// `origin` is the picture row the planes start at (a streamed band).
pub(super) fn ycbcr_row(
    planes: &Planes,
    origin: usize,
    source_row: usize,
    x0: usize,
    width: usize,
    sink: &mut dyn RowSink,
) -> std::io::Result<()> {
    let bytes = |plane: usize| planes.planes[plane].bytes().unwrap_or(&[]);
    let row = source_row - origin;
    let luma = &bytes(0)[row * planes.width + x0..][..width];
    let chroma_width = planes.sizes[1].0;
    let chroma = (source_row % 2 == 0).then(|| {
        let start = (row / 2) * chroma_width + x0 / 2;
        let count = width.div_ceil(2);
        (
            &bytes(1)[start..start + count],
            &bytes(2)[start..start + count],
        )
    });
    sink.ycbcr_row(luma, chroma)
}

/// The colour coefficients to convert with: the `nclx` property's, else
/// the HEVC stream's, else BT.601 full range (as phone files without
/// either mean).
fn coefficients(meta: &Meta<'_>, item: &Item, planes: &Planes) -> (u16, bool) {
    resolve_coefficients(nclx(meta, item), planes)
}

/// The `nclx` property's matrix and range, if the item has one.
fn nclx(meta: &Meta<'_>, item: &Item) -> Option<(u16, bool)> {
    meta.properties_of(item)
        .find_map(|property| match property {
            Property::Colour(Colour::Nclx {
                matrix, full_range, ..
            }) => Some((*matrix, *full_range)),
            _ => None,
        })
}

/// The file's coefficients, else the stream's, else BT.601 full range;
/// matrix 2 is "unspecified".
pub(super) fn resolve_coefficients(nclx: Option<(u16, bool)>, planes: &Planes) -> (u16, bool) {
    let stream = planes
        .vui_colour
        .map(|colour| (u16::from(colour.matrix), colour.full_range));
    nclx.filter(|&(matrix, _)| matrix != 2)
        .or(stream.filter(|&(matrix, _)| matrix != 2))
        .unwrap_or((6, true))
}

/// The primary image's Exif, as TIFF, with any orientation set to 1:
/// the rows come out turned already, as HEIF's own properties say.
fn exif(meta: &Meta<'_>, file: &Source<'_>) -> Option<Vec<u8>> {
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
    read_meta_rows(&boxes::read_meta(file)?, &Source::Bytes(file), sink, true)
}

/// `read_heif_rows` with every image decoded whole before its rows go
/// out: for checking the streamed paths against it.
#[doc(hidden)]
pub fn read_heif_rows_whole(
    file: &[u8],
    sink: &mut dyn RowSink,
) -> Result<HeifNotes, HeifRowsError> {
    read_meta_rows(&boxes::read_meta(file)?, &Source::Bytes(file), sink, false)
}

/// Reads a HEIF file as `read_heif_rows` does, from a file (or anything
/// that seeks): its `meta` box is read, then each item's bytes as they
/// are decoded, so the file is never held whole.
pub fn read_heif_rows_from(
    reader: &mut dyn crate::converter::RewindableRead,
    sink: &mut dyn RowSink,
) -> Result<HeifNotes, HeifRowsError> {
    let head = boxes::read_head(reader)?;
    let meta = boxes::read_meta(&head)?;
    read_meta_rows(
        &meta,
        &Source::Reader(std::sync::Mutex::new(reader)),
        sink,
        true,
    )
}

/// The rows of the image `meta` describes; `by_bands` lets a grid or a
/// picture go a band at a time when it can.
fn read_meta_rows(
    meta: &Meta<'_>,
    file: &Source<'_>,
    sink: &mut dyn RowSink,
    by_bands: bool,
) -> Result<HeifNotes, HeifRowsError> {
    let item = meta
        .item(meta.primary)
        .ok_or_else(|| HeifError::new("no primary image"))?;
    let mut notes = HeifNotes::default();
    if let Some(profile) = meta
        .properties_of(item)
        .find_map(|property| match property {
            Property::Colour(Colour::Icc(profile)) => Some(profile),
            _ => None,
        })
    {
        notes.profile_dropped = !sink.icc_profile(profile);
    }
    if let Some(tiff) = exif(meta, file) {
        notes.exif_dropped = !sink.exif(&tiff);
    }
    notes.other_images = other_images(meta);
    // A grid whose rows come out top to bottom streams a band of tiles at
    // a time; anything else is decoded whole first.
    if &item.kind == b"grid" && alpha_item(meta).is_none() {
        let grid = Grid::of(meta, file, item)?;
        let placed = placement(meta, item, grid.width, grid.height);
        let even_tiles = grid
            .tiles
            .first()
            .and_then(|&tile| meta.item(tile))
            .and_then(|tile| {
                meta.properties_of(tile)
                    .find_map(|property| match property {
                        Property::Size { height, .. } => Some(height % 2 == 0),
                        _ => None,
                    })
            })
            .unwrap_or(false);
        if by_bands && even_tiles && stream::streams(&placed) {
            stream::stream_grid(
                meta,
                file,
                &grid,
                stream::Stream {
                    placed,
                    nclx: nclx(meta, item),
                    sink,
                    notes: &mut notes,
                },
            )?;
            return Ok(notes);
        }
    }
    // One picture streams by CTB rows on its wavefront threads (none in
    // WebAssembly, where the code is left out).
    let threads = std::thread::available_parallelism().map_or(1, |count| count.get());
    let threaded = cfg!(not(target_family = "wasm")) && threads > 1;
    if &item.kind == b"hvc1" && alpha_item(meta).is_none() && threaded && by_bands {
        let streamed = stream::stream_picture(
            meta,
            file,
            item,
            threads,
            nclx(meta, item),
            sink,
            &mut notes,
        )?;
        if streamed {
            return Ok(notes);
        }
    }
    let planes = decode_image_item(meta, file, meta.primary)?;
    let alpha = alpha_item(meta)
        .map(|id| decode_image_item(meta, file, id))
        .transpose()?;
    notes.deep = (planes.bit_depth > 8).then_some(planes.bit_depth);
    let (matrix, full_range) = coefficients(meta, item, &planes);
    let colour_channels = if planes.planes.len() < 3 { 1 } else { 3 };
    let channels = colour_channels + usize::from(alpha.is_some());
    let color = match channels {
        1 => ColorType::Gray,
        2 => ColorType::GrayAlpha,
        3 => ColorType::Rgb,
        _ => ColorType::Rgba,
    };
    // Deeper samples go on at 16 bits to a sink that holds them.
    let deep_source =
        planes.bit_depth > 8 || alpha.as_ref().is_some_and(|alpha| alpha.bit_depth > 8);
    let deep = deep_source && sink.accept_deep(color);
    if deep {
        notes.deep = None;
    }
    let placed = placement(meta, item, planes.width, planes.height);
    if alpha.is_none() && ycbcr_exact(&planes, matrix, full_range, &placed) && sink.accept_ycbcr() {
        sink.start(placed.width as u32, placed.height as u32, color)?;
        let x0 = placed.x_from[2] as usize;
        for y in 0..placed.height {
            let (_, source_row) = placed.source(0, y);
            ycbcr_row(&planes, 0, source_row, x0, placed.width, sink)?;
        }
        return Ok(notes);
    }
    let mut converter = rgb::Converter::new(&planes, matrix, full_range, deep);
    let (width, height) = (placed.width, placed.height);
    let sample_bytes = if deep { 2 } else { 1 };
    let mut alpha_rows = alpha
        .as_ref()
        .map(|alpha| AlphaRows::new(alpha, &planes, deep));
    sink.start(width as u32, height as u32, color)?;
    let pixel_bytes = channels * sample_bytes;
    let colour_bytes = colour_channels * sample_bytes;
    let mut out = vec![0u8; width * pixel_bytes];
    let convert = |converter: &mut rgb::Converter<'_>, y: usize, x0: usize, out: &mut [u8]| {
        if deep {
            converter.row_deep(y, x0, out);
        } else {
            converter.row(y, x0, out);
        }
    };
    if placed.rows_stay_rows() {
        // Each output row is one source row, read forwards or backwards
        // from its first column.
        let backwards = placed.x_from[0] < 0;
        let x0 = if backwards {
            (placed.x_from[2] - (width as i64 - 1)) as usize
        } else {
            placed.x_from[2] as usize
        };
        let mut colour = vec![0u8; width * colour_bytes];
        let mut opacity = vec![0u8; width * sample_bytes];
        for y in 0..height {
            let (_, sy) = placed.source(0, y);
            if alpha_rows.is_none() && !backwards {
                convert(&mut converter, sy, x0, &mut out);
                sink.row(&out)?;
                continue;
            }
            convert(&mut converter, sy, x0, &mut colour);
            if let Some(alpha_rows) = alpha_rows.as_mut() {
                alpha_rows.row(sy, x0, &mut opacity);
            }
            for (x, pixel) in out.chunks_exact_mut(pixel_bytes).enumerate() {
                let from = if backwards { width - 1 - x } else { x };
                pixel[..colour_bytes]
                    .copy_from_slice(&colour[from * colour_bytes..(from + 1) * colour_bytes]);
                if channels > colour_channels {
                    pixel[colour_bytes..]
                        .copy_from_slice(&opacity[from * sample_bytes..(from + 1) * sample_bytes]);
                }
            }
            sink.row(&out)?;
        }
    } else {
        // Turned a quarter: each output row is one source column. A strip
        // of source columns is converted down every source row, then read
        // out as that many output rows.
        const STRIP: usize = 64;
        let source_height = planes.height;
        let mut strip = vec![0u8; STRIP * source_height * pixel_bytes];
        let mut colour = vec![0u8; STRIP * colour_bytes];
        let mut opacity = vec![0u8; STRIP * sample_bytes];
        let mut y0 = 0;
        while y0 < height {
            let rows = STRIP.min(height - y0);
            // The source columns these output rows read.
            let columns: Vec<usize> = (y0..y0 + rows).map(|y| placed.source(0, y).0).collect();
            let first = *columns.iter().min().unwrap_or(&0);
            let span = rows;
            for sy in 0..source_height {
                let colour = &mut colour[..span * colour_bytes];
                convert(&mut converter, sy, first, colour);
                let line = &mut strip[sy * span * pixel_bytes..(sy + 1) * span * pixel_bytes];
                if let Some(alpha_rows) = alpha_rows.as_mut() {
                    let opacity = &mut opacity[..span * sample_bytes];
                    alpha_rows.row(sy, first, opacity);
                    for (column, pixel) in line.chunks_exact_mut(pixel_bytes).enumerate() {
                        pixel[..colour_bytes].copy_from_slice(
                            &colour[column * colour_bytes..(column + 1) * colour_bytes],
                        );
                        pixel[colour_bytes..].copy_from_slice(
                            &opacity[column * sample_bytes..(column + 1) * sample_bytes],
                        );
                    }
                } else {
                    line.copy_from_slice(colour);
                }
            }
            for (offset, &column) in columns.iter().enumerate() {
                let y = y0 + offset;
                let within = column - first;
                for (x, pixel) in out.chunks_exact_mut(pixel_bytes).enumerate() {
                    let (_, sy) = placed.source(x, y);
                    let at = (sy * span + within) * pixel_bytes;
                    pixel.copy_from_slice(&strip[at..at + pixel_bytes]);
                }
                sink.row(&out)?;
            }
            y0 += rows;
        }
    }

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
