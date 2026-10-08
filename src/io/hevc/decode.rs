//! Decoding an intra picture (H.265 7.3.8, 8.4, 8.6): slice segments into
//! coding tree units, each parsed and reconstructed as it is read, then the
//! in-loop filters (`filter.rs`).

use super::HevcError;
use super::cabac::{self, Cabac, Contexts};
use super::intra::{self, References};
use super::params::{Pps, ScalingFactors, SliceHeader, Sps};
use super::sample::{Sample, Samples};
use super::transform::{self, LEVEL_SCALE};

/// A decoded picture: its planes at their own bit depth, before the
/// conformance window is applied.
pub struct Picture {
    pub width: usize,
    pub height: usize,
    /// 0 monochrome, 1 4:2:0, 2 4:2:2, 3 4:4:4.
    pub chroma_format: u32,
    pub bit_depth_luma: u32,
    pub bit_depth_chroma: u32,
    /// Luma, then Cb and Cr (absent for monochrome), row-major.
    pub planes: Vec<Samples>,
    /// Each plane's width and height.
    pub sizes: Vec<(usize, usize)>,
    /// The conformance window: left, right, top, bottom, in luma samples.
    pub crop: [u32; 4],
    pub vui_colour: Option<super::params::VuiColour>,
}

/// What a coded block leaves for its neighbours, the filters, and the
/// sample adaptive offset, per 4x4 luma unit.
#[derive(Clone, Copy, Default)]
pub struct Unit {
    /// The coding quadtree depth (`split_cu_flag`'s context).
    pub depth: u8,
    /// The luma intra prediction mode.
    pub mode: u8,
    /// QpY of the coding unit.
    pub qp: i8,
    /// Transquant bypass, or PCM with its loop filter off: the filters
    /// leave the samples alone.
    pub unfiltered: bool,
    pub pcm: bool,
    /// A transform or prediction block's left edge runs along this unit's
    /// left side, and its top edge along its top.
    pub left_edge: bool,
    pub top_edge: bool,
}

/// One component's sample adaptive offset in a CTB.
#[derive(Clone, Copy, Default)]
pub struct Sao {
    /// 0 off, 1 band, 2 edge.
    pub kind: u8,
    pub offsets: [i32; 5],
    pub band: u8,
    pub class: u8,
}

/// What a picture's decoding reads and never changes once it starts: the
/// parameter sets, the sizes, and the scans. Every row reads it at once.
pub struct Layout {
    pub sps: Sps,
    pub pps: Pps,
    pub width: usize,
    pub height: usize,
    pub ctb_log2: u32,
    pub width_ctbs: usize,
    pub height_ctbs: usize,
    pub sub_x: u32,
    pub sub_y: u32,
    pub chroma: u32,
    pub strides: Vec<usize>,
    pub units_wide: usize,
    pub rs_to_ts: Vec<usize>,
    pub ts_to_rs: Vec<usize>,
    pub tile_of: Vec<usize>,
    scaling: Option<ScalingFactors>,
    /// Threads the picture's rows and filters may use.
    pub threads: usize,
}

/// The state of a picture being decoded.
pub struct Decoder {
    pub layout: Layout,
    pub planes: Vec<Samples>,
    pub units: Vec<Unit>,
    /// Per CTB (raster): the index of its slice segment header, and the
    /// raster address of its slice's first CTB.
    pub slice_of: Vec<usize>,
    slice_address_of: Vec<usize>,
    pub slices: Vec<SliceHeader>,
    pub sao: Vec<[Sao; 3]>,
    decoded: Vec<bool>,
    /// Context state saved after a row's second CTB, for the row below
    /// (wavefront parallel processing).
    saved_row_contexts: Option<Contexts>,
    /// Context state and last QpY at the end of a slice segment, for a
    /// dependent segment after it.
    saved_slice_contexts: Option<(Contexts, i32)>,
}

/// One component's samples in a band of rows, writable.
pub(super) enum Band<'a> {
    Eight(&'a mut [u8]),
    Deep(&'a mut [u16]),
}

/// Runs `$body` with `$samples` bound to a band's samples of either width.
macro_rules! with_band {
    ($band:expr, $samples:ident => $body:expr) => {
        match $band {
            Band::Eight($samples) => $body,
            Band::Deep($samples) => $body,
        }
    };
}

pub(super) use with_band;

/// The bottom of the CTB row above a band, as the band reads it: each
/// component's last line of samples, the last row of 4x4 units, and each
/// CTB's record.
pub(super) struct Edge {
    pub lines: Vec<Samples>,
    pub units: Vec<Unit>,
    pub decoded: Vec<bool>,
    pub slice_address_of: Vec<usize>,
    pub sao: Vec<[Sao; 3]>,
    /// The context state after the row's second CTB (wavefront storage).
    pub contexts: Option<Contexts>,
    /// CTBs of the row received so far.
    received: usize,
}

impl Edge {
    fn new(layout: &Layout) -> Edge {
        let lines = (0..layout.strides.len())
            .map(|component| {
                let depth = if component == 0 {
                    layout.sps.bit_depth_luma
                } else {
                    layout.sps.bit_depth_chroma
                };
                Samples::new(depth, layout.strides[component])
            })
            .collect();
        Edge {
            lines,
            units: vec![Unit::default(); layout.units_wide],
            decoded: vec![false; layout.width_ctbs],
            slice_address_of: vec![usize::MAX; layout.width_ctbs],
            sao: vec![[Sao::default(); 3]; layout.width_ctbs],
            contexts: None,
            received: 0,
        }
    }

    fn apply(&mut self, update: RowUpdate) {
        match update {
            RowUpdate::Contexts(contexts) => self.contexts = Some(*contexts),
            RowUpdate::Ctb(ctb) => {
                for (line, (from, segment)) in self.lines.iter_mut().zip(ctb.lines) {
                    line.copy_from(from, &segment, 0, segment.len());
                }
                self.units[ctb.unit_from..ctb.unit_from + ctb.units.len()]
                    .copy_from_slice(&ctb.units);
                self.decoded[ctb.ctb_x] = true;
                self.slice_address_of[ctb.ctb_x] = ctb.slice_address;
                self.sao[ctb.ctb_x] = ctb.sao;
                self.received = ctb.ctb_x + 1;
            }
        }
    }
}

/// What a row tells the row below as it goes.
enum RowUpdate {
    Ctb(Box<CtbBottom>),
    /// After the row's second CTB.
    Contexts(Box<Contexts>),
}

/// A finished CTB's bottom: its last line of each component (from where
/// in the line), its last row of units, and its record.
struct CtbBottom {
    ctb_x: usize,
    lines: Vec<(usize, Samples)>,
    unit_from: usize,
    units: Vec<Unit>,
    slice_address: usize,
    sao: [Sao; 3],
}

/// A CTB row's share of the picture, to decode on its own.
struct RowParts<'a> {
    row: usize,
    units: Units<'a>,
    slice_of: &'a mut [usize],
    slice_address_of: &'a mut [usize],
    sao: &'a mut [[Sao; 3]],
    decoded: &'a mut [bool],
}

/// A row's 4x4 units: a share of the whole picture's, or a buffer of its
/// own when the picture streams.
enum Units<'a> {
    Shared(&'a mut [Unit]),
    Own(Vec<Unit>),
}

impl std::ops::Deref for Units<'_> {
    type Target = [Unit];
    fn deref(&self) -> &[Unit] {
        match self {
            Units::Shared(units) => units,
            Units::Own(units) => units,
        }
    }
}

impl std::ops::DerefMut for Units<'_> {
    fn deref_mut(&mut self) -> &mut [Unit] {
        match self {
            Units::Shared(units) => units,
            Units::Own(units) => units,
        }
    }
}

/// A band of CTB rows being decoded: its samples, units, and CTB records
/// (all from the band's first row), and the edge of the row above it,
/// when the band does not start the picture. A band can be the whole
/// picture.
pub(super) struct Rows<'a> {
    layout: &'a Layout,
    /// The band's first CTB row.
    top_ctb: usize,
    planes: Vec<Band<'a>>,
    units: &'a mut [Unit],
    slice_of: &'a mut [usize],
    slice_address_of: &'a mut [usize],
    sao: &'a mut [[Sao; 3]],
    decoded: &'a mut [bool],
    above: Option<Edge>,
    /// The last luma block's residual, for cross-component prediction.
    luma_residual: Vec<i32>,
    /// A block's levels, then residuals: kept between blocks.
    levels: Vec<i32>,
}

impl std::ops::Deref for Rows<'_> {
    type Target = Layout;

    fn deref(&self) -> &Layout {
        self.layout
    }
}

impl<'a> Rows<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        layout: &'a Layout,
        top_ctb: usize,
        planes: Vec<Band<'a>>,
        units: &'a mut [Unit],
        slice_of: &'a mut [usize],
        slice_address_of: &'a mut [usize],
        sao: &'a mut [[Sao; 3]],
        decoded: &'a mut [bool],
        above: Option<Edge>,
    ) -> Rows<'a> {
        Rows {
            layout,
            top_ctb,
            planes,
            units,
            slice_of,
            slice_address_of,
            sao,
            decoded,
            above,
            luma_residual: Vec::new(),
            levels: vec![0; 32 * 32],
        }
    }

    /// The band's first row of `component`'s samples.
    fn top_of(&self, component: usize) -> usize {
        let top = self.top_ctb << self.ctb_log2;
        if component == 0 {
            top
        } else {
            top >> self.sub_y
        }
    }

    /// CTB `rs`'s index in the band's records, when it is in the band.
    fn in_band(&self, rs: usize) -> Option<usize> {
        rs.checked_sub(self.top_ctb * self.width_ctbs)
    }

    /// The 4x4 unit holding luma (x, y): the band's, or the row above's.
    pub fn unit(&self, x: usize, y: usize) -> Unit {
        let top = (self.top_ctb << self.ctb_log2) >> 2;
        match (y >> 2).checked_sub(top) {
            Some(row) => self.units[row * self.units_wide + (x >> 2)],
            None => self
                .above
                .as_ref()
                .map_or(Unit::default(), |edge| edge.units[x >> 2]),
        }
    }

    fn unit_mut(&mut self, x: usize, y: usize) -> &mut Unit {
        let top = (self.top_ctb << self.ctb_log2) >> 2;
        let at = ((y >> 2) - top) * self.units_wide + (x >> 2);
        &mut self.units[at]
    }

    fn decoded_at(&self, rs: usize) -> bool {
        match self.in_band(rs) {
            Some(at) => self.decoded[at],
            None => self
                .above
                .as_ref()
                .is_some_and(|edge| edge.decoded[rs % self.width_ctbs]),
        }
    }

    fn slice_address(&self, rs: usize) -> usize {
        match self.in_band(rs) {
            Some(at) => self.slice_address_of[at],
            None => self.above.as_ref().map_or(usize::MAX, |edge| {
                edge.slice_address_of[rs % self.width_ctbs]
            }),
        }
    }

    fn sao_of(&self, rs: usize) -> [Sao; 3] {
        match self.in_band(rs) {
            Some(at) => self.sao[at],
            None => self
                .above
                .as_ref()
                .map_or([Sao::default(); 3], |edge| edge.sao[rs % self.width_ctbs]),
        }
    }

    fn sao_mut(&mut self, rs: usize) -> &mut [Sao; 3] {
        let at = rs - self.top_ctb * self.width_ctbs;
        &mut self.sao[at]
    }

    /// Records CTB `rs` as in a slice segment and its slice.
    fn place_ctb(&mut self, rs: usize, slice_index: usize, slice_address: usize) {
        let at = rs - self.top_ctb * self.width_ctbs;
        self.slice_of[at] = slice_index;
        self.slice_address_of[at] = slice_address;
    }

    fn mark_decoded(&mut self, rs: usize) {
        let at = rs - self.top_ctb * self.width_ctbs;
        self.decoded[at] = true;
    }

    /// The z-scan order of the 4x4 unit at luma (`x`, `y`) (6.5.2): its
    /// CTB's tile scan address, then the unit's place in the CTB's quadtree.
    fn z_order(&self, x: usize, y: usize) -> usize {
        let unit_log2 = self.ctb_log2 - 2;
        let mask = (1 << unit_log2) - 1;
        let inside = SPREAD[(x >> 2) & mask] | (SPREAD[(y >> 2) & mask] << 1);
        (self.rs_to_ts[self.ctb_address(x, y)] << (unit_log2 * 2)) | inside
    }

    fn ctb_address(&self, x: usize, y: usize) -> usize {
        (y >> self.ctb_log2) * self.width_ctbs + (x >> self.ctb_log2)
    }

    /// Whether the block at (`nx`, `ny`) is available to the block at
    /// (`x`, `y`) (6.4.1): inside the picture, decoded already, in the same
    /// slice and tile.
    pub fn available(&self, x: usize, y: usize, nx: isize, ny: isize) -> bool {
        if nx < 0 || ny < 0 || nx as usize >= self.width || ny as usize >= self.height {
            return false;
        }
        let (nx, ny) = (nx as usize, ny as usize);
        if self.z_order(nx, ny) > self.z_order(x, y) {
            return false;
        }
        let (a, b) = (self.ctb_address(x, y), self.ctb_address(nx, ny));
        if !self.decoded_at(b) && a != b {
            return false;
        }
        self.slice_address(a) == self.slice_address(b) && self.tile_of[a] == self.tile_of[b]
    }
}

