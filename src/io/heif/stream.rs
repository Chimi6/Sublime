//! A grid read a band of tiles at a time: when its rows come out as the
//! canvas's rows, top to bottom (no quarter turn, no flip down), each row
//! of tiles is converted and handed on as soon as it and the row below
//! are decoded, while the tiles after it decode. A band's buffer carries
//! margins for the rows the chroma interpolation reads across its edges
//! (the last of the band above, the first of the band below), and goes
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

use super::boxes::Meta;

/// Picture rows in a band's margins: chroma rows are interpolated with
/// the one above and below, so two luma rows (one 4:2:0 chroma row).
const MARGIN: usize = 2;

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
    file: &[u8],
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
                let rows = top + band_rows(grid, row, tile.height) + MARGIN;
                let planes = take_buffer(&mut bands.pool, grid.width, rows, tile);
                entry.insert(Band { planes, tiles: 0 })
            }
        };
        let (x, _) = grid.origin(index, tile);
        band.planes.place(tile, x, top);
        band.tiles += 1;
        Ok(())
    };
    let mut handing = Handing {
        grid,
        stream,
        next: 0,
        started: None,
        row: 0,
    };
    decode_tiles(meta, file, &grid.tiles, &place, &mut |_| {
        handing.drain(&lock)
    })?;
    if handing.next < grid.rows {
        return Err(HeifError::new("a grid whose tiles did not all decode").into());
    }
    Ok(())
}

/// The top margin of band `row`: none for the first.
fn margin_top(row: usize) -> usize {
    if row == 0 { 0 } else { MARGIN }
}

/// The picture rows of band `row`, the last cut at the canvas's edge.
fn band_rows(grid: &Grid, row: usize, tile_height: usize) -> usize {
    tile_height.min(grid.height - row * tile_height)
}

/// A buffer for a band from the pool when one of its size is there.
fn take_buffer(pool: &mut Vec<Planes>, width: usize, rows: usize, like: &Planes) -> Planes {
    match pool.iter().position(|planes| planes.height == rows) {
        Some(at) => pool.swap_remove(at),
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
}

impl Handing<'_, '_> {
    /// Hands on every band that is complete with the band below it (or is
    /// the last).
    fn drain<'b>(&mut self, lock: &dyn Fn() -> MutexGuard<'b, Bands>) -> Result<(), HeifRowsError> {
        loop {
            let (band, tile_height, format) = {
                let mut bands = lock();
                let columns = self.grid.columns;
                let complete = |bands: &Bands, row: usize| {
                    bands
                        .filling
                        .get(&row)
                        .is_some_and(|band| band.tiles == columns)
                };
                let row = self.next;
                let ready = row < self.grid.rows
                    && complete(&bands, row)
                    && (row + 1 == self.grid.rows || complete(&bands, row + 1));
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
                let Some(mut band) = bands.filling.remove(&row).map(|band| band.planes) else {
                    return Ok(());
                };
                // The margins: this band's bottom from the band below's
                // first rows, the band below's top from this band's last.
                if let Some(below) = bands.filling.get_mut(&(row + 1)) {
                    let end = margin_top(row) + band_rows(self.grid, row, tile_height);
                    band.copy_rows(end, &below.planes, MARGIN, MARGIN);
                    below.planes.copy_rows(0, &band, end - MARGIN, MARGIN);
                }
                (band, tile_height, format)
            };
            if self.started.is_none() {
                self.start(&format)?;
            }
            let outcome = self.hand_on(self.next, tile_height, &band);
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
        let Some(Started {
            deep,
            ycbcr,
            channels,
            matrix,
            full_range,
        }) = self.started
        else {
            return Ok(());
        };
        let band_top = row * tile_height;
        let band_end = band_top + band_rows(self.grid, row, tile_height);
        let mut converter = Converter::new(band, matrix, full_range, deep)
            .windowed(band_top - margin_top(row), self.grid.height);
        let placed = self.stream.placed;
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
        while self.row < placed.height {
            let (_, source_row) = placed.source(0, self.row);
            if source_row >= band_end {
                break;
            }
            if ycbcr {
                let origin = band_top - margin_top(row);
                super::ycbcr_row(band, origin, source_row, x0, width, &mut *self.stream.sink)?;
                self.row += 1;
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
            self.stream.sink.row(&out)?;
            self.row += 1;
        }
        Ok(())
    }
}

impl Planes {
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
