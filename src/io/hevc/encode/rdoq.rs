//! Rate-distortion optimized quantization (the method of the HM
//! reference encoder): each level is chosen among rounding to nearest,
//! one less, and zero, by its squared error plus lambda times the bits
//! its syntax would take at the current context states; then whole 4x4
//! groups are dropped where that is cheaper, and the last significant
//! position is moved down where the bits saved outweigh the error.

use super::super::cabac::{self, Contexts};
use super::super::tables::{Scan, scan};
use super::cabac::{BYPASS_COST, bin_cost};
use super::quant::{QUANT_SCALE, Quantizer};

/// The costs of the bins residual coding uses, from context states, in
/// 1/32768ths of a bit.
pub struct Rates {
    sig: [[u32; 2]; 71],
    greater1: [[u32; 2]; 24],
    greater2: [[u32; 2]; 6],
    coded_sub: [[u32; 2]; 4],
    last_x: [[u32; 2]; 18],
    last_y: [[u32; 2]; 18],
    /// The coded-block flags of a block at the top of its transform tree:
    /// luma's and chroma's.
    cbf: [[u32; 2]; 2],
    /// The bits of each last position's coordinates across and down, by
    /// block size (4 to 32) and luma or chroma.
    last: [[([u32; 32], [u32; 32]); 2]; 4],
}

impl Rates {
    pub fn of(contexts: &Contexts) -> Rates {
        let costs = |start: usize| -> [u32; 2] {
            let packed = contexts.contexts[start].packed;
            [bin_cost(packed, 0), bin_cost(packed, 1)]
        };
        Rates {
            sig: std::array::from_fn(|index| costs(cabac::SIG_COEFF + index)),
            greater1: std::array::from_fn(|index| costs(cabac::GREATER1 + index)),
            greater2: std::array::from_fn(|index| costs(cabac::GREATER2 + index)),
            coded_sub: std::array::from_fn(|index| costs(cabac::CODED_SUB_BLOCK + index)),
            last_x: std::array::from_fn(|index| costs(cabac::LAST_X + index)),
            last_y: std::array::from_fn(|index| costs(cabac::LAST_Y + index)),
            cbf: [costs(cabac::CBF_LUMA + 1), costs(cabac::CBF_CHROMA)],
            last: [[([0; 32], [0; 32]); 2]; 4],
        }
        .with_last_rates()
    }

    fn with_last_rates(mut self) -> Rates {
        for log2 in 2..=5u32 {
            for luma in [false, true] {
                self.last[log2 as usize - 2][usize::from(luma)] = last_rates(&self, log2, luma);
            }
        }
        self
    }
}

const GROUP: [u8; 32] = [
    0, 1, 2, 3, 4, 4, 5, 5, 6, 6, 6, 6, 7, 7, 7, 7, 8, 8, 8, 8, 8, 8, 8, 8, 9, 9, 9, 9, 9, 9, 9, 9,
];

/// The bits of `coeff_abs_level_remaining` of `value` at Rice parameter
/// `rice`, as `syntax` writes it.
fn remaining_bits(value: u32, rice: u32) -> u32 {
    if value < (4 << rice) {
        (value >> rice) + 1 + rice
    } else {
        let extra = 31 - ((value >> rice) - 2).leading_zeros();
        (3 + extra + 1) + extra + rice
    }
}

/// What the level of a coefficient costs past its significance flag:
/// its greater-than flags, remainder, and sign.
struct LevelRate {
    /// Within the first eight of the group: the greater-than-one context.
    greater1: Option<usize>,
    /// The greater-than-two context, for the group's first level over one.
    greater2: Option<usize>,
    rice: u32,
}

impl LevelRate {
    fn of(&self, level: u32, rates: &Rates) -> u32 {
        let mut bits = BYPASS_COST; // the sign
        match self.greater1 {
            None => bits += remaining_bits(level - 1, self.rice) * BYPASS_COST,
            Some(context) => {
                if level == 1 {
                    bits += rates.greater1[context][0];
                } else {
                    bits += rates.greater1[context][1];
                    match self.greater2 {
                        Some(two) => {
                            if level == 2 {
                                bits += rates.greater2[two][0];
                            } else {
                                bits += rates.greater2[two][1]
                                    + remaining_bits(level - 3, self.rice) * BYPASS_COST;
                            }
                        }
                        None => bits += remaining_bits(level - 2, self.rice) * BYPASS_COST,
                    }
                }
            }
        }
        bits
    }
}