/// A unit's column or row in its CTB with a zero between each bit: half of
/// a z-scan order (CTBs are at most 16 units across).
const SPREAD: [usize; 16] = [0, 1, 4, 5, 16, 17, 20, 21, 64, 65, 68, 69, 80, 81, 84, 85];

/// A row of samples above a band, at the band's sample width.
pub(super) fn above_line<T: Sample>(line: Option<&Samples>) -> &[T] {
    line.map_or(&[], T::slice)
}

/// What the loop filters read around the samples they change: the
/// picture's layout, its 4x4 units, its slices, and each CTB's slice and
/// offsets. The whole decoder is one; a window of decoded bands another.
pub(super) trait Neighbourhood: Sync {
    fn layout(&self) -> &Layout;
    fn unit(&self, x: usize, y: usize) -> Unit;
    fn slice_index(&self, ctb: usize) -> usize;
    fn slices(&self) -> &[SliceHeader];
    fn sao_params(&self, ctb: usize) -> [Sao; 3];
}

impl Neighbourhood for Decoder {
    fn layout(&self) -> &Layout {
        &self.layout
    }

    fn unit(&self, x: usize, y: usize) -> Unit {
        self.units[(y >> 2) * self.units_wide + (x >> 2)]
    }

    fn slice_index(&self, ctb: usize) -> usize {
        self.slice_of[ctb]
    }

    fn slices(&self) -> &[SliceHeader] {
        &self.slices
    }

    fn sao_params(&self, ctb: usize) -> [Sao; 3] {
        self.sao[ctb]
    }
}

impl std::ops::Deref for Decoder {
    type Target = Layout;

    fn deref(&self) -> &Layout {
        &self.layout
    }
}

impl Decoder {
    /// A decoder whose large buffers come from `workspace`, when it holds
    /// some from a picture before.
    pub fn reusing(
        sps: Sps,
        mut pps: Pps,
        workspace: &mut Workspace,
        threads: usize,
        whole: bool,
    ) -> Result<Decoder, HevcError> {
        let width = sps.width as usize;
        let height = sps.height as usize;
        // The size is a whole number of minimum coding blocks (7.4.3.2.1).
        let min_cb = 1usize << sps.log2_min_cb;
        if width == 0 || height == 0 || width % min_cb != 0 || height % min_cb != 0 {
            return Err(HevcError::new("a picture size not in whole coding blocks"));
        }
        // Four times the largest picture any HEVC level allows.
        if width * height > 1 << 27 {
            return Err(HevcError::new("a picture too large to decode"));
        }
        // QP groups are no smaller than the smallest coding block (7.4.3.3.1).
        if pps.diff_cu_qp_delta_depth > sps.log2_ctb.saturating_sub(sps.log2_min_cb) {
            return Err(HevcError::new("a QP group smaller than a coding block"));
        }
        let ctb_log2 = sps.log2_ctb;
        let ctb = 1usize << ctb_log2;
        let width_ctbs = width.div_ceil(ctb);
        let height_ctbs = height.div_ceil(ctb);
        pps.lay_out_tiles(width_ctbs as u32, height_ctbs as u32);
        let chroma = sps.chroma_array_type();
        let (sub_x, sub_y) = match chroma {
            1 => (1, 1),
            2 => (1, 0),
            _ => (0, 0),
        };
        // Every sample is predicted before it is read, so reused planes
        // need no clearing.
        let mut spare = std::mem::take(&mut workspace.planes).into_iter();
        let mut plane = |depth: u32, count: usize| match spare.next() {
            Some(samples) => samples.reused(depth, count),
            None => Samples::new(depth, count),
        };
        // A picture streamed by bands holds no whole planes.
        let mut planes = Vec::new();
        let mut strides = vec![width];
        if whole {
            planes.push(plane(sps.bit_depth_luma, width * height));
        }
        if chroma != 0 {
            let (chroma_width, chroma_height) = (width >> sub_x, height >> sub_y);
            if whole {
                planes.push(plane(sps.bit_depth_chroma, chroma_width * chroma_height));
                planes.push(plane(sps.bit_depth_chroma, chroma_width * chroma_height));
            }
            strides.push(chroma_width);
            strides.push(chroma_width);
        }
        // Tile scan (6.5.1).
        let ctbs = width_ctbs * height_ctbs;
        let mut rs_to_ts = vec![0usize; ctbs];
        let mut tile_of = vec![0usize; ctbs];
        let columns = &pps.tile_columns;
        let rows = &pps.tile_rows;
        for address in 0..ctbs {
            let (x, y) = ((address % width_ctbs) as u32, (address / width_ctbs) as u32);
            let tile_x = columns
                .windows(2)
                .position(|w| x >= w[0] && x < w[1])
                .unwrap_or(0);
            let tile_y = rows
                .windows(2)
                .position(|w| y >= w[0] && y < w[1])
                .unwrap_or(0);
            let mut ts = 0usize;
            for i in 0..tile_x {
                ts += ((rows[tile_y + 1] - rows[tile_y]) * (columns[i + 1] - columns[i])) as usize;
            }
            for j in 0..tile_y {
                ts += (width_ctbs as u32 * (rows[j + 1] - rows[j])) as usize;
            }
            ts += ((y - rows[tile_y]) * (columns[tile_x + 1] - columns[tile_x]) + x
                - columns[tile_x]) as usize;
            rs_to_ts[address] = ts;
            tile_of[address] = tile_y * (columns.len() - 1) + tile_x;
        }
        let mut ts_to_rs = vec![0usize; ctbs];
        for (rs, &ts) in rs_to_ts.iter().enumerate() {
            ts_to_rs[ts] = rs;
        }
        let units_wide = width.div_ceil(4);
        let units_high = height.div_ceil(4);
        let scaling = pps.scaling.clone().or_else(|| sps.scaling.clone());
        Ok(Decoder {
            layout: Layout {
                width,
                height,
                ctb_log2,
                width_ctbs,
                height_ctbs,
                sub_x,
                sub_y,
                chroma,
                strides,
                units_wide,
                rs_to_ts,
                ts_to_rs,
                tile_of,
                scaling,
                sps,
                pps,
                threads: threads.max(1),
            },
            planes,
            units: {
                let mut units = std::mem::take(&mut workspace.units);
                units.clear();
                // A streamed picture's rows hold their own.
                if whole {
                    units.resize(units_wide * units_high, Unit::default());
                }
                units
            },
            slice_of: vec![usize::MAX; ctbs],
            slice_address_of: vec![usize::MAX; ctbs],
            slices: Vec::new(),
            sao: vec![[Sao::default(); 3]; ctbs],
            decoded: vec![false; ctbs],
            saved_row_contexts: None,
            saved_slice_contexts: None,
        })
    }

