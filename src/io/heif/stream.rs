//! A grid read a band of tiles at a time: when its rows come out as the
//! canvas's rows, top to bottom (no quarter turn, no flip down), each row
//! of tiles is converted and handed on as soon as it is decoded, while
//! the tiles after it decode. The chroma interpolation reads a row across
//! a band's edges, so a band's last rows wait for the band below, and a
//! band's buffer has a margin on top for the rows above it. Buffers go
//! back to a pool for a later band: a 48-megapixel photo holds a few
//! bands, not its canvas, and allocates them once.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use super::rgb::Converter;
use super::{
    Grid, HeifError, HeifNotes, HeifRowsError, Placement, Planes, TileShape, decode_tiles,
    subsampling,
};
use crate::image::ColorType;
use crate::io::png::RowSink;

use super::boxes::{Meta, Source};

/// Picture rows in a band's margins: chroma rows are interpolated with
/// the one above and below, so two luma rows (one 4:2:0 chroma row).
const MARGIN: usize = 2;

/// Rows of tiles decoding at once from the band being handed on: it and
/// the band below it; and past them a tile a thread, so every thread
/// has work while a band converts. The bands in hand stay a few however
/// slow the sink.
const AHEAD: usize = 2;

/// Rows on top of a band's buffer from the band above: its last rows,
/// which wait for this band, and the rows above them they read.
const CARRIED: usize = 2 * MARGIN;

/// What the streamed rows need beyond the tiles.
pub(super) struct Stream<'s> {
    pub placed: Placement,
    /// The file's `nclx` coefficients; the stream's fill in.
    pub nclx: Option<(u16, bool)>,
    pub sink: &'s mut dyn RowSink,
    pub notes: &'s mut HeifNotes,
}

/// Whether a grid placed this way can stream: rows top to bottom.
pub(super) fn streams(placed: &Placement) -> bool {
    placed.rows_stay_rows() && placed.y_from[1] == 1
}

/// A row of tiles being assembled: its planes (margins included) and how
/// many tiles are in.
struct Band {
    planes: Planes,
    tiles: usize,
}

/// The bands the workers fill, and buffers to fill them with.
#[derive(Default)]
struct Bands {
    filling: HashMap<usize, Band>,
    pool: Vec<Planes>,
    shape: Option<TileShape>,
    /// The tiles' height, and their format as empty planes, once a tile
    /// is in.
    tile: Option<(usize, Planes)>,
}

pub(super) fn stream_grid(
    meta: &Meta<'_>,
    file: &Source<'_>,
    grid: &Grid,
    stream: Stream<'_>,
) -> Result<(), HeifRowsError> {
    let bands = Mutex::new(Bands::default());
    let lock = || bands.lock().unwrap_or_else(|poison| poison.into_inner());
    let place = |index: usize, tile: &Planes| -> Result<(), HeifError> {
        let mut bands = lock();
        let bands = &mut *bands;
        grid.check(tile, &mut bands.shape)?;
        let row = index / grid.columns;
        let top = margin_top(row);
        if bands.tile.is_none() {
            bands.tile = Some((tile.height, Planes::blank(0, 0, tile)));
        }
        let band = match bands.filling.entry(row) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                let rows = top + band_rows(grid, row, tile.height);
                let planes = take_buffer(&mut bands.pool, grid.width, rows, tile);
                entry.insert(Band { planes, tiles: 0 })
            }
        };
        let (x, _) = grid.origin(index, tile);
        band.planes.place(tile, x, top);
        band.tiles += 1;
        Ok(())
    };
    let threads = std::thread::available_parallelism().map_or(1, |count| count.get());
    let until = std::sync::atomic::AtomicUsize::new((AHEAD - 1) * grid.columns + threads);
    let mut handing = Handing {
        grid,
        stream,
        next: 0,
        started: None,
        row: 0,
        carry: None,
    };
    decode_tiles(meta, file, &grid.tiles, &place, Some(&until), &mut |_| {
        let outcome = handing.drain(&lock);
        let ahead = (handing.next + AHEAD - 1) * grid.columns + threads;
        until.store(ahead, std::sync::atomic::Ordering::Release);
        outcome
    })?;
    if handing.next < grid.rows {
        return Err(HeifError::new("a grid whose tiles did not all decode").into());
    }
    Ok(())
}

/// The top margin of band `row`: none for the first.
fn margin_top(row: usize) -> usize {
    if row == 0 { 0 } else { CARRIED }
}

