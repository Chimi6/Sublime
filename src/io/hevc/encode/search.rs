//! The coding decisions of a CTB, by rate and distortion: the CU
//! quadtree, each CU's intra modes (a SATD pass over all 35, then the
//! best few coded for real), its transform tree, and its chroma mode;
//! each choice priced by the syntax that would be written, through the
//! estimator, from the context states it would be written with.

use super::super::cabac::{self, Contexts};
use super::super::intra::{self, References};
use super::Settings;
use super::cabac::{Coder, Estimator};
use super::quant::{self, Quantizer};
use super::syntax::{self, Block};

/// A plane of samples, at most 16 bits.
pub struct Plane {
    pub samples: Vec<u16>,
    pub width: usize,
    pub height: usize,
}

impl Plane {
    pub fn new(width: usize, height: usize) -> Plane {
        Plane {
            samples: vec![0; width * height],
            width,
            height,
        }
    }

    #[inline(always)]
    fn at(&self, x: usize, y: usize) -> i32 {
        i32::from(self.samples[y * self.width + x])
    }
}

/// A coded block's levels and how they were coded.
#[derive(Clone)]
pub struct Coded {
    pub levels: Vec<i32>,
    pub transform_skip: bool,
}

/// A transform tree: leaves hold their luma block and, where chroma lies
/// with them, their Cb and Cr blocks.
#[derive(Clone)]
pub enum Tree {
    Leaf {
        luma: Option<Coded>,
        chroma: [Option<Coded>; 2],
    },
    Split(Box<[Tree; 4]>),
}

/// A coding unit's decisions.
#[derive(Clone)]
pub struct Cu {
    pub x: usize,
    pub y: usize,
    pub log2: u32,
    /// Four 4x4 prediction blocks (an 8x8 CU only).
    pub split_intra: bool,
    pub luma_modes: [u32; 4],
    /// The chroma mode's code: 0 to 3 a fixed mode, 4 the luma mode.
    pub chroma_code: u32,
    pub chroma_mode: u32,
    pub tree: Tree,
}

/// A CTB's decisions in coding order: the split flags of its quadtree
/// (each node's, depth first) and its coding units.
#[derive(Clone, Default)]
pub struct CtbDecisions {
    pub splits: Vec<bool>,
    pub cus: Vec<Cu>,
}

/// Per 4x4 luma unit: the CU depth (for split contexts) and luma mode
/// (for most probable modes).
pub struct Units {
    pub depth: Vec<u8>,
    pub mode: Vec<u8>,
    /// Each CU's QpY, as the decoder derives it (for QP prediction).
    pub qp: Vec<i8>,
    pub wide: usize,
}

/// What a row hears of the row above: the reconstruction's last line of
/// each component and the CU depths of its last units, as far as the
/// row above has gone.
pub struct Edge {
    pub lines: [Vec<u16>; 3],
    pub depth: Vec<u8>,
}

impl Edge {
    pub fn new(settings: &Settings) -> Edge {
        let width = settings.coded_width;
        Edge {
            lines: [vec![0; width], vec![0; width / 2], vec![0; width / 2]],
            depth: vec![0; width / 4],
        }
    }
}

/// The scratch buffers of coding a block, kept between blocks.
#[derive(Default)]
pub struct Scratch {
    prediction: Vec<i32>,
    residual: Vec<i32>,
    coefficients: Vec<i32>,
    levels: Vec<i32>,
    errors: Vec<i32>,
    back: Vec<i32>,
    recon: Vec<i32>,
    best: Vec<i32>,
    middle: Vec<i32>,
    rdoq: super::rdoq::Work,
}

/// One CTB row being coded: the picture's source, the row's band of the
/// reconstruction and of the units (from luma row `top`), and the edge
/// of the row above.
pub struct Picture<'s> {
    pub settings: &'s Settings,
    pub source: &'s [Plane; 3],
    pub qp_map: &'s super::aq::QpMap,
    pub top: usize,
    pub recon: [Plane; 3],
    pub units: Units,
    pub above: Option<Edge>,
    pub scratch: Scratch,
    /// The QP the CTB in hand is coded at, and its level.
    pub qp: i32,
    pub level: Level,
    /// The contexts at the CU in hand's start, and their bins' costs:
    /// what its blocks are priced with.
    pub block_contexts: Option<Contexts>,
    pub rates: Option<super::rdoq::Rates>,
    /// Whether 4x4 blocks try transform skip now (only once a mode is
    /// chosen, when `Rd::skip_last`).
    pub skip_trials: bool,
    /// The references of the block whose modes are being tried: the same
    /// for each mode, as nothing outside the block changes meanwhile.
    pub neighbours: Option<((usize, usize, usize, usize), Neighbours)>,
}

const MODES: u32 = 35;

/// A unit's column or row in its CTB with a zero between each bit: half
/// of a z-scan order (CTBs are at most 16 units across).
const SPREAD: [usize; 16] = [0, 1, 4, 5, 16, 17, 20, 21, 64, 65, 68, 69, 80, 81, 84, 85];

