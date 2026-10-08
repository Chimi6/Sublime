//! HEVC parameter sets and slice segment headers (H.265 7.3.2, 7.3.6):
//! what an intra picture needs, the rest parsed past.

use super::HevcError;
use super::bits::BitReader;
use super::tables::{DIAGONAL_4X4, DIAGONAL_8X8};

/// `ScalingFactor` (7.4.5): per size (4, 8, 16, 32) and matrix (intra Y,
/// Cb, Cr, then inter), the factor of each coefficient, row-major.
#[derive(Clone)]
pub struct ScalingFactors {
    pub factors: [[Vec<u8>; 6]; 4],
}

impl ScalingFactors {
    /// Every factor 16: no scaling list.
    pub fn flat() -> ScalingFactors {
        let list = |size: usize| vec![16u8; size * size];
        ScalingFactors {
            factors: [
                std::array::from_fn(|_| list(4)),
                std::array::from_fn(|_| list(8)),
                std::array::from_fn(|_| list(16)),
                std::array::from_fn(|_| list(32)),
            ],
        }
    }

    /// The default lists (Tables 7-5 and 7-6).
    pub fn default_lists() -> ScalingFactors {
        let mut lists = ScalingLists::default();
        for size in 0..4 {
            for matrix in 0..6 {
                lists.set_default(size, matrix);
            }
        }
        lists.factors()
    }
}

/// Scaling lists as coded: per size and matrix the list in diagonal scan
/// order (16 or 64 entries) and, for sizes 16 and 32, the DC.
#[derive(Clone)]
struct ScalingLists {
    lists: [[[u8; 64]; 6]; 4],
    dc: [[u8; 6]; 4],
}

impl Default for ScalingLists {
    fn default() -> ScalingLists {
        ScalingLists {
            lists: [[[16; 64]; 6]; 4],
            dc: [[16; 6]; 4],
        }
    }
}

const DEFAULT_8X8_INTRA: [u8; 64] = [
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 17, 16, 17, 16, 17, 18, 17, 18, 18, 17, 18, 21, 19, 20,
    21, 20, 19, 21, 24, 22, 22, 24, 24, 22, 22, 24, 25, 25, 27, 30, 27, 25, 25, 29, 31, 35, 35, 31,
    29, 36, 41, 44, 41, 36, 47, 54, 54, 47, 65, 70, 65, 88, 88, 115,
];
const DEFAULT_8X8_INTER: [u8; 64] = [
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 17, 17, 17, 17, 17, 18, 18, 18, 18, 18, 18, 20, 20, 20,
    20, 20, 20, 20, 24, 24, 24, 24, 24, 24, 24, 24, 25, 25, 25, 25, 25, 25, 25, 28, 28, 28, 28, 28,
    28, 33, 33, 33, 33, 33, 41, 41, 41, 41, 54, 54, 54, 71, 71, 91,
];

impl ScalingLists {
    fn set_default(&mut self, size: usize, matrix: usize) {
        if size == 0 {
            self.lists[size][matrix] = [16; 64];
        } else if matrix < 3 {
            self.lists[size][matrix] = DEFAULT_8X8_INTRA;
        } else {
            self.lists[size][matrix] = DEFAULT_8X8_INTER;
        }
        self.dc[size][matrix] = 16;
    }

    /// `scaling_list_data()` (7.3.4).
    fn read(reader: &mut BitReader<'_>) -> Result<ScalingLists, HevcError> {
        let mut lists = ScalingLists::default();
        for size in 0..4 {
            let step = if size == 3 { 3 } else { 1 };
            let mut matrix = 0;
            while matrix < 6 {
                if !reader.flag()? {
                    // Predicted: the default, or a copy of an earlier list.
                    let delta = reader.ue()? as usize * step;
                    if delta == 0 {
                        lists.set_default(size, matrix);
                    } else {
                        let source = matrix
                            .checked_sub(delta)
                            .ok_or_else(|| HevcError::new("a scaling list copied from none"))?;
                        lists.lists[size][matrix] = lists.lists[size][source];
                        lists.dc[size][matrix] = lists.dc[size][source];
                    }
                } else {
                    let count = 64.min(1 << (4 + (size << 1)));
                    let mut next: i32 = 8;
                    if size > 1 {
                        let dc = reader.se()? + 8;
                        next = dc;
                        lists.dc[size][matrix] = dc.clamp(1, 255) as u8;
                    }
                    for index in 0..count {
                        let delta = reader.se()?;
                        next = (next + delta + 256).rem_euclid(256);
                        lists.lists[size][matrix][index] = next as u8;
                    }
                }
                matrix += step;
            }
        }
        // 32x32 chroma (4:4:4): the 16x16 lists' matrices (7.3.4, RExt).
        for matrix in [1, 2, 4, 5] {
            lists.lists[3][matrix] = lists.lists[2][matrix];
            lists.dc[3][matrix] = lists.dc[2][matrix];
        }
        Ok(lists)
    }