/// The picture rows of band `row`, the last cut at the canvas's edge.
fn band_rows(grid: &Grid, row: usize, tile_height: usize) -> usize {
    tile_height.min(grid.height - row * tile_height)
}

/// A buffer for a band: one from the pool made `rows` high, or a new one.
fn take_buffer(pool: &mut Vec<Planes>, width: usize, rows: usize, like: &Planes) -> Planes {
    match pool.pop() {
        Some(mut planes) => {
            planes.reshape(rows);
            planes
        }
        None => Planes::blank(width, rows, like),
    }
}

/// The output format, settled by the first band.
#[derive(Clone, Copy)]
struct Started {
    deep: bool,
    /// The sink takes the tiles' own YCbCr.
    ycbcr: bool,
    channels: usize,
    matrix: u16,
    full_range: bool,
}

/// The calling thread's side: bands handed on in order.
struct Handing<'g, 's> {
    grid: &'g Grid,
    stream: Stream<'s>,
    /// The next band to hand on.
    next: usize,
    started: Option<Started>,
    /// Output rows handed on so far.
    row: usize,
    /// The last rows of the band handed on, for the next band's margin.
    carry: Option<Planes>,
}

impl Handing<'_, '_> {
    /// Hands on every band that is complete, in order.
    fn drain<'b>(&mut self, lock: &dyn Fn() -> MutexGuard<'b, Bands>) -> Result<(), HeifRowsError> {
        loop {
            let (mut band, tile_height, format) = {
                let mut bands = lock();
                let row = self.next;
                let ready = row < self.grid.rows
                    && bands
                        .filling
                        .get(&row)
                        .is_some_and(|band| band.tiles == self.grid.columns);
                if !ready {
                    return Ok(());
                }
                let Some((tile_height, format)) = bands
                    .tile
                    .as_ref()
                    .map(|(height, format)| (*height, Planes::blank(0, 0, format)))
                else {
                    return Ok(());
                };
                let Some(band) = bands.filling.remove(&row).map(|band| band.planes) else {
                    return Ok(());
                };
                (band, tile_height, format)
            };
            if self.started.is_none() {
                self.start(&format)?;
            }
            if let Some(carry) = &self.carry {
                band.copy_rows(0, carry, 0, carry.height);
            }
            let outcome = self.hand_on(self.next, tile_height, &band);
            // The band's last rows (with the rows above them) for the band
            // below's margin.
            let keep = band.height.min(CARRIED);
            let carry = self
                .carry
                .get_or_insert_with(|| Planes::blank(band.width, keep, &band));
            carry.reshape(keep);
            carry.copy_rows(0, &band, band.height - keep, keep);
            lock().pool.push(band);
            outcome?;
            self.next += 1;
        }
    }

    /// Settles the output from the tiles' format and starts the sink.
    fn start(&mut self, format: &Planes) -> Result<(), HeifRowsError> {
        let colour_channels = if format.planes.len() < 3 { 1 } else { 3 };
        let color = if colour_channels == 1 {
            ColorType::Gray
        } else {
            ColorType::Rgb
        };
        let (matrix, full_range) = super::resolve_coefficients(self.stream.nclx, format);
        let ycbcr = super::ycbcr_exact(format, matrix, full_range, &self.stream.placed)
            && self.stream.sink.accept_ycbcr();
        let deep = !ycbcr && format.bit_depth > 8 && self.stream.sink.accept_deep(color);
        self.stream.notes.deep = (format.bit_depth > 8 && !deep).then_some(format.bit_depth);
        let placed = &self.stream.placed;
        self.stream
            .sink
            .start(placed.width as u32, placed.height as u32, color)?;
        self.started = Some(Started {
            deep,
            ycbcr,
            channels: colour_channels,
            matrix,
            full_range,
        });
        Ok(())
    }

    /// Converts and hands on the output rows that come from band `row`.
    fn hand_on(&mut self, row: usize, tile_height: usize, band: &Planes) -> std::io::Result<()> {
        let Some(started) = self.started else {
            return Ok(());
        };
        let band_top = row * tile_height;
        // The last rows wait for the band below, but for the last band.
        let held = if row + 1 == self.grid.rows { 0 } else { MARGIN };
        let band_end = band_top + band_rows(self.grid, row, tile_height) - held;
        hand_on_rows(
            &started,
            &self.stream.placed,
            band,
            band_top - margin_top(row),
            self.grid.height,
            band_end,
            &mut self.row,
            &mut *self.stream.sink,
        )
    }
}

