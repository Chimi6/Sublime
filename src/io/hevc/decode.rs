//! Decoding an intra picture (H.265 7.3.8, 8.4, 8.6): slice segments into
//! coding tree units, each parsed and reconstructed as it is read, then the
//! in-loop filters (`filter.rs`).

use super::HevcError;
use super::cabac::{self, Cabac, Contexts};
use super::intra::{self, References, Side};
use super::params::{Pps, ScalingFactors, SliceHeader, Sps};
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
    pub planes: Vec<Vec<u16>>,
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

/// The state of a picture being decoded.
pub struct Decoder {
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
    pub planes: Vec<Vec<u16>>,
    pub strides: Vec<usize>,
    pub units: Vec<Unit>,
    pub units_wide: usize,
    /// Z-scan order of each 4x4 unit (6.5.2), for availability.
    zscan: Vec<u32>,
    pub rs_to_ts: Vec<usize>,
    pub ts_to_rs: Vec<usize>,
    pub tile_of: Vec<usize>,
    /// Per CTB (raster): the index of its slice segment header, and the
    /// raster address of its slice's first CTB.
    pub slice_of: Vec<usize>,
    slice_address_of: Vec<usize>,
    pub slices: Vec<SliceHeader>,
    pub sao: Vec<[Sao; 3]>,
    scaling: Option<ScalingFactors>,
    decoded: Vec<bool>,
    /// Context state saved after a row's second CTB, for the row below
    /// (wavefront parallel processing).
    saved_row_contexts: Option<Contexts>,
    /// Context state and last QpY at the end of a slice segment, for a
    /// dependent segment after it.
    saved_slice_contexts: Option<(Contexts, i32)>,
    /// The last luma block's residual, for cross-component prediction.
    luma_residual: Vec<i32>,
    /// A block's levels, then residuals: kept between blocks.
    levels: Vec<i32>,
}