    /// `ScalingFactor` (7.4.5): each list spread over its block, 8x8
    /// lists upsampled for 16x16 and 32x32, the DC set apart.
    fn factors(&self) -> ScalingFactors {
        let mut factors = ScalingFactors::flat();
        for matrix in 0..6 {
            let list = &self.lists[0][matrix];
            for (index, &(x, y)) in DIAGONAL_4X4.iter().enumerate() {
                factors.factors[0][matrix][usize::from(y) * 4 + usize::from(x)] = list[index];
            }
            for size in 1..4 {
                let side = 4usize << size;
                let ratio = side / 8;
                let list = &self.lists[size][matrix];
                let block = &mut factors.factors[size][matrix];
                for (index, &(x, y)) in DIAGONAL_8X8.iter().enumerate() {
                    for dy in 0..ratio {
                        for dx in 0..ratio {
                            let row = usize::from(y) * ratio + dy;
                            let column = usize::from(x) * ratio + dx;
                            block[row * side + column] = list[index];
                        }
                    }
                }
                if size >= 2 {
                    block[0] = self.dc[size][matrix];
                }
            }
        }
        factors
    }
}

/// `profile_tier_level()` (7.3.3), parsed past.
fn skip_profile_tier_level(
    reader: &mut BitReader<'_>,
    max_sub_layers_minus1: u32,
) -> Result<(), HevcError> {
    reader.skip(96)?;
    let mut profile_present = [false; 8];
    let mut level_present = [false; 8];
    for layer in 0..max_sub_layers_minus1 as usize {
        profile_present[layer] = reader.flag()?;
        level_present[layer] = reader.flag()?;
    }
    if max_sub_layers_minus1 > 0 {
        for _ in max_sub_layers_minus1..8 {
            reader.skip(2)?;
        }
    }
    for layer in 0..max_sub_layers_minus1 as usize {
        if profile_present[layer] {
            reader.skip(88)?;
        }
        if level_present[layer] {
            reader.skip(8)?;
        }
    }
    Ok(())
}

/// A short-term reference picture set's deltas, for parsing the sets
/// predicted from it.
#[derive(Clone, Default)]
struct ShortTermSet {
    negative: Vec<i32>,
    positive: Vec<i32>,
}

/// `st_ref_pic_set()` (7.3.7).
fn read_short_term_set(
    reader: &mut BitReader<'_>,
    index: usize,
    sets: &[ShortTermSet],
    in_slice: bool,
) -> Result<ShortTermSet, HevcError> {
    let predicted = index != 0 && reader.flag()?;
    if !predicted {
        let negatives = reader.ue()? as usize;
        let positives = reader.ue()? as usize;
        if negatives > 16 || positives > 16 {
            return Err(HevcError::new("a reference picture set over 16 pictures"));
        }
        let mut set = ShortTermSet::default();
        let mut poc = 0i32;
        for _ in 0..negatives {
            poc -= reader.ue()? as i32 + 1;
            reader.skip(1)?;
            set.negative.push(poc);
        }
        poc = 0;
        for _ in 0..positives {
            poc += reader.ue()? as i32 + 1;
            reader.skip(1)?;
            set.positive.push(poc);
        }
        return Ok(set);
    }
    let delta_index = if in_slice {
        reader.ue()? as usize + 1
    } else {
        1
    };
    let source = index
        .checked_sub(delta_index)
        .and_then(|source| sets.get(source))
        .ok_or_else(|| HevcError::new("a reference picture set predicted from none"))?;
    let sign = reader.flag()?;
    let magnitude = reader.ue()? as i32 + 1;
    let delta_rps = if sign { -magnitude } else { magnitude };
    let count = source.negative.len() + source.positive.len();
    let mut use_delta = Vec::with_capacity(count + 1);
    for _ in 0..=count {
        let used = reader.flag()?;
        use_delta.push(used || reader.flag()?);
    }
    let negatives = source.negative.len();
    let mut set = ShortTermSet::default();
    for (j, &delta) in source.positive.iter().enumerate().rev() {
        let poc = delta + delta_rps;
        if poc < 0 && use_delta[negatives + j] {
            set.negative.push(poc);
        }
    }
    if delta_rps < 0 && use_delta[count] {
        set.negative.push(delta_rps);
    }
    for (j, &delta) in source.negative.iter().enumerate() {
        let poc = delta + delta_rps;
        if poc < 0 && use_delta[j] {
            set.negative.push(poc);
        }
    }
    for (j, &delta) in source.negative.iter().enumerate().rev() {
        let poc = delta + delta_rps;
        if poc > 0 && use_delta[j] {
            set.positive.push(poc);
        }
    }
    if delta_rps > 0 && use_delta[count] {
        set.positive.push(delta_rps);
    }
    for (j, &delta) in source.positive.iter().enumerate() {
        let poc = delta + delta_rps;
        if poc > 0 && use_delta[negatives + j] {
            set.positive.push(poc);
        }
    }
    Ok(set)
}