    /// Decodes one slice segment.
    pub fn slice(&mut self, header: SliceHeader, data: &[u8]) -> Result<(), HevcError> {
        let slice_index = self.slices.len();
        let slice_qp = self.pps.init_qp + header.qp_delta;
        let qp_offset = 6 * (self.sps.bit_depth_luma as i32 - 8);
        if !(-qp_offset..=51).contains(&slice_qp) {
            return Err(HevcError::new("a slice QP out of range"));
        }
        let ctbs = self.width_ctbs * self.height_ctbs;
        let mut ts = self.rs_to_ts[header.address as usize];
        // An independent segment starts a slice; a dependent one continues
        // the slice before it.
        let slice_address = if header.dependent {
            let previous = self
                .ts_to_rs
                .get(ts.wrapping_sub(1))
                .ok_or_else(|| HevcError::new("a dependent slice at the picture's start"))?;
            self.slice_address_of[*previous]
        } else {
            header.address as usize
        };
        self.slices.push(header.clone());
        let Decoder {
            layout,
            planes,
            units,
            slice_of,
            slice_address_of,
            sao,
            decoded,
            saved_row_contexts,
            saved_slice_contexts,
            ..
        } = self;
        let planes = planes.iter_mut().map(Samples::band).collect();
        let mut rows = Rows::new(
            layout,
            0,
            planes,
            units,
            slice_of,
            slice_address_of,
            sao,
            decoded,
            None,
        );
        let mut cabac = Cabac::new(data, header.data_offset);
        let mut contexts = Contexts::new(slice_qp);
        let mut state = SliceState {
            qp: slice_qp,
            header,
            last_qp: slice_qp,
            quant_group: (usize::MAX, usize::MAX),
            group_qp: slice_qp,
            qp_delta: 0,
            qp_delta_coded: false,
            chroma_offset: (0, 0),
            chroma_offset_coded: false,
            bypass: false,
        };
        let mut first = true;
        loop {
            if ts >= ctbs {
                return Err(HevcError::new("a slice running past the picture"));
            }
            let rs = rows.ts_to_rs[ts];
            let (ctb_x, ctb_y) = (rs % rows.width_ctbs, rs / rows.width_ctbs);
            rows.place_ctb(rs, slice_index, slice_address);
            // The context state and QP predictor at a CTB's start (9.3.1,
            // 8.6.1): fresh at a tile's start; at a row's start under
            // wavefronts, the state after the second CTB above when that CTB
            // is available, else fresh; at a dependent segment's start, the
            // state the segment before ended with.
            let tile_start = ts == 0 || rows.tile_of[rs] != rows.tile_of[rows.ts_to_rs[ts - 1]];
            let row_start = rows.pps.entropy_sync && ctb_x == rows.tile_column_start(ctb_x);
            if tile_start {
                contexts = Contexts::new(slice_qp);
                state.last_qp = slice_qp;
            } else if row_start {
                let x = ctb_x << rows.ctb_log2;
                let y = ctb_y << rows.ctb_log2;
                let ctb = 1isize << rows.ctb_log2;
                let above_right = rows.available(x, y, x as isize + ctb, y as isize - ctb);
                contexts = match (&*saved_row_contexts, above_right) {
                    (Some(saved), true) => saved.clone(),
                    _ => Contexts::new(slice_qp),
                };
                state.last_qp = slice_qp;
            } else if first && state.header.dependent {
                if let Some((saved, last_qp)) = &*saved_slice_contexts {
                    contexts = saved.clone();
                    state.last_qp = *last_qp;
                }
            }
            first = false;
            rows.coding_tree_unit(&mut cabac, &mut contexts, &mut state, ctb_x, ctb_y)?;
            rows.mark_decoded(rs);
            // Storage for the row below (9.3.2.4): after a row's second CTB
            // in its tile.
            if rows.pps.entropy_sync && ctb_x == rows.tile_column_start(ctb_x) + 1 {
                *saved_row_contexts = Some(contexts.clone());
            }
            let end_of_slice = cabac.terminate() == 1;
            ts += 1;
            if end_of_slice {
                if rows.pps.dependent_slices {
                    *saved_slice_contexts = Some((contexts, state.last_qp));
                }
                return Ok(());
            }
            if ts >= ctbs {
                return Err(HevcError::new("a slice without its end"));
            }
            let next = rows.ts_to_rs[ts];
            let new_tile = rows.pps.tiles && rows.tile_of[next] != rows.tile_of[rs];
            let new_row = rows.pps.entropy_sync
                && (next % rows.width_ctbs == 0 || rows.tile_of[next] != rows.tile_of[next - 1]);
            if new_tile || new_row {
                if cabac.terminate() != 1 {
                    return Err(HevcError::new("a substream without its end bit"));
                }
                cabac.restart();
            }
        }
    }
}

impl Decoder {
    /// Decodes a picture's one slice segment, its wavefront rows on up to
    /// `threads` threads at once (9.3.1, 9.3.2.4): each CTB row from its own
    /// substream (`starts`, positions in `data`), a row's CTB waiting for
    /// the CTB above and to its right, a row starting from the context
    /// state after the second CTB above. Each row owns its band of the
    /// picture and hears the bottom of the row above as it is decoded.
    pub fn slice_in_rows(
        &mut self,
        header: SliceHeader,
        data: &[u8],
        starts: &[usize],
        threads: usize,
    ) -> Result<(), HevcError> {
        let slice_qp = self.pps.init_qp + header.qp_delta;
        let qp_offset = 6 * (self.sps.bit_depth_luma as i32 - 8);
        if !(-qp_offset..=51).contains(&slice_qp) {
            return Err(HevcError::new("a slice QP out of range"));
        }
        let rows = self.height_ctbs;
        if starts.len() != rows
            || starts.windows(2).any(|pair| pair[0] >= pair[1])
            || starts.last().is_some_and(|&last| last >= data.len())
        {
            return Err(HevcError::new("entry points that do not divide the rows"));
        }
        self.slices.push(header.clone());
        let Decoder {
            layout,
            planes,
            units,
            slice_of,
            slice_address_of,
            sao,
            decoded,
            ..
        } = self;
        let layout: &Layout = layout;
        let width_ctbs = layout.width_ctbs;
        let ctb = 1usize << layout.ctb_log2;
        // Each row's share: its band of every plane, units, and records.
        let mut plane_bands: Vec<std::vec::IntoIter<Band<'_>>> = planes
            .iter_mut()
            .enumerate()
            .map(|(component, plane)| {
                let rows_per_band = if component == 0 {
                    ctb
                } else {
                    ctb >> layout.sub_y
                };
                plane
                    .band()
                    .split(rows_per_band * layout.strides[component])
                    .into_iter()
            })
            .collect();
        let unit_rows = (ctb >> 2) * layout.units_wide;
        let mut unit_bands = units.chunks_mut(unit_rows);
        let mut slice_bands = slice_of.chunks_mut(width_ctbs);
        let mut address_bands = slice_address_of.chunks_mut(width_ctbs);
        let mut sao_bands = sao.chunks_mut(width_ctbs);
        let mut decoded_bands = decoded.chunks_mut(width_ctbs);
        let workers = threads.clamp(1, rows);
        let mut shares: Vec<Vec<(RowParts<'_>, Vec<Band<'_>>)>> =
            (0..workers).map(|_| Vec::new()).collect();
        for row in 0..rows {
            let planes: Vec<Band<'_>> = plane_bands.iter_mut().filter_map(Iterator::next).collect();
            let parts = RowParts {
                row,
                units: Units::Shared(unit_bands.next().unwrap_or_default()),
                slice_of: slice_bands.next().unwrap_or_default(),
                slice_address_of: address_bands.next().unwrap_or_default(),
                sao: sao_bands.next().unwrap_or_default(),
                decoded: decoded_bands.next().unwrap_or_default(),
            };
            shares[row % workers].push((parts, planes));
        }
        // Row r tells row r + 1.
        let mut senders = Vec::new();
        let mut receivers = vec![None];
        for _ in 1..rows {
            let (sender, receiver) = std::sync::mpsc::channel::<RowUpdate>();
            senders.push(Some(sender));
            receivers.push(Some(receiver));
        }
        senders.push(None);
        let mut links: Vec<Vec<_>> = (0..workers).map(|_| Vec::new()).collect();
        for (row, (sender, receiver)) in senders.into_iter().zip(receivers).enumerate() {
            links[row % workers].push((sender, receiver));
        }
        let header = &header;
        let failures: Vec<(usize, HevcError)> = std::thread::scope(|scope| {
            let handles: Vec<_> = shares
                .into_iter()
                .zip(links)
                .map(|(share, links)| {
                    scope.spawn(move || {
                        for ((mut parts, planes), (sender, receiver)) in
                            share.into_iter().zip(links)
                        {
                            let row = parts.row;
                            let end = starts.get(row + 1).copied().unwrap_or(data.len());
                            let outcome = decode_row(
                                layout,
                                &mut parts,
                                planes,
                                header,
                                &data[..end],
                                starts[row],
                                slice_qp,
                                receiver,
                                sender,
                            );
                            if let Err(error) = outcome {
                                return Some((row, error));
                            }
                        }
                        None
                    })
                })
                .collect();
            handles
                .into_iter()
                .filter_map(|handle| {
                    handle.join().unwrap_or_else(|_| {
                        Some((usize::MAX, HevcError::new("a row's decoding failed")))
                    })
                })
                .collect()
        });
        // The first failure in picture order: rows below a failed row fail
        // too, for want of it.
        match failures.into_iter().min_by_key(|(row, _)| *row) {
            Some((_, error)) => Err(error),
            None => Ok(()),
        }
    }
}

/// Decodes CTB row `parts.row` from its substream at `start`, hearing the
/// row above on `above` and telling the row below on `below`.
#[allow(clippy::too_many_arguments)]
fn decode_row(
    layout: &Layout,
    parts: &mut RowParts<'_>,
    planes: Vec<Band<'_>>,
    header: &SliceHeader,
    data: &[u8],
    start: usize,
    slice_qp: i32,
    above: Option<std::sync::mpsc::Receiver<RowUpdate>>,
    below: Option<std::sync::mpsc::Sender<RowUpdate>>,
) -> Result<(), HevcError> {
    let row = parts.row;
    let width_ctbs = layout.width_ctbs;
    let edge = above.as_ref().map(|_| Edge::new(layout));
    let mut rows = Rows::new(
        layout,
        row,
        planes,
        &mut parts.units,
        parts.slice_of,
        parts.slice_address_of,
        parts.sao,
        parts.decoded,
        edge,
    );
    let lost = || HevcError::new("the row above stopped");
    let mut cabac = Cabac::new(data, start);
    let mut contexts = Contexts::new(slice_qp);
    let mut state = SliceState {
        qp: slice_qp,
        header: header.clone(),
        last_qp: slice_qp,
        quant_group: (usize::MAX, usize::MAX),
        group_qp: slice_qp,
        qp_delta: 0,
        qp_delta_coded: false,
        chroma_offset: (0, 0),
        chroma_offset_coded: false,
        bypass: false,
    };
    for ctb_x in 0..width_ctbs {
        let rs = row * width_ctbs + ctb_x;
        rows.place_ctb(rs, 0, 0);
        // The CTB above and to the right (or above, at the row's end) must
        // be in before this one reads it.
        if let (Some(receiver), Some(edge)) = (&above, rows.above.as_mut()) {
            let needed = (ctb_x + 2).min(width_ctbs);
            while edge.received < needed {
                edge.apply(receiver.recv().map_err(|_| lost())?);
            }
        }
        if ctb_x == 0 {
            // A row starts from the state after the second CTB above.
            contexts = match rows.above.as_ref().and_then(|edge| edge.contexts.clone()) {
                Some(saved) if width_ctbs > 1 => saved,
                _ => Contexts::new(slice_qp),
            };
            state.last_qp = slice_qp;
        }
        rows.coding_tree_unit(&mut cabac, &mut contexts, &mut state, ctb_x, row)?;
        rows.mark_decoded(rs);
        if let Some(sender) = &below {
            if ctb_x == 1 {
                let _ = sender.send(RowUpdate::Contexts(Box::new(contexts.clone())));
            }
            let _ = sender.send(RowUpdate::Ctb(Box::new(rows.bottom_of(ctb_x))));
        }
        let end_of_slice = cabac.terminate() == 1;
        let last_row = row + 1 == layout.height_ctbs;
        if ctb_x + 1 == width_ctbs {
            if end_of_slice != last_row {
                return Err(HevcError::new("a wavefront row ending out of place"));
            }
            if !last_row && cabac.terminate() != 1 {
                return Err(HevcError::new("a substream without its end bit"));
            }
        } else if end_of_slice {
            return Err(HevcError::new("a slice ending inside a row"));
        }
    }
    Ok(())
}