impl Decoder {
    pub fn new(sps: Sps, mut pps: Pps) -> Result<Decoder, HevcError> {
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
        let mut planes = vec![vec![0u16; width * height]];
        let mut strides = vec![width];
        if chroma != 0 {
            let (chroma_width, chroma_height) = (width >> sub_x, height >> sub_y);
            planes.push(vec![0u16; chroma_width * chroma_height]);
            planes.push(vec![0u16; chroma_width * chroma_height]);
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
        // Z-scan of 4x4 units (6.5.2).
        let units_wide = width.div_ceil(4);
        let units_high = height.div_ceil(4);
        let unit_log2 = ctb_log2 - 2;
        let mut zscan = vec![0u32; units_wide * units_high];
        for y in 0..units_high {
            for x in 0..units_wide {
                let ctb_address = (y >> unit_log2) * width_ctbs + (x >> unit_log2);
                let mut order = (rs_to_ts[ctb_address] as u32) << (unit_log2 * 2);
                for bit in 0..unit_log2 {
                    let mask = 1usize << bit;
                    if x & mask != 0 {
                        order += (mask * mask) as u32;
                    }
                    if y & mask != 0 {
                        order += (2 * mask * mask) as u32;
                    }
                }
                zscan[y * units_wide + x] = order;
            }
        }
        let scaling = pps.scaling.clone().or_else(|| sps.scaling.clone());
        Ok(Decoder {
            width,
            height,
            ctb_log2,
            width_ctbs,
            height_ctbs,
            sub_x,
            sub_y,
            chroma,
            planes,
            strides,
            units: vec![Unit::default(); units_wide * units_high],
            units_wide,
            zscan,
            rs_to_ts,
            ts_to_rs,
            tile_of,
            slice_of: vec![usize::MAX; ctbs],
            slice_address_of: vec![usize::MAX; ctbs],
            slices: Vec::new(),
            sao: vec![[Sao::default(); 3]; ctbs],
            scaling,
            decoded: vec![false; ctbs],
            saved_row_contexts: None,
            saved_slice_contexts: None,
            luma_residual: Vec::new(),
            levels: vec![0; 32 * 32],
            sps,
            pps,
        })
    }

    pub fn unit(&self, x: usize, y: usize) -> &Unit {
        &self.units[(y >> 2) * self.units_wide + (x >> 2)]
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
        let neighbour = self.zscan[(ny >> 2) * self.units_wide + (nx >> 2)];
        let current = self.zscan[(y >> 2) * self.units_wide + (x >> 2)];
        if neighbour > current {
            return false;
        }
        let (a, b) = (self.ctb_address(x, y), self.ctb_address(nx, ny));
        if !self.decoded[b] && a != b {
            return false;
        }
        self.slice_address_of[a] == self.slice_address_of[b] && self.tile_of[a] == self.tile_of[b]
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
            let rs = self.ts_to_rs[ts];
            let (ctb_x, ctb_y) = (rs % self.width_ctbs, rs / self.width_ctbs);
            self.slice_of[rs] = slice_index;
            self.slice_address_of[rs] = slice_address;
            // The context state and QP predictor at a CTB's start (9.3.1,
            // 8.6.1): fresh at a tile's start; at a row's start under
            // wavefronts, the state after the second CTB above when that CTB
            // is available, else fresh; at a dependent segment's start, the
            // state the segment before ended with.
            let tile_start = ts == 0 || self.tile_of[rs] != self.tile_of[self.ts_to_rs[ts - 1]];
            let row_start = self.pps.entropy_sync && ctb_x == self.tile_column_start(ctb_x);
            if tile_start {
                contexts = Contexts::new(slice_qp);
                state.last_qp = slice_qp;
            } else if row_start {
                let x = ctb_x << self.ctb_log2;
                let y = ctb_y << self.ctb_log2;
                let ctb = 1isize << self.ctb_log2;
                let above_right = self.available(x, y, x as isize + ctb, y as isize - ctb);
                contexts = match (&self.saved_row_contexts, above_right) {
                    (Some(saved), true) => saved.clone(),
                    _ => Contexts::new(slice_qp),
                };
                state.last_qp = slice_qp;
            } else if first && state.header.dependent {
                if let Some((saved, last_qp)) = &self.saved_slice_contexts {
                    contexts = saved.clone();
                    state.last_qp = *last_qp;
                }
            }
            first = false;
            self.coding_tree_unit(&mut cabac, &mut contexts, &mut state, ctb_x, ctb_y)?;
            self.decoded[rs] = true;
            // Storage for the row below (9.3.2.4): after a row's second CTB
            // in its tile.
            if self.pps.entropy_sync && ctb_x == self.tile_column_start(ctb_x) + 1 {
                self.saved_row_contexts = Some(contexts.clone());
            }
            let end_of_slice = cabac.terminate() == 1;
            ts += 1;
            if end_of_slice {
                if self.pps.dependent_slices {
                    self.saved_slice_contexts = Some((contexts, state.last_qp));
                }
                return Ok(());
            }
            if ts >= ctbs {
                return Err(HevcError::new("a slice without its end"));
            }
            let next = self.ts_to_rs[ts];
            let new_tile = self.pps.tiles && self.tile_of[next] != self.tile_of[rs];
            let new_row = self.pps.entropy_sync
                && (next % self.width_ctbs == 0 || self.tile_of[next] != self.tile_of[next - 1]);
            if new_tile || new_row {
                if cabac.terminate() != 1 {
                    return Err(HevcError::new("a substream without its end bit"));
                }
                cabac.restart();
            }
        }
    }
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

impl Decoder {
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
            self.slice_address_of[other] == self.slice_address_of[rs]
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
            self.sao[rs] = self.sao[source];
            // A merged component off for this slice stays off.
            if !state.header.sao_luma {
                self.sao[rs][0] = Sao::default();
            }
            if !state.header.sao_chroma {
                self.sao[rs][1] = Sao::default();
                self.sao[rs][2] = Sao::default();
            }
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
        self.sao[rs] = params;
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
                let unit = &mut self.units[(uy >> 2) * self.units_wide + (ux >> 2)];
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
                        self.units[(uy >> 2) * self.units_wide + (ux >> 2)].unfiltered = true;
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
                    self.units[(uy >> 2) * self.units_wide + (ux >> 2)].mode = mode as u8;
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
                self.units[(uy >> 2) * self.units_wide + (ux >> 2)].qp = state.qp as i8;
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
        let mut write_block = |reader: &mut super::bits::BitReader<'_>,
                               plane: usize,
                               px: usize,
                               py: usize,
                               w: usize,
                               h: usize,
                               pcm_depth: u32,
                               depth: u32|
         -> Result<(), HevcError> {
            let stride = self.strides[plane];
            for row in 0..h {
                for column in 0..w {
                    let value = reader.bits(pcm_depth)? << (depth - pcm_depth);
                    self.planes[plane][(py + row) * stride + px + column] = value as u16;
                }
            }
            Ok(())
        };
        let (bit_depth_luma, bit_depth_chroma) =
            (self.sps.bit_depth_luma, self.sps.bit_depth_chroma);
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
        if self.chroma != 0 {
            let (w, h) = (size >> self.sub_x, size >> self.sub_y);
            let (cx, cy) = (x >> self.sub_x, y >> self.sub_y);
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

impl Decoder {
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
        let chroma_mode = |decoder: &Decoder, x: usize, y: usize| -> u32 {
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
                let unit = &mut self.units[(uy >> 2) * self.units_wide + (ux >> 2)];
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
        let available = |sx: isize, sy: isize| -> bool {
            if sx < 0 || sy < 0 || sx as usize >= plane_width || sy as usize >= plane_height {
                return false;
            }
            self.available(
                luma_x,
                luma_y,
                (sx as usize as isize) << shift_x,
                (sy as usize as isize) << shift_y,
            )
        };
        let plane = &self.planes[component];
        let sample = |sx: isize, sy: isize| -> Option<i32> {
            if available(sx, sy) {
                Some(i32::from(plane[sy as usize * stride + sx as usize]))
            } else {
                None
            }
        };
        let (bx, by) = (x as isize, y as isize);
        let mut references = References::substitute(n, bit_depth, |side, index| match side {
            Side::Left => sample(bx - 1, by + index as isize),
            Side::Corner => sample(bx - 1, by - 1),
            Side::Top => sample(bx + index as isize, by - 1),
        });
        let filter_allowed = component == 0 || self.chroma == 3;
        if filter_allowed && !self.sps.intra_smoothing_disabled && intra::filters(mode, n) {
            let strong = component == 0 && self.sps.strong_intra_smoothing;
            references.filter(n, strong, bit_depth);
        }
        let mut predicted = vec![0i32; n * n];
        let edge_filters = component == 0 && n < 32;
        intra::predict(
            &references,
            mode,
            n,
            edge_filters,
            bit_depth,
            &mut predicted,
        );
        let plane = &mut self.planes[component];
        for row in 0..n {
            if y + row >= plane_height {
                break;
            }
            for column in 0..n {
                if x + column >= plane_width {
                    break;
                }
                plane[(y + row) * stride + x + column] = predicted[row * n + column] as u16;
            }
        }
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
        let transform_skip = coded
            && super::residual::residual_coding(
                self,
                cabac,
                contexts,
                state.bypass,
                component,
                log2,
                mode,
                levels,
            )?;
        let bit_depth = if component == 0 {
            self.sps.bit_depth_luma
        } else {
            self.sps.bit_depth_chroma
        };
        if coded && !state.bypass {
            // Scaling (8.6.3).
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
            let shift = bit_depth as i32 + log2 as i32 - 5;
            let scale = LEVEL_SCALE[(qp % 6) as usize] << (qp / 6);
            let factors = self.scaling.as_ref().filter(|_| !(transform_skip && n > 4));
            let matrix = &factors.map(|factors| &factors.factors[log2 as usize - 2][component]);
            for (index, level) in levels.iter_mut().enumerate() {
                if *level == 0 {
                    continue;
                }
                let m = matrix.map_or(16, |matrix| i32::from(matrix[index]));
                let scaled = (i64::from(*level) * i64::from(m) * i64::from(scale)
                    + (1i64 << (shift - 1)))
                    >> shift;
                *level = scaled.clamp(-32768, 32767) as i32;
            }
            if transform_skip {
                let rotate = self.sps.transform_skip_rotation && n == 4;
                transform::transform_skip(levels, log2, bit_depth, rotate);
            } else {
                let dst = component == 0 && n == 4;
                transform::inverse_transform(levels, log2, dst, bit_depth);
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
        let (plane_width, plane_height) = if component == 0 {
            (self.width, self.height)
        } else {
            (self.width >> self.sub_x, self.height >> self.sub_y)
        };
        let plane = &mut self.planes[component];
        for row in 0..n {
            if y + row >= plane_height {
                break;
            }
            for column in 0..n {
                if x + column >= plane_width {
                    break;
                }
                let at = (y + row) * stride + x + column;
                plane[at] = (i32::from(plane[at]) + levels[row * n + column]).clamp(0, max) as u16;
            }
        }
        self.levels = buffer;
        Ok(())
    }

    /// Whether every CTB was decoded: a picture whose data ends early is
    /// not filtered or handed on.
    pub fn complete(&self) -> bool {
        self.decoded.iter().all(|&done| done)
    }

    /// The picture, filtered, with its crop.
    pub fn finish(mut self) -> Picture {
        super::filter::deblock(&mut self);
        super::filter::sample_adaptive_offset(&mut self);
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