/// `hrd_parameters()` (E.2.2), parsed past.
fn skip_hrd(
    reader: &mut BitReader<'_>,
    common: bool,
    max_sub_layers_minus1: u32,
) -> Result<(), HevcError> {
    let mut nal = false;
    let mut vcl = false;
    let mut sub_pic = false;
    if common {
        nal = reader.flag()?;
        vcl = reader.flag()?;
        if nal || vcl {
            sub_pic = reader.flag()?;
            if sub_pic {
                reader.skip(8 + 5 + 1 + 5)?;
            }
            reader.skip(4 + 4)?;
            if sub_pic {
                reader.skip(4)?;
            }
            reader.skip(5 + 5 + 5)?;
        }
    }
    for _ in 0..=max_sub_layers_minus1 {
        let fixed_general = reader.flag()?;
        let fixed_within = if fixed_general { true } else { reader.flag()? };
        let mut low_delay = false;
        if fixed_within {
            reader.ue()?;
        } else {
            low_delay = reader.flag()?;
        }
        let mut count = 1;
        if !low_delay {
            count = reader.ue()? + 1;
        }
        for present in [nal, vcl] {
            if present {
                for _ in 0..count {
                    reader.ue()?;
                    reader.ue()?;
                    if sub_pic {
                        reader.ue()?;
                        reader.ue()?;
                    }
                    reader.skip(1)?;
                }
            }
        }
    }
    Ok(())
}

/// What the video usability information says of colour (E.2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VuiColour {
    pub full_range: bool,
    pub matrix: u8,
}

/// A sequence parameter set: what decoding an intra picture needs.
#[derive(Clone)]
pub struct Sps {
    pub id: u32,
    /// 0 monochrome, 1 4:2:0, 2 4:2:2, 3 4:4:4.
    pub chroma_format: u32,
    pub separate_planes: bool,
    pub width: u32,
    pub height: u32,
    /// The conformance window in luma samples: left, right, top, bottom.
    pub crop: [u32; 4],
    pub bit_depth_luma: u32,
    pub bit_depth_chroma: u32,
    pub log2_max_poc_lsb: u32,
    pub log2_min_cb: u32,
    pub log2_ctb: u32,
    pub log2_min_tb: u32,
    pub log2_max_tb: u32,
    pub max_transform_depth_intra: u32,
    pub scaling: Option<ScalingFactors>,
    pub sao: bool,
    pub pcm: Option<Pcm>,
    pub short_term_sets: usize,
    short_term: Vec<ShortTermSet>,
    pub long_term_present: bool,
    pub long_term_count: u32,
    pub temporal_mvp: bool,
    pub strong_intra_smoothing: bool,
    pub vui_colour: Option<VuiColour>,
    // Range extension.
    pub transform_skip_rotation: bool,
    pub transform_skip_context: bool,
    pub implicit_rdpcm: bool,
    pub extended_precision: bool,
    pub intra_smoothing_disabled: bool,
    pub persistent_rice_adaptation: bool,
    pub cabac_bypass_alignment: bool,
}

