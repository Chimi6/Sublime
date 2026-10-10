//! Residuals to levels and back: the forward DCT and DST (the transposes
//! of the decoder's matrices, with HM's shifts), quantization, and the
//! decoder's own scaling and inverse transforms for the reconstruction,
//! so it is the reconstruction any decoder makes.

use super::super::transform::{self, DCT32, DST4, LEVEL_SCALE};

/// The quantizer's scale of each `qP % 6`: 2^14 over the level scale's
/// step, so a level is a coefficient times this over 2^(14 + qP / 6).
pub const QUANT_SCALE: [i64; 6] = [26214, 23302, 20560, 18396, 16384, 14564];

/// The DCT matrix's rows for each size, widened: `DCT[log2 - 2][k][x]`.
fn dct(log2: u32) -> &'static [[i32; 32]; 32] {
    static TABLES: std::sync::OnceLock<[[[i32; 32]; 32]; 4]> = std::sync::OnceLock::new();
    &TABLES.get_or_init(|| {
        std::array::from_fn(|size| {
            let step = 1usize << (3 - size);
            std::array::from_fn(|k| {
                std::array::from_fn(|x| {
                    if k * step < 32 {
                        i32::from(DCT32[k * step][x])
                    } else {
                        0
                    }
                })
            })
        })
    })[log2 as usize - 2]
}

/// One dimension of the forward DCT of `n` samples (`n` from 4 to 32):
/// the even outputs are the half-size DCT of the sums of mirrored
/// samples, the odd ones the differences against the odd rows, so a
/// size takes a third of a matrix product's multiplies. Sums are exact:
/// the same as the matrix product.
fn dct_1d(input: &[i32], output: &mut [i32], n: usize) {
    let log2 = n.trailing_zeros();
    let matrix = dct(log2);
    if n == 4 {
        for (k, out) in output.iter_mut().take(4).enumerate() {
            *out = (0..4).map(|x| matrix[k][x] * input[x]).sum();
        }
        return;
    }
    let half = n / 2;
    let mut even = [0i32; 16];
    let mut odd = [0i32; 16];
    for x in 0..half {
        even[x] = input[x] + input[n - 1 - x];
        odd[x] = input[x] - input[n - 1 - x];
    }
    let mut even_out = [0i32; 16];
    dct_1d(&even[..half], &mut even_out[..half], half);
    for k in 0..half {
        output[2 * k] = even_out[k];
        let row = &matrix[2 * k + 1];
        let mut sum = 0;
        for x in 0..half {
            sum += row[x] * odd[x];
        }
        output[2 * k + 1] = sum;
    }
}

/// The forward transform of an `n` by `n` residual, row-major, into
/// coefficients (vertical frequency by row): rows first, then columns,
/// with HM's shifts.
#[cfg(test)]
pub fn forward(residual: &[i32], coefficients: &mut [i32], log2: u32, dst: bool, bit_depth: u32) {
    let mut middle = [0i32; 32 * 32];
    let n = 1usize << log2;
    forward_with(
        residual,
        coefficients,
        &mut middle[..n * n],
        log2,
        dst,
        bit_depth,
    );
}

/// `forward`, its intermediate in `middle` (`n * n` long).
pub fn forward_with(
    residual: &[i32],
    coefficients: &mut [i32],
    middle: &mut [i32],
    log2: u32,
    dst: bool,
    bit_depth: u32,
) {
    let n = 1usize << log2;
    let shift1 = log2 as i32 + bit_depth as i32 - 9;
    let shift2 = log2 as i32 + 6;
    let round1 = 1i32 << (shift1 - 1).max(0);
    let round2 = 1i32 << (shift2 - 1);
    let mut line = [0i32; 32];
    let mut out = [0i32; 32];
    // First stage: each row's horizontal frequencies, kept transposed
    // (frequency by row) for the second stage.
    for y in 0..n {
        let row = &residual[y * n..(y + 1) * n];
        if dst {
            for (u, value) in out.iter_mut().take(4).enumerate() {
                *value = (0..4).map(|x| DST4[u][x] * row[x]).sum();
            }
        } else {
            dct_1d(row, &mut out, n);
        }
        for u in 0..n {
            middle[u * n + y] = if shift1 > 0 {
                (out[u] + round1) >> shift1
            } else {
                out[u] << -shift1
            };
        }
    }
    // Second stage: each column's vertical frequencies.
    for u in 0..n {
        line[..n].copy_from_slice(&middle[u * n..(u + 1) * n]);
        if dst {
            for (v, value) in out.iter_mut().take(4).enumerate() {
                *value = (0..4).map(|y| DST4[v][y] * line[y]).sum();
            }
        } else {
            dct_1d(&line[..n], &mut out, n);
        }
        for v in 0..n {
            coefficients[v * n + u] = (out[v] + round2) >> shift2;
        }
    }
}