/// The bits of each coordinate of the last position of a block: across
/// (from `last_x`'s contexts) and down (`last_y`'s), as coded.
fn last_rates(rates: &Rates, log2: u32, luma: bool) -> ([u32; 32], [u32; 32]) {
    let n = 1usize << log2;
    let mut across = [0u32; 32];
    let mut down = [0u32; 32];
    for value in 0..n {
        across[value] = last_rate_part(&rates.last_x, log2, luma, value);
        down[value] = last_rate_part(&rates.last_y, log2, luma, value);
    }
    (across, down)
}

/// The bits of one coordinate of a last position.
fn last_rate_part(table: &[[u32; 2]; 18], log2: u32, luma: bool, value: usize) -> u32 {
    let (offset, shift) = if luma {
        (
            3 * (log2 as usize - 2) + ((log2 as usize - 1) >> 2),
            (log2 as usize + 1) >> 2,
        )
    } else {
        (15, log2 as usize - 2)
    };
    let largest = (log2 << 1) - 1;
    let prefix = u32::from(GROUP[value]);
    let mut bits = 0;
    for index in 0..prefix {
        bits += table[offset + ((index as usize) >> shift)][1];
    }
    if prefix < largest {
        bits += table[offset + ((prefix as usize) >> shift)][0];
    }
    if prefix > 3 {
        bits += ((prefix >> 1) - 1) * BYPASS_COST;
    }
    bits
}

/// RDOQ's working arrays, kept between blocks: each position's scaled
/// coefficient, nearest level, chosen level, and costs. Empty until first
/// used, so a placeholder costs nothing.
#[derive(Default)]
pub struct Work {
    scaled: Vec<i64>,
    nearest: Vec<u32>,
    chosen: Vec<u32>,
    coded_cost: Vec<f64>,
    uncoded_cost: Vec<f64>,
    sig_cost: Vec<f64>,
    /// How a level at each position would be coded: its significance
    /// context, greater-than contexts, and Rice parameter (for sign
    /// hiding's costs).
    coding: Vec<Coding>,
}

/// How a level at a scan position is coded.
#[derive(Clone, Copy, Default)]
struct Coding {
    sig: u8,
    greater1: Option<u8>,
    greater2: Option<u8>,
    rice: u8,
}

/// Each block size's and scan's positions in scan order (sub-block by
/// sub-block), as offsets in the block.
fn positions(log2: u32, kind: Scan) -> &'static [u16] {
    static TABLES: std::sync::OnceLock<Vec<Vec<u16>>> = std::sync::OnceLock::new();
    let tables = TABLES.get_or_init(|| {
        let mut tables = Vec::new();
        for log2 in 2..=5u32 {
            for kind in [Scan::Diagonal, Scan::Horizontal, Scan::Vertical] {
                let n = 1usize << log2;
                let sub_scan = scan(log2 - 2, kind);
                let position_scan = scan(2, kind);
                let mut table = Vec::with_capacity(n * n);
                for &(sx, sy) in sub_scan {
                    for &(px, py) in position_scan {
                        let at = (usize::from(sy) * 4 + usize::from(py)) * n
                            + usize::from(sx) * 4
                            + usize::from(px);
                        table.push(at as u16);
                    }
                }
                tables.push(table);
            }
        }
        tables
    });
    let kind = match kind {
        Scan::Diagonal => 0,
        Scan::Horizontal => 1,
        Scan::Vertical => 2,
    };
    &tables[(log2 as usize - 2) * 3 + kind]
}

impl Coding {
    fn rate(&self) -> LevelRate {
        LevelRate {
            greater1: self.greater1.map(usize::from),
            greater2: self.greater2.map(usize::from),
            rice: u32::from(self.rice),
        }
    }
}

