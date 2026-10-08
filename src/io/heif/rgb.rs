//! Decoded planes to 8-bit RGB (or gray), a row at a time: the chroma
//! brought up to the luma's size by interpolating between its samples,
//! each centred among the luma samples it covers (as Apple's decoder and
//! libheif both take it), then the matrix the file names.

use super::Planes;

/// Fraction bits of the fixed-point arithmetic.
const FRACTION: u32 = 16;
/// The upsampled chroma's scale: weights across and down each sum to 4.
const CHROMA_SCALE: i64 = 16;

/// How a row of planes becomes a row of pixels.
pub struct Converter<'a> {
    planes: &'a Planes,
    /// Luma: the offset (black) and the multiplier to 8 bits.
    luma_offset: i64,
    luma_scale: i64,
    /// Chroma: the midpoint, and the coefficients on Cb and Cr for red,
    /// green, and blue, on chroma upsampled by `CHROMA_SCALE`.
    chroma_mid: i64,
    matrix: Matrix,
    cb: [Vec<i64>; 2],
    /// Scratch rows: luma, and two chroma rows widened.
    luma: Vec<i64>,
    near: Vec<i64>,
    far: Vec<i64>,
}

enum Matrix {
    /// Red from Cr, green from both, blue from Cb.
    Luma {
        r_cr: i64,
        g_cb: i64,
        g_cr: i64,
        b_cb: i64,
    },
    /// GBR stored as Y, Cb, Cr (matrix 0).
    Identity { chroma_scale: i64 },
    /// YCgCo (matrix 8).
    CgCo { chroma_scale: i64 },
}

