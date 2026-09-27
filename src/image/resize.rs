//! Area-averaging downscale, fed a source row at a time: every target
//! pixel is the average of the source area it covers, fractional edges
//! weighted by coverage. Color is averaged weighted by alpha, so a
//! transparent pixel's color does not bleed into its neighbors. Only
//! the target rows in progress are held.

use crate::image::ColorType;

/// One source pixel's share of a target pixel or row.
#[derive(Clone, Copy)]
struct Share {
    source: usize,
    weight: f64,
}

/// For each target index, the source indices it covers and how much.
fn shares(source: usize, target: usize) -> Vec<Vec<Share>> {
    let scale = source as f64 / target as f64;
    (0..target)
        .map(|index| {
            let (start, end) = (index as f64 * scale, (index + 1) as f64 * scale);
            let mut out = Vec::new();
            let mut at = start.floor() as usize;
            while (at as f64) < end && at < source {
                let covered = (end.min(at as f64 + 1.0) - start.max(at as f64)) / scale;
                if covered > 0.0 {
                    out.push(Share {
                        source: at,
                        weight: covered,
                    });
                }
                at += 1;
            }
            out
        })
        .collect()
}

/// Sums each target column's source area, color weighted by alpha when
/// `ALPHA` (the last channel).
fn horizontal<const N: usize, const ALPHA: bool>(
    pixels: &[u8],
    columns: &[Vec<Share>],
    wide: &mut [f64],
) {
    for (target, list) in wide.chunks_exact_mut(N).zip(columns) {
        let mut sums = [0.0f64; N];
        for share in list {
            let cell = &pixels[share.source * N..share.source * N + N];
            let weight = share.weight;
            if ALPHA {
                let alpha = f64::from(cell[N - 1]);
                let weighted = weight * alpha / 255.0;
                for channel in 0..N - 1 {
                    sums[channel] += weighted * f64::from(cell[channel]);
                }
                sums[N - 1] += weight * alpha;
            } else {
                for channel in 0..N {
                    sums[channel] += weight * f64::from(cell[channel]);
                }
            }
        }
        target.copy_from_slice(&sums);
    }
}

/// Downscales to `width` by `height`, emitting target rows as the
/// source rows that cover them arrive.
pub struct Downscale {
    channels: usize,
    alpha: bool,
    target_width: usize,
    columns: Vec<Vec<Share>>,
    /// For each source row, the target rows it touches and how much.
    rows: Vec<Vec<(usize, f64)>>,
    /// Accumulated (alpha-weighted) sums of the target rows in progress,
    /// keyed by target row.
    pending: Vec<(usize, Vec<f64>)>,
    /// The last source row that touches each target row.
    last_source: Vec<usize>,
    source_row: usize,
    wide: Vec<f64>,
}

impl Downscale {
    pub fn new(
        source_width: u32,
        source_height: u32,
        width: u32,
        height: u32,
        color: ColorType,
    ) -> Downscale {
        let channels = color.channels();
        let alpha = matches!(color, ColorType::GrayAlpha | ColorType::Rgba);
        let vertical = shares(source_height as usize, height as usize);
        let mut rows = vec![Vec::new(); source_height as usize];
        let mut last_source = vec![0; height as usize];
        for (target, list) in vertical.iter().enumerate() {
            for share in list {
                rows[share.source].push((target, share.weight));
                last_source[target] = last_source[target].max(share.source);
            }
        }
        Downscale {
            channels,
            alpha,
            target_width: width as usize,
            columns: shares(source_width as usize, width as usize),
            rows,
            pending: Vec::new(),
            last_source,
            source_row: 0,
            wide: vec![0.0; width as usize * channels],
        }
    }

    /// Takes the next source row; returns the target rows it completes,
    /// top first.
    pub fn row(&mut self, pixels: &[u8]) -> Vec<Vec<u8>> {
        let channels = self.channels;
        // Horizontal pass: alpha-weighted sums over each column's area,
        // with the pixel width a constant for the loops.
        match (channels, self.alpha) {
            (1, _) => horizontal::<1, false>(pixels, &self.columns, &mut self.wide),
            (2, _) => horizontal::<2, true>(pixels, &self.columns, &mut self.wide),
            (3, _) => horizontal::<3, false>(pixels, &self.columns, &mut self.wide),
            _ => horizontal::<4, true>(pixels, &self.columns, &mut self.wide),
        }
        let source_row = self.source_row;
        self.source_row += 1;
        for &(target, weight) in &self.rows[source_row] {
            let slot = match self.pending.iter().position(|(row, _)| *row == target) {
                Some(slot) => slot,
                None => {
                    self.pending
                        .push((target, vec![0.0; self.target_width * channels]));
                    self.pending.len() - 1
                }
            };
            for (sum, value) in self.pending[slot].1.iter_mut().zip(&self.wide) {
                *sum += weight * value;
            }
        }
        let mut done = Vec::new();
        while let Some(slot) = self
            .pending
            .iter()
            .position(|(row, _)| self.last_source[*row] == source_row)
        {
            let (_, sums) = self.pending.remove(slot);
            done.push(self.finish(&sums));
        }
        done
    }

    /// Target bytes from sums: color divided back out of alpha.
    fn finish(&self, sums: &[f64]) -> Vec<u8> {
        let channels = self.channels;
        let mut out = vec![0u8; sums.len()];
        for (target, cell) in out
            .chunks_exact_mut(channels)
            .zip(sums.chunks_exact(channels))
        {
            let alpha = if self.alpha {
                cell[channels - 1]
            } else {
                255.0
            };
            for channel in 0..channels {
                let value = if self.alpha && channel == channels - 1 {
                    cell[channel]
                } else if alpha > 0.0 {
                    cell[channel] * 255.0 / alpha
                } else {
                    0.0
                };
                target[channel] = value.round().clamp(0.0, 255.0) as u8;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(width: u32, height: u32, color: ColorType, pixels: &[u8], to: (u32, u32)) -> Vec<u8> {
        let mut scale = Downscale::new(width, height, to.0, to.1, color);
        let row = width as usize * color.channels();
        let mut out = Vec::new();
        for source in pixels.chunks_exact(row) {
            for done in scale.row(source) {
                out.extend(done);
            }
        }
        out
    }

    #[test]
    fn a_constant_image_stays_constant_at_any_size() {
        let pixels = [77u8, 140, 201].repeat(37 * 23);
        for to in [(1, 1), (5, 3), (16, 16), (36, 22)] {
            let out = run(37, 23, ColorType::Rgb, &pixels, to);
            assert_eq!(out.len(), (to.0 * to.1 * 3) as usize, "{to:?}");
            assert!(
                out.chunks_exact(3).all(|cell| cell == [77, 140, 201]),
                "{to:?}"
            );
        }
    }

    #[test]
    fn a_two_by_two_block_averages() {
        let out = run(2, 2, ColorType::Gray, &[0, 100, 200, 60], (1, 1));
        assert_eq!(out, vec![90]);
    }

    #[test]
    fn a_fractional_edge_is_weighted_by_coverage() {
        // Three pixels into two: the middle one is split between them.
        let out = run(3, 1, ColorType::Gray, &[0, 90, 180], (2, 1));
        assert_eq!(out, vec![30, 150]);
    }

    #[test]
    fn transparent_pixels_do_not_bleed_color() {
        // Red opaque beside green fully transparent: the average is red
        // at half alpha, not a brown.
        let pixels = [255, 0, 0, 255, 0, 255, 0, 0];
        let out = run(2, 1, ColorType::Rgba, &pixels, (1, 1));
        assert_eq!(out, vec![255, 0, 0, 128]);
    }
}