impl Rows<'_> {
    /// The bottom of the band's CTB `ctb_x`, for the row below.
    fn bottom_of(&self, ctb_x: usize) -> CtbBottom {
        let layout = self.layout;
        let ctb = 1usize << layout.ctb_log2;
        let top = self.top_ctb << layout.ctb_log2;
        let last_row = (top + ctb).min(layout.height) - 1;
        let lines = self
            .planes
            .iter()
            .enumerate()
            .map(|(component, band)| {
                let (shift_x, shift_y) = if component == 0 {
                    (0, 0)
                } else {
                    (layout.sub_x, layout.sub_y)
                };
                let stride = layout.strides[component];
                let from = (ctb_x * ctb) >> shift_x;
                let to = (((ctb_x + 1) * ctb) >> shift_x).min(stride);
                let row = (last_row >> shift_y) - (top >> shift_y);
                let at = row * stride;
                let segment = match band {
                    Band::Eight(samples) => Samples::Eight(samples[at + from..at + to].to_vec()),
                    Band::Deep(samples) => Samples::Deep(samples[at + from..at + to].to_vec()),
                };
                (from, segment)
            })
            .collect();
        let unit_from = (ctb_x * ctb) >> 2;
        let unit_to = (((ctb_x + 1) * ctb) >> 2).min(layout.units_wide);
        let unit_row = (last_row >> 2) - (top >> 2);
        let units = self.units
            [unit_row * layout.units_wide + unit_from..unit_row * layout.units_wide + unit_to]
            .to_vec();
        let rs = self.top_ctb * layout.width_ctbs + ctb_x;
        CtbBottom {
            ctb_x,
            lines,
            unit_from,
            units,
            slice_address: self.slice_address(rs),
            sao: self.sao_of(rs),
        }
    }
}

impl<'a> Band<'a> {
    /// The band cut in two at sample `at`.
    pub(super) fn split_at(self, at: usize) -> (Band<'a>, Band<'a>) {
        match self {
            Band::Eight(samples) => {
                let (before, after) = samples.split_at_mut(at);
                (Band::Eight(before), Band::Eight(after))
            }
            Band::Deep(samples) => {
                let (before, after) = samples.split_at_mut(at);
                (Band::Deep(before), Band::Deep(after))
            }
        }
    }

    /// The band cut into pieces of `size` samples (the last shorter).
    pub(super) fn split(self, size: usize) -> Vec<Band<'a>> {
        match self {
            Band::Eight(samples) => samples.chunks_mut(size).map(Band::Eight).collect(),
            Band::Deep(samples) => samples.chunks_mut(size).map(Band::Deep).collect(),
        }
    }
}

/// A band of picture rows the loop filters have finished, handed on in
/// order: the CTB row's luma rows from `top`, and each component's samples
/// as rows of the plane's width.
pub struct DoneBand {
    pub top: usize,
    pub rows: usize,
    pub planes: Vec<Samples>,
}

/// A decoded row's records, back from its worker for the filters.
struct RowRecords<'a> {
    units: Units<'a>,
    slice_of: &'a [usize],
    sao: &'a [[Sao; 3]],
    /// Some unit of the row leaves its samples unfiltered.
    unfiltered: bool,
}

/// The rows the filters can read around a band: the picture's layout and
/// slices, and the records of the decoded rows still held.
struct Window<'w, 'a> {
    layout: &'a Layout,
    slices: &'a [SliceHeader],
    rows: &'w std::collections::BTreeMap<usize, RowRecords<'a>>,
}

impl Neighbourhood for Window<'_, '_> {
    fn layout(&self) -> &Layout {
        self.layout
    }

    fn unit(&self, x: usize, y: usize) -> Unit {
        let row = y >> self.layout.ctb_log2;
        let top = (row << self.layout.ctb_log2) >> 2;
        self.rows.get(&row).map_or(Unit::default(), |records| {
            records.units[((y >> 2) - top) * self.layout.units_wide + (x >> 2)]
        })
    }

    fn slice_index(&self, ctb: usize) -> usize {
        let width = self.layout.width_ctbs;
        self.rows
            .get(&(ctb / width))
            .map_or(0, |records| records.slice_of[ctb % width])
    }

    fn slices(&self) -> &[SliceHeader] {
        self.slices
    }

    fn sao_params(&self, ctb: usize) -> [Sao; 3] {
        let width = self.layout.width_ctbs;
        self.rows
            .get(&(ctb / width))
            .map_or([Sao::default(); 3], |records| records.sao[ctb % width])
    }
}

/// A finished row's band buffers and records, or why it failed.
type RowDone<'a> = Result<(usize, Vec<Samples>, RowRecords<'a>), (usize, HevcError)>;

impl Decoder {
    /// Decodes a picture's one slice segment as `slice_in_rows` does, but
    /// holds no whole planes: each row decodes into a band buffer from a
    /// small pool, a thread of its own runs the loop filters a band at a
    /// time as the rows around it finish, and `emit` (on the calling
    /// thread) takes each finished band before its buffer goes back to
    /// the pool. Rows start in order,
    /// so the pool cannot starve the row a finished band waits on.
    pub fn stream_in_rows(
        &mut self,
        header: SliceHeader,
        data: &[u8],
        starts: &[usize],
        threads: usize,
        emit: &mut dyn FnMut(&DoneBand) -> Result<(), HevcError>,
    ) -> Result<(), HevcError> {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::sync::{Condvar, Mutex};
        let slice_qp = self.pps.init_qp + header.qp_delta;
        let qp_offset = 6 * (self.sps.bit_depth_luma as i32 - 8);
        if !(-qp_offset..=51).contains(&slice_qp) {
            return Err(HevcError::new("a slice QP out of range"));
        }
        let rows = self.height_ctbs;
        if starts.len() != rows
            || starts.windows(2).any(|pair| pair[0] >= pair[1])
            || starts.last().is_some_and(|&last| last >= data.len())
        {
            return Err(HevcError::new("entry points that do not divide the rows"));
        }
        self.slices.push(header.clone());
        let Decoder {
            layout,
            slice_of,
            slice_address_of,
            sao,
            decoded,
            slices,
            ..
        } = self;
        let layout: &Layout = layout;
        let slices: &[SliceHeader] = slices;
        let width_ctbs = layout.width_ctbs;
        let ctb = 1usize << layout.ctb_log2;
        // Each row's records, taken by the worker that decodes it; its
        // units come from a pool as it starts, like its samples.
        let units_high = layout.height.div_ceil(4);
        let unit_pool: Mutex<Vec<Vec<Unit>>> = Mutex::new(Vec::new());
        let shares: Vec<Mutex<Option<RowParts<'_>>>> = slice_of
            .chunks_mut(width_ctbs)
            .zip(slice_address_of.chunks_mut(width_ctbs))
            .zip(sao.chunks_mut(width_ctbs))
            .zip(decoded.chunks_mut(width_ctbs))
            .enumerate()
            .map(|(row, (((slice_of, slice_address_of), sao), decoded))| {
                Mutex::new(Some(RowParts {
                    row,
                    units: Units::Own(Vec::new()),
                    slice_of,
                    slice_address_of,
                    sao,
                    decoded,
                }))
            })
            .collect();
        // Band buffers: a CTB row of each component.
        let band_samples = |component: usize| {
            let rows_here = if component == 0 {
                ctb
            } else {
                ctb >> layout.sub_y
            };
            rows_here * layout.strides[component]
        };
        let depths = [
            layout.sps.bit_depth_luma,
            layout.sps.bit_depth_chroma,
            layout.sps.bit_depth_chroma,
        ];
        let components = layout.strides.len();
        let workers = threads.clamp(1, rows);
        let pool: Mutex<Vec<Vec<Samples>>> = Mutex::new(Vec::new());
        let pool_free = Condvar::new();
        // Enough that the row the loop filters wait on always finds one:
        // the rows past it hold at most `workers - 1`, the filters one,
        // and the band handed on and the one waiting for it two.
        let pool_size = workers + 4;
        let made = AtomicUsize::new(0);
        let next_row = AtomicUsize::new(0);
        let stop = AtomicBool::new(false);
        // Row r tells row r + 1.
        let mut senders: Vec<Mutex<Option<std::sync::mpsc::Sender<RowUpdate>>>> = Vec::new();
        let mut receivers: Vec<Mutex<Option<std::sync::mpsc::Receiver<RowUpdate>>>> =
            vec![Mutex::new(None)];
        for _ in 1..rows {
            let (sender, receiver) = std::sync::mpsc::channel::<RowUpdate>();
            senders.push(Mutex::new(Some(sender)));
            receivers.push(Mutex::new(Some(receiver)));
        }
        senders.push(Mutex::new(None));
        let (done_sender, done_receiver) = std::sync::mpsc::channel::<RowDone<'_>>();
        let header = &header;
        fn lock(mutex: &Mutex<Vec<Vec<Samples>>>) -> std::sync::MutexGuard<'_, Vec<Vec<Samples>>> {
            mutex.lock().unwrap_or_else(|poison| poison.into_inner())
        }
        std::thread::scope(|scope| {
            for _ in 0..workers {
                let done_sender = done_sender.clone();
                let (shares, senders, receivers, unit_pool) =
                    (&shares, &senders, &receivers, &unit_pool);
                let (pool, pool_free, made, next_row, stop) =
                    (&pool, &pool_free, &made, &next_row, &stop);
                scope.spawn(move || {
                    loop {
                        let row = next_row.fetch_add(1, Ordering::Relaxed);
                        if row >= rows {
                            break;
                        }
                        // The row's links, taken first: a row given up
                        // drops its sender, so the row below stops waiting.
                        let receiver = receivers[row]
                            .lock()
                            .unwrap_or_else(|poison| poison.into_inner())
                            .take();
                        let sender = senders[row]
                            .lock()
                            .unwrap_or_else(|poison| poison.into_inner())
                            .take();
                        if stop.load(Ordering::Relaxed) {
                            break;
                        }
                        // A band buffer: a spare one, a new one while the
                        // pool is not full, or one handed back.
                        let mut planes = {
                            let mut free = lock(pool);
                            loop {
                                if let Some(planes) = free.pop() {
                                    break Some(planes);
                                }
                                if made.load(Ordering::Relaxed) < pool_size {
                                    made.fetch_add(1, Ordering::Relaxed);
                                    break Some(
                                        (0..components)
                                            .map(|component| {
                                                Samples::new(
                                                    depths[component],
                                                    band_samples(component),
                                                )
                                            })
                                            .collect(),
                                    );
                                }
                                if stop.load(Ordering::Relaxed) {
                                    break None;
                                }
                                free = pool_free
                                    .wait(free)
                                    .unwrap_or_else(|poison| poison.into_inner());
                            }
                        };
                        let Some(mut planes) = planes.take() else {
                            break;
                        };
                        let Some(mut parts) = shares[row]
                            .lock()
                            .unwrap_or_else(|poison| poison.into_inner())
                            .take()
                        else {
                            break;
                        };
                        let count =
                            (units_high - row * (ctb >> 2)).min(ctb >> 2) * layout.units_wide;
                        let mut units = unit_pool
                            .lock()
                            .unwrap_or_else(|poison| poison.into_inner())
                            .pop()
                            .unwrap_or_default();
                        units.clear();
                        units.resize(count, Unit::default());
                        parts.units = Units::Own(units);
                        let end = starts.get(row + 1).copied().unwrap_or(data.len());
                        let outcome = decode_row(
                            layout,
                            &mut parts,
                            planes.iter_mut().map(Samples::band).collect(),
                            header,
                            &data[..end],
                            starts[row],
                            slice_qp,
                            receiver,
                            sender,
                        );
                        let RowParts {
                            units,
                            slice_of,
                            sao,
                            ..
                        } = parts;
                        let records = RowRecords {
                            unfiltered: units.iter().any(|unit| unit.unfiltered),
                            units,
                            slice_of,
                            sao,
                        };
                        let message = match outcome {
                            Ok(()) => {
                                // The deblocking inside the band reads only
                                // the band: done here, on the row's thread.
                                let mut own = std::collections::BTreeMap::new();
                                own.insert(row, records);
                                super::filter::deblock_inside(
                                    &Window {
                                        layout,
                                        slices,
                                        rows: &own,
                                    },
                                    &mut planes,
                                    row,
                                );
                                let records = own.remove(&row).expect("inserted above");
                                Ok((row, planes, records))
                            }
                            Err(error) => Err((row, error)),
                        };
                        let failed = message.is_err();
                        if done_sender.send(message).is_err() || failed {
                            break;
                        }
                    }
                });
            }
            drop(done_sender);
            // The loop filters run on a thread of their own, a band ahead
            // of this one, which hands each band on.
            let (band_sender, band_receiver) = std::sync::mpsc::sync_channel::<DoneBand>(1);
            let (pool_free, stop, unit_pool) = (&pool_free, &stop, &unit_pool);
            let filtering = scope.spawn(move || {
                let outcome = filter_bands(
                    layout,
                    slices,
                    rows,
                    &done_receiver,
                    &mut |band| {
                        band_sender
                            .send(band)
                            .map_err(|_| HevcError::new("the bands stopped being taken"))
                    },
                    &mut |units| {
                        if let Units::Own(units) = units {
                            unit_pool
                                .lock()
                                .unwrap_or_else(|poison| poison.into_inner())
                                .push(units);
                        }
                    },
                );
                if outcome.is_err() {
                    stop.store(true, Ordering::Relaxed);
                    pool_free.notify_all();
                }
                outcome
            });
            let mut handed = Ok(());
            for band in &band_receiver {
                if let Err(error) = emit(&band) {
                    handed = Err(error);
                    stop.store(true, Ordering::Relaxed);
                    pool_free.notify_all();
                    break;
                }
                lock(&pool).push(band.planes);
                pool_free.notify_all();
            }
            // The filters' thread stops at its next band if this one did.
            drop(band_receiver);
            let filtered = filtering
                .join()
                .unwrap_or_else(|_| Err(HevcError::new("the loop filters failed")));
            handed.and(filtered)
        })
    }
}

