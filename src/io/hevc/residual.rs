//! `residual_coding()` (H.265 7.3.8.11): a transform block's coefficient
//! levels, sub-block by sub-block from the last significant one, with the
//! context selection of 9.3.4.2.

use super::HevcError;
use super::cabac::{self, Cabac, Contexts};
use super::decode::Layout;
use super::tables::{Scan, scan};

/// The scan of a block (7.4.9.11): intra 4x4 blocks, and 8x8 luma (or
/// any 8x8 in 4:4:4), scan across or down when predicted near-vertically
/// or near-horizontally.
fn scan_kind(log2: u32, component: usize, chroma: u32, mode: u32) -> Scan {
    if log2 == 2 || (log2 == 3 && (component == 0 || chroma == 3)) {
        if (6..=14).contains(&mode) {
            return Scan::Vertical;
        }
        if (22..=30).contains(&mode) {
            return Scan::Horizontal;
        }
    }
    Scan::Diagonal
}

const SIG_CTX_4X4: [u8; 16] = [0, 1, 4, 5, 2, 3, 4, 5, 6, 6, 8, 8, 7, 7, 8, 8];

/// The significance context of each position in a sub-block (row-major)
/// by the pattern of coded sub-blocks to its right (1) and below (2),
/// before the offsets of size, component, and place (9.3.4.2.5).
const SIG_CTX_PATTERN: [[u8; 16]; 4] = {
    let mut tables = [[0u8; 16]; 4];
    let mut pattern = 0;
    while pattern < 4 {
        let mut position = 0;
        while position < 16 {
            let (px, py) = (position % 4, position / 4);
            tables[pattern][position] = match pattern {
                0 => match px + py {
                    0 => 2,
                    1 | 2 => 1,
                    _ => 0,
                },
                1 => match py {
                    0 => 2,
                    1 => 1,
                    _ => 0,
                },
                2 => match px {
                    0 => 2,
                    1 => 1,
                    _ => 0,
                },
                _ => 2,
            };
            position += 1;
        }
        pattern += 1;
    }
    tables
};

/// The scaling of levels as they are read (8.6.3): times the level scale
/// and the scaling list's factor (16 without one), rounded down `shift`
/// bits, clipped to 16 bits.
pub struct Dequant<'s> {
    pub scale: i64,
    pub shift: u32,
    /// The block's scaling factors, row-major; not used by a transform
    /// skip block over 4x4.
    pub matrix: Option<&'s [u8]>,
}

/// Each significance context table, once: by block size (4x4, 8x8, 16x16
/// and up), component, scan, the pattern of coded neighbours, and whether
/// the sub-block is the block's first (which holds the DC).
pub(super) fn significance_table(
    log2: u32,
    luma: bool,
    kind: Scan,
    pattern: usize,
    first: bool,
) -> &'static [u8; 16] {
    static TABLES: std::sync::OnceLock<Vec<[u8; 16]>> = std::sync::OnceLock::new();
    let tables = TABLES.get_or_init(|| {
        let mut tables = Vec::with_capacity(144);
        for size in 0..3u32 {
            for luma in [false, true] {
                for kind in [Scan::Diagonal, Scan::Horizontal, Scan::Vertical] {
                    for pattern_table in &SIG_CTX_PATTERN {
                        for first in [false, true] {
                            let position_scan = scan(2, kind);
                            let chroma_base = if luma { 0 } else { 27 };
                            let mut table = [0u8; 16];
                            for (p, context) in table.iter_mut().enumerate() {
                                let (px, py) = position_scan[p];
                                let at = usize::from(py) * 4 + usize::from(px);
                                *context = if size == 0 {
                                    chroma_base + SIG_CTX_4X4[at]
                                } else {
                                    let offset = chroma_base
                                        + if luma {
                                            (if first { 0 } else { 3 })
                                                + if size == 1 {
                                                    if kind == Scan::Diagonal { 9 } else { 15 }
                                                } else {
                                                    21
                                                }
                                        } else if size == 1 {
                                            9
                                        } else {
                                            12
                                        };
                                    offset + pattern_table[at]
                                };
                            }
                            if size > 0 && first {
                                // The block's DC.
                                table[0] = chroma_base;
                            }
                            tables.push(table);
                        }
                    }
                }
            }
        }
        tables
    });
    let size = (log2.min(4) - 2) as usize;
    let kind = match kind {
        Scan::Diagonal => 0,
        Scan::Horizontal => 1,
        Scan::Vertical => 2,
    };
    &tables[(((size * 2 + usize::from(luma)) * 3 + kind) * 4 + pattern) * 2 + usize::from(first)]
}

