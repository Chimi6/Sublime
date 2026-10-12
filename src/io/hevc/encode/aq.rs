//! Adaptive quantization: a QP for each quantization group (the area one
//! QP delta covers) from how busy its samples are. Flat areas, where the
//! eye (and SSIM) sees every step, get a lower QP; textured ones, which
//! hide error, a higher one. The offset is the strength times how far
//! the group's log energy sits from the picture's mean, so the picture's
//! overall level stays near the QP asked for (as x264's and x265's
//! variance AQ).

use super::Settings;
use super::search::Plane;

/// The QPs of a picture's quantization groups.
pub struct QpMap {
    pub qps: Vec<i32>,
    /// A group's size (log2) and the groups across.
    pub log2: u32,
    pub wide: usize,
}

impl QpMap {
    /// The QP of the group at luma (x, y), or for a block larger than a
    /// group (`log2` above the group's), the mean of the groups it covers.
    pub fn at(&self, x: usize, y: usize, log2: u32) -> i32 {
        let (gx, gy) = (x >> self.log2, y >> self.log2);
        if log2 <= self.log2 {
            return self.qps[gy * self.wide + gx];
        }
        let span = 1usize << (log2 - self.log2);
        let high = self.qps.len() / self.wide;
        let (mut sum, mut count) = (0, 0);
        for row in gy..(gy + span).min(high) {
            for column in gx..(gx + span).min(self.wide) {
                sum += self.qps[row * self.wide + column];
                count += 1;
            }
        }
        (sum + count / 2) / count.max(1)
    }
}

/// The QP of each quantization group, in raster order.
pub fn qp_map(settings: &Settings, source: &[Plane; 3]) -> QpMap {
    let log2 = settings.ctb_log2 - settings.qp_delta_depth.unwrap_or(0);
    let size = 1usize << log2;
    let (wide, high) = (
        settings.coded_width.div_ceil(size),
        settings.coded_height.div_ceil(size),
    );
    let strength = settings.aq_strength;
    if strength == 0.0 || settings.qp_delta_depth.is_none() {
        return QpMap {
            qps: vec![settings.qp; wide * high],
            log2,
            wide,
        };
    }
    let mut logs = Vec::with_capacity(wide * high);
    for group_y in 0..high {
        for group_x in 0..wide {
            let (x, y) = (group_x * size, group_y * size);
            // Luma by 8x8 blocks, chroma by 4x4: each block's variance
            // times its area, so a group of mixed detail counts its busy
            // parts as they are.
            let mut energy = 0.0;
            for (component, block) in [(0usize, 8usize), (1, 4), (2, 4)] {
                let plane = &source[component];
                let shift = usize::from(component > 0);
                let (x0, y0) = (x >> shift, y >> shift);
                let span = size >> shift;
                for by in (y0..(y0 + span).min(plane.height)).step_by(block) {
                    for bx in (x0..(x0 + span).min(plane.width)).step_by(block) {
                        energy += block_energy(plane, bx, by, block);
                    }
                }
            }
            logs.push((energy + 1.0).log2());
        }
    }
    let mean = logs.iter().sum::<f64>() / logs.len() as f64;
    let qps = logs
        .iter()
        .map(|&energy| {
            let offset = (strength * (energy - mean)).clamp(-12.0, 12.0);
            (f64::from(settings.qp) + offset).round().clamp(0.0, 51.0) as i32
        })
        .collect();
    QpMap { qps, log2, wide }
}

/// The sum of squared deviations from the mean of an `n` by `n` block.
fn block_energy(plane: &Plane, x: usize, y: usize, n: usize) -> f64 {
    let (mut sum, mut squares, mut count) = (0u64, 0u64, 0u64);
    for row in y..(y + n).min(plane.height) {
        let start = row * plane.width + x;
        let end = row * plane.width + (x + n).min(plane.width);
        for &sample in &plane.samples[start..end] {
            let value = u64::from(sample);
            sum += value;
            squares += value * value;
            count += 1;
        }
    }
    if count == 0 {
        return 0.0;
    }
    let deviation = squares as f64 - (sum as f64 * sum as f64) / count as f64;
    deviation.max(0.0)
}