/// The loop filters' side of a streamed picture: takes each decoded row,
/// runs the filters on a band once the rows around it are in, and hands
/// it on (its buffers to come back to the pool); `recycle_units` takes
/// each row's units back once no band reads them.
fn filter_bands<'a>(
    layout: &'a Layout,
    slices: &'a [SliceHeader],
    rows: usize,
    done: &std::sync::mpsc::Receiver<RowDone<'a>>,
    hand: &mut dyn FnMut(DoneBand) -> Result<(), HevcError>,
    recycle_units: &mut dyn FnMut(Units<'a>),
) -> Result<(), HevcError> {
    use std::collections::BTreeMap;
    let ctb = 1usize << layout.ctb_log2;
    let mut records: BTreeMap<usize, RowRecords<'a>> = BTreeMap::new();
    let mut bands: BTreeMap<usize, Vec<Samples>> = BTreeMap::new();
    let mut next = 0;
    // The last row of the band handed on, as deblocked, for the next
    // band's offsets.
    let mut above: Option<Vec<Samples>> = None;
    let sao_on = slices
        .iter()
        .any(|slice| slice.sao_luma || slice.sao_chroma);
    while next < rows {
        let ready = |records: &BTreeMap<usize, RowRecords<'a>>| {
            records.contains_key(&next) && (next + 1 == rows || records.contains_key(&(next + 1)))
        };
        if !ready(&records) {
            match done.recv() {
                Ok(Ok((row, planes, row_records))) => {
                    records.insert(row, row_records);
                    bands.insert(row, planes);
                }
                Ok(Err((_, error))) => return Err(error),
                Err(_) => return Err(HevcError::new("a row's decoding stopped")),
            }
            continue;
        }
        let window = Window {
            layout,
            slices,
            rows: &records,
        };
        // Each band comes deblocked within itself; the edge between this
        // band and the next is done here.
        if next + 1 < rows {
            let mut lower = bands.remove(&(next + 1)).unwrap_or_default();
            if let Some(upper) = bands.get_mut(&next) {
                super::filter::deblock_between(&window, upper, &mut lower, next + 1);
            }
            bands.insert(next + 1, lower);
        }
        let band_rows = ctb.min(layout.height - next * ctb);
        let mut planes = bands.remove(&next).unwrap_or_default();
        let last_row = layout_lines(layout, &planes, band_rows, LineOf::Last);
        if sao_on {
            let below = bands
                .get(&(next + 1))
                .map(|lower| layout_lines(layout, lower, ctb, LineOf::First));
            let unfiltered = records.get(&next).is_some_and(|records| records.unfiltered);
            super::filter::sao_band_planes(
                &window,
                &mut planes,
                next,
                above.as_deref(),
                below.as_deref(),
                unfiltered,
            );
        }
        above = Some(last_row);
        let band = DoneBand {
            top: next * ctb,
            rows: band_rows,
            planes,
        };
        hand(band)?;
        // The records of the band above are read no more.
        if let Some(done) = next.checked_sub(1).and_then(|above| records.remove(&above)) {
            recycle_units(done.units);
        }
        next += 1;
    }
    Ok(())
}

/// Which line of a band.
#[derive(Clone, Copy)]
enum LineOf {
    First,
    Last,
}

/// A band's first or last row of each component, copied.
fn layout_lines(
    layout: &Layout,
    planes: &[Samples],
    band_rows: usize,
    which: LineOf,
) -> Vec<Samples> {
    planes
        .iter()
        .enumerate()
        .map(|(component, samples)| {
            let stride = layout.strides[component];
            let rows = if component == 0 {
                band_rows
            } else {
                band_rows >> layout.sub_y
            };
            let row = match which {
                LineOf::First => 0,
                LineOf::Last => rows - 1,
            };
            samples.slice_of(row * stride, stride)
        })
        .collect()
}
/// The state that runs through a slice segment's CTBs.
struct SliceState {
    qp: i32,
    header: SliceHeader,
    /// QpY of the last coding unit decoded.
    last_qp: i32,
    /// The quantization group being decoded and its predicted QpY.
    quant_group: (usize, usize),
    group_qp: i32,
    qp_delta: i32,
    qp_delta_coded: bool,
    chroma_offset: (i32, i32),
    chroma_offset_coded: bool,
    bypass: bool,
}

/// A coding unit's intra modes: luma per prediction block (one, or four
/// for NxN) and chroma per prediction block (one unless 4:4:4 NxN).
struct IntraModes {
    luma: [u32; 4],
    chroma: [u32; 4],
    /// Whether each chroma mode was coded as the luma mode's
    /// (`intra_chroma_pred_mode` 4), as cross-component prediction asks.
    derived: [bool; 4],
}

const CHROMA_422: [u8; 35] = [
    0, 1, 2, 2, 2, 2, 3, 5, 7, 8, 10, 12, 13, 15, 17, 18, 19, 20, 21, 22, 23, 23, 24, 24, 25, 25,
    26, 27, 27, 28, 28, 29, 29, 30, 31,
];

/// QpC of 4:2:0 for qPi from 30 to 43 (Table 8-10).
const CHROMA_QP: [i32; 14] = [29, 30, 31, 32, 33, 33, 34, 34, 35, 35, 36, 36, 37, 37];

fn chroma_qp(qpi: i32, chroma_format: u32) -> i32 {
    if chroma_format != 1 {
        return qpi.min(51);
    }
    match qpi {
        ..30 => qpi,
        30..=43 => CHROMA_QP[(qpi - 30) as usize],
        _ => qpi - 6,
    }
}