impl Picture<'_> {
    fn ctb_log2(&self) -> u32 {
        self.settings.ctb_log2
    }

    /// The coding order of the 4x4 unit at luma (x, y): its CTB in raster
    /// order, then its place in the CTB's z-scan.
    fn z_order(&self, x: usize, y: usize) -> usize {
        let log2 = self.ctb_log2();
        let wide = self.settings.coded_width.div_ceil(1 << log2);
        let ctb = (y >> log2) * wide + (x >> log2);
        let (ux, uy) = ((x & ((1 << log2) - 1)) >> 2, (y & ((1 << log2) - 1)) >> 2);
        let morton = SPREAD[ux] | (SPREAD[uy] << 1);
        (ctb << (2 * (log2 - 2))) | morton
    }

    /// Whether luma (nx, ny) is coded before the block at (x, y).
    fn available(&self, x: usize, y: usize, nx: isize, ny: isize) -> bool {
        if nx < 0 || ny < 0 {
            return false;
        }
        let (nx, ny) = (nx as usize, ny as usize);
        nx < self.settings.coded_width
            && ny < self.settings.coded_height
            && self.z_order(nx, ny) < self.z_order(x, y)
    }

    /// A unit of the band (never one above it).
    fn unit_index(&self, x: usize, y: usize) -> usize {
        ((y - self.top) >> 2) * self.units.wide + (x >> 2)
    }

    /// The CU depth of the unit at luma (x, y): the band's, or the edge's.
    fn depth_at(&self, x: usize, y: usize) -> u8 {
        if y >= self.top {
            self.units.depth[self.unit_index(x, y)]
        } else {
            self.above.as_ref().map_or(0, |edge| edge.depth[x >> 2])
        }
    }

    /// A reconstructed sample of `component` at (x, y) in its samples: the
    /// band's, or the edge's line above it.
    #[inline(always)]
    fn recon_at(&self, component: usize, x: usize, y: usize) -> i32 {
        let top = self.top >> usize::from(component > 0);
        if y >= top {
            self.recon[component].at(x, y - top)
        } else {
            self.above
                .as_ref()
                .map_or(0, |edge| i32::from(edge.lines[component][x]))
        }
    }

    fn band_top(&self, component: usize) -> usize {
        self.top >> usize::from(component > 0)
    }

    /// The three most probable luma modes of a block at (x, y) (8.4.2).
    pub fn most_probable_modes(&self, x: usize, y: usize) -> [u32; 3] {
        let ctb_top = (y >> self.ctb_log2()) << self.ctb_log2();
        let neighbour = |nx: isize, ny: isize, above: bool| -> u32 {
            if !self.available(x, y, nx, ny) || (above && (ny as usize) < ctb_top) {
                return 1;
            }
            u32::from(self.units.mode[self.unit_index(nx as usize, ny as usize)])
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

    /// The split flag's context increment at (x, y) for `depth`.
    pub fn split_context(&self, x: usize, y: usize, depth: u32) -> usize {
        let mut increment = 0;
        if self.available(x, y, x as isize - 1, y as isize)
            && u32::from(self.depth_at(x - 1, y)) > depth
        {
            increment += 1;
        }
        if self.available(x, y, x as isize, y as isize - 1)
            && u32::from(self.depth_at(x, y - 1)) > depth
        {
            increment += 1;
        }
        increment
    }

    /// The reference samples of an `n` by `n` block of `component` at
    /// (x, y) in its samples, from the reconstruction (8.4.4.2.2).
    fn references(&self, component: usize, x: usize, y: usize, n: usize) -> References {
        let shift = usize::from(component > 0);
        let plane = &self.recon[component];
        let (luma_x, luma_y) = (x << shift, y << shift);
        let plane_height = self.settings.coded_height >> shift;
        let mut line = [0i32; intra::LINE];
        let mut known = [false; intra::LINE];
        // Availability a 4x4 luma unit at a time: 4 samples, or 2 chroma.
        let step = 4 >> shift;
        let usable = |sx: isize, sy: isize| -> bool {
            sx >= 0
                && sy >= 0
                && (sx as usize) < plane.width
                && (sy as usize) < plane_height
                && self.available(luma_x, luma_y, sx << shift, sy << shift)
        };
        // The left column, bottom up: line[0] is p[-1][2n-1].
        let mut k = 0;
        while k < 2 * n {
            let sy = (y + 2 * n - 1 - k) as isize;
            let sx = x as isize - 1;
            // The unit holding this sample covers `step` rows.
            let ok = usable(sx, sy);
            for j in 0..step.min(2 * n - k) {
                let row = y + 2 * n - 1 - (k + j);
                known[k + j] = ok;
                if ok {
                    line[k + j] = self.recon_at(component, x - 1, row);
                }
            }
            k += step;
        }
        // The corner.
        if usable(x as isize - 1, y as isize - 1) {
            known[2 * n] = true;
            line[2 * n] = self.recon_at(component, x - 1, y - 1);
        }
        // The top row.
        let mut k = 0;
        while k < 2 * n {
            let sx = (x + k) as isize;
            let ok = usable(sx, y as isize - 1);
            for j in 0..step.min(2 * n - k) {
                known[2 * n + 1 + k + j] = ok;
                if ok {
                    line[2 * n + 1 + k + j] = self.recon_at(component, x + k + j, y - 1);
                }
            }
            k += step;
        }
        References::substitute(n, self.settings.bit_depth, &mut line, &known)
    }
}

/// A block's references, unfiltered and filtered, for trying modes.
pub struct Neighbours {
    plain: References,
    filtered: References,
}

impl Clone for Neighbours {
    fn clone(&self) -> Neighbours {
        let copy = |references: &References| References {
            corner: references.corner,
            top: references.top,
            left: references.left,
        };
        Neighbours {
            plain: copy(&self.plain),
            filtered: copy(&self.filtered),
        }
    }
}

impl Neighbours {
    fn of(picture: &Picture<'_>, component: usize, x: usize, y: usize, n: usize) -> Neighbours {
        let plain = picture.references(component, x, y, n);
        let mut filtered = References {
            corner: plain.corner,
            top: plain.top,
            left: plain.left,
        };
        // Only luma above 4x4 is ever predicted from filtered samples.
        if component == 0 && n > 4 {
            filtered.filter(n, true, picture.settings.bit_depth);
        }
        Neighbours { plain, filtered }
    }

    fn predict(&self, component: usize, mode: u32, n: usize, bit_depth: u32, out: &mut [i32]) {
        let references = if component == 0 && intra::filters(mode, n) {
            &self.filtered
        } else {
            &self.plain
        };
        intra::predict(
            references,
            mode,
            n,
            component == 0 && n < 32,
            bit_depth,
            out,
        );
    }
}

/// The sum of absolute Hadamard-transformed differences, by 8x8 blocks
/// (4x4 for a 4x4 block): the usual estimate of a residual's cost.
fn satd(source: &[i32], prediction: &[i32], n: usize) -> u32 {
    let mut total = 0u32;
    if n == 4 {
        let mut d = [0i32; 16];
        for (index, value) in d.iter_mut().enumerate() {
            *value = source[index] - prediction[index];
        }
        return hadamard4(&d);
    }
    for by in (0..n).step_by(8) {
        for bx in (0..n).step_by(8) {
            let mut d = [0i32; 64];
            for y in 0..8 {
                for x in 0..8 {
                    let at = (by + y) * n + bx + x;
                    d[y * 8 + x] = source[at] - prediction[at];
                }
            }
            total += hadamard8(&d);
        }
    }
    total
}

fn hadamard4(d: &[i32; 16]) -> u32 {
    let mut m = [0i32; 16];
    for y in 0..4 {
        let row = &d[y * 4..y * 4 + 4];
        let (a, b) = (row[0] + row[1], row[0] - row[1]);
        let (c, e) = (row[2] + row[3], row[2] - row[3]);
        m[y * 4] = a + c;
        m[y * 4 + 1] = b + e;
        m[y * 4 + 2] = a - c;
        m[y * 4 + 3] = b - e;
    }
    let mut sum = 0u32;
    for x in 0..4 {
        let (a, b) = (m[x] + m[4 + x], m[x] - m[4 + x]);
        let (c, e) = (m[8 + x] + m[12 + x], m[8 + x] - m[12 + x]);
        sum += ((a + c).abs() + (b + e).abs() + (a - c).abs() + (b - e).abs()) as u32;
    }
    (sum + 1) >> 1
}

fn hadamard8(d: &[i32; 64]) -> u32 {
    let mut m = *d;
    for row in m.chunks_exact_mut(8) {
        butterfly8(row);
    }
    let mut column = [0i32; 8];
    let mut sum = 0u32;
    for x in 0..8 {
        for y in 0..8 {
            column[y] = m[y * 8 + x];
        }
        butterfly8(&mut column);
        sum += column.iter().map(|value| value.unsigned_abs()).sum::<u32>();
    }
    (sum + 2) >> 2
}

fn butterfly8(v: &mut [i32]) {
    for step in [4usize, 2, 1] {
        for start in (0..8).step_by(step * 2) {
            for i in start..start + step {
                let (a, b) = (v[i], v[i + step]);
                v[i] = a + b;
                v[i + step] = a - b;
            }
        }
    }
}

/// A rectangle of a plane saved, to put back when a choice loses.
struct Saved {
    samples: Vec<u16>,
}

impl Picture<'_> {
    /// Saves an `n` by `n` block of `component`'s reconstruction at (x, y)
    /// in its samples.
    fn save(&self, component: usize, x: usize, y: usize, n: usize) -> Saved {
        let plane = &self.recon[component];
        let y = y - self.band_top(component);
        let mut samples = Vec::with_capacity(n * n);
        for row in y..y + n {
            let start = row * plane.width + x;
            samples.extend_from_slice(&plane.samples[start..start + n]);
        }
        Saved { samples }
    }

    fn restore(&mut self, component: usize, x: usize, y: usize, n: usize, saved: &Saved) {
        let y = y - self.band_top(component);
        let plane = &mut self.recon[component];
        for (index, row) in (y..y + n).enumerate() {
            let start = row * plane.width + x;
            plane.samples[start..start + n]
                .copy_from_slice(&saved.samples[index * n..(index + 1) * n]);
        }
    }
}

/// What coding at one QP takes: the Lagrange multipliers and the
/// quantizers of luma and chroma.
#[derive(Clone, Copy)]
pub struct Level {
    /// For squared error against bits (in 1/32768ths of a bit).
    lambda: f64,
    /// For SATD against whole bits.
    lambda_satd: f64,
    luma: Quantizer,
    chroma: Quantizer,
}