/// Converts and hands on every output row whose source row is above
/// `band_end`, from `band` (the picture's rows from `origin`, enough above
/// and below for the chroma interpolation), counting in `row`.
#[allow(clippy::too_many_arguments)]
fn hand_on_rows(
    started: &Started,
    placed: &Placement,
    band: &Planes,
    origin: usize,
    picture_height: usize,
    band_end: usize,
    row: &mut usize,
    sink: &mut dyn RowSink,
) -> std::io::Result<()> {
    let Started {
        deep,
        ycbcr,
        channels,
        matrix,
        full_range,
    } = *started;
    let mut converter =
        Converter::new(band, matrix, full_range, deep).windowed(origin, picture_height);
    let sample_bytes = if deep { 2 } else { 1 };
    let width = placed.width;
    let pixel_bytes = channels * sample_bytes;
    let backwards = placed.x_from[0] < 0;
    let x0 = if backwards {
        (placed.x_from[2] - (width as i64 - 1)) as usize
    } else {
        placed.x_from[2] as usize
    };
    let mut out = vec![0u8; width * pixel_bytes];
    let mut colour = vec![0u8; width * pixel_bytes];
    while *row < placed.height {
        let (_, source_row) = placed.source(0, *row);
        if source_row >= band_end {
            break;
        }
        if ycbcr {
            super::ycbcr_row(band, origin, source_row, x0, width, sink)?;
            *row += 1;
            continue;
        }
        let target = if backwards { &mut colour } else { &mut out };
        if deep {
            converter.row_deep(source_row, x0, target);
        } else {
            converter.row(source_row, x0, target);
        }
        if backwards {
            for (pixel, from) in out
                .chunks_exact_mut(pixel_bytes)
                .zip(colour.chunks_exact(pixel_bytes).rev())
            {
                pixel.copy_from_slice(from);
            }
        }
        sink.row(&out)?;
        *row += 1;
    }
    Ok(())
}

/// Settles a stream's output from the picture's format and starts the
/// sink: 16 bits to a sink that takes them, the picture's own YCbCr to
/// one that takes it.
fn start_output(
    format: &Planes,
    nclx: Option<(u16, bool)>,
    placed: &Placement,
    sink: &mut dyn RowSink,
    notes: &mut HeifNotes,
) -> std::io::Result<Started> {
    let colour_channels = if format.planes.len() < 3 { 1 } else { 3 };
    let color = if colour_channels == 1 {
        ColorType::Gray
    } else {
        ColorType::Rgb
    };
    let (matrix, full_range) = super::resolve_coefficients(nclx, format);
    let ycbcr = super::ycbcr_exact(format, matrix, full_range, placed) && sink.accept_ycbcr();
    let deep = !ycbcr && format.bit_depth > 8 && sink.accept_deep(color);
    notes.deep = (format.bit_depth > 8 && !deep).then_some(format.bit_depth);
    sink.start(placed.width as u32, placed.height as u32, color)?;
    Ok(Started {
        deep,
        ycbcr,
        channels: colour_channels,
        matrix,
        full_range,
    })
}

/// Streams one HEVC picture (no grid, no alpha) by its CTB rows: each band
/// is converted once the band below is in, from a window of the last rows
/// of the band above, the band, and the first rows of the band below.
/// False, with nothing started, when it cannot stream (its rows do not
/// come out in order, or its size is not known ahead).
pub(super) fn stream_picture(
    meta: &Meta<'_>,
    file: &Source<'_>,
    item: &super::Item,
    threads: usize,
    nclx: Option<(u16, bool)>,
    sink: &mut dyn RowSink,
    notes: &mut HeifNotes,
) -> Result<bool, HeifRowsError> {
    let Some((width, height)) = meta
        .properties_of(item)
        .find_map(|property| match property {
            super::Property::Size { width, height } => Some((*width as usize, *height as usize)),
            _ => None,
        })
    else {
        return Ok(false);
    };
    let placed = super::placement(meta, item, width, height);
    if !streams(&placed) {
        return Ok(false);
    }
    let (nals, data, ranges) = super::hevc_units(meta, file, item)?;
    let units = nals
        .iter()
        .map(Vec::as_slice)
        .chain(ranges.iter().map(|&(start, end)| &data[start..end]));
    let state = std::cell::RefCell::new(PictureStream {
        placed,
        size: (width, height),
        nclx,
        sink,
        notes,
        started: None,
        window: None,
        crop: [0; 4],
        coded_width: 0,
        coded_height: 0,
        held: None,
        row: 0,
        failure: None,
    });
    let fail = || crate::io::hevc::HevcError::new("the rows' sink failed");
    let outcome = crate::io::hevc::decode_picture_by_bands(
        units,
        threads,
        &mut |shape| state.borrow_mut().shape(shape).map_err(|_| fail()),
        &mut |done| state.borrow_mut().band(done).map_err(|_| fail()),
    );
    let mut state = state.into_inner();
    if let Some(failure) = state.failure.take() {
        return Err(HeifRowsError::Io(failure));
    }
    outcome.map_err(HeifError::from)?;
    state.finish()?;
    if state.row < state.placed.height {
        return Err(HeifError::new("a picture whose rows did not all decode").into());
    }
    Ok(true)
}