impl Rows<'_> {
    /// The first CTB column of the tile column holding `ctb_x`.
    fn tile_column_start(&self, ctb_x: usize) -> usize {
        self.pps
            .tile_columns
            .windows(2)
            .find(|bounds| (ctb_x as u32) >= bounds[0] && (ctb_x as u32) < bounds[1])
            .map_or(0, |bounds| bounds[0] as usize)
    }

    fn coding_tree_unit(
        &mut self,
        cabac: &mut Cabac<'_>,
        contexts: &mut Contexts,
        state: &mut SliceState,
        ctb_x: usize,
        ctb_y: usize,
    ) -> Result<(), HevcError> {
        let x = ctb_x << self.ctb_log2;
        let y = ctb_y << self.ctb_log2;
        if state.header.sao_luma || state.header.sao_chroma {
            self.sao_syntax(cabac, contexts, state, ctb_x, ctb_y);
        }
        self.coding_quadtree(cabac, contexts, state, x, y, self.ctb_log2, 0)
    }

    fn sao_syntax(
        &mut self,
        cabac: &mut Cabac<'_>,
        contexts: &mut Contexts,
        state: &SliceState,
        ctb_x: usize,
        ctb_y: usize,
    ) {
        let rs = ctb_y * self.width_ctbs + ctb_x;
        let same = |other: usize| {
            self.slice_address(other) == self.slice_address(rs)
                && self.tile_of[other] == self.tile_of[rs]
        };
        let mut merge_left = false;
        if ctb_x > 0 && same(rs - 1) {
            merge_left = cabac.decision(&mut contexts.contexts[cabac::SAO_MERGE]) == 1;
        }
        let mut merge_up = false;
        if !merge_left && ctb_y > 0 && same(rs - self.width_ctbs) {
            merge_up = cabac.decision(&mut contexts.contexts[cabac::SAO_MERGE]) == 1;
        }
        if merge_left || merge_up {
            let source = if merge_left {
                rs - 1
            } else {
                rs - self.width_ctbs
            };
            let mut merged = self.sao_of(source);
            // A merged component off for this slice stays off.
            if !state.header.sao_luma {
                merged[0] = Sao::default();
            }
            if !state.header.sao_chroma {
                merged[1] = Sao::default();
                merged[2] = Sao::default();
            }
            *self.sao_mut(rs) = merged;
            return;
        }
        let components = if self.chroma == 0 { 1 } else { 3 };
        let mut params = [Sao::default(); 3];
        for component in 0..components {
            let enabled = if component == 0 {
                state.header.sao_luma
            } else {
                state.header.sao_chroma
            };
            if !enabled {
                continue;
            }
            let bit_depth = if component == 0 {
                self.sps.bit_depth_luma
            } else {
                self.sps.bit_depth_chroma
            };
            if component == 2 {
                params[2].kind = params[1].kind;
                params[2].class = params[1].class;
            } else {
                params[component].kind =
                    if cabac.decision(&mut contexts.contexts[cabac::SAO_TYPE]) == 0 {
                        0
                    } else if cabac.bypass() == 0 {
                        1
                    } else {
                        2
                    };
            }
            let kind = params[component].kind;
            if kind == 0 {
                continue;
            }
            let largest = (1 << (bit_depth.min(10) - 5)) - 1;
            let mut magnitudes = [0i32; 4];
            for magnitude in &mut magnitudes {
                let mut value = 0;
                while value < largest && cabac.bypass() == 1 {
                    value += 1;
                }
                *magnitude = value;
            }
            let scale = if component == 0 {
                self.pps.log2_sao_offset_scale_luma
            } else {
                self.pps.log2_sao_offset_scale_chroma
            };
            if kind == 1 {
                for magnitude in &mut magnitudes {
                    if *magnitude != 0 && cabac.bypass() == 1 {
                        *magnitude = -*magnitude;
                    }
                }
                params[component].band = cabac.bypass_bits(5) as u8;
                for (index, magnitude) in magnitudes.iter().enumerate() {
                    params[component].offsets[index + 1] = magnitude << scale;
                }
            } else {
                for (index, magnitude) in magnitudes.iter().enumerate() {
                    let signed = if index < 2 { *magnitude } else { -*magnitude };
                    params[component].offsets[index + 1] = signed << scale;
                }
                if component == 0 {
                    params[0].class = cabac.bypass_bits(2) as u8;
                } else if component == 1 {
                    params[1].class = cabac.bypass_bits(2) as u8;
                }
            }
        }
        *self.sao_mut(rs) = params;
    }

    #[allow(clippy::too_many_arguments)]
    fn coding_quadtree(
        &mut self,
        cabac: &mut Cabac<'_>,
        contexts: &mut Contexts,
        state: &mut SliceState,
        x: usize,
        y: usize,
        log2: u32,
        depth: u32,
    ) -> Result<(), HevcError> {
        let size = 1usize << log2;
        let split =
            if x + size <= self.width && y + size <= self.height && log2 > self.sps.log2_min_cb {
                let mut increment = 0;
                if self.available(x, y, x as isize - 1, y as isize)
                    && u32::from(self.unit(x - 1, y).depth) > depth
                {
                    increment += 1;
                }
                if self.available(x, y, x as isize, y as isize - 1)
                    && u32::from(self.unit(x, y - 1).depth) > depth
                {
                    increment += 1;
                }
                cabac.decision(&mut contexts.contexts[cabac::SPLIT_CU + increment]) == 1
            } else {
                log2 > self.sps.log2_min_cb
            };
        if self.pps.cu_qp_delta && log2 >= self.sps.log2_ctb - self.pps.diff_cu_qp_delta_depth {
            state.qp_delta_coded = false;
            state.qp_delta = 0;
        }
        if state.header.cu_chroma_qp_offset
            && log2 >= self.sps.log2_ctb - self.pps.diff_cu_chroma_qp_offset_depth
        {
            state.chroma_offset_coded = false;
        }
        if split {
            let half = size / 2;
            for (dx, dy) in [(0, 0), (half, 0), (0, half), (half, half)] {
                if x + dx < self.width && y + dy < self.height {
                    self.coding_quadtree(
                        cabac,
                        contexts,
                        state,
                        x + dx,
                        y + dy,
                        log2 - 1,
                        depth + 1,
                    )?;
                }
            }
            return Ok(());
        }
        self.coding_unit(cabac, contexts, state, x, y, log2, depth)
    }

    /// Enters the quantization group of the coding unit at (x, y): at a new
    /// group, QpY's prediction from the group's left and upper neighbours
    /// in the same CTB, else the last QpY decoded (8.6.1).
    fn enter_quant_group(&self, state: &mut SliceState, x: usize, y: usize) {
        let size_log2 = self.sps.log2_ctb - self.pps.diff_cu_qp_delta_depth;
        let mask = !((1usize << size_log2) - 1);
        let group = (x & mask, y & mask);
        if group == state.quant_group {
            return;
        }
        state.quant_group = group;
        let (gx, gy) = group;
        let ctb = self.ctb_address(gx, gy);
        let neighbour = |nx: isize, ny: isize| -> i32 {
            if self.available(x, y, nx, ny) && self.ctb_address(nx as usize, ny as usize) == ctb {
                i32::from(self.unit(nx as usize, ny as usize).qp)
            } else {
                state.last_qp
            }
        };
        let left = neighbour(gx as isize - 1, gy as isize);
        let above = neighbour(gx as isize, gy as isize - 1);
        state.group_qp = (left + above + 1) >> 1;
    }

    fn update_qp(&self, state: &mut SliceState) {
        let offset = 6 * (self.sps.bit_depth_luma as i32 - 8);
        state.qp =
            (state.group_qp + state.qp_delta + 52 + 2 * offset).rem_euclid(52 + offset) - offset;
    }

    #[allow(clippy::too_many_arguments)]
    fn coding_unit(
        &mut self,
        cabac: &mut Cabac<'_>,
        contexts: &mut Contexts,
        state: &mut SliceState,
        x: usize,
        y: usize,
        log2: u32,
        depth: u32,
    ) -> Result<(), HevcError> {
        let size = 1usize << log2;
        state.bypass = self.pps.transquant_bypass
            && cabac.decision(&mut contexts.contexts[cabac::TRANSQUANT_BYPASS]) == 1;
        // A quantization group's prediction holds for each of its CUs.
        self.enter_quant_group(state, x, y);
        self.update_qp(state);
        let split_intra = log2 == self.sps.log2_min_cb
            && cabac.decision(&mut contexts.contexts[cabac::PART_MODE]) == 0;
        let mut pcm = false;
        if !split_intra
            && let Some(params) = self.sps.pcm
            && log2 >= params.log2_min
            && log2 <= params.log2_max
        {
            pcm = cabac.terminate() == 1;
        }
        // Mark the CU's units: its depth, now, for neighbours' contexts.
        let width = size.min(self.width - x);
        let height = size.min(self.height - y);
        for uy in (y..y + height).step_by(4) {
            for ux in (x..x + width).step_by(4) {
                let unit = self.unit_mut(ux, uy);
                *unit = Unit {
                    depth: depth as u8,
                    mode: 1,
                    qp: state.qp as i8,
                    unfiltered: state.bypass,
                    pcm,
                    left_edge: ux == x,
                    top_edge: uy == y,
                };
            }
        }
        if pcm {
            self.pcm_samples(cabac, x, y, log2)?;
            if self
                .sps
                .pcm
                .is_some_and(|params| params.loop_filter_disabled)
            {
                for uy in (y..y + height).step_by(4) {
                    for ux in (x..x + width).step_by(4) {
                        self.unit_mut(ux, uy).unfiltered = true;
                    }
                }
            }
            state.last_qp = state.qp;
            return Ok(());
        }
        let parts = if split_intra { 4 } else { 1 };
        let half = size / 2;
        let mut previous = [false; 4];
        for flag in previous.iter_mut().take(parts) {
            *flag = cabac.decision(&mut contexts.contexts[cabac::PREV_INTRA_LUMA]) == 1;
        }
        let mut modes = IntraModes {
            luma: [1; 4],
            chroma: [1; 4],
            derived: [false; 4],
        };
        for (part, &predicted) in previous.iter().enumerate().take(parts) {
            let (px, py) = if split_intra {
                (x + (part & 1) * half, y + (part >> 1) * half)
            } else {
                (x, y)
            };
            let candidates = self.most_probable_modes(px, py);
            let mode = if predicted {
                let index = if cabac.bypass() == 0 {
                    0
                } else if cabac.bypass() == 0 {
                    1
                } else {
                    2
                };
                candidates[index]
            } else {
                let mut sorted = candidates;
                sorted.sort_unstable();
                let mut mode = cabac.bypass_bits(5);
                for candidate in sorted {
                    if mode >= candidate {
                        mode += 1;
                    }
                }
                mode
            };
            modes.luma[part] = mode;
            let part_size = if split_intra { half } else { size };
            for uy in (py..(py + part_size).min(self.height)).step_by(4) {
                for ux in (px..(px + part_size).min(self.width)).step_by(4) {
                    self.unit_mut(ux, uy).mode = mode as u8;
                }
            }
        }
        if self.chroma != 0 {
            let chroma_parts = if self.chroma == 3 { parts } else { 1 };
            for part in 0..chroma_parts {
                let coded = if cabac.decision(&mut contexts.contexts[cabac::INTRA_CHROMA]) == 0 {
                    4
                } else {
                    cabac.bypass_bits(2)
                };
                let luma = modes.luma[part];
                let mut mode = match coded {
                    4 => luma,
                    _ => {
                        let candidate = [0, 26, 10, 1][coded as usize];
                        if candidate == luma { 34 } else { candidate }
                    }
                };
                if self.chroma == 2 {
                    mode = u32::from(CHROMA_422[mode as usize]);
                }
                modes.chroma[part] = mode;
                modes.derived[part] = coded == 4;
            }
            if chroma_parts == 1 {
                modes.chroma = [modes.chroma[0]; 4];
                modes.derived = [modes.derived[0]; 4];
            }
        }
        let max_depth = self.sps.max_transform_depth_intra + u32::from(split_intra);
        let mut cbf_parent = [true, true, true, true];
        self.transform_tree(
            cabac,
            contexts,
            state,
            TreeNode {
                x,
                y,
                base_x: x,
                base_y: y,
                cu_x: x,
                cu_y: y,
                log2,
                depth: 0,
                block: 0,
            },
            max_depth,
            split_intra,
            &modes,
            &mut cbf_parent,
        )?;
        // The CU's final QpY, for its neighbours and the deblocking filter.
        for uy in (y..y + height).step_by(4) {
            for ux in (x..x + width).step_by(4) {
                self.unit_mut(ux, uy).qp = state.qp as i8;
            }
        }
        state.last_qp = state.qp;
        Ok(())
    }

    /// The three most probable luma modes of a prediction block (8.4.2).
    fn most_probable_modes(&self, x: usize, y: usize) -> [u32; 3] {
        let neighbour = |nx: isize, ny: isize, above: bool| -> u32 {
            if !self.available(x, y, nx, ny) {
                return 1;
            }
            let unit = self.unit(nx as usize, ny as usize);
            if unit.pcm {
                return 1;
            }
            // The row above outside this CTB counts as DC.
            if above && (ny as usize) < ((y >> self.ctb_log2) << self.ctb_log2) {
                return 1;
            }
            u32::from(unit.mode)
        };
        let a = neighbour(x as isize - 1, y as isize, false);
        let b = neighbour(x as isize, y as isize - 1, true);
        if a == b {
            if a < 2 {
                [0, 1, 26]
            } else {
                [a, 2 + ((a + 29) % 32), 2 + ((a - 2 + 1) % 32)]
            }
        } else {
            let third = if a != 0 && b != 0 {
                0
            } else if a != 1 && b != 1 {
                1
            } else {
                26
            };
            [a, b, third]
        }
    }

    fn pcm_samples(
        &mut self,
        cabac: &mut Cabac<'_>,
        x: usize,
        y: usize,
        log2: u32,
    ) -> Result<(), HevcError> {
        let params = self
            .sps
            .pcm
            .ok_or_else(|| HevcError::new("PCM without its parameters"))?;
        let data = cabac.data();
        let mut reader = super::bits::BitReader::new(&data[cabac.byte_position()..]);
        let size = 1usize << log2;
        let (bit_depth_luma, bit_depth_chroma) = (
            self.layout.sps.bit_depth_luma,
            self.layout.sps.bit_depth_chroma,
        );
        let (chroma, sub_x, sub_y) = (self.layout.chroma, self.layout.sub_x, self.layout.sub_y);
        let layout = self.layout;
        let tops = [self.top_of(0), self.top_of(1), self.top_of(2)];
        let mut write_block = |reader: &mut super::bits::BitReader<'_>,
                               plane: usize,
                               px: usize,
                               py: usize,
                               w: usize,
                               h: usize,
                               pcm_depth: u32,
                               depth: u32|
         -> Result<(), HevcError> {
            let stride = layout.strides[plane];
            let top = tops[plane];
            with_band!(&mut self.planes[plane], samples => {
                for row in 0..h {
                    for column in 0..w {
                        let value = reader.bits(pcm_depth)? << (depth - pcm_depth);
                        samples[(py + row - top) * stride + px + column] = Sample::of(value as i32);
                    }
                }
            });
            Ok(())
        };

        write_block(
            &mut reader,
            0,
            x,
            y,
            size,
            size,
            params.bit_depth_luma,
            bit_depth_luma,
        )?;
        if chroma != 0 {
            let (w, h) = (size >> sub_x, size >> sub_y);
            let (cx, cy) = (x >> sub_x, y >> sub_y);
            for plane in 1..3 {
                write_block(
                    &mut reader,
                    plane,
                    cx,
                    cy,
                    w,
                    h,
                    params.bit_depth_chroma,
                    bit_depth_chroma,
                )?;
            }
        }
        let next = cabac.byte_position() + reader.byte_position();
        cabac.restart_at(next);
        Ok(())
    }
}