impl Level {
    fn new(settings: &Settings, qp: i32) -> Level {
        let depth_scale = f64::from(1u32 << (2 * (settings.bit_depth - 8)));
        let lambda = 0.57 * 2f64.powf(f64::from(qp - 12) / 3.0) * depth_scale;
        let offset = 6 * (settings.bit_depth as i32 - 8);
        let chroma_qp =
            super::super::decode::chroma_qp((qp + settings.chroma_qp_offset).clamp(-offset, 57), 1);
        Level {
            lambda: lambda / 32768.0,
            lambda_satd: lambda.sqrt(),
            luma: Quantizer {
                qp: qp + offset,
                bit_depth: settings.bit_depth,
            },
            chroma: Quantizer {
                qp: chroma_qp + offset,
                bit_depth: settings.bit_depth,
            },
        }
    }

    fn cost(&self, distortion: u64, bits: u64) -> f64 {
        distortion as f64 + self.lambda * bits as f64
    }
}

/// What a transform tree is searched against: the cost at which its
/// result no longer matters, and the error and bits already spent outside
/// it. A tree that reaches the limit is given up (its result then costs
/// at least the limit), which leaves every choice as it was.
#[derive(Clone, Copy)]
struct Bound {
    limit: f64,
    distortion: u64,
    bits: u64,
}

impl Bound {
    const NONE: Bound = Bound {
        limit: f64::INFINITY,
        distortion: 0,
        bits: 0,
    };
}

/// Modes taken to the full coding after the SATD pass, by block size 4
/// to 32 (more than HM's 8, 8, 3, 3 for the larger blocks).
const CANDIDATES: [usize; 4] = [8, 8, 6, 6];

/// The search's constants: each QP's level.
pub struct Rd {
    levels: Vec<Level>,
    /// The contexts at the slice's start: estimates without a history.
    initial: Contexts,
    /// Their bins' costs, for quantizing a block before its CU's own.
    rates: super::rdoq::Rates,
}

impl Rd {
    pub fn new(settings: &Settings) -> Rd {
        let initial = Contexts::new(settings.qp);
        Rd {
            levels: (0..=51).map(|qp| Level::new(settings, qp)).collect(),
            rates: super::rdoq::Rates::of(&initial),
            initial,
        }
    }

    pub fn level(&self, qp: i32) -> Level {
        self.levels[qp.clamp(0, 51) as usize]
    }
}

/// A transform block coded: its levels, reconstruction placed, squared
/// error, and whether anything is coded.
struct BlockResult {
    coded: Option<Coded>,
    distortion: u64,
}

impl Picture<'_> {
    /// Predicts, transforms, quantizes, and reconstructs one block of
    /// `component` (in its samples), leaving the reconstruction in place.
    #[allow(clippy::too_many_arguments)]
    fn code_block(
        &mut self,
        rd: &Rd,
        component: usize,
        x: usize,
        y: usize,
        log2: u32,
        mode: u32,
        try_skip: bool,
    ) -> BlockResult {
        let n = 1usize << log2;
        let area = n * n;
        let bit_depth = self.settings.bit_depth;
        let mut scratch = std::mem::take(&mut self.scratch);
        for buffer in [
            &mut scratch.prediction,
            &mut scratch.residual,
            &mut scratch.coefficients,
            &mut scratch.levels,
            &mut scratch.errors,
            &mut scratch.back,
            &mut scratch.recon,
            &mut scratch.best,
            &mut scratch.middle,
        ] {
            buffer.resize(32 * 32, 0);
        }
        let key = (component, x, y, n);
        let cached = self
            .neighbours
            .as_ref()
            .filter(|(cached, _)| *cached == key)
            .map(|(_, neighbours)| neighbours.clone());
        let neighbours = cached.unwrap_or_else(|| Neighbours::of(self, component, x, y, n));
        neighbours.predict(
            component,
            mode,
            n,
            bit_depth,
            &mut scratch.prediction[..area],
        );
        let source = &self.source[component];
        for row in 0..n {
            let start = (y + row) * source.width + x;
            for ((out, &sample), &predicted) in scratch.residual[row * n..(row + 1) * n]
                .iter_mut()
                .zip(&source.samples[start..start + n])
                .zip(&scratch.prediction[row * n..(row + 1) * n])
            {
                *out = i32::from(sample) - predicted;
            }
        }
        let quantizer = if component == 0 {
            self.level.luma
        } else {
            self.level.chroma
        };
        let dst = component == 0 && n == 4;
        let max = (1i32 << bit_depth) - 1;
        let mut best: Option<(f64, Option<Coded>, u64, u64)> = None;
        for skip in [false, true] {
            if skip && !(try_skip && n == 4) {
                continue;
            }
            if skip {
                quant::forward_skip(
                    &scratch.residual,
                    &mut scratch.coefficients,
                    log2,
                    bit_depth,
                );
            } else {
                quant::forward_with(
                    &scratch.residual[..area],
                    &mut scratch.coefficients,
                    &mut scratch.middle[..area],
                    log2,
                    dst,
                    bit_depth,
                );
            }
            let levels = &mut scratch.levels[..area];
            let rates = self.rates.as_ref().unwrap_or(&rd.rates);
            super::rdoq::quantize(
                &super::rdoq::Block {
                    coefficients: &scratch.coefficients[..area],
                    log2,
                    luma: component == 0,
                    kind: syntax::scan_kind(log2, component == 0, mode),
                    quantizer,
                    lambda: self.level.lambda,
                    sign_hiding: self.settings.sign_hiding,
                },
                rates,
                &mut scratch.rdoq,
                levels,
            );
            let any = levels.iter().any(|&level| level != 0);
            if any {
                quantizer.reconstruct(levels, &mut scratch.back[..area], log2, dst, skip);
            } else {
                scratch.back[..area].fill(0);
            }
            let mut distortion = 0u64;
            for (((recon, &predicted), &back), &residual) in scratch.recon[..area]
                .iter_mut()
                .zip(&scratch.prediction[..area])
                .zip(&scratch.back[..area])
                .zip(&scratch.residual[..area])
            {
                let value = (predicted + back).clamp(0, max);
                *recon = value;
                // Under 2^16 in magnitude: its square fits 32 bits unsigned.
                let d = residual + predicted - value;
                distortion += u64::from((d * d) as u32);
            }
            // Bits matter here only to choose between transform skip and
            // not; otherwise the caller prices the levels.
            let choosing = try_skip && n == 4;
            let bits = if choosing && any {
                let mut estimator = Estimator::default();
                let mut contexts = self
                    .block_contexts
                    .clone()
                    .unwrap_or_else(|| rd.initial.clone());
                syntax::residual(
                    &mut estimator,
                    &mut contexts,
                    &Block {
                        levels,
                        log2,
                        luma: component == 0,
                        mode,
                        transform_skip: (self.settings.transform_skip && n == 4).then_some(skip),
                        sign_hiding: self.settings.sign_hiding,
                    },
                );
                estimator.bits
            } else {
                0
            };
            let cost = self.level.cost(distortion, bits);
            if best
                .as_ref()
                .is_none_or(|(best_cost, _, _, _)| cost < *best_cost)
            {
                let coded = any.then(|| Coded {
                    levels: levels.to_vec(),
                    transform_skip: skip,
                });
                best = Some((cost, coded, distortion, bits));
                std::mem::swap(&mut scratch.best, &mut scratch.recon);
            }
        }
        let (_, coded, distortion, _) = best.expect("one way was tried");
        let top = self.band_top(component);
        let plane = &mut self.recon[component];
        for row in 0..n {
            let start = (y - top + row) * plane.width + x;
            for (sample, &value) in plane.samples[start..start + n]
                .iter_mut()
                .zip(&scratch.best[row * n..(row + 1) * n])
            {
                *sample = value as u16;
            }
        }
        self.scratch = scratch;
        BlockResult { coded, distortion }
    }
}

/// What coding a CU costs and left: its decisions, and the contexts after.
pub struct Outcome {
    pub cost: f64,
    pub contexts: Contexts,
}