/// Transform skip's forward scaling: the residual shifted up to the
/// coefficient range (the inverse of `transform::transform_skip`).
pub fn forward_skip(residual: &[i32], coefficients: &mut [i32], log2: u32, bit_depth: u32) {
    let n = 1usize << log2;
    let shift = 15 - bit_depth as i32 - log2 as i32;
    for (out, &value) in coefficients[..n * n].iter_mut().zip(&residual[..n * n]) {
        *out = if shift >= 0 {
            value << shift
        } else {
            value >> -shift
        };
    }
}

/// How a block is quantized.
#[derive(Clone, Copy)]
pub struct Quantizer {
    pub qp: i32,
    pub bit_depth: u32,
}

impl Quantizer {
    /// The shift of a level for a block of `log2`.
    #[cfg(test)]
    fn qbits(&self, log2: u32) -> i32 {
        14 + self.qp / 6 + (15 - self.bit_depth as i32 - log2 as i32)
    }

    /// Quantizes with a dead zone (rounding a third of a step, intra's
    /// offset): the plain quantizer RDOQ improves on, kept to test the
    /// transforms. Each level's rounding error goes in `errors`, in 1/256
    /// of a step.
    #[cfg(test)]
    pub fn quantize(
        &self,
        coefficients: &[i32],
        levels: &mut [i32],
        errors: &mut [i32],
        log2: u32,
    ) -> bool {
        let n = 1usize << log2;
        let qbits = self.qbits(log2);
        let scale = QUANT_SCALE[(self.qp % 6) as usize];
        let offset = 171i64 << (qbits - 9);
        let mut any = false;
        for ((level, error), &coefficient) in levels[..n * n]
            .iter_mut()
            .zip(errors[..n * n].iter_mut())
            .zip(&coefficients[..n * n])
        {
            let scaled = i64::from(coefficient.abs()) * scale;
            let magnitude = ((scaled + offset) >> qbits).min(32767);
            // How far the coefficient sat above its level, in 1/256 of a
            // step (above the dead zone's offset: positive rounds up next).
            *error = ((scaled - (magnitude << qbits)) >> (qbits - 8)) as i32;
            *level = if coefficient < 0 {
                -(magnitude as i32)
            } else {
                magnitude as i32
            };
            any |= magnitude != 0;
        }
        any
    }

    /// The decoder's scaling of levels (8.6.3), then its inverse
    /// transform: the residual a decoder adds.
    pub fn reconstruct(
        &self,
        levels: &[i32],
        residual: &mut [i32],
        log2: u32,
        dst: bool,
        transform_skip: bool,
    ) {
        let n = 1usize << log2;
        let scale = i64::from(LEVEL_SCALE[(self.qp % 6) as usize] << (self.qp / 6));
        let shift = self.bit_depth + log2 - 5;
        let round = 1i64 << (shift - 1);
        let (mut rows, mut columns) = (0, 0);
        for (index, (out, &level)) in residual[..n * n].iter_mut().zip(levels).enumerate() {
            *out = if level == 0 {
                0
            } else {
                rows = rows.max(index / n + 1);
                columns = columns.max(index % n + 1);
                ((i64::from(level) * 16 * scale + round) >> shift).clamp(-32768, 32767) as i32
            };
        }
        if transform_skip {
            transform::transform_skip(residual, log2, self.bit_depth, false);
        } else {
            transform::inverse_transform(residual, log2, dst, self.bit_depth, rows, columns);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A residual through the forward transform, fine quantization, and
    /// the decoder's inverse comes back within a step.
    #[test]
    fn transforms_round_trip() {
        for log2 in 2..=5u32 {
            for dst in [false, true] {
                if dst && log2 != 2 {
                    continue;
                }
                let n = 1usize << log2;
                let residual: Vec<i32> = (0..n * n)
                    .map(|index| ((index * 37 + index / n * 11) % 61) as i32 - 30)
                    .collect();
                let mut coefficients = vec![0; n * n];
                forward(&residual, &mut coefficients, log2, dst, 8);
                let quantizer = Quantizer {
                    qp: 4,
                    bit_depth: 8,
                };
                let mut levels = vec![0; n * n];
                let mut errors = vec![0; n * n];
                quantizer.quantize(&coefficients, &mut levels, &mut errors, log2);
                let mut back = vec![0; n * n];
                quantizer.reconstruct(&levels, &mut back, log2, dst, false);
                for (index, (&a, &b)) in residual.iter().zip(&back).enumerate() {
                    assert!(
                        (a - b).abs() <= 2,
                        "log2 {log2} dst {dst} at {index}: {a} {b}"
                    );
                }
            }
        }
    }
}