/// A node of the transform tree.
#[derive(Clone, Copy)]
struct TreeNode {
    x: usize,
    y: usize,
    /// The parent node's position (where 4x4 luma blocks' chroma lies).
    base_x: usize,
    base_y: usize,
    cu_x: usize,
    cu_y: usize,
    log2: u32,
    depth: u32,
    block: usize,
}

impl Rows<'_> {
    #[allow(clippy::too_many_arguments)]
    fn transform_tree(
        &mut self,
        cabac: &mut Cabac<'_>,
        contexts: &mut Contexts,
        state: &mut SliceState,
        node: TreeNode,
        max_depth: u32,
        split_intra: bool,
        modes: &IntraModes,
        parent_cbf: &mut [bool; 4],
    ) -> Result<(), HevcError> {
        let log2 = node.log2;
        let split = if log2 <= self.sps.log2_max_tb
            && log2 > self.sps.log2_min_tb
            && node.depth < max_depth
            && !(split_intra && node.depth == 0)
        {
            cabac.decision(&mut contexts.contexts[cabac::SPLIT_TRANSFORM + (5 - log2 as usize)])
                == 1
        } else {
            log2 > self.sps.log2_max_tb || (split_intra && node.depth == 0)
        };
        // cbf_cb and cbf_cr, each with a second for 4:2:2's lower block.
        let mut cbf = [false; 4];
        if (log2 > 2 && self.chroma != 0) || self.chroma == 3 {
            for component in 0..2 {
                if node.depth == 0 || parent_cbf[component * 2] || parent_cbf[component * 2 + 1] {
                    let context = cabac::CBF_CHROMA + node.depth as usize;
                    cbf[component * 2] = cabac.decision(&mut contexts.contexts[context]) == 1;
                    if self.chroma == 2 && (!split || log2 == 3) {
                        cbf[component * 2 + 1] =
                            cabac.decision(&mut contexts.contexts[context]) == 1;
                    }
                }
            }
        } else if log2 == 2 && self.chroma != 3 && self.chroma != 0 {
            // 4x4 luma blocks: chroma lies with the parent.
            cbf = *parent_cbf;
        }
        if split {
            let half = 1usize << (log2 - 1);
            for (index, (dx, dy)) in [(0, 0), (half, 0), (0, half), (half, half)]
                .into_iter()
                .enumerate()
            {
                let mut child_cbf = cbf;
                self.transform_tree(
                    cabac,
                    contexts,
                    state,
                    TreeNode {
                        x: node.x + dx,
                        y: node.y + dy,
                        base_x: node.x,
                        base_y: node.y,
                        cu_x: node.cu_x,
                        cu_y: node.cu_y,
                        log2: log2 - 1,
                        depth: node.depth + 1,
                        block: index,
                    },
                    max_depth,
                    split_intra,
                    modes,
                    &mut child_cbf,
                )?;
            }
            return Ok(());
        }
        let cbf_luma = cabac
            .decision(&mut contexts.contexts[cabac::CBF_LUMA + usize::from(node.depth == 0)])
            == 1;
        self.transform_unit(cabac, contexts, state, node, modes, cbf_luma, cbf)
    }

    #[allow(clippy::too_many_arguments)]
    fn transform_unit(
        &mut self,
        cabac: &mut Cabac<'_>,
        contexts: &mut Contexts,
        state: &mut SliceState,
        node: TreeNode,
        modes: &IntraModes,
        cbf_luma: bool,
        cbf: [bool; 4],
    ) -> Result<(), HevcError> {
        let log2 = node.log2;
        let chroma_here = self.chroma == 3 || (log2 > 2 && self.chroma != 0);
        let chroma_at_parent = self.chroma != 0 && self.chroma != 3 && log2 == 2;
        let cbf_chroma = (chroma_here || chroma_at_parent) && cbf.iter().any(|&flag| flag);
        if cbf_luma || cbf_chroma {
            if self.pps.cu_qp_delta && !state.qp_delta_coded {
                let mut magnitude = 0;
                while magnitude < 5
                    && cabac.decision(
                        &mut contexts.contexts[cabac::CU_QP_DELTA + usize::from(magnitude > 0)],
                    ) == 1
                {
                    magnitude += 1;
                }
                if magnitude == 5 {
                    let mut k = 0;
                    while cabac.bypass() == 1 {
                        magnitude += 1 << k;
                        k += 1;
                        if k > 30 {
                            return Err(HevcError::new("a QP delta out of range"));
                        }
                    }
                    magnitude += cabac.bypass_bits(k) as i32;
                }
                let delta = if magnitude > 0 && cabac.bypass() == 1 {
                    -magnitude
                } else {
                    magnitude
                };
                state.qp_delta_coded = true;
                state.qp_delta = delta;
                self.update_qp(state);
            }
            if state.header.cu_chroma_qp_offset
                && cbf_chroma
                && !state.bypass
                && !state.chroma_offset_coded
            {
                let flag =
                    cabac.decision(&mut contexts.contexts[cabac::CHROMA_QP_OFFSET_FLAG]) == 1;
                state.chroma_offset = (0, 0);
                if flag {
                    let count = self.pps.cb_qp_offset_list.len();
                    let mut index = 0;
                    while index + 1 < count
                        && cabac.decision(&mut contexts.contexts[cabac::CHROMA_QP_OFFSET_IDX]) == 1
                    {
                        index += 1;
                    }
                    state.chroma_offset = (
                        self.pps.cb_qp_offset_list.get(index).copied().unwrap_or(0),
                        self.pps.cr_qp_offset_list.get(index).copied().unwrap_or(0),
                    );
                }
                state.chroma_offset_coded = true;
            }
        }
        // Luma: predict, then add the residual.
        let luma_mode = u32::from(self.unit(node.x, node.y).mode);
        self.predict_block(0, node.x, node.y, log2, luma_mode)?;
        if cbf_luma {
            self.residual_block(
                cabac, contexts, state, 0, node.x, node.y, log2, luma_mode, true, 0,
            )?;
        }
        // The chroma of 4:4:4 takes the prediction block's own mode.
        let chroma_mode = |decoder: &Rows<'_>, x: usize, y: usize| -> u32 {
            if decoder.chroma == 3 {
                let index = decoder.partition_index(node, x, y);
                modes.chroma[index]
            } else {
                modes.chroma[0]
            }
        };
        if chroma_here {
            let log2_c = if self.chroma == 3 { log2 } else { log2 - 1 };
            let (cx, cy) = (node.x >> self.sub_x, node.y >> self.sub_y);
            let mode = chroma_mode(self, node.x, node.y);
            // Cross-component prediction (range extension): 4:4:4 chroma
            // residuals take a scaled share of the luma residual.
            let cross = self.chroma == 3
                && self.pps.cross_component_prediction
                && cbf_luma
                && modes.derived[self.partition_index(node, node.x, node.y)];
            self.chroma_blocks(cabac, contexts, state, cx, cy, log2_c, mode, &cbf, cross)?;
        } else if chroma_at_parent && node.block == 3 {
            let (cx, cy) = (node.base_x >> self.sub_x, node.base_y >> self.sub_y);
            let mode = modes.chroma[0];
            self.chroma_blocks(cabac, contexts, state, cx, cy, 2, mode, &cbf, false)?;
        }
        // Transform block edges, for the deblocking filter.
        let size = 1usize << log2;
        for uy in (node.y..(node.y + size).min(self.height)).step_by(4) {
            for ux in (node.x..(node.x + size).min(self.width)).step_by(4) {
                let unit = self.unit_mut(ux, uy);
                if ux == node.x {
                    unit.left_edge = true;
                }
                if uy == node.y {
                    unit.top_edge = true;
                }
            }
        }
        Ok(())
    }

    /// Which of a 4:4:4 NxN coding unit's four prediction blocks holds (x, y).
    fn partition_index(&self, node: TreeNode, x: usize, y: usize) -> usize {
        let cb_size = 1usize << self.sps.log2_min_cb;
        let half = cb_size / 2;
        if (x - node.cu_x) >= cb_size || (y - node.cu_y) >= cb_size {
            return 0;
        }
        usize::from(x - node.cu_x >= half) + 2 * usize::from(y - node.cu_y >= half)
    }

    /// The Cb and Cr blocks of a transform unit: in 4:2:2 two of each, the
    /// lower predicted from the upper's reconstruction.
    #[allow(clippy::too_many_arguments)]
    fn chroma_blocks(
        &mut self,
        cabac: &mut Cabac<'_>,
        contexts: &mut Contexts,
        state: &mut SliceState,
        cx: usize,
        cy: usize,
        log2_c: u32,
        mode: u32,
        cbf: &[bool; 4],
        cross: bool,
    ) -> Result<(), HevcError> {
        let blocks = if self.chroma == 2 { 2 } else { 1 };
        for component in 1..3 {
            // `cross_comp_pred()` (7.3.8.12): the scale, a power of two
            // from 1 to 8, in eighths of the luma residual.
            let mut scale = 0;
            if cross {
                let base = cabac::LOG2_RES_SCALE + 4 * (component - 1);
                let mut magnitude = 0;
                while magnitude < 4 && cabac.decision(&mut contexts.contexts[base + magnitude]) == 1
                {
                    magnitude += 1;
                }
                if magnitude > 0 {
                    let negative = cabac
                        .decision(&mut contexts.contexts[cabac::RES_SCALE_SIGN + component - 1])
                        == 1;
                    scale = (1 << (magnitude - 1)) * if negative { -1 } else { 1 };
                }
            }
            for index in 0..blocks {
                let by = cy + (index << log2_c);
                self.predict_block(component, cx, by, log2_c, mode)?;
                let coded = cbf[(component - 1) * 2 + index];
                if coded || scale != 0 {
                    self.residual_block(
                        cabac, contexts, state, component, cx, by, log2_c, mode, coded, scale,
                    )?;
                }
            }
        }
        Ok(())
    }

    /// Intra prediction of one block of `component` at (x, y) in that
    /// component's samples.
    fn predict_block(
        &mut self,
        component: usize,
        x: usize,
        y: usize,
        log2: u32,
        mode: u32,
    ) -> Result<(), HevcError> {
        let n = 1usize << log2;
        let (shift_x, shift_y) = if component == 0 {
            (0, 0)
        } else {
            (self.sub_x, self.sub_y)
        };
        let bit_depth = if component == 0 {
            self.sps.bit_depth_luma
        } else {
            self.sps.bit_depth_chroma
        };
        let stride = self.strides[component];
        let plane_width = self.width >> shift_x;
        let plane_height = self.height >> shift_y;
        let (luma_x, luma_y) = (x << shift_x, y << shift_y);
        // Availability holds for a whole 4x4 luma unit: one check a unit,
        // its samples copied as a run (8.4.4.2.2's order: the left column
        // bottom up, the corner, the top row).
        let unit_x = 4 >> shift_x;
        let unit_y = 4 >> shift_y;
        let mut line = [0i32; intra::LINE];
        let mut known = [false; intra::LINE];
        let available = |sx: usize, sy: usize| -> bool {
            sx < plane_width
                && sy < plane_height
                && self.available(
                    luma_x,
                    luma_y,
                    (sx << shift_x) as isize,
                    (sy << shift_y) as isize,
                )
        };
        let gather = Gather {
            x,
            y,
            n,
            stride,
            unit_x,
            unit_y,
            top: self.top_of(component),
        };
        let above = self.above.as_ref().map(|edge| &edge.lines[component]);
        with_band!(&self.planes[component], samples => {
            gather.run(&samples[..], above_line(above), &available, &mut line, &mut known)
        });
        let mut references = References::substitute(n, bit_depth, &mut line, &known);
        let filter_allowed = component == 0 || self.chroma == 3;
        if filter_allowed && !self.sps.intra_smoothing_disabled && intra::filters(mode, n) {
            let strong = component == 0 && self.sps.strong_intra_smoothing;
            references.filter(n, strong, bit_depth);
        }
        let mut predicted = [0i32; 32 * 32];
        let predicted = &mut predicted[..n * n];
        let edge_filters = component == 0 && n < 32;
        intra::predict(&references, mode, n, edge_filters, bit_depth, predicted);
        // Coded blocks lie inside the picture: its size is whole minimum
        // coding blocks, and no block crosses one.
        let top = self.top_of(component);
        with_band!(&mut self.planes[component], samples => {
            for (row, values) in predicted.chunks_exact(n).enumerate() {
                let start = (y + row - top) * stride + x;
                for (sample, &value) in samples[start..start + n].iter_mut().zip(values) {
                    *sample = Sample::of(value);
                }
            }
        });
        Ok(())
    }

    /// Parses a block's coefficients and adds its residual (7.3.8.11,
    /// 8.6.2).
    #[allow(clippy::too_many_arguments)]
    fn residual_block(
        &mut self,
        cabac: &mut Cabac<'_>,
        contexts: &mut Contexts,
        state: &mut SliceState,
        component: usize,
        x: usize,
        y: usize,
        log2: u32,
        mode: u32,
        coded: bool,
        cross_scale: i32,
    ) -> Result<(), HevcError> {
        let n = 1usize << log2;
        let mut buffer = std::mem::take(&mut self.levels);
        buffer.resize(32 * 32, 0);
        let levels = &mut buffer[..n * n];
        levels.fill(0);
        let bit_depth = if component == 0 {
            self.sps.bit_depth_luma
        } else {
            self.sps.bit_depth_chroma
        };
        // Scaling (8.6.3), applied as the levels are read.
        let dequant = (coded && !state.bypass).then(|| {
            let qp = if component == 0 {
                state.qp + 6 * (self.sps.bit_depth_luma as i32 - 8)
            } else {
                let offset_c = 6 * (self.sps.bit_depth_chroma as i32 - 8);
                let (pps_offset, slice_offset, cu_offset) = if component == 1 {
                    (
                        self.pps.cb_qp_offset,
                        state.header.cb_qp_offset,
                        state.chroma_offset.0,
                    )
                } else {
                    (
                        self.pps.cr_qp_offset,
                        state.header.cr_qp_offset,
                        state.chroma_offset.1,
                    )
                };
                let qpi = (state.qp + pps_offset + slice_offset + cu_offset).clamp(-offset_c, 57);
                chroma_qp(qpi, self.chroma) + offset_c
            };
            super::residual::Dequant {
                scale: i64::from(LEVEL_SCALE[(qp % 6) as usize] << (qp / 6)),
                shift: bit_depth + log2 - 5,
                matrix: self
                    .scaling
                    .as_ref()
                    .map(|factors| &factors.factors[log2 as usize - 2][component][..]),
            }
        });
        let extent = if coded {
            super::residual::residual_coding(
                self.layout,
                cabac,
                contexts,
                state.bypass,
                component,
                log2,
                mode,
                levels,
                dequant,
            )?
        } else {
            super::residual::Coded {
                transform_skip: false,
                rows: 0,
                columns: 0,
            }
        };
        let transform_skip = extent.transform_skip;
        if coded && !state.bypass {
            if transform_skip {
                let rotate = self.sps.transform_skip_rotation && n == 4;
                transform::transform_skip(levels, log2, bit_depth, rotate);
            } else {
                let dst = component == 0 && n == 4;
                transform::inverse_transform(
                    levels,
                    log2,
                    dst,
                    bit_depth,
                    extent.rows,
                    extent.columns,
                );
            }
        }
        // Implicit residual DPCM of transform skip and bypass blocks
        // predicted straight across or down (range extension).
        if self.sps.implicit_rdpcm && (transform_skip || state.bypass) && (mode == 10 || mode == 26)
        {
            if mode == 10 {
                for row in 0..n {
                    for column in 1..n {
                        levels[row * n + column] += levels[row * n + column - 1];
                    }
                }
            } else {
                for row in 1..n {
                    for column in 0..n {
                        levels[row * n + column] += levels[(row - 1) * n + column];
                    }
                }
            }
        }
        if component == 0 {
            if self.pps.cross_component_prediction {
                self.luma_residual.clear();
                self.luma_residual.extend_from_slice(levels);
            }
        } else if cross_scale != 0 && self.luma_residual.len() == levels.len() {
            // 7.3.8.12 and 8.6.6: chroma plus the scaled luma residual.
            let (luma_depth, chroma_depth) = (self.sps.bit_depth_luma, self.sps.bit_depth_chroma);
            for (level, &luma) in levels.iter_mut().zip(&self.luma_residual) {
                *level += (cross_scale * ((luma << chroma_depth) >> luma_depth)) >> 3;
            }
        }
        let stride = self.strides[component];
        let max = (1i32 << bit_depth) - 1;
        let top = self.top_of(component);
        with_band!(&mut self.planes[component], samples => {
            for (row, residuals) in levels.chunks_exact(n).enumerate() {
                let start = (y + row - top) * stride + x;
                for (sample, &residual) in samples[start..start + n].iter_mut().zip(residuals) {
                    *sample = Sample::of((sample.value() + residual).clamp(0, max));
                }
            }
        });
        self.levels = buffer;
        Ok(())
    }
}