#[derive(Clone, Copy)]
pub struct Pcm {
    pub bit_depth_luma: u32,
    pub bit_depth_chroma: u32,
    pub log2_min: u32,
    pub log2_max: u32,
    pub loop_filter_disabled: bool,
}

impl Sps {
    /// The chroma array type: the chroma format, or monochrome when the
    /// planes are coded apart.
    pub fn chroma_array_type(&self) -> u32 {
        if self.separate_planes {
            0
        } else {
            self.chroma_format
        }
    }

    pub fn parse(rbsp: &[u8]) -> Result<Sps, HevcError> {
        let mut reader = BitReader::new(rbsp);
        reader.skip(4)?;
        let max_sub_layers_minus1 = reader.bits(3)?;
        reader.skip(1)?;
        skip_profile_tier_level(&mut reader, max_sub_layers_minus1)?;
        let id = reader.ue()?;
        let chroma_format = reader.ue()?;
        if chroma_format > 3 {
            return Err(HevcError::new("a chroma format past 4:4:4"));
        }
        let separate_planes = chroma_format == 3 && reader.flag()?;
        let width = reader.ue()?;
        let height = reader.ue()?;
        if width == 0 || height == 0 || width > 16_888 || height > 16_888 {
            return Err(HevcError::new("a picture size out of range"));
        }
        let mut crop = [0u32; 4];
        if reader.flag()? {
            let (unit_x, unit_y) = match chroma_format {
                1 if !separate_planes => (2, 2),
                2 if !separate_planes => (2, 1),
                _ => (1, 1),
            };
            for (index, value) in crop.iter_mut().enumerate() {
                *value = reader.ue()? * if index < 2 { unit_x } else { unit_y };
            }
        }
        let bit_depth_luma = reader.ue()? + 8;
        let bit_depth_chroma = reader.ue()? + 8;
        if bit_depth_luma > 16 || bit_depth_chroma > 16 {
            return Err(HevcError::new("a bit depth past 16"));
        }
        let log2_max_poc_lsb = reader.ue()? + 4;
        let ordering_all = reader.flag()?;
        let first = if ordering_all {
            0
        } else {
            max_sub_layers_minus1
        };
        for _ in first..=max_sub_layers_minus1 {
            reader.ue()?;
            reader.ue()?;
            reader.ue()?;
        }
        let log2_min_cb = reader.ue()? + 3;
        let log2_ctb = log2_min_cb + reader.ue()?;
        let log2_min_tb = reader.ue()? + 2;
        let log2_max_tb = log2_min_tb + reader.ue()?;
        if !(4..=6).contains(&log2_ctb) || log2_max_tb > 5 || log2_min_tb > log2_min_cb {
            return Err(HevcError::new("block sizes out of range"));
        }
        let _max_transform_depth_inter = reader.ue()?;
        let max_transform_depth_intra = reader.ue()?;
        let scaling = if reader.flag()? {
            if reader.flag()? {
                Some(ScalingLists::read(&mut reader)?.factors())
            } else {
                Some(ScalingFactors::default_lists())
            }
        } else {
            None
        };
        let _amp = reader.flag()?;
        let sao = reader.flag()?;
        let pcm = if reader.flag()? {
            let bit_depth_luma = reader.bits(4)? + 1;
            let bit_depth_chroma = reader.bits(4)? + 1;
            let log2_min = reader.ue()? + 3;
            let log2_max = log2_min + reader.ue()?;
            let loop_filter_disabled = reader.flag()?;
            Some(Pcm {
                bit_depth_luma,
                bit_depth_chroma,
                log2_min,
                log2_max,
                loop_filter_disabled,
            })
        } else {
            None
        };
        let short_term_sets = reader.ue()? as usize;
        if short_term_sets > 64 {
            return Err(HevcError::new("over 64 reference picture sets"));
        }
        let mut short_term = Vec::with_capacity(short_term_sets);
        for index in 0..short_term_sets {
            let set = read_short_term_set(&mut reader, index, &short_term, false)?;
            short_term.push(set);
        }
        let long_term_present = reader.flag()?;
        let mut long_term_count = 0;
        if long_term_present {
            long_term_count = reader.ue()?;
            for _ in 0..long_term_count {
                reader.skip(log2_max_poc_lsb as usize + 1)?;
            }
        }
        let temporal_mvp = reader.flag()?;
        let strong_intra_smoothing = reader.flag()?;
        let mut vui_colour = None;
        if reader.flag()? {
            vui_colour = read_vui(&mut reader, max_sub_layers_minus1)?;
        }
        let mut sps = Sps {
            id,
            chroma_format,
            separate_planes,
            width,
            height,
            crop,
            bit_depth_luma,
            bit_depth_chroma,
            log2_max_poc_lsb,
            log2_min_cb,
            log2_ctb,
            log2_min_tb,
            log2_max_tb,
            max_transform_depth_intra,
            scaling,
            sao,
            pcm,
            short_term_sets,
            short_term,
            long_term_present,
            long_term_count,
            temporal_mvp,
            strong_intra_smoothing,
            vui_colour,
            transform_skip_rotation: false,
            transform_skip_context: false,
            implicit_rdpcm: false,
            extended_precision: false,
            intra_smoothing_disabled: false,
            persistent_rice_adaptation: false,
            cabac_bypass_alignment: false,
        };
        if reader.flag()? {
            let range = reader.flag()?;
            reader.skip(7)?;
            if range {
                sps.transform_skip_rotation = reader.flag()?;
                sps.transform_skip_context = reader.flag()?;
                sps.implicit_rdpcm = reader.flag()?;
                let _explicit_rdpcm = reader.flag()?;
                sps.extended_precision = reader.flag()?;
                sps.intra_smoothing_disabled = reader.flag()?;
                let _high_precision_offsets = reader.flag()?;
                sps.persistent_rice_adaptation = reader.flag()?;
                sps.cabac_bypass_alignment = reader.flag()?;
            }
        }
        if sps.extended_precision {
            return Err(HevcError::new(
                "extended precision processing (a 16-bit profile) is not supported",
            ));
        }
        Ok(sps)
    }
}