impl Picture<'_> {
    /// What the row below hears of CTB `ctb_x`: its last line of each
    /// component and its last units' depths.
    pub fn bottom_of(&self, ctb_x: usize) -> super::RowUpdate {
        let size = 1usize << self.ctb_log2();
        let lines = std::array::from_fn(|component| {
            let plane = &self.recon[component];
            let shift = usize::from(component > 0);
            let (start, width) = (
                (ctb_x * size) >> shift,
                (size >> shift).min(plane.width - ((ctb_x * size) >> shift)),
            );
            let row = (plane.samples.len() / plane.width.max(1)).saturating_sub(1);
            plane.samples[row * plane.width + start..row * plane.width + start + width].to_vec()
        });
        let unit_rows = self.units.depth.len() / self.units.wide;
        let (start, count) = (
            ctb_x * size / 4,
            (size / 4).min(self.units.wide - ctb_x * size / 4),
        );
        let row = (unit_rows - 1) * self.units.wide;
        super::RowUpdate::Ctb {
            ctb_x,
            lines,
            depth: self.units.depth[row + start..row + start + count].to_vec(),
        }
    }

    /// Chooses a CTB's coding, leaving its reconstruction in place.
    pub fn search_ctb(
        &mut self,
        rd: &Rd,
        contexts: &Contexts,
        ctb_x: usize,
        ctb_y: usize,
    ) -> CtbDecisions {
        let log2 = self.ctb_log2();
        let mut decisions = CtbDecisions::default();
        self.search_quadtree(
            rd,
            contexts,
            ctb_x << log2,
            ctb_y << log2,
            log2,
            0,
            &mut decisions,
        );
        decisions
    }

    #[allow(clippy::too_many_arguments)]
    fn search_quadtree(
        &mut self,
        rd: &Rd,
        contexts: &Contexts,
        x: usize,
        y: usize,
        log2: u32,
        depth: u32,
        decisions: &mut CtbDecisions,
    ) -> Outcome {
        let size = 1usize << log2;
        if log2 >= self.qp_map.log2 {
            let qp = self.qp_map.at(x, y, log2);
            self.qp = qp;
            self.level = rd.level(qp);
        }
        let inside =
            x + size <= self.settings.coded_width && y + size <= self.settings.coded_height;
        let can_split = log2 > 3;
        let must_split = !inside || log2 > 5;
        let split_coded = inside && can_split;
        // As a single CU.
        let mut leaf = None;
        if !must_split {
            let mut after = contexts.clone();
            let mut estimator = Estimator::default();
            if split_coded {
                let increment = self.split_context(x, y, depth);
                estimator.decision(&mut after.contexts[cabac::SPLIT_CU + increment], 0);
            }
            let (cu, distortion) = self.search_cu(rd, &after, x, y, log2, depth);
            self.write_cu(&mut estimator, &mut after, &cu, &mut QpState::estimate());
            let cost = self.level.cost(distortion, estimator.bits);
            leaf = Some((cost, cu, after));
        }
        // A CU that codes nothing whole is not tried split: finer CUs
        // have little to gain where prediction alone does.
        let empty_leaf = leaf.as_ref().is_some_and(|(_, cu, _)| tree_empty(&cu.tree));
        if !can_split || empty_leaf {
            let (cost, cu, after) = leaf.expect("an 8x8 CU is coded whole");
            if split_coded {
                decisions.splits.push(false);
            }
            decisions.cus.push(cu);
            return Outcome {
                cost,
                contexts: after,
            };
        }
        // Split, if it can win: saved first, the single CU's
        // reconstruction and units, to put back if it does not.
        let saved = leaf.as_ref().map(|_| self.save_region(x, y, size));
        let mut split_decisions = CtbDecisions::default();
        let mut after = contexts.clone();
        let mut estimator = Estimator::default();
        if split_coded {
            let increment = self.split_context(x, y, depth);
            estimator.decision(&mut after.contexts[cabac::SPLIT_CU + increment], 1);
        }
        let mut cost = self.level.cost(0, estimator.bits);
        let half = size / 2;
        for (dx, dy) in [(0, 0), (half, 0), (0, half), (half, half)] {
            if x + dx < self.settings.coded_width && y + dy < self.settings.coded_height {
                let outcome = self.search_quadtree(
                    rd,
                    &after,
                    x + dx,
                    y + dy,
                    log2 - 1,
                    depth + 1,
                    &mut split_decisions,
                );
                cost += outcome.cost;
                after = outcome.contexts;
                if let Some((leaf_cost, _, _)) = &leaf
                    && cost >= *leaf_cost
                {
                    break;
                }
            }
        }
        match leaf {
            Some((leaf_cost, cu, leaf_after)) if leaf_cost <= cost => {
                let saved = saved.expect("saved with the leaf");
                self.restore_region(x, y, size, &saved);
                if split_coded {
                    decisions.splits.push(false);
                }
                decisions.cus.push(cu);
                Outcome {
                    cost: leaf_cost,
                    contexts: leaf_after,
                }
            }
            _ => {
                if split_coded {
                    decisions.splits.push(true);
                }
                decisions.splits.extend(split_decisions.splits);
                decisions.cus.extend(split_decisions.cus);
                Outcome {
                    cost,
                    contexts: after,
                }
            }
        }
    }

    fn save_region(&self, x: usize, y: usize, size: usize) -> (Vec<Saved>, Vec<(u8, u8)>) {
        let planes = vec![
            self.save(0, x, y, size),
            self.save(1, x / 2, y / 2, size / 2),
            self.save(2, x / 2, y / 2, size / 2),
        ];
        let mut units = Vec::with_capacity((size / 4) * (size / 4));
        for uy in (y..y + size).step_by(4) {
            for ux in (x..x + size).step_by(4) {
                let index = self.unit_index(ux, uy);
                units.push((self.units.depth[index], self.units.mode[index]));
            }
        }
        (planes, units)
    }

    fn restore_region(
        &mut self,
        x: usize,
        y: usize,
        size: usize,
        saved: &(Vec<Saved>, Vec<(u8, u8)>),
    ) {
        self.restore(0, x, y, size, &saved.0[0]);
        self.restore(1, x / 2, y / 2, size / 2, &saved.0[1]);
        self.restore(2, x / 2, y / 2, size / 2, &saved.0[2]);
        let mut index = 0;
        for uy in (y..y + size).step_by(4) {
            for ux in (x..x + size).step_by(4) {
                let at = self.unit_index(ux, uy);
                (self.units.depth[at], self.units.mode[at]) = saved.1[index];
                index += 1;
            }
        }
    }

    fn mark_units(&mut self, x: usize, y: usize, size: usize, depth: u32, mode: Option<u32>) {
        for uy in (y..y + size).step_by(4) {
            for ux in (x..x + size).step_by(4) {
                let at = self.unit_index(ux, uy);
                self.units.depth[at] = depth as u8;
                if let Some(mode) = mode {
                    self.units.mode[at] = mode as u8;
                }
            }
        }
    }

    /// One CU's best coding as 2Nx2N (or, at 8x8, four 4x4 blocks), its
    /// reconstruction placed; and its squared error.
    fn search_cu(
        &mut self,
        rd: &Rd,
        contexts: &Contexts,
        x: usize,
        y: usize,
        log2: u32,
        depth: u32,
    ) -> (Cu, u64) {
        let size = 1usize << log2;
        self.mark_units(x, y, size, depth, None);
        self.rates = Some(super::rdoq::Rates::of(contexts));
        self.block_contexts = Some(contexts.clone());
        // Whole.
        let (whole_cost, whole_mode, whole_tree, whole_luma) =
            self.search_luma(rd, contexts, x, y, log2, 0);
        let mut best = (whole_cost, false, [whole_mode; 4], whole_tree, whole_luma);
        // Four 4x4 blocks at the smallest CU.
        if log2 == 3 {
            let saved = self.save(0, x, y, size);
            let saved_units = self.save_region(x, y, size).1;
            let mut cost = 0.0;
            let mut modes = [1u32; 4];
            let mut trees = Vec::with_capacity(4);
            let mut distortion = 0;
            for (index, (dx, dy)) in [(0, 0), (4, 0), (0, 4), (4, 4)].into_iter().enumerate() {
                let (part_cost, mode, tree, part_distortion) =
                    self.search_luma(rd, contexts, x + dx, y + dy, 2, 1);
                self.mark_units(x + dx, y + dy, 4, depth, Some(mode));
                cost += part_cost;
                modes[index] = mode;
                trees.push(tree);
                distortion += part_distortion;
                if cost >= best.0 {
                    break;
                }
            }
            if cost < best.0 && trees.len() == 4 {
                let trees: [Tree; 4] = std::array::from_fn(|index| trees[index].clone());
                best = (cost, true, modes, Tree::Split(Box::new(trees)), distortion);
            } else {
                self.restore(0, x, y, size, &saved);
                let mut index = 0;
                for uy in (y..y + size).step_by(4) {
                    for ux in (x..x + size).step_by(4) {
                        let at = self.unit_index(ux, uy);
                        (self.units.depth[at], self.units.mode[at]) = saved_units[index];
                        index += 1;
                    }
                }
                self.restore_whole_luma(x, y, log2, whole_mode, &best.3);
            }
        }
        let (_, split_intra, luma_modes, mut tree, luma_distortion) = best;
        // Units carry the modes, for the chroma DM and later blocks' MPMs.
        if split_intra {
            for (index, (dx, dy)) in [(0, 0), (4, 0), (0, 4), (4, 4)].into_iter().enumerate() {
                self.mark_units(x + dx, y + dy, 4, depth, Some(luma_modes[index]));
            }
        } else {
            self.mark_units(x, y, size, depth, Some(luma_modes[0]));
        }
        let (chroma_code, chroma_mode, chroma_distortion) =
            self.search_chroma(rd, x, y, log2, luma_modes[0], &mut tree);
        (
            Cu {
                x,
                y,
                log2,
                split_intra,
                luma_modes,
                chroma_code,
                chroma_mode,
                tree,
            },
            luma_distortion + chroma_distortion,
        )
    }

    /// Puts back the whole-CU luma reconstruction after a losing 4x4 try,
    /// by coding its tree's levels again.
    fn restore_whole_luma(&mut self, x: usize, y: usize, log2: u32, mode: u32, tree: &Tree) {
        match tree {
            Tree::Split(children) => {
                let half = 1usize << (log2 - 1);
                for (child, (dx, dy)) in
                    children
                        .iter()
                        .zip([(0, 0), (half, 0), (0, half), (half, half)])
                {
                    self.restore_whole_luma(x + dx, y + dy, log2 - 1, mode, child);
                }
            }
            Tree::Leaf { luma, .. } => {
                self.place_block(0, x, y, log2, mode, luma.as_ref());
            }
        }
    }

    /// Reconstructs a block from given levels (prediction plus residual).
    #[allow(clippy::too_many_arguments)]
    fn place_block(
        &mut self,
        component: usize,
        x: usize,
        y: usize,
        log2: u32,
        mode: u32,
        coded: Option<&Coded>,
    ) {
        let n = 1usize << log2;
        let bit_depth = self.settings.bit_depth;
        let neighbours = Neighbours::of(self, component, x, y, n);
        let mut prediction = [0i32; 32 * 32];
        neighbours.predict(component, mode, n, bit_depth, &mut prediction[..n * n]);
        let mut back = [0i32; 32 * 32];
        if let Some(coded) = coded {
            let quantizer = if component == 0 {
                self.level.luma
            } else {
                self.level.chroma
            };
            quantizer.reconstruct(
                &coded.levels,
                &mut back[..n * n],
                log2,
                component == 0 && n == 4,
                coded.transform_skip,
            );
        }
        let max = (1i32 << bit_depth) - 1;
        let top = self.band_top(component);
        let plane = &mut self.recon[component];
        for row in 0..n {
            let start = (y - top + row) * plane.width + x;
            for column in 0..n {
                let value = (prediction[row * n + column] + back[row * n + column]).clamp(0, max);
                plane.samples[start + column] = value as u16;
            }
        }
    }

    /// The best luma mode and transform tree of a prediction block at
    /// (x, y), its reconstruction placed: (cost, mode, tree, distortion).
    fn search_luma(
        &mut self,
        rd: &Rd,
        contexts: &Contexts,
        x: usize,
        y: usize,
        log2: u32,
        tree_depth: u32,
    ) -> (f64, u32, Tree, u64) {
        let n = 1usize << log2;
        let bit_depth = self.settings.bit_depth;
        let mpm = self.most_probable_modes(x, y);
        let mode_bits = |mode: u32| -> f64 {
            match mpm.iter().position(|&candidate| candidate == mode) {
                Some(0) => 2.0,
                Some(_) => 3.0,
                None => 6.0,
            }
        };
        // Rough: every mode's SATD.
        let neighbours = Neighbours::of(self, 0, x, y, n);
        self.neighbours = Some(((0, x, y, n), neighbours.clone()));
        let mut source = [0i32; 32 * 32];
        let plane = &self.source[0];
        for row in 0..n {
            for column in 0..n {
                source[row * n + column] = plane.at(x + column, y + row);
            }
        }
        let mut prediction = [0i32; 32 * 32];
        let mut rough: Vec<(f64, u32)> = (0..MODES)
            .map(|mode| {
                neighbours.predict(0, mode, n, bit_depth, &mut prediction[..n * n]);
                let cost = f64::from(satd(&source[..n * n], &prediction[..n * n], n))
                    + self.level.lambda_satd * mode_bits(mode);
                (cost, mode)
            })
            .collect();
        rough.sort_by(|a, b| a.0.total_cmp(&b.0));
        let count = CANDIDATES[log2 as usize - 2];
        let mut candidates: Vec<u32> = rough.iter().take(count).map(|&(_, mode)| mode).collect();
        for &mode in &mpm {
            if !candidates.contains(&mode) && candidates.len() < count + 1 {
                candidates.push(mode);
            }
        }
        // Full: each candidate coded, its transform tree chosen; transform
        // skip waits for the chosen mode.
        let skip_after = self.settings.transform_skip && log2 <= 3;
        if skip_after {
            self.skip_trials = false;
        }
        let saved = self.save(0, x, y, n);
        type Best = (f64, u32, Tree, u64, Saved, Contexts, u64);
        let mut best: Option<Best> = None;
        for &mode in &candidates {
            if best.is_some() {
                self.restore(0, x, y, n, &saved);
            }
            let mut after = contexts.clone();
            let mut estimator = Estimator::default();
            self.write_luma_mode(&mut estimator, &mut after, x, y, mode);
            let bound = Bound {
                limit: best.as_ref().map_or(f64::INFINITY, |best| best.0),
                distortion: 0,
                bits: estimator.bits,
            };
            let (tree, distortion, bits) = self.search_luma_tree(
                rd,
                &after,
                x,
                y,
                log2,
                tree_depth,
                self.settings.transform_depth,
                mode,
                bound,
            );
            let cost = self.level.cost(distortion, estimator.bits + bits);
            if best.as_ref().is_none_or(|best| cost < best.0) {
                let placed = self.save(0, x, y, n);
                best = Some((cost, mode, tree, distortion, placed, after, estimator.bits));
            }
        }
        let (mut cost, mode, mut tree, mut distortion, placed, after, mode_bits) =
            best.expect("a mode was tried");
        self.restore(0, x, y, n, &placed);
        self.neighbours = None;
        if skip_after {
            self.skip_trials = true;
            let chosen = self.save(0, x, y, n);
            self.restore(0, x, y, n, &saved);
            let (skip_tree, skip_distortion, skip_bits) = self.search_luma_tree(
                rd,
                &after,
                x,
                y,
                log2,
                tree_depth,
                self.settings.transform_depth,
                mode,
                Bound::NONE,
            );
            let skip_cost = self.level.cost(skip_distortion, mode_bits + skip_bits);
            if skip_cost < cost {
                (cost, tree, distortion) = (skip_cost, skip_tree, skip_distortion);
            } else {
                self.restore(0, x, y, n, &chosen);
            }
        }
        (cost, mode, tree, distortion)
    }

    /// The luma transform tree of a block predicted with `mode`: split or
    /// whole, by cost. (tree, distortion, bits)
    #[allow(clippy::too_many_arguments)]
    fn search_luma_tree(
        &mut self,
        rd: &Rd,
        contexts: &Contexts,
        x: usize,
        y: usize,
        log2: u32,
        depth: u32,
        searched: u32,
        mode: u32,
        bound: Bound,
    ) -> (Tree, u64, u64) {
        let n = 1usize << log2;
        let max_depth = self.settings.transform_depth;
        // The split flag is coded to `max_depth`; splits are tried to
        // `searched`.
        let split_coded = log2 <= 5 && log2 > 2 && depth < max_depth;
        let must_split = log2 > 5;
        let mut leaf = None;
        if !must_split {
            let try_skip = self.settings.transform_skip && self.skip_trials;
            let result = self.code_block(rd, 0, x, y, log2, mode, try_skip);
            let mut after = contexts.clone();
            let mut estimator = Estimator::default();
            if split_coded {
                estimator.decision(
                    &mut after.contexts[cabac::SPLIT_TRANSFORM + (5 - log2 as usize)],
                    0,
                );
            }
            estimator.decision(
                &mut after.contexts[cabac::CBF_LUMA + usize::from(depth == 0)],
                u32::from(result.coded.is_some()),
            );
            if let Some(coded) = &result.coded {
                syntax::residual(
                    &mut estimator,
                    &mut after,
                    &Block {
                        levels: &coded.levels,
                        log2,
                        luma: true,
                        mode,
                        transform_skip: (self.settings.transform_skip && n == 4)
                            .then_some(coded.transform_skip),
                        sign_hiding: self.settings.sign_hiding,
                    },
                );
            }
            let tree = Tree::Leaf {
                luma: result.coded,
                chroma: [None, None],
            };
            leaf = Some((tree, result.distortion, estimator.bits));
        }
        let empty = matches!(&leaf, Some((Tree::Leaf { luma: None, .. }, _, _)));
        // A block that codes nothing whole is not tried split.
        if !((split_coded && depth < searched && !empty) || must_split) {
            return leaf.expect("a block that cannot split is coded whole");
        }
        let saved = leaf.as_ref().map(|_| self.save(0, x, y, n));
        let mut estimator = Estimator::default();
        let mut after = contexts.clone();
        if split_coded {
            estimator.decision(
                &mut after.contexts[cabac::SPLIT_TRANSFORM + (5 - log2 as usize)],
                1,
            );
        }
        let half = n / 2;
        let mut children = Vec::with_capacity(4);
        let (mut distortion, mut bits) = (0u64, estimator.bits);
        // The split is given up once it costs as much as the leaf (which
        // then wins) or reaches the bound.
        let limit = leaf
            .as_ref()
            .map_or(bound.limit, |(_, leaf_distortion, leaf_bits)| {
                bound.limit.min(
                    self.level
                        .cost(bound.distortion + leaf_distortion, bound.bits + leaf_bits),
                )
            });
        for (dx, dy) in [(0, 0), (half, 0), (0, half), (half, half)] {
            let (child, child_distortion, child_bits) = self.search_luma_tree(
                rd,
                &after,
                x + dx,
                y + dy,
                log2 - 1,
                depth + 1,
                searched,
                mode,
                Bound {
                    limit,
                    distortion: bound.distortion + distortion,
                    bits: bound.bits + bits,
                },
            );
            children.push(child);
            distortion += child_distortion;
            bits += child_bits;
            if leaf.is_some()
                && self
                    .level
                    .cost(bound.distortion + distortion, bound.bits + bits)
                    >= limit
            {
                break;
            }
        }
        if children.len() < 4 {
            let (leaf_tree, leaf_distortion, leaf_bits) = leaf.expect("given up beside a leaf");
            self.restore(0, x, y, n, &saved.expect("saved with the leaf"));
            return (leaf_tree, leaf_distortion, leaf_bits);
        }
        if let Some((leaf_tree, leaf_distortion, leaf_bits)) = leaf {
            if self.level.cost(leaf_distortion, leaf_bits) <= self.level.cost(distortion, bits) {
                self.restore(0, x, y, n, &saved.expect("saved with the leaf"));
                return (leaf_tree, leaf_distortion, leaf_bits);
            }
        }
        let children: [Tree; 4] = std::array::from_fn(|index| children[index].clone());
        (Tree::Split(Box::new(children)), distortion, bits)
    }

    /// The chroma mode of a CU, by cost over its chroma blocks along its
    /// transform tree; the blocks' levels go into `tree`.
    fn search_chroma(
        &mut self,
        rd: &Rd,
        x: usize,
        y: usize,
        log2: u32,
        luma_mode: u32,
        tree: &mut Tree,
    ) -> (u32, u32, u64) {
        let (cx, cy, size) = (x / 2, y / 2, (1usize << log2) / 2);
        let saved = [self.save(1, cx, cy, size), self.save(2, cx, cy, size)];
        // Chroma 4x4 blocks try transform skip once the mode is chosen.
        let skip_after = self.settings.transform_skip && log2 == 3;
        if skip_after {
            self.skip_trials = false;
        }
        type Best = (f64, u32, u32, Vec<ChromaPair>, u64, [Saved; 2]);
        let mut best: Option<Best> = None;
        for code in [4u32, 0, 1, 2, 3] {
            let mode = if code == 4 {
                luma_mode
            } else {
                let candidate = [0, 26, 10, 1][code as usize];
                if candidate == luma_mode {
                    34
                } else {
                    candidate
                }
            };
            if best.is_some() {
                self.restore(1, cx, cy, size, &saved[0]);
                self.restore(2, cx, cy, size, &saved[1]);
            }
            let mut blocks = Vec::new();
            let (distortion, bits) =
                self.code_chroma_tree(rd, x, y, log2, mode, tree, 0, &mut blocks);
            let mode_bits = if code == 4 { 32768 / 2 } else { 32768 * 3 };
            let cost = self.level.cost(distortion, bits + mode_bits);
            if best.as_ref().is_none_or(|best| cost < best.0) {
                let placed = [self.save(1, cx, cy, size), self.save(2, cx, cy, size)];
                best = Some((cost, code, mode, blocks, distortion, placed));
            }
        }
        let (cost, code, mode, mut blocks, mut distortion, placed) = best.expect("DM was tried");
        self.restore(1, cx, cy, size, &placed[0]);
        self.restore(2, cx, cy, size, &placed[1]);
        if skip_after {
            self.skip_trials = true;
            self.restore(1, cx, cy, size, &saved[0]);
            self.restore(2, cx, cy, size, &saved[1]);
            let mut skipped = Vec::new();
            let (skip_distortion, bits) =
                self.code_chroma_tree(rd, x, y, log2, mode, tree, 0, &mut skipped);
            let mode_bits = if code == 4 { 32768 / 2 } else { 32768 * 3 };
            let skip_cost = self.level.cost(skip_distortion, bits + mode_bits);
            if skip_cost < cost {
                (blocks, distortion) = (skipped, skip_distortion);
            } else {
                self.restore(1, cx, cy, size, &placed[0]);
                self.restore(2, cx, cy, size, &placed[1]);
            }
        }
        place_chroma(tree, log2, &mut blocks.into_iter());
        (code, mode, distortion)
    }

    /// Codes the chroma blocks of a transform tree with `mode`: at each
    /// leaf over 4x4 luma, or for four 4x4 luma leaves at their parent;
    /// their levels go into `blocks` in the tree's order. (distortion,
    /// bits)
    #[allow(clippy::too_many_arguments)]
    fn code_chroma_tree(
        &mut self,
        rd: &Rd,
        x: usize,
        y: usize,
        log2: u32,
        mode: u32,
        tree: &Tree,
        depth: u32,
        blocks: &mut Vec<ChromaPair>,
    ) -> (u64, u64) {
        match tree {
            Tree::Split(children) if log2 > 3 => {
                let half = 1usize << (log2 - 1);
                let mut total = (0, 0);
                for (child, (dx, dy)) in
                    children
                        .iter()
                        .zip([(0, 0), (half, 0), (0, half), (half, half)])
                {
                    let (d, b) = self.code_chroma_tree(
                        rd,
                        x + dx,
                        y + dy,
                        log2 - 1,
                        mode,
                        child,
                        depth + 1,
                        blocks,
                    );
                    total.0 += d;
                    total.1 += b;
                }
                total
            }
            _ => {
                // A leaf's chroma, or for an 8x8 node of 4x4 luma, one 4x4
                // chroma block each.
                let chroma_log2 = if matches!(tree, Tree::Split(_)) {
                    2
                } else {
                    log2 - 1
                };
                let [cb, cr] = self.code_chroma_pair(rd, x / 2, y / 2, chroma_log2, mode);
                let bits = chroma_bits(rd, self.settings, [&cb, &cr], chroma_log2, mode, depth);
                let distortion = cb.1 + cr.1;
                blocks.push([cb.0, cr.0]);
                (distortion, bits)
            }
        }
    }

    fn code_chroma_pair(
        &mut self,
        rd: &Rd,
        cx: usize,
        cy: usize,
        log2: u32,
        mode: u32,
    ) -> [(Option<Coded>, u64); 2] {
        let skip = self.settings.transform_skip && self.skip_trials;
        let cb = self.code_block(rd, 1, cx, cy, log2, mode, skip);
        let cr = self.code_block(rd, 2, cx, cy, log2, mode, skip);
        [(cb.coded, cb.distortion), (cr.coded, cr.distortion)]
    }
}

