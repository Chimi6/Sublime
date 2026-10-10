//! `residual_coding()` written (H.265 7.3.8.11): the mirror of the
//! decoder's reading, bin for bin, through any `Coder`, so its cost is
//! measured by the syntax that will be written.

use super::super::cabac::{self, Contexts};
use super::super::tables::{Scan, scan};
use super::cabac::Coder;

/// The scan of an intra block (7.4.9.11), as the decoder chooses it.
pub fn scan_kind(log2: u32, luma: bool, mode: u32) -> Scan {
    if log2 == 2 || (log2 == 3 && luma) {
        if (6..=14).contains(&mode) {
            return Scan::Vertical;
        }
        if (22..=30).contains(&mode) {
            return Scan::Horizontal;
        }
    }
    Scan::Diagonal
}

/// The prefix group of a last position, and where each group starts.
const GROUP: [u8; 32] = [
    0, 1, 2, 3, 4, 4, 5, 5, 6, 6, 6, 6, 7, 7, 7, 7, 8, 8, 8, 8, 8, 8, 8, 8, 9, 9, 9, 9, 9, 9, 9, 9,
];
const GROUP_START: [u32; 10] = [0, 1, 2, 3, 4, 6, 8, 12, 16, 24];

/// A transform block to write: its levels (row-major), size, component,
/// intra mode, and whether its transform skip flag is coded and set.
pub struct Block<'a> {
    pub levels: &'a [i32],
    pub log2: u32,
    pub luma: bool,
    pub mode: u32,
    /// `Some(flag)` when the flag is coded (a 4x4 block with transform skip
    /// enabled).
    pub transform_skip: Option<bool>,
    pub sign_hiding: bool,
}