/// `vui_parameters()` (E.2.1): the colour it states, the rest parsed past.
fn read_vui(
    reader: &mut BitReader<'_>,
    max_sub_layers_minus1: u32,
) -> Result<Option<VuiColour>, HevcError> {
    if reader.flag()? && reader.bits(8)? == 255 {
        reader.skip(32)?;
    }
    if reader.flag()? {
        reader.skip(1)?;
    }
    let mut colour = None;
    if reader.flag()? {
        reader.skip(3)?;
        let full_range = reader.flag()?;
        let mut matrix = 2;
        if reader.flag()? {
            reader.skip(16)?;
            matrix = reader.bits(8)? as u8;
        }
        colour = Some(VuiColour { full_range, matrix });
    }
    if reader.flag()? {
        reader.ue()?;
        reader.ue()?;
    }
    reader.skip(3)?;
    if reader.flag()? {
        for _ in 0..4 {
            reader.ue()?;
        }
    }
    if reader.flag()? {
        reader.skip(64)?;
        if reader.flag()? {
            reader.ue()?;
        }
        if reader.flag()? {
            skip_hrd(reader, true, max_sub_layers_minus1)?;
        }
    }
    if reader.flag()? {
        reader.skip(3)?;
        for _ in 0..5 {
            reader.ue()?;
        }
    }
    Ok(colour)
}

/// A picture parameter set.
#[derive(Clone)]
pub struct Pps {
    pub id: u32,
    pub sps_id: u32,
    pub dependent_slices: bool,
    pub output_flag_present: bool,
    pub extra_slice_header_bits: u32,
    pub sign_data_hiding: bool,
    pub init_qp: i32,
    pub transform_skip: bool,
    pub cu_qp_delta: bool,
    pub diff_cu_qp_delta_depth: u32,
    pub cb_qp_offset: i32,
    pub cr_qp_offset: i32,
    pub slice_chroma_qp_offsets: bool,
    pub transquant_bypass: bool,
    pub tiles: bool,
    pub entropy_sync: bool,
    /// Tile column and row boundaries, in CTBs (from 0 to the picture's
    /// width or height in CTBs), when tiles are on.
    pub tile_columns: Vec<u32>,
    pub tile_rows: Vec<u32>,
    tile_uniform: bool,
    tile_column_widths: Vec<u32>,
    tile_row_heights: Vec<u32>,
    pub loop_filter_across_tiles: bool,
    pub loop_filter_across_slices: bool,
    pub deblocking_override: bool,
    pub deblocking_disabled: bool,
    pub beta_offset: i32,
    pub tc_offset: i32,
    pub scaling: Option<ScalingFactors>,
    pub slice_header_extension: bool,
    // Range extension.
    pub log2_max_transform_skip: u32,
    pub cross_component_prediction: bool,
    pub chroma_qp_offset_list: bool,
    pub diff_cu_chroma_qp_offset_depth: u32,
    pub cb_qp_offset_list: Vec<i32>,
    pub cr_qp_offset_list: Vec<i32>,
    pub log2_sao_offset_scale_luma: u32,
    pub log2_sao_offset_scale_chroma: u32,
}