/// What a block's coding said beyond its levels.
pub struct Coded {
    pub transform_skip: bool,
    /// The rows and columns that hold every nonzero level.
    pub rows: usize,
    pub columns: usize,
}

/// Reads a block's levels into `levels` (row-major, zero beforehand).
#[allow(clippy::too_many_arguments)]
pub fn residual_coding(
    layout: &Layout,
    cabac: &mut Cabac<'_>,
    contexts: &mut Contexts,
    bypass: bool,
    component: usize,
    log2: u32,
    mode: u32,
    levels: &mut [i32],
    dequant: Option<Dequant<'_>>,
) -> Result<Coded, HevcError> {
    let sps = &layout.sps;
    let pps = &layout.pps;
    let n = 1usize << log2;
    let luma = component == 0;
    let mut transform_skip = false;
    if pps.transform_skip && !bypass && log2 <= pps.log2_max_transform_skip {
        transform_skip =
            cabac.decision(&mut contexts.contexts[cabac::TRANSFORM_SKIP + usize::from(!luma)]) == 1;
    }
    // Scaling as levels are read; a transform skip block over 4x4 takes
    // no scaling list.
    let matrix = dequant
        .as_ref()
        .and_then(|dequant| dequant.matrix)
        .filter(|_| !(transform_skip && n > 4));
    let round = dequant
        .as_ref()
        .map_or(0, |dequant| 1i64 << (dequant.shift - 1));
    // The last significant coefficient (9.3.4.2.3).
    let (offset, shift) = if luma {
        (
            3 * (log2 as usize - 2) + ((log2 as usize - 1) >> 2),
            (log2 as usize + 1) >> 2,
        )
    } else {
        (15, log2 as usize - 2)
    };
    let largest = (log2 << 1) - 1;
    let mut prefix = |contexts: &mut Contexts, base: usize| -> u32 {
        let mut value = 0;
        while value < largest
            && cabac.decision(&mut contexts.contexts[base + offset + ((value as usize) >> shift)])
                == 1
        {
            value += 1;
        }
        value
    };
    let x_prefix = prefix(contexts, cabac::LAST_X);
    let y_prefix = prefix(contexts, cabac::LAST_Y);
    let suffix = |cabac: &mut Cabac<'_>, prefix: u32| -> u32 {
        if prefix > 3 {
            let bits = (prefix >> 1) - 1;
            (1 << bits) * (2 + (prefix & 1)) + cabac.bypass_bits(bits)
        } else {
            prefix
        }
    };
    let mut last_x = suffix(cabac, x_prefix) as usize;
    let mut last_y = suffix(cabac, y_prefix) as usize;
    let kind = scan_kind(log2, component, layout.chroma, mode);
    if kind == Scan::Vertical {
        std::mem::swap(&mut last_x, &mut last_y);
    }
    if last_x >= n || last_y >= n {
        return Err(HevcError::new("a last coefficient outside its block"));
    }
    let log2_subs = log2 - 2;
    let subs = 1usize << log2_subs;
    let sub_scan = scan(log2_subs, kind);
    let position_scan = scan(2, kind);
    // The last sub-block and position in scan order.
    let in_sub = ((last_x >> 2) as u8, (last_y >> 2) as u8);
    let in_position = ((last_x & 3) as u8, (last_y & 3) as u8);
    let last_sub = sub_scan
        .iter()
        .position(|&place| place == in_sub)
        .ok_or_else(|| HevcError::new("a last coefficient not on the scan"))?;
    let last_position = position_scan
        .iter()
        .position(|&place| place == in_position)
        .ok_or_else(|| HevcError::new("a last coefficient not on the scan"))?;
    let mut coded_sub = [false; 64];
    let sign_hiding_allowed = pps.sign_data_hiding
        && !bypass
        && !(sps.implicit_rdpcm && transform_skip && (mode == 10 || mode == 26));
    let skip_context = sps.transform_skip_context && (transform_skip || bypass);
    let mut greater1_ctx = 1u32;
    let (mut rows, mut columns) = (0usize, 0usize);
    let stat_index = 2 * usize::from(luma) + usize::from(transform_skip || bypass);
    for sub in (0..=last_sub).rev() {
        let (sx, sy) = (usize::from(sub_scan[sub].0), usize::from(sub_scan[sub].1));
        let mut infer_dc = false;
        let right = sx + 1 < subs && coded_sub[sy * subs + sx + 1];
        let below = sy + 1 < subs && coded_sub[(sy + 1) * subs + sx];
        if sub < last_sub && sub > 0 {
            let increment = usize::from(right || below) + if luma { 0 } else { 2 };
            coded_sub[sy * subs + sx] =
                cabac.decision(&mut contexts.contexts[cabac::CODED_SUB_BLOCK + increment]) == 1;
            infer_dc = true;
        } else {
            coded_sub[sy * subs + sx] = true;
        }
        let pattern = usize::from(right) + 2 * usize::from(below);
        // Significance (9.3.4.2.5): the significant positions, highest
        // scan position first, as every later pass walks them.
        let mut found = [0u8; 16];
        let mut count = 0usize;
        if sub == last_sub {
            found[0] = last_position as u8;
            count = 1;
        }
        let start = if sub == last_sub {
            last_position as isize - 1
        } else {
            15
        };
        let coded_here = coded_sub[sy * subs + sx];
        if coded_here && start >= 0 {
            // Each scan position's context, for this sub-block.
            let chroma_base: u8 = if luma { 0 } else { 27 };
            let skip_table = [chroma_base + if luma { 42 } else { 16 }; 16];
            let contexts_here = if skip_context {
                &skip_table
            } else {
                significance_table(log2, luma, kind, pattern, sx == 0 && sy == 0)
            };
            let significance = &mut contexts.contexts[cabac::SIG_COEFF..cabac::SIG_COEFF + 44 + 27];
            for p in (1..=start as usize).rev() {
                if cabac.decision(&mut significance[usize::from(contexts_here[p])]) == 1 {
                    found[count] = p as u8;
                    count += 1;
                    infer_dc = false;
                }
            }
            // The DC position: coded, or inferred when nothing after it was.
            if infer_dc || cabac.decision(&mut significance[usize::from(contexts_here[0])]) == 1 {
                found[count] = 0;
                count += 1;
            }
        }
        if count == 0 {
            continue;
        }
        let found = &found[..count];
        // Greater-than-one and -two flags (9.3.4.2.6, 9.3.4.2.7) for the
        // first eight; each level starts at 1, 2, or 3.
        let mut ctx_set = if sub == 0 || !luma { 0 } else { 2 };
        if sub != last_sub && greater1_ctx == 0 {
            ctx_set += 1;
        }
        greater1_ctx = 1;
        let greater1_base = cabac::GREATER1 + ctx_set * 4 + if luma { 0 } else { 16 };
        let mut base = [1i32; 16];
        let mut first_greater1: Option<usize> = None;
        let mut escape = count > 8;
        for (index, value) in base.iter_mut().enumerate().take(count.min(8)) {
            let increment = greater1_ctx.min(3) as usize;
            if cabac.decision(&mut contexts.contexts[greater1_base + increment]) == 1 {
                *value = 2;
                greater1_ctx = 0;
                if first_greater1.is_none() {
                    first_greater1 = Some(index);
                } else {
                    escape = true;
                }
            } else if greater1_ctx > 0 {
                greater1_ctx += 1;
            }
        }
        if let Some(index) = first_greater1 {
            let increment = ctx_set + if luma { 0 } else { 4 };
            if cabac.decision(&mut contexts.contexts[cabac::GREATER2 + increment]) == 1 {
                base[index] = 3;
                escape = true;
            }
        }
        if sps.cabac_bypass_alignment && escape {
            cabac.align_bypass();
        }
        // The signs, as one run of bypass bins, first coefficient first;
        // with sign hiding the lowest position's sign is not sent.
        let (first_sig, last_sig) = (usize::from(found[count - 1]), usize::from(found[0]));
        let sign_hidden = sign_hiding_allowed && last_sig - first_sig > 3;
        let signed = (count - usize::from(sign_hidden)) as u32;
        let signs = cabac.bypass_bits(signed);
        // Remaining levels (9.3.3.11), the Rice parameter adapting.
        let mut rice = if sps.persistent_rice_adaptation {
            u32::from(contexts.stat_coeff[stat_index]) / 4
        } else {
            0
        };
        let mut first_remaining = true;
        let mut sum = 0i32;
        let origin = (sy << 2) * n + (sx << 2);
        for (index, &p) in found.iter().enumerate() {
            let base = base[index];
            let threshold = if index < 8 {
                if Some(index) == first_greater1 { 3 } else { 2 }
            } else {
                1
            };
            let mut level = base;
            if base == threshold {
                let remaining = remaining_level(cabac, rice)?;
                if sps.persistent_rice_adaptation && first_remaining {
                    let stat = &mut contexts.stat_coeff[stat_index];
                    if remaining >= (3 << (u32::from(*stat) / 4)) {
                        *stat += 1;
                    } else if 2 * remaining < (1 << (u32::from(*stat) / 4)) && *stat > 0 {
                        *stat -= 1;
                    }
                }
                first_remaining = false;
                level = base + remaining as i32;
                if level > 3 * (1 << rice) {
                    rice = (rice + 1).min(4);
                }
            }
            sum = sum.wrapping_add(level);
            // The hidden sign is the last coefficient's: odd levels' sum
            // makes it negative.
            let negative = if (index as u32) < signed {
                (signs >> (signed - 1 - index as u32)) & 1 == 1
            } else {
                sum & 1 == 1
            };
            let (px, py) = position_scan[usize::from(p)];
            let (x, y) = ((sx << 2) + usize::from(px), (sy << 2) + usize::from(py));
            rows = rows.max(y + 1);
            columns = columns.max(x + 1);
            let at = origin + usize::from(py) * n + usize::from(px);
            let signed_level = if negative { -level } else { level };
            levels[at] = match &dequant {
                Some(dequant) => {
                    let factor = matrix.map_or(16, |matrix| i64::from(matrix[at]));
                    ((i64::from(signed_level) * factor * dequant.scale + round) >> dequant.shift)
                        .clamp(-32768, 32767) as i32
                }
                None => signed_level,
            };
        }
    }
    Ok(Coded {
        transform_skip,
        rows,
        columns,
    })
}

/// `coeff_abs_level_remaining`: a unary prefix, then the Rice part or,
/// past a prefix of 3, an Exp-Golomb-like escape (9.3.3.11).
fn remaining_level(cabac: &mut Cabac<'_>, rice: u32) -> Result<u32, HevcError> {
    let mut prefix = 0u32;
    while cabac.bypass() == 1 {
        prefix += 1;
        if prefix > 32 {
            return Err(HevcError::new("a coefficient level out of range"));
        }
    }
    if prefix <= 3 {
        Ok((prefix << rice) + cabac.bypass_bits(rice))
    } else {
        let extra = prefix - 3;
        if extra + rice > 31 {
            return Err(HevcError::new("a coefficient level out of range"));
        }
        Ok((((1 << extra) + 2) << rice) + cabac.bypass_bits(extra + rice))
    }
}