/// The bits of a chroma pair's flags and levels, from fresh contexts (an
/// estimate: chroma's contexts are its own).
fn chroma_bits(
    rd: &Rd,
    settings: &Settings,
    chroma: [&(Option<Coded>, u64); 2],
    log2: u32,
    mode: u32,
    depth: u32,
) -> u64 {
    let mut estimator = Estimator::default();
    let mut contexts = rd.initial.clone();
    for (coded, _) in chroma {
        estimator.decision(
            &mut contexts.contexts[cabac::CBF_CHROMA + depth as usize],
            u32::from(coded.is_some()),
        );
        if let Some(coded) = coded {
            syntax::residual(
                &mut estimator,
                &mut contexts,
                &Block {
                    levels: &coded.levels,
                    log2,
                    luma: false,
                    mode,
                    transform_skip: (settings.transform_skip && log2 == 2)
                        .then_some(coded.transform_skip),
                    sign_hiding: settings.sign_hiding,
                },
            );
        }
    }
    estimator.bits
}

/// The QP of the quantization group being written, as the decoder
/// derives it (8.6.1): predicted from the groups to the left and above
/// in the CTB (else from the last CU's), its delta to the target QP
/// written with the group's first transform unit that has levels.
#[derive(Clone, Copy)]
pub struct QpState {
    pub enabled: bool,
    pub group: Option<(usize, usize)>,
    pub predicted: i32,
    pub target: i32,
    pub coded: bool,
    /// QpY of the last CU written: the slice's at a row's start.
    pub last: i32,
}