/// A block to quantize.
pub struct Block<'a> {
    pub coefficients: &'a [i32],
    pub log2: u32,
    pub luma: bool,
    pub kind: Scan,
    pub quantizer: Quantizer,
    /// Per 1/32768th of a bit, in squared samples.
    pub lambda: f64,
    /// Signs are hidden in 4x4 groups (the parity is set here, by cost).
    pub sign_hiding: bool,
}

/// Quantizes `block` into `levels` (row-major), its signs hidden where
/// `block.sign_hiding` says.
#[inline(never)]
pub fn quantize(block: &Block<'_>, rates: &Rates, work: &mut Work, levels: &mut [i32]) {
    if work.scaled.len() < 1024 {
        work.scaled.resize(1024, 0);
        work.nearest.resize(1024, 0);
        work.chosen.resize(1024, 0);
        work.coded_cost.resize(1024, 0.0);
        work.uncoded_cost.resize(1024, 0.0);
        work.sig_cost.resize(1024, 0.0);
        work.coding.resize(1024, Coding::default());
    }
    let Work {
        scaled,
        nearest,
        chosen,
        coded_cost,
        uncoded_cost,
        sig_cost,
        coding,
    } = work;
    let log2 = block.log2;
    let n = 1usize << log2;
    let area = n * n;
    let quantizer = block.quantizer;
    let qbits = 14 + quantizer.qp / 6 + (15 - quantizer.bit_depth as i32 - log2 as i32);
    let scale = QUANT_SCALE[(quantizer.qp % 6) as usize];
    // Squared error in the coefficient domain, scaled to samples.
    let transform_shift = 15 - quantizer.bit_depth as i32 - log2 as i32;
    let error_scale = 1.0 / ((scale as f64) * (scale as f64) * 2f64.powi(2 * transform_shift));
    let lambda = block.lambda;
    let log2_subs = log2 - 2;
    let subs = 1usize << log2_subs;
    let sub_scan = scan(log2_subs, block.kind);
    // Scan index to position in the block.
    let table = positions(log2, block.kind);
    let position = |s: usize| -> usize { usize::from(table[s]) };
    let mut last = None;
    for s in 0..area {
        let coefficient = block.coefficients[position(s)];
        let value = i64::from(coefficient.unsigned_abs()) * scale;
        scaled[s] = value;
        let level = ((value + (1i64 << (qbits - 1))) >> qbits).min(32767) as u32;
        nearest[s] = level;
        if level > 0 {
            last = Some(s);
        }
    }
    levels[..area].fill(0);
    let Some(last_scan) = last else {
        return;
    };
    let distortion = |s: usize, level: u32| -> f64 {
        let difference = (scaled[s] - (i64::from(level) << qbits)) as f64;
        difference * difference * error_scale
    };
    // Per scan position (from here on written before read): the level
    // chosen, the cost coded as chosen (with its significance flag),
    // uncoded, and of its significance flag alone.
    let mut coded_sub = [false; 64];
    let last_sub = last_scan >> 4;
    let mut greater1_ctx_carry = 1u32;
    for sub in (0..=last_sub).rev() {
        let (sx, sy) = (usize::from(sub_scan[sub].0), usize::from(sub_scan[sub].1));
        let right = sx + 1 < subs && coded_sub[sy * subs + sx + 1];
        let below = sy + 1 < subs && coded_sub[(sy + 1) * subs + sx];
        let pattern = usize::from(right) + 2 * usize::from(below);
        let table = super::super::residual::significance_table(
            log2,
            block.luma,
            block.kind,
            pattern,
            sx == 0 && sy == 0,
        );
        let mut ctx_set = if sub == 0 || !block.luma { 0 } else { 2 };
        if sub != last_sub && greater1_ctx_carry == 0 {
            ctx_set += 1;
        }
        let greater1_base = ctx_set * 4 + if block.luma { 0 } else { 16 };
        let greater2_context = ctx_set + if block.luma { 0 } else { 4 };
        let mut greater1_ctx = 1u32;
        let mut count = 0usize;
        let mut first_greater1_seen = false;
        let mut rice = 0u32;
        let top = if sub == last_sub { last_scan & 15 } else { 15 };
        for p in (0..=top).rev() {
            let s = sub * 16 + p;
            uncoded_cost[s] = distortion(s, 0);
            let nearest_level = nearest[s];
            let is_last = s == last_scan;
            let context = usize::from(table[p]);
            let sig = &rates.sig[context];
            coding[s] = Coding {
                sig: table[p],
                greater1: (count < 8).then(|| (greater1_base + greater1_ctx.min(3) as usize) as u8),
                greater2: (count < 8 && !first_greater1_seen).then_some(greater2_context as u8),
                rice: rice as u8,
            };
            if nearest_level == 0 && !is_last {
                chosen[s] = 0;
                coded_cost[s] = uncoded_cost[s] + lambda * f64::from(sig[0]);
                sig_cost[s] = lambda * f64::from(sig[0]);
                continue;
            }
            let rate = LevelRate {
                greater1: (count < 8).then(|| greater1_base + greater1_ctx.min(3) as usize),
                greater2: (count < 8 && !first_greater1_seen).then_some(greater2_context),
                rice,
            };
            // Candidates: nearest, one less, and (unless the forced last)
            // zero.
            let mut best_level = 0;
            let mut best_cost = if is_last {
                f64::INFINITY
            } else {
                uncoded_cost[s] + lambda * f64::from(sig[0])
            };
            let sig_one = if is_last {
                0.0
            } else {
                lambda * f64::from(sig[1])
            };
            let low = nearest_level.saturating_sub(1).max(1);
            for level in low..=nearest_level {
                let cost =
                    distortion(s, level) + lambda * f64::from(rate.of(level, rates)) + sig_one;
                if cost < best_cost {
                    best_cost = cost;
                    best_level = level;
                }
            }
            chosen[s] = best_level;
            coded_cost[s] = best_cost;
            sig_cost[s] = if best_level > 0 {
                sig_one
            } else {
                lambda * f64::from(sig[0])
            };
            if best_level > 0 {
                // The states the next level is coded with.
                if count < 8 {
                    if best_level > 1 {
                        greater1_ctx = 0;
                        first_greater1_seen = true;
                    } else if greater1_ctx > 0 {
                        greater1_ctx += 1;
                    }
                }
                let threshold = if count >= 8 {
                    1
                } else if rate.greater2.is_some() && best_level > 1 {
                    3
                } else {
                    2
                };
                if best_level >= threshold && best_level > 3 * (1 << rice) {
                    rice = (rice + 1).min(4);
                }
                count += 1;
            }
        }
        // The whole group dropped, where that is cheaper (not the first
        // group, whose flag is not coded, nor the last).
        let any = (0..=top).any(|p| chosen[sub * 16 + p] != 0);
        if any && sub != 0 && sub != last_sub {
            let flag = usize::from(right || below) + if block.luma { 0 } else { 2 };
            let coded: f64 = (0..16).map(|p| coded_cost[sub * 16 + p]).sum::<f64>()
                + lambda * f64::from(rates.coded_sub[flag][1]);
            let dropped: f64 = (0..16).map(|p| uncoded_cost[sub * 16 + p]).sum::<f64>()
                + lambda * f64::from(rates.coded_sub[flag][0]);
            if dropped < coded {
                for p in 0..16 {
                    let s = sub * 16 + p;
                    chosen[s] = 0;
                    coded_cost[s] = uncoded_cost[s];
                    sig_cost[s] = 0.0;
                }
            }
        }
        coded_sub[sy * subs + sx] =
            sub == 0 || sub == last_sub || (0..16).any(|p| chosen[sub * 16 + p] != 0);
        if (0..=top).any(|p| chosen[sub * 16 + p] != 0) {
            greater1_ctx_carry = greater1_ctx;
        }
    }
    // The last position: where it ends cheapest.
    let (across, down) = &rates.last[log2 as usize - 2][usize::from(block.luma)];
    let mut below_cost: f64 = (0..last_scan).map(|s| coded_cost[s]).sum();
    let mut above_uncoded = 0.0;
    let mut best: Option<(f64, usize)> = None;
    for s in (0..=last_scan).rev() {
        if chosen[s] != 0 {
            let p = position(s);
            let (x, y) = (p % n, p / n);
            let (code_x, code_y) = if block.kind == Scan::Vertical {
                (y, x)
            } else {
                (x, y)
            };
            let total = below_cost
                + (coded_cost[s] - sig_cost[s])
                + lambda * f64::from(across[code_x] + down[code_y])
                + above_uncoded;
            if best.is_none_or(|(cost, _)| total < cost) {
                best = Some((total, s));
            }
        }
        if s > 0 {
            below_cost -= coded_cost[s - 1];
        }
        above_uncoded += uncoded_cost[s];
    }
    // Or nothing coded at all, when the block's flag and levels cost more
    // than they save.
    let cbf = &rates.cbf[usize::from(!block.luma)];
    let nothing: f64 =
        (0..=last_scan).map(|s| uncoded_cost[s]).sum::<f64>() + lambda * f64::from(cbf[0]);
    let best = best.filter(|&(cost, _)| cost + lambda * f64::from(cbf[1]) < nothing);
    let Some((_, end)) = best else {
        return;
    };
    for (s, level) in chosen
        .iter_mut()
        .enumerate()
        .take(last_scan + 1)
        .skip(end + 1)
    {
        let _ = s;
        *level = 0;
    }
    if block.sign_hiding {
        // Sign hiding (HM's, after RDOQ): where a group's parity
        // disagrees with its first level's sign, the level whose change by
        // one costs least in error and bits moves.
        for sub in 0..=(end >> 4) {
            let in_group = |p: usize| sub * 16 + p;
            let top = if sub == end >> 4 { end & 15 } else { 15 };
            let (Some(first), Some(last)) = (
                (0..=top).find(|&p| chosen[in_group(p)] != 0),
                (0..=top).rev().find(|&p| chosen[in_group(p)] != 0),
            ) else {
                continue;
            };
            if last - first <= 3 {
                continue;
            }
            let sum: u32 = (0..=top).map(|p| chosen[in_group(p)]).sum();
            let sign = u32::from(block.coefficients[position(in_group(first))] < 0);
            if sign == (sum & 1) {
                continue;
            }
            let mut best_change: Option<(f64, usize, i32)> = None;
            for p in 0..=top {
                let s = in_group(p);
                let level = chosen[s];
                let info = coding[s];
                let rate = info.rate();
                let sig = &rates.sig[usize::from(info.sig)];
                let bits_of = |level: u32| -> f64 {
                    if level == 0 {
                        f64::from(sig[0])
                    } else {
                        f64::from(sig[1]) + f64::from(rate.of(level, rates))
                    }
                };
                let delta = |to: u32| -> f64 {
                    distortion(s, to) - distortion(s, level)
                        + lambda * (bits_of(to) - bits_of(level))
                };
                let mut consider = |cost: f64, change: i32| {
                    if best_change.is_none_or(|(best, _, _)| cost < best) {
                        best_change = Some((cost, s, change));
                    }
                };
                if level > 0 {
                    if level < 32767 {
                        consider(delta(level + 1), 1);
                    }
                    // Not the group's first to zero (the hidden sign moves),
                    // nor the block's last (its position is coded).
                    if !(level == 1 && (p == first || s == end)) {
                        consider(delta(level - 1), -1);
                    }
                } else if p > first || i32::from(block.coefficients[position(s)] < 0) == sign as i32
                {
                    // A new first level must carry the sign the parity says.
                    consider(delta(1), 1);
                }
            }
            if let Some((_, s, change)) = best_change {
                chosen[s] = (chosen[s] as i32 + change) as u32;
            }
        }
    }
    // The levels, with their coefficients' signs (zeros past the last
    // were filled at the start).
    for (s, &level) in chosen.iter().enumerate().take(end + 1) {
        let at = position(s);
        levels[at] = if block.coefficients[at] < 0 {
            -(level as i32)
        } else {
            level as i32
        };
    }
}