/// A single picture being streamed: its output's setting, and a window
/// holding the band in hand after the last rows of the band before it.
struct PictureStream<'s> {
    placed: Placement,
    size: (usize, usize),
    nclx: Option<(u16, bool)>,
    sink: &'s mut dyn RowSink,
    notes: &'s mut HeifNotes,
    started: Option<Started>,
    /// Kept from band to band, so its buffers are allocated once.
    window: Option<Planes>,
    /// The conformance window: left, right, top, bottom; and the coded
    /// picture's height.
    crop: [usize; 4],
    coded_width: usize,
    coded_height: usize,
    /// The band in hand: its first picture row, and the window's rows
    /// carried from above it and its own.
    held: Option<(usize, usize, usize)>,
    row: usize,
    failure: Option<std::io::Error>,
}

impl PictureStream<'_> {
    fn shape(&mut self, shape: &crate::io::hevc::Shape) -> Result<(), ()> {
        let [left, right, top, bottom] = shape.crop.map(|value| value as usize);
        if (
            shape.width.saturating_sub(left + right),
            shape.height.saturating_sub(top + bottom),
        ) != self.size
        {
            self.failure = Some(std::io::Error::other(
                "a picture not the size its item says",
            ));
            return Err(());
        }
        // Bands are cut to the conformance window: the samples past it are
        // not the picture's, and the chroma interpolation stops at its edge.
        self.crop = [left, right, top, bottom];
        self.coded_width = shape.width;
        self.coded_height = shape.height;
        let components = if shape.chroma_format == 0 { 1 } else { 3 };
        let format = Planes::blank(
            self.size.0,
            0,
            &Planes {
                width: 0,
                height: 0,
                chroma_format: shape.chroma_format,
                bit_depth: shape.bit_depth_luma,
                bit_depth_chroma: shape.bit_depth_chroma,
                planes: (0..components)
                    .map(|_| super::Samples::new(shape.bit_depth_luma, 0))
                    .collect(),
                sizes: Vec::new(),
                vui_colour: shape.vui_colour,
            },
        );
        match start_output(
            &format,
            self.nclx,
            &self.placed,
            &mut *self.sink,
            self.notes,
        ) {
            Ok(started) => self.started = Some(started),
            Err(error) => {
                self.failure = Some(error);
                return Err(());
            }
        }
        self.window = Some(format);
        Ok(())
    }

    fn band(&mut self, done: &crate::io::hevc::DoneBand) -> Result<(), ()> {
        let Some(mut window) = self.window.take() else {
            return Ok(());
        };
        // The band's rows inside the window, in the window's rows.
        let [left, _, top_crop, bottom_crop] = self.crop;
        let first = done.top.max(top_crop);
        let end = (done.top + done.rows).min(self.coded_height - bottom_crop);
        let outcome = if first < end {
            let (skip, rows) = (first - done.top, end - first);
            let mut carried = 0;
            let mut outcome = Ok(());
            if let Some((top, above, held)) = self.held.take() {
                // The band in hand converts with this band's first rows
                // below it, then leaves its last rows on top.
                let head = rows.min(MARGIN);
                window.reshape(above + held + head);
                copy_band(
                    &mut window,
                    above + held,
                    done,
                    self.coded_width,
                    left,
                    skip,
                    head,
                );
                outcome = self.convert(top, above, held, &window);
                carried = held.min(MARGIN);
                window.raise(above + held - carried, carried);
            }
            window.reshape(carried + rows);
            copy_band(
                &mut window,
                carried,
                done,
                self.coded_width,
                left,
                skip,
                rows,
            );
            self.held = Some((first - top_crop, carried, rows));
            outcome
        } else {
            Ok(())
        };
        self.window = Some(window);
        outcome
    }

    /// Converts the rows of the band in hand, `held` rows from picture row
    /// `top` after `above` rows carried, from `window`.
    fn convert(
        &mut self,
        top: usize,
        above: usize,
        held: usize,
        window: &Planes,
    ) -> Result<(), ()> {
        let Some(started) = self.started else {
            return Ok(());
        };
        let outcome = hand_on_rows(
            &started,
            &self.placed,
            window,
            top - above,
            self.size.1,
            top + held,
            &mut self.row,
            &mut *self.sink,
        );
        outcome.map_err(|error| self.failure = Some(error))
    }

    /// The last band, with nothing below it.
    fn finish(&mut self) -> Result<(), HeifRowsError> {
        if let (Some((top, above, held)), Some(window)) = (self.held.take(), self.window.take()) {
            if self.convert(top, above, held, &window).is_err() {
                let failure = self
                    .failure
                    .take()
                    .unwrap_or_else(|| std::io::Error::other("sink"));
                return Err(HeifRowsError::Io(failure));
            }
        }
        Ok(())
    }
}