impl QpState {
    pub fn new(enabled: bool, slice_qp: i32) -> QpState {
        QpState {
            enabled,
            group: None,
            predicted: slice_qp,
            target: slice_qp,
            coded: false,
            last: slice_qp,
        }
    }

    /// For estimates: the delta's few bits are left out.
    pub fn estimate() -> QpState {
        QpState {
            enabled: false,
            ..QpState::new(false, 0)
        }
    }

    /// QpY of a CU written now.
    pub fn current(&self) -> i32 {
        if self.coded {
            self.target
        } else {
            self.predicted
        }
    }
}

/// `cu_qp_delta_abs` and its sign (7.3.8.14, 9.3.3.10).
fn write_qp_delta<C: Coder>(coder: &mut C, contexts: &mut Contexts, delta: i32) {
    let magnitude = delta.unsigned_abs();
    let prefix = magnitude.min(5);
    for index in 0..prefix {
        coder.decision(
            &mut contexts.contexts[cabac::CU_QP_DELTA + usize::from(index > 0)],
            1,
        );
    }
    if prefix < 5 {
        coder.decision(
            &mut contexts.contexts[cabac::CU_QP_DELTA + usize::from(prefix > 0)],
            0,
        );
    } else {
        // The rest in zeroth-order Exp-Golomb.
        let mut value = magnitude - 5;
        let mut k = 0;
        while value >= (1 << k) {
            value -= 1 << k;
            coder.bypass(1, 1);
            k += 1;
        }
        coder.bypass(0, 1);
        coder.bypass(value, k);
    }
    if magnitude > 0 {
        coder.bypass(u32::from(delta < 0), 1);
    }
}