impl<'a> Converter<'a> {
    /// `matrix` is the H.273 matrix coefficients code; `full_range` says
    /// whether samples use their whole range or video's 16-235.
    pub fn new(planes: &'a Planes, matrix: u16, full_range: bool) -> Converter<'a> {
        let unit = 1i64 << FRACTION;
        let depth = planes.bit_depth;
        let (luma_offset, luma_range) = if full_range {
            (0, (1i64 << depth) - 1)
        } else {
            (16i64 << (depth - 8), 219i64 << (depth - 8))
        };
        let chroma_depth = planes.bit_depth_chroma;
        let chroma_range = if full_range {
            (1i64 << chroma_depth) - 1
        } else {
            224i64 << (chroma_depth - 8)
        };
        let luma_scale = (255 * unit + luma_range / 2) / luma_range;
        // 8-bit units per upsampled chroma step, times a coefficient.
        let per_chroma = 255.0 * unit as f64 / (chroma_range as f64 * CHROMA_SCALE as f64);
        let fixed = |coefficient: f64| (coefficient * per_chroma).round() as i64;
        let matrix = match matrix {
            0 => Matrix::Identity {
                chroma_scale: (255 * unit + luma_range / 2) / luma_range,
            },
            8 => Matrix::CgCo {
                chroma_scale: fixed(1.0),
            },
            code => {
                let (kr, kb) = match code {
                    1 => (0.2126, 0.0722),
                    4 => (0.30, 0.11),
                    7 => (0.212, 0.087),
                    9 | 10 => (0.2627, 0.0593),
                    _ => (0.299, 0.114),
                };
                let kg = 1.0 - kr - kb;
                Matrix::Luma {
                    r_cr: fixed(2.0 * (1.0 - kr)),
                    g_cb: fixed(2.0 * kb * (1.0 - kb) / kg),
                    g_cr: fixed(2.0 * kr * (1.0 - kr) / kg),
                    b_cb: fixed(2.0 * (1.0 - kb)),
                }
            }
        };
        let chroma_width = planes.sizes.get(1).map_or(0, |size| size.0);
        Converter {
            planes,
            luma_offset,
            luma_scale,
            chroma_mid: 1i64 << (chroma_depth - 1),
            matrix,
            cb: [vec![0; chroma_width], vec![0; chroma_width]],
            luma: vec![0; planes.width],
            near: vec![0; chroma_width],
            far: vec![0; chroma_width],
        }
    }

    /// Channels per pixel: 1 for monochrome, else 3.
    pub fn channels(&self) -> usize {
        if self.planes.planes.len() < 3 { 1 } else { 3 }
    }

    /// Row `y` of the planes, as `channels()` bytes per pixel.
    pub fn row(&mut self, y: usize, out: &mut [u8]) {
        let planes = self.planes;
        let width = planes.width;
        planes.planes[0].widen(y * width, width, &mut self.luma);
        let luma = &self.luma;
        let round = 1i64 << (FRACTION - 1);
        let to_byte = |value: i64| ((value + round) >> FRACTION).clamp(0, 255) as u8;
        if self.channels() == 1 {
            for (pixel, &sample) in out.iter_mut().zip(luma) {
                *pixel = to_byte((sample - self.luma_offset) * self.luma_scale);
            }
            return;
        }
        // Down: chroma rows sit between luma rows in 4:2:0, so luma row
        // 2j takes 3/4 of chroma row j and 1/4 of row j-1, row 2j+1 of j+1.
        let (chroma_width, chroma_height) = planes.sizes[1];
        let vertical = planes.chroma_format == 1;
        for component in 0..2 {
            let plane = &planes.planes[component + 1];
            let target = &mut self.cb[component];
            if vertical {
                let j = y / 2;
                let other = if y % 2 == 0 {
                    j.saturating_sub(1)
                } else {
                    (j + 1).min(chroma_height - 1)
                };
                plane.widen(j * chroma_width, chroma_width, &mut self.near);
                plane.widen(other * chroma_width, chroma_width, &mut self.far);
                for ((value, &a), &b) in target.iter_mut().zip(&self.near).zip(&self.far) {
                    *value = 3 * a + b;
                }
            } else {
                plane.widen(y * chroma_width, chroma_width, &mut self.near);
                for (value, &a) in target.iter_mut().zip(&self.near) {
                    *value = 4 * a;
                }
            }
        }
        let horizontal = planes.chroma_format != 3;
        // Across the same way: column 2i takes 3/4 of chroma column i and
        // 1/4 of column i-1, column 2i+1 of i+1.
        let mid = self.chroma_mid * CHROMA_SCALE;
        let last = chroma_width - 1;
        let [cb, cr] = &self.cb;
        let chroma_at = |line: &[i64], x: usize| -> i64 {
            if horizontal {
                let i = x / 2;
                let other = if x % 2 == 0 {
                    i.saturating_sub(1)
                } else {
                    (i + 1).min(last)
                };
                3 * line[i] + line[other]
            } else {
                4 * line[x]
            }
        };
        for (x, (pixel, &sample)) in out.chunks_exact_mut(3).zip(luma).enumerate() {
            let u = chroma_at(cb, x) - mid;
            let v = chroma_at(cr, x) - mid;
            let (r, g, b) = match self.matrix {
                Matrix::Luma {
                    r_cr,
                    g_cb,
                    g_cr,
                    b_cb,
                } => {
                    let base = (sample - self.luma_offset) * self.luma_scale;
                    (base + r_cr * v, base - g_cb * u - g_cr * v, base + b_cb * u)
                }
                Matrix::Identity { chroma_scale } => (
                    (v / CHROMA_SCALE + self.chroma_mid - self.luma_offset) * chroma_scale,
                    (sample - self.luma_offset) * self.luma_scale,
                    (u / CHROMA_SCALE + self.chroma_mid - self.luma_offset) * chroma_scale,
                ),
                Matrix::CgCo { chroma_scale } => {
                    let base = (sample - self.luma_offset) * self.luma_scale;
                    let (cg, co) = (u * chroma_scale, v * chroma_scale);
                    (base - cg + co, base + cg, base - cg - co)
                }
            };
            pixel[0] = to_byte(r);
            pixel[1] = to_byte(g);
            pixel[2] = to_byte(b);
        }
    }
}