/// Copies `rows` rows of a decoded band from its row `skip`, from column
/// `left`, to `window`'s row `at` on.
fn copy_band(
    window: &mut Planes,
    at: usize,
    done: &crate::io::hevc::DoneBand,
    coded_width: usize,
    left: usize,
    skip: usize,
    rows: usize,
) {
    let (sub_x, sub_y) = subsampling(window.chroma_format);
    for (component, (to, from)) in window.planes.iter_mut().zip(&done.planes).enumerate() {
        let (shift_x, shift_y) = if component == 0 {
            (0, 0)
        } else {
            (sub_x, sub_y)
        };
        let (width, height) = window.sizes[component];
        let stride = coded_width >> shift_x;
        let first = at >> shift_y;
        let count = rows
            .div_ceil(1 << shift_y)
            .min(height.saturating_sub(first));
        for row in 0..count {
            let source = ((skip >> shift_y) + row) * stride + (left >> shift_x);
            to.copy_from((first + row) * width, from, source, width);
        }
    }
}

impl Planes {
    /// Makes these planes `height` rows high, keeping their storage and
    /// the rows they share with what they were.
    fn reshape(&mut self, height: usize) {
        let (_, sub_y) = subsampling(self.chroma_format);
        self.height = height;
        for (plane, (size, samples)) in self.sizes.iter_mut().zip(&mut self.planes).enumerate() {
            size.1 = if plane == 0 {
                height
            } else {
                height.div_ceil(1 << sub_y)
            };
            samples.resize(size.0 * size.1);
        }
    }

    /// Moves `rows` rows from luma row `from` (even in 4:2:0) to the top.
    fn raise(&mut self, from: usize, rows: usize) {
        let (_, sub_y) = subsampling(self.chroma_format);
        for (plane, ((width, height), samples)) in
            self.sizes.iter().zip(&mut self.planes).enumerate()
        {
            let shift = if plane == 0 { 0 } else { sub_y };
            let start = from >> shift;
            let count = rows.div_ceil(1 << shift).min(height.saturating_sub(start));
            samples.raise(start * width, count * width);
        }
    }

    /// Copies `rows` picture rows of `from` (luma row `from_row` on) to
    /// luma row `to_row` on; both even in 4:2:0.
    fn copy_rows(&mut self, to_row: usize, from: &Planes, from_row: usize, rows: usize) {
        let (_, sub_y) = subsampling(self.chroma_format);
        for plane in 0..self.planes.len() {
            let shift = if plane == 0 { 0 } else { sub_y };
            let width = self.sizes[plane].0;
            let (to, source) = (to_row >> shift, from_row >> shift);
            let count = (rows >> shift)
                .min(self.sizes[plane].1.saturating_sub(to))
                .min(from.sizes[plane].1.saturating_sub(source));
            self.planes[plane].copy_from(
                to * width,
                &from.planes[plane],
                source * width,
                count * width,
            );
        }
    }
}