/// Writing a CU, for both the estimator and the encoder.
impl Picture<'_> {
    pub fn write_luma_mode<C: Coder>(
        &self,
        coder: &mut C,
        contexts: &mut Contexts,
        x: usize,
        y: usize,
        mode: u32,
    ) {
        let mpm = self.most_probable_modes(x, y);
        match mpm.iter().position(|&candidate| candidate == mode) {
            Some(index) => {
                coder.decision(&mut contexts.contexts[cabac::PREV_INTRA_LUMA], 1);
                match index {
                    0 => coder.bypass(0, 1),
                    1 => coder.bypass(0b10, 2),
                    _ => coder.bypass(0b11, 2),
                }
            }
            None => {
                coder.decision(&mut contexts.contexts[cabac::PREV_INTRA_LUMA], 0);
                let mut sorted = mpm;
                sorted.sort_unstable();
                let mut value = mode;
                for &candidate in sorted.iter().rev() {
                    if value > candidate {
                        value -= 1;
                    }
                }
                coder.bypass(value, 5);
            }
        }
    }

    /// `coding_unit()` from part_mode on: the modes and the transform tree.
    pub fn write_cu<C: Coder>(
        &self,
        coder: &mut C,
        contexts: &mut Contexts,
        cu: &Cu,
        qp: &mut QpState,
    ) {
        if qp.enabled {
            self.enter_group(cu, qp);
        }
        if cu.log2 == 3 {
            coder.decision(
                &mut contexts.contexts[cabac::PART_MODE],
                u32::from(!cu.split_intra),
            );
        }
        let parts = if cu.split_intra { 4 } else { 1 };
        let half = 1usize << (cu.log2 - 1);
        // The flags first, then each block's index or remainder.
        let places: Vec<(usize, usize)> = if cu.split_intra {
            vec![
                (cu.x, cu.y),
                (cu.x + half, cu.y),
                (cu.x, cu.y + half),
                (cu.x + half, cu.y + half),
            ]
        } else {
            vec![(cu.x, cu.y)]
        };
        let mut mpm_index = [None; 4];
        for (part, &(px, py)) in places.iter().enumerate().take(parts) {
            let mpm = self.mpm_during(cu, part, px, py);
            mpm_index[part] = mpm.iter().position(|&m| m == cu.luma_modes[part]);
            coder.decision(
                &mut contexts.contexts[cabac::PREV_INTRA_LUMA],
                u32::from(mpm_index[part].is_some()),
            );
        }
        for (part, &(px, py)) in places.iter().enumerate().take(parts) {
            match mpm_index[part] {
                Some(0) => coder.bypass(0, 1),
                Some(1) => coder.bypass(0b10, 2),
                Some(_) => coder.bypass(0b11, 2),
                None => {
                    let mut sorted = self.mpm_during(cu, part, px, py);
                    sorted.sort_unstable();
                    let mut value = cu.luma_modes[part];
                    for &candidate in sorted.iter().rev() {
                        if value > candidate {
                            value -= 1;
                        }
                    }
                    coder.bypass(value, 5);
                }
            }
        }
        if cu.chroma_code == 4 {
            coder.decision(&mut contexts.contexts[cabac::INTRA_CHROMA], 0);
        } else {
            coder.decision(&mut contexts.contexts[cabac::INTRA_CHROMA], 1);
            coder.bypass(cu.chroma_code, 2);
        }
        let max_depth = self.settings.transform_depth + u32::from(cu.split_intra);
        self.write_tree(
            coder,
            contexts,
            cu,
            &cu.tree,
            cu.x,
            cu.y,
            cu.log2,
            0,
            max_depth,
            [true, true],
            0,
            qp,
        );
    }

    /// The most probable modes of part `part` of `cu` as the decoder sees
    /// them while reading it: earlier parts of the CU carry their modes.
    fn mpm_during(&self, cu: &Cu, part: usize, x: usize, y: usize) -> [u32; 3] {
        if !cu.split_intra {
            return self.most_probable_modes(x, y);
        }
        // The units of this CU hold its final modes already; the
        // decoder's view differs only for parts not yet read, which are
        // never neighbours of an earlier part (left and above only).
        let _ = part;
        self.most_probable_modes(x, y)
    }

    #[allow(clippy::too_many_arguments)]
    fn write_tree<C: Coder>(
        &self,
        coder: &mut C,
        contexts: &mut Contexts,
        cu: &Cu,
        tree: &Tree,
        x: usize,
        y: usize,
        log2: u32,
        depth: u32,
        max_depth: u32,
        parent_cbf: [bool; 2],
        block: usize,
        qp: &mut QpState,
    ) {
        let split = matches!(tree, Tree::Split(_));
        if log2 <= 5 && log2 > 2 && depth < max_depth && !(cu.split_intra && depth == 0) {
            coder.decision(
                &mut contexts.contexts[cabac::SPLIT_TRANSFORM + (5 - log2 as usize)],
                u32::from(split),
            );
        }
        let mut cbf = parent_cbf;
        if log2 > 2 {
            for (component, flag) in cbf.iter_mut().enumerate() {
                if depth == 0 || parent_cbf[component] {
                    *flag = chroma_coded(tree, log2, component);
                    coder.decision(
                        &mut contexts.contexts[cabac::CBF_CHROMA + depth as usize],
                        u32::from(*flag),
                    );
                } else {
                    *flag = false;
                }
            }
        }
        match tree {
            Tree::Split(children) => {
                let half = 1usize << (log2 - 1);
                for (index, (child, (dx, dy))) in children
                    .iter()
                    .zip([(0, 0), (half, 0), (0, half), (half, half)])
                    .enumerate()
                {
                    self.write_tree(
                        coder,
                        contexts,
                        cu,
                        child,
                        x + dx,
                        y + dy,
                        log2 - 1,
                        depth + 1,
                        max_depth,
                        cbf,
                        index,
                        qp,
                    );
                }
            }
            Tree::Leaf { luma, chroma } => {
                coder.decision(
                    &mut contexts.contexts[cabac::CBF_LUMA + usize::from(depth == 0)],
                    u32::from(luma.is_some()),
                );
                let mode = cu.luma_modes[self.part_of(cu, x, y)];
                // The QP delta, before the first levels of the group: with
                // luma's, or with chroma's (a 4x4 luma block's chroma
                // flags are its parent's).
                let chroma_flags = cbf.iter().any(|&flag| flag);
                if qp.enabled && !qp.coded && (luma.is_some() || chroma_flags) {
                    write_qp_delta(coder, contexts, qp.target - qp.predicted);
                    qp.coded = true;
                }
                if let Some(coded) = luma {
                    syntax::residual(
                        coder,
                        contexts,
                        &Block {
                            levels: &coded.levels,
                            log2,
                            luma: true,
                            mode,
                            transform_skip: (self.settings.transform_skip && log2 == 2)
                                .then_some(coded.transform_skip),
                            sign_hiding: self.settings.sign_hiding,
                        },
                    );
                }
                let chroma_log2 = if log2 > 2 {
                    Some(log2 - 1)
                } else if block == 3 {
                    Some(2)
                } else {
                    None
                };
                if let Some(chroma_log2) = chroma_log2 {
                    for coded in chroma.iter().flatten() {
                        syntax::residual(
                            coder,
                            contexts,
                            &Block {
                                levels: &coded.levels,
                                log2: chroma_log2,
                                luma: false,
                                mode: cu.chroma_mode,
                                transform_skip: (self.settings.transform_skip && chroma_log2 == 2)
                                    .then_some(coded.transform_skip),
                                sign_hiding: self.settings.sign_hiding,
                            },
                        );
                    }
                }
            }
        }
    }

    /// Enters the quantization group of `cu` (8.6.1): at a new group,
    /// QpY's prediction from the group's left and upper neighbours in the
    /// same CTB, else the last CU's; and the group's target from the map.
    fn enter_group(&self, cu: &Cu, qp: &mut QpState) {
        let log2 = self.qp_map.log2;
        let group = ((cu.x >> log2) << log2, (cu.y >> log2) << log2);
        if qp.group == Some(group) {
            return;
        }
        qp.group = Some(group);
        let ctb_log2 = self.ctb_log2();
        let ctb = (group.0 >> ctb_log2, group.1 >> ctb_log2);
        let neighbour = |nx: isize, ny: isize| -> i32 {
            if self.available(cu.x, cu.y, nx, ny)
                && ((nx as usize) >> ctb_log2, (ny as usize) >> ctb_log2) == ctb
            {
                i32::from(self.units.qp[self.unit_index(nx as usize, ny as usize)])
            } else {
                qp.last
            }
        };
        let left = neighbour(group.0 as isize - 1, group.1 as isize);
        let above = neighbour(group.0 as isize, group.1 as isize - 1);
        qp.predicted = (left + above + 1) >> 1;
        qp.target = self.qp_map.at(cu.x, cu.y, cu.log2);
    }

    /// Records a written CU's QpY in its units, for later predictions.
    pub fn mark_qp(&mut self, cu: &Cu, qp: &mut QpState) {
        let value = qp.current();
        let size = 1usize << cu.log2;
        for uy in (cu.y..cu.y + size).step_by(4) {
            for ux in (cu.x..cu.x + size).step_by(4) {
                let at = self.unit_index(ux, uy);
                self.units.qp[at] = value as i8;
            }
        }
        qp.last = value;
    }

    fn part_of(&self, cu: &Cu, x: usize, y: usize) -> usize {
        if !cu.split_intra {
            return 0;
        }
        let half = 1usize << (cu.log2 - 1);
        usize::from(x - cu.x >= half) + 2 * usize::from(y - cu.y >= half)
    }
}