impl Decoder {
    /// Whether every CTB was decoded: a picture whose data ends early is
    /// not filtered or handed on.
    pub fn complete(&self) -> bool {
        self.decoded.iter().all(|&done| done)
    }

    /// The picture, filtered, with its crop; the block buffers go back to
    /// `workspace` for the next picture.
    pub fn finish_into(mut self, workspace: &mut Workspace) -> Picture {
        let picture = self.filtered();
        workspace.units = std::mem::take(&mut self.units);
        picture
    }

    /// Runs the loop filters and takes the planes out as the picture.
    fn filtered(&mut self) -> Picture {
        super::filter::deblock(self);
        super::filter::sample_adaptive_offset(self);
        let sizes = (0..self.planes.len())
            .map(|plane| {
                if plane == 0 {
                    (self.width, self.height)
                } else {
                    (self.width >> self.sub_x, self.height >> self.sub_y)
                }
            })
            .collect();
        Picture {
            width: self.width,
            height: self.height,
            chroma_format: self.chroma,
            bit_depth_luma: self.sps.bit_depth_luma,
            bit_depth_chroma: self.sps.bit_depth_chroma,
            planes: std::mem::take(&mut self.planes),
            sizes,
            crop: self.sps.crop,
            vui_colour: self.sps.vui_colour,
        }
    }
}

/// The large buffers a decoder keeps between pictures of a size (a grid's
/// tiles), so decoding many pictures allocates them once: the planes
/// (handed back with `recycle`) and the per-block tables.
#[derive(Default)]
pub struct Workspace {
    planes: Vec<Samples>,
    units: Vec<Unit>,
}

impl Workspace {
    /// Takes a picture's planes back for the next picture.
    pub fn recycle(&mut self, planes: Vec<Samples>) {
        self.planes = planes;
    }
}

/// Where a block's reference samples are, in its component's samples.
struct Gather {
    x: usize,
    y: usize,
    n: usize,
    stride: usize,
    /// A 4x4 luma unit's size in this component's samples.
    unit_x: usize,
    unit_y: usize,
    /// The band's first row in this component's samples: rows above it
    /// come from the line above.
    top: usize,
}

impl Gather {
    /// Fills `line` and `known` in 8.4.4.2.2's order (the left column
    /// bottom up, the corner, the top row): availability once a unit,
    /// its samples copied as a run.
    fn run<T: Sample>(
        &self,
        band: &[T],
        above: &[T],
        available: &dyn Fn(usize, usize) -> bool,
        line: &mut [i32; intra::LINE],
        known: &mut [bool; intra::LINE],
    ) {
        let Gather {
            x,
            y,
            n,
            stride,
            unit_x,
            unit_y,
            top,
        } = *self;
        // Row `y` of the plane: the band's, or the line above it.
        let row_of = |y: usize| -> &[T] {
            if y >= top {
                &band[(y - top) * stride..(y - top + 1) * stride]
            } else {
                above
            }
        };
        if x > 0 {
            let column = x - 1;
            let mut index = 0;
            while index < 2 * n {
                let sy = y + index;
                let run = unit_y.min(2 * n - index);
                if available(column, sy) {
                    for k in 0..run {
                        let at = 2 * n - 1 - (index + k);
                        line[at] = row_of(sy + k)[column].value();
                        known[at] = true;
                    }
                }
                index += run;
            }
            if y > 0 && available(column, y - 1) {
                line[2 * n] = row_of(y - 1)[column].value();
                known[2 * n] = true;
            }
        }
        if y > 0 {
            let row = row_of(y - 1);
            let mut index = 0;
            while index < 2 * n {
                let sx = x + index;
                let run = unit_x.min(2 * n - index);
                if available(sx, y - 1) {
                    let at = 2 * n + 1 + index;
                    for (value, &sample) in line[at..at + run].iter_mut().zip(&row[sx..sx + run]) {
                        *value = sample.value();
                    }
                    known[at..at + run].fill(true);
                }
                index += run;
            }
        }
    }
}