impl Pps {
    pub fn parse(rbsp: &[u8]) -> Result<Pps, HevcError> {
        let mut reader = BitReader::new(rbsp);
        let id = reader.ue()?;
        let sps_id = reader.ue()?;
        let dependent_slices = reader.flag()?;
        let output_flag_present = reader.flag()?;
        let extra_slice_header_bits = reader.bits(3)?;
        let sign_data_hiding = reader.flag()?;
        let _cabac_init_present = reader.flag()?;
        reader.ue()?;
        reader.ue()?;
        let init_qp = 26 + reader.se()?;
        // constrained_intra_pred_flag: every neighbour of an intra
        // picture is intra already.
        reader.flag()?;
        let transform_skip = reader.flag()?;
        let cu_qp_delta = reader.flag()?;
        let diff_cu_qp_delta_depth = if cu_qp_delta { reader.ue()? } else { 0 };
        let cb_qp_offset = reader.se()?;
        let cr_qp_offset = reader.se()?;
        let slice_chroma_qp_offsets = reader.flag()?;
        let _weighted_pred = reader.flag()?;
        let _weighted_bipred = reader.flag()?;
        let transquant_bypass = reader.flag()?;
        let tiles = reader.flag()?;
        let entropy_sync = reader.flag()?;
        let mut pps = Pps {
            id,
            sps_id,
            dependent_slices,
            output_flag_present,
            extra_slice_header_bits,
            sign_data_hiding,
            init_qp,
            transform_skip,
            cu_qp_delta,
            diff_cu_qp_delta_depth,
            cb_qp_offset,
            cr_qp_offset,
            slice_chroma_qp_offsets,
            transquant_bypass,
            tiles,
            entropy_sync,
            tile_columns: Vec::new(),
            tile_rows: Vec::new(),
            tile_uniform: true,
            tile_column_widths: Vec::new(),
            tile_row_heights: Vec::new(),
            loop_filter_across_tiles: true,
            loop_filter_across_slices: false,
            deblocking_override: false,
            deblocking_disabled: false,
            beta_offset: 0,
            tc_offset: 0,
            scaling: None,
            slice_header_extension: false,
            log2_max_transform_skip: 2,
            cross_component_prediction: false,
            chroma_qp_offset_list: false,
            diff_cu_chroma_qp_offset_depth: 0,
            cb_qp_offset_list: Vec::new(),
            cr_qp_offset_list: Vec::new(),
            log2_sao_offset_scale_luma: 0,
            log2_sao_offset_scale_chroma: 0,
        };
        if tiles {
            let columns = reader.ue()? + 1;
            let rows = reader.ue()? + 1;
            if columns > 64 || rows > 64 {
                return Err(HevcError::new("over 64 tile columns or rows"));
            }
            pps.tile_uniform = reader.flag()?;
            pps.tile_column_widths = vec![0; columns as usize];
            pps.tile_row_heights = vec![0; rows as usize];
            if !pps.tile_uniform {
                for index in 0..columns as usize - 1 {
                    pps.tile_column_widths[index] = reader.ue()? + 1;
                }
                for index in 0..rows as usize - 1 {
                    pps.tile_row_heights[index] = reader.ue()? + 1;
                }
            }
            pps.loop_filter_across_tiles = reader.flag()?;
        }
        pps.loop_filter_across_slices = reader.flag()?;
        if reader.flag()? {
            pps.deblocking_override = reader.flag()?;
            pps.deblocking_disabled = reader.flag()?;
            if !pps.deblocking_disabled {
                pps.beta_offset = reader.se()? * 2;
                pps.tc_offset = reader.se()? * 2;
            }
        }
        if reader.flag()? {
            pps.scaling = Some(ScalingLists::read(&mut reader)?.factors());
        }
        let _lists_modification = reader.flag()?;
        reader.ue()?;
        pps.slice_header_extension = reader.flag()?;
        if reader.flag()? {
            let range = reader.flag()?;
            reader.skip(7)?;
            if range {
                if pps.transform_skip {
                    pps.log2_max_transform_skip = reader.ue()? + 2;
                }
                pps.cross_component_prediction = reader.flag()?;
                pps.chroma_qp_offset_list = reader.flag()?;
                if pps.chroma_qp_offset_list {
                    pps.diff_cu_chroma_qp_offset_depth = reader.ue()?;
                    let count = reader.ue()? + 1;
                    for _ in 0..count.min(6) {
                        pps.cb_qp_offset_list.push(reader.se()?);
                        pps.cr_qp_offset_list.push(reader.se()?);
                    }
                }
                pps.log2_sao_offset_scale_luma = reader.ue()?;
                pps.log2_sao_offset_scale_chroma = reader.ue()?;
            }
        }
        Ok(pps)
    }