/// A transform tree leaf's Cb and Cr levels.
type ChromaPair = [Option<Coded>; 2];

/// Puts chroma levels coded along a tree (in `code_chroma_tree`'s order)
/// into its leaves: an 8x8 node of 4x4 leaves keeps them in its last.
fn place_chroma(tree: &mut Tree, log2: u32, blocks: &mut impl Iterator<Item = ChromaPair>) {
    match tree {
        Tree::Split(children) if log2 > 3 => {
            for child in children.iter_mut() {
                place_chroma(child, log2 - 1, blocks);
            }
        }
        Tree::Split(children) => {
            if let Tree::Leaf { chroma, .. } = &mut children[3] {
                *chroma = blocks.next().unwrap_or([None, None]);
            }
        }
        Tree::Leaf { chroma, .. } => *chroma = blocks.next().unwrap_or([None, None]),
    }
}

/// Whether a transform tree codes no levels at all.
fn tree_empty(tree: &Tree) -> bool {
    match tree {
        Tree::Leaf { luma, chroma } => luma.is_none() && chroma.iter().all(Option::is_none),
        Tree::Split(children) => children.iter().all(tree_empty),
    }
}

/// Whether a tree node's chroma (Cb at 0, Cr at 1) has levels anywhere
/// below it; an 8x8 node of 4x4 leaves keeps them in its last leaf.
fn chroma_coded(tree: &Tree, log2: u32, component: usize) -> bool {
    match tree {
        Tree::Leaf { chroma, .. } => chroma[component].is_some(),
        Tree::Split(children) => {
            if log2 == 3 {
                matches!(&children[3], Tree::Leaf { chroma, .. } if chroma[component].is_some())
            } else {
                children
                    .iter()
                    .any(|child| chroma_coded(child, log2 - 1, component))
            }
        }
    }
}