/// Writes a block with at least one nonzero level.
pub fn residual<C: Coder>(coder: &mut C, contexts: &mut Contexts, block: &Block<'_>) {
    let log2 = block.log2;
    let n = 1usize << log2;
    let luma = block.luma;
    let levels = block.levels;
    if let Some(flag) = block.transform_skip {
        coder.decision(
            &mut contexts.contexts[cabac::TRANSFORM_SKIP + usize::from(!luma)],
            u32::from(flag),
        );
    }
    let kind = scan_kind(log2, luma, block.mode);
    let log2_subs = log2 - 2;
    let subs = 1usize << log2_subs;
    let sub_scan = scan(log2_subs, kind);
    let position_scan = scan(2, kind);
    // Each position of a sub-block in scan order, as an offset in the block.
    let mut offsets = [0usize; 16];
    for (offset, &(px, py)) in offsets.iter_mut().zip(position_scan) {
        *offset = usize::from(py) * n + usize::from(px);
    }
    let gather = |sub: usize, values: &mut [i32; 16]| -> bool {
        let (sx, sy) = (usize::from(sub_scan[sub].0), usize::from(sub_scan[sub].1));
        let origin = (sy << 2) * n + (sx << 2);
        let mut any = 0;
        for (value, &offset) in values.iter_mut().zip(&offsets) {
            *value = levels[origin + offset];
            any |= *value;
        }
        any != 0
    };
    // The last significant level in scan order.
    let mut values = [0i32; 16];
    let mut last_sub = sub_scan.len();
    for sub in (0..sub_scan.len()).rev() {
        if gather(sub, &mut values) {
            last_sub = sub;
            break;
        }
    }
    if last_sub == sub_scan.len() {
        return;
    }
    let last_position = (0..16).rev().find(|&p| values[p] != 0).unwrap_or(0);
    let last_x =
        (usize::from(sub_scan[last_sub].0) << 2) + usize::from(position_scan[last_position].0);
    let last_y =
        (usize::from(sub_scan[last_sub].1) << 2) + usize::from(position_scan[last_position].1);
    // The last position (9.3.4.2.3), across and down swapped for a
    // vertical scan.
    let (code_x, code_y) = if kind == Scan::Vertical {
        (last_y, last_x)
    } else {
        (last_x, last_y)
    };
    let (offset, shift) = if luma {
        (
            3 * (log2 as usize - 2) + ((log2 as usize - 1) >> 2),
            (log2 as usize + 1) >> 2,
        )
    } else {
        (15, log2 as usize - 2)
    };
    let largest = (log2 << 1) - 1;
    let prefix_x = u32::from(GROUP[code_x]);
    let prefix_y = u32::from(GROUP[code_y]);
    for (base, prefix) in [(cabac::LAST_X, prefix_x), (cabac::LAST_Y, prefix_y)] {
        for index in 0..prefix {
            coder.decision(
                &mut contexts.contexts[base + offset + ((index as usize) >> shift)],
                1,
            );
        }
        if prefix < largest {
            coder.decision(
                &mut contexts.contexts[base + offset + ((prefix as usize) >> shift)],
                0,
            );
        }
    }
    for (prefix, value) in [(prefix_x, code_x), (prefix_y, code_y)] {
        if prefix > 3 {
            let bits = (prefix >> 1) - 1;
            coder.bypass(value as u32 - GROUP_START[prefix as usize], bits);
        }
    }
    let mut coded_sub = [false; 64];
    let mut greater1_ctx = 1u32;
    for sub in (0..=last_sub).rev() {
        let (sx, sy) = (usize::from(sub_scan[sub].0), usize::from(sub_scan[sub].1));
        let any = sub == last_sub || gather(sub, &mut values);
        let right = sx + 1 < subs && coded_sub[sy * subs + sx + 1];
        let below = sy + 1 < subs && coded_sub[(sy + 1) * subs + sx];
        let mut infer_dc = false;
        if sub < last_sub && sub > 0 {
            let increment = usize::from(right || below) + if luma { 0 } else { 2 };
            coder.decision(
                &mut contexts.contexts[cabac::CODED_SUB_BLOCK + increment],
                u32::from(any),
            );
            coded_sub[sy * subs + sx] = any;
            infer_dc = true;
        } else {
            coded_sub[sy * subs + sx] = true;
        }
        if !coded_sub[sy * subs + sx] {
            continue;
        }
        let pattern = usize::from(right) + 2 * usize::from(below);
        let table = super::super::residual::significance_table(
            log2,
            luma,
            kind,
            pattern,
            sx == 0 && sy == 0,
        );
        // Significance, highest scan position first.
        let mut found = [0u8; 16];
        let mut count = 0usize;
        let start = if sub == last_sub {
            found[0] = last_position as u8;
            count = 1;
            last_position
        } else {
            16
        };
        for p in (0..start).rev() {
            let significant = values[p] != 0;
            if p == 0 && infer_dc {
                // Nothing after the DC was significant: it is, unsent.
                found[count] = 0;
                count += 1;
                continue;
            }
            coder.decision(
                &mut contexts.contexts[cabac::SIG_COEFF + usize::from(table[p])],
                u32::from(significant),
            );
            if significant {
                found[count] = p as u8;
                count += 1;
                infer_dc = false;
            }
        }
        if count == 0 {
            continue;
        }
        let found = &found[..count];
        let magnitude = |index: usize| values[usize::from(found[index])].unsigned_abs();
        // Greater-than-one and greater-than-two flags.
        let mut ctx_set = if sub == 0 || !luma { 0 } else { 2 };
        if sub != last_sub && greater1_ctx == 0 {
            ctx_set += 1;
        }
        greater1_ctx = 1;
        let greater1_base = cabac::GREATER1 + ctx_set * 4 + if luma { 0 } else { 16 };
        let mut first_greater1 = None;
        for index in 0..count.min(8) {
            let increment = greater1_ctx.min(3) as usize;
            let above = magnitude(index) > 1;
            coder.decision(
                &mut contexts.contexts[greater1_base + increment],
                u32::from(above),
            );
            if above {
                greater1_ctx = 0;
                if first_greater1.is_none() {
                    first_greater1 = Some(index);
                }
            } else if greater1_ctx > 0 {
                greater1_ctx += 1;
            }
        }
        if let Some(index) = first_greater1 {
            let increment = ctx_set + if luma { 0 } else { 4 };
            coder.decision(
                &mut contexts.contexts[cabac::GREATER2 + increment],
                u32::from(magnitude(index) > 2),
            );
        }
        // Signs, the lowest position's hidden when sign hiding applies.
        let (first_sig, last_sig) = (usize::from(found[count - 1]), usize::from(found[0]));
        let hidden = block.sign_hiding && last_sig - first_sig > 3;
        let signed = count - usize::from(hidden);
        let mut signs = 0u32;
        for &p in &found[..signed] {
            signs = (signs << 1) | u32::from(values[usize::from(p)] < 0);
        }
        coder.bypass(signs, signed as u32);
        // Remaining levels, the Rice parameter adapting.
        let mut rice = 0u32;
        for index in 0..count {
            let level = magnitude(index);
            // What the flags said (the decoder's base level), and the base
            // at which a remainder follows.
            let (base, threshold) = if index >= 8 {
                (1, 1)
            } else if Some(index) == first_greater1 {
                (level.min(3), 3)
            } else {
                (level.min(2), 2)
            };
            if base == threshold {
                remaining_level(coder, level - base, rice);
                if level > 3 * (1 << rice) {
                    rice = (rice + 1).min(4);
                }
            }
        }
    }
}

/// `coeff_abs_level_remaining` (9.3.3.11), as the decoder reads it: a
/// unary prefix of up to three with a Rice suffix, or an escape.
fn remaining_level<C: Coder>(coder: &mut C, value: u32, rice: u32) {
    if value < (4 << rice) {
        let prefix = value >> rice;
        // `prefix` ones and a zero, then the suffix.
        coder.bypass(((1u32 << prefix) - 1) << 1, prefix + 1);
        coder.bypass(value & ((1 << rice) - 1), rice);
    } else {
        let high = (value >> rice) - 2;
        let extra = 31 - high.leading_zeros();
        let prefix = 3 + extra;
        let suffix = value - (((1 << extra) + 2) << rice);
        // Up to 32 ones and a zero: written in two parts.
        let ones = prefix + 1;
        if ones > 16 {
            coder.bypass(0xFFFF, 16);
            coder.bypass(((1u32 << (ones - 17)) - 1) << 1, ones - 16);
        } else {
            coder.bypass(((1u32 << prefix) - 1) << 1, ones);
        }
        let length = extra + rice;
        if length > 16 {
            coder.bypass(suffix >> 16, length - 16);
            coder.bypass(suffix & 0xFFFF, 16);
        } else {
            coder.bypass(suffix, length);
        }
    }
}