    /// The tile boundaries for a picture `width` by `height` CTBs (6.5.1).
    pub fn lay_out_tiles(&mut self, width: u32, height: u32) {
        if !self.tiles {
            self.tile_columns = vec![0, width];
            self.tile_rows = vec![0, height];
            return;
        }
        let spread = |count: usize, total: u32, sizes: &[u32], uniform: bool| -> Vec<u32> {
            let mut bounds = vec![0u32];
            for index in 0..count {
                let size = if uniform {
                    ((index as u32 + 1) * total) / count as u32
                        - (index as u32 * total) / count as u32
                } else if index + 1 < count {
                    sizes.get(index).copied().unwrap_or(0)
                } else {
                    total.saturating_sub(*bounds.last().unwrap_or(&0))
                };
                let next = (bounds.last().copied().unwrap_or(0) + size).min(total);
                bounds.push(next);
            }
            bounds
        };
        self.tile_columns = spread(
            self.tile_column_widths.len(),
            width,
            &self.tile_column_widths,
            self.tile_uniform,
        );
        self.tile_rows = spread(
            self.tile_row_heights.len(),
            height,
            &self.tile_row_heights,
            self.tile_uniform,
        );
    }
}

/// A slice segment header (7.3.6.1), for intra slices.
#[derive(Clone)]
pub struct SliceHeader {
    pub first_in_picture: bool,
    pub dependent: bool,
    pub address: u32,
    pub sao_luma: bool,
    pub sao_chroma: bool,
    pub qp_delta: i32,
    pub cb_qp_offset: i32,
    pub cr_qp_offset: i32,
    pub cu_chroma_qp_offset: bool,
    pub deblocking_disabled: bool,
    pub beta_offset: i32,
    pub tc_offset: i32,
    pub loop_filter_across_slices: bool,
    /// Where the slice data begins in the RBSP.
    pub data_offset: usize,
}

impl SliceHeader {
    /// The header of a slice segment. `previous` is the picture's last
    /// independent segment header, which a dependent segment takes its
    /// fields from.
    pub fn parse(
        rbsp: &[u8],
        nal_kind: u8,
        sps_of: impl Fn(u32) -> Option<(Sps, Pps)>,
        previous: Option<&SliceHeader>,
    ) -> Result<(SliceHeader, Sps, Pps), HevcError> {
        let mut reader = BitReader::new(rbsp);
        let first_in_picture = reader.flag()?;
        if (16..=23).contains(&nal_kind) {
            reader.skip(1)?;
        }
        let pps_id = reader.ue()?;
        let (sps, pps) =
            sps_of(pps_id).ok_or_else(|| HevcError::new("a slice naming a missing PPS"))?;
        let ctb = 1u32 << sps.log2_ctb;
        let picture_ctbs = sps.width.div_ceil(ctb) * sps.height.div_ceil(ctb);
        let mut dependent = false;
        let mut address = 0;
        if !first_in_picture {
            if pps.dependent_slices {
                dependent = reader.flag()?;
            }
            let bits = 32 - (picture_ctbs - 1).leading_zeros();
            address = reader.bits(bits)?;
            if address >= picture_ctbs {
                return Err(HevcError::new("a slice address past the picture"));
            }
        }
        let mut header = match (dependent, previous) {
            (true, Some(previous)) => SliceHeader {
                first_in_picture,
                dependent,
                address,
                data_offset: 0,
                ..previous.clone()
            },
            (true, None) => return Err(HevcError::new("a dependent slice with none before it")),
            (false, _) => SliceHeader {
                first_in_picture,
                dependent,
                address,
                sao_luma: false,
                sao_chroma: false,
                qp_delta: 0,
                cb_qp_offset: 0,
                cr_qp_offset: 0,
                cu_chroma_qp_offset: false,
                deblocking_disabled: pps.deblocking_disabled,
                beta_offset: pps.beta_offset,
                tc_offset: pps.tc_offset,
                loop_filter_across_slices: pps.loop_filter_across_slices,
                data_offset: 0,
            },
        };
        if !dependent {
            reader.skip(pps.extra_slice_header_bits as usize)?;
            let slice_type = reader.ue()?;
            if slice_type != 2 {
                return Err(HevcError::new(
                    "a predicted (P or B) slice; only intra pictures are decoded",
                ));
            }
            if pps.output_flag_present {
                reader.skip(1)?;
            }
            if sps.separate_planes {
                reader.skip(2)?;
            }
            if nal_kind != 19 && nal_kind != 20 {
                reader.skip(sps.log2_max_poc_lsb as usize)?;
                if !reader.flag()? {
                    read_short_term_set(&mut reader, sps.short_term_sets, &sps.short_term, true)?;
                } else if sps.short_term_sets > 1 {
                    let bits = 32 - (sps.short_term_sets as u32 - 1).leading_zeros();
                    reader.skip(bits as usize)?;
                }
                if sps.long_term_present {
                    let mut from_sps = 0;
                    if sps.long_term_count > 0 {
                        from_sps = reader.ue()?;
                    }
                    let pictures = reader.ue()?;
                    for index in 0..from_sps + pictures {
                        if index < from_sps {
                            if sps.long_term_count > 1 {
                                let bits = 32 - (sps.long_term_count - 1).leading_zeros();
                                reader.skip(bits as usize)?;
                            }
                        } else {
                            reader.skip(sps.log2_max_poc_lsb as usize + 1)?;
                        }
                        if reader.flag()? {
                            reader.ue()?;
                        }
                    }
                }
                if sps.temporal_mvp {
                    reader.skip(1)?;
                }
            }
            if sps.sao {
                header.sao_luma = reader.flag()?;
                if sps.chroma_array_type() != 0 {
                    header.sao_chroma = reader.flag()?;
                }
            }
            header.qp_delta = reader.se()?;
            if pps.slice_chroma_qp_offsets {
                header.cb_qp_offset = reader.se()?;
                header.cr_qp_offset = reader.se()?;
            }
            if pps.chroma_qp_offset_list {
                header.cu_chroma_qp_offset = reader.flag()?;
            }
            let overridden = pps.deblocking_override && reader.flag()?;
            if overridden {
                header.deblocking_disabled = reader.flag()?;
                if !header.deblocking_disabled {
                    header.beta_offset = reader.se()? * 2;
                    header.tc_offset = reader.se()? * 2;
                }
            }
            if pps.loop_filter_across_slices
                && (header.sao_luma || header.sao_chroma || !header.deblocking_disabled)
            {
                header.loop_filter_across_slices = reader.flag()?;
            }
        }
        if pps.tiles || pps.entropy_sync {
            let offsets = reader.ue()?;
            if offsets > 0 {
                let bits = reader.ue()? + 1;
                if bits > 32 {
                    return Err(HevcError::new("an entry point offset over 32 bits"));
                }
                for _ in 0..offsets {
                    reader.skip(bits as usize)?;
                }
            }
        }
        if pps.slice_header_extension {
            let length = reader.ue()? as usize;
            reader.skip(length * 8)?;
        }
        // byte_alignment(): a one, then zeros to the byte.
        if !reader.flag()? {
            return Err(HevcError::new("a slice header without its alignment bit"));
        }
        header.data_offset = reader.byte_position();
        Ok((header, sps, pps))
    }
}
