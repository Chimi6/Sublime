//! Decoded planes to RGB (or gray), a row at a time, at 8 or 16 bits: the
//! chroma brought up to the luma's size by interpolating between its
//! samples, each centred among the luma samples it covers, then the
//! matrix the file names. Every step is a loop over a whole row of plain
//! integers, so the compiler runs it in vector lanes.

use super::{Planes, Samples};

/// The upsampled chroma's scale: weights across and down each sum to 4.
const CHROMA_SCALE: i32 = 16;

/// How a row of planes becomes a row of pixels.
pub struct Converter<'a> {
    planes: &'a Planes,
    /// The picture row the planes' first luma row is (a window of a
    /// picture streamed by bands), and the picture's height in rows.
    origin: usize,
    picture_height: usize,
    /// Fraction bits of the fixed-point arithmetic: 16 for 8-bit output
    /// (whose sums stay in 32 bits), 24 for 16-bit (worked in 64).
    fraction: u32,
    max: i32,
    /// Luma: the offset (black) and the multiplier to the output's range.
    luma_offset: i32,
    luma_scale: i64,
    /// Chroma: the midpoint, upsampled.
    chroma_mid: i32,
    matrix: Matrix,
    /// Scratch rows: luma, the chroma rows interpolated down, then across.
    luma: Vec<i32>,
    near: Vec<i32>,
    far: Vec<i32>,
    down: [Vec<i32>; 2],
    across: [Vec<i32>; 2],
    channels: [Vec<i32>; 3],
}

#[derive(Clone, Copy)]
enum Matrix {
    /// Red from Cr, green from both, blue from Cb.
    Luma {
        r_cr: i64,
        g_cb: i64,
        g_cr: i64,
        b_cb: i64,
    },
    /// GBR stored as Y, Cb, Cr (matrix 0).
    Identity,
    /// YCgCo (matrix 8).
    CgCo { scale: i64 },
}

impl<'a> Converter<'a> {
    /// `matrix` is the H.273 matrix coefficients code; `full_range` says
    /// whether samples use their whole range or video's 16-235; `deep`
    /// asks for 16-bit output.
    pub fn new(planes: &'a Planes, matrix: u16, full_range: bool, deep: bool) -> Converter<'a> {
        let fraction = if deep { 24 } else { 16 };
        let unit = (1u64 << fraction) as f64;
        let max = if deep { 65_535 } else { 255 };
        let depth = planes.bit_depth;
        let (luma_offset, luma_range) = if full_range {
            (0, (1i32 << depth) - 1)
        } else {
            (16 << (depth - 8), 219 << (depth - 8))
        };
        let chroma_depth = planes.bit_depth_chroma;
        let chroma_range = if full_range {
            (1i32 << chroma_depth) - 1
        } else {
            224 << (chroma_depth - 8)
        };
        let luma_scale = (f64::from(max) * unit / f64::from(luma_range)).round() as i64;
        // Output units per upsampled chroma step, times a coefficient.
        let per_chroma =
            f64::from(max) * unit / (f64::from(chroma_range) * f64::from(CHROMA_SCALE));
        let fixed = |coefficient: f64| (coefficient * per_chroma).round() as i64;
        let matrix = match matrix {
            0 => Matrix::Identity,
            8 => Matrix::CgCo { scale: fixed(1.0) },
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
        let width = planes.width;
        let chroma_width = planes.sizes.get(1).map_or(0, |size| size.0);
        Converter {
            planes,
            origin: 0,
            picture_height: planes.height,
            fraction,
            max,
            luma_offset,
            luma_scale,
            chroma_mid: (1i32 << (chroma_depth - 1)) * CHROMA_SCALE,
            matrix,
            luma: vec![0; width],
            near: vec![0; chroma_width],
            far: vec![0; chroma_width],
            down: [vec![0; chroma_width], vec![0; chroma_width]],
            across: [vec![0; width], vec![0; width]],
            channels: [vec![0; width], vec![0; width], vec![0; width]],
        }
    }

    /// Reads `planes` as rows `origin` on of a picture `picture_height`
    /// rows high (`origin` even, so chroma rows line up in 4:2:0).
    pub fn windowed(mut self, origin: usize, picture_height: usize) -> Converter<'a> {
        self.origin = origin;
        self.picture_height = picture_height;
        self
    }

    /// Channels per pixel: 1 for monochrome, else 3.
    pub fn channels(&self) -> usize {
        if self.planes.planes.len() < 3 { 1 } else { 3 }
    }

    /// Row `y`'s pixels from column `x0`, `out.len() / channels()` of
    /// them, a byte per sample.
    pub fn row(&mut self, y: usize, x0: usize, out: &mut [u8]) {
        let count = out.len() / self.channels();
        if let (3, Matrix::Luma { .. }) = (self.channels(), self.matrix) {
            self.row_luma_matrix(y, x0, out);
            return;
        }
        self.compute(y, x0, count);
        if self.channels() == 1 {
            for (pixel, &value) in out.iter_mut().zip(&self.channels[0][..count]) {
                *pixel = value as u8;
            }
            return;
        }
        let [r, g, b] = &self.channels;
        for (((pixel, &r), &g), &b) in out
            .chunks_exact_mut(3)
            .zip(&r[..count])
            .zip(&g[..count])
            .zip(&b[..count])
        {
            pixel[0] = r as u8;
            pixel[1] = g as u8;
            pixel[2] = b as u8;
        }
    }

    /// The common case in one pass: YCbCr by a luma matrix straight into
    /// RGB bytes.
    fn row_luma_matrix(&mut self, y: usize, x0: usize, out: &mut [u8]) {
        let count = out.len() / 3;
        let Matrix::Luma {
            r_cr,
            g_cb,
            g_cr,
            b_cb,
        } = self.matrix
        else {
            return;
        };
        // At 16 fraction bits every product and sum fits 32 bits.
        let (r_cr, g_cb, g_cr, b_cb) = (r_cr as i32, g_cb as i32, g_cr as i32, b_cb as i32);
        let row = y - self.origin;
        widen(
            &self.planes.planes[0],
            row * self.planes.width + x0,
            &mut self.luma[..count],
        );
        self.chroma(y, x0, count);
        let round = 1i32 << (self.fraction - 1);
        let fraction = self.fraction;
        let (offset, scale) = (self.luma_offset, self.luma_scale as i32);
        let byte = |value: i32| ((value + round) >> fraction).clamp(0, 255) as u8;
        let [u, v] = &self.across;
        for (((pixel, &luma), &u), &v) in out
            .chunks_exact_mut(3)
            .zip(&self.luma[..count])
            .zip(&u[..count])
            .zip(&v[..count])
        {
            let base = (luma - offset) * scale;
            pixel[0] = byte(base + r_cr * v);
            pixel[1] = byte(base - g_cb * u - g_cr * v);
            pixel[2] = byte(base + b_cb * u);
        }
    }

    /// Row `y`'s pixels from column `x0` at 16 bits, big-endian, two
    /// bytes per sample (as PNG and TIFF hold them).
    pub fn row_deep(&mut self, y: usize, x0: usize, out: &mut [u8]) {
        let channels = self.channels();
        let count = out.len() / (2 * channels);
        self.compute(y, x0, count);
        for (index, pixel) in out.chunks_exact_mut(2 * channels).enumerate() {
            for (channel, sample) in pixel.chunks_exact_mut(2).enumerate() {
                sample.copy_from_slice(&(self.channels[channel][index] as u16).to_be_bytes());
            }
        }
    }

    /// Fills the channel rows with `count` output samples from column
    /// `x0`, shifted and clamped to the output's range.
    fn compute(&mut self, y: usize, x0: usize, count: usize) {
        let planes = self.planes;
        let width = planes.width;
        widen(
            &planes.planes[0],
            (y - self.origin) * width + x0,
            &mut self.luma[..count],
        );
        let round = 1i64 << (self.fraction - 1);
        let (fraction, max) = (self.fraction, i64::from(self.max));
        let (offset, scale) = (self.luma_offset, self.luma_scale);
        let finish = |value: i64| ((value + round) >> fraction).clamp(0, max) as i32;
        let base = |sample: i32| i64::from(sample - offset) * scale;
        if self.channels() == 1 {
            for (out, &sample) in self.channels[0][..count]
                .iter_mut()
                .zip(&self.luma[..count])
            {
                *out = finish(base(sample));
            }
            return;
        }
        self.chroma(y, x0, count);
        let [u, v] = &self.across;
        let [r, g, b] = &mut self.channels;
        let luma = &self.luma[..count];
        let (u, v) = (&u[..count], &v[..count]);
        let (r, g, b) = (&mut r[..count], &mut g[..count], &mut b[..count]);
        match self.matrix {
            Matrix::Luma {
                r_cr,
                g_cb,
                g_cr,
                b_cb,
            } => {
                for ((((r, g), b), (&luma, &u)), &v) in r
                    .iter_mut()
                    .zip(g.iter_mut())
                    .zip(b.iter_mut())
                    .zip(luma.iter().zip(u))
                    .zip(v)
                {
                    let (base, u, v) = (base(luma), i64::from(u), i64::from(v));
                    *r = finish(base + r_cr * v);
                    *g = finish(base - g_cb * u - g_cr * v);
                    *b = finish(base + b_cb * u);
                }
            }
            Matrix::Identity => {
                // u and v are centred and upsampled; undo both.
                let mid = self.chroma_mid / CHROMA_SCALE;
                for index in 0..count {
                    g[index] = finish(base(luma[index]));
                    b[index] = finish(base(u[index] / CHROMA_SCALE + mid));
                    r[index] = finish(base(v[index] / CHROMA_SCALE + mid));
                }
            }
            Matrix::CgCo { scale: chroma } => {
                for index in 0..count {
                    let base = base(luma[index]);
                    let (cg, co) = (i64::from(u[index]) * chroma, i64::from(v[index]) * chroma);
                    r[index] = finish(base - cg + co);
                    g[index] = finish(base + cg);
                    b[index] = finish(base - cg - co);
                }
            }
        }
    }

    /// The chroma of row `y` for columns `x0` on, interpolated down then
    /// across, centred: `across` holds Cb and Cr times `CHROMA_SCALE`
    /// less the midpoint.
    fn chroma(&mut self, y: usize, x0: usize, count: usize) {
        let planes = self.planes;
        let chroma_width = planes.sizes[1].0;
        // Rows are the picture's; the window holds them from its origin.
        let shift_y = u32::from(planes.chroma_format == 1);
        let chroma_height = self.picture_height.div_ceil(1 << shift_y);
        let chroma_origin = self.origin >> shift_y;
        let mid = self.chroma_mid;
        for component in 0..2 {
            let plane = &planes.planes[component + 1];
            let down = &mut self.down[component];
            // Down: chroma rows sit between luma rows in 4:2:0, so luma
            // row 2j takes 3/4 of chroma row j and 1/4 of row j-1, and
            // row 2j+1 1/4 of row j+1.
            if planes.chroma_format == 1 {
                let j = y / 2;
                let other = if y % 2 == 0 {
                    j.saturating_sub(1)
                } else {
                    (j + 1).min(chroma_height - 1)
                };
                widen(plane, (j - chroma_origin) * chroma_width, &mut self.near);
                widen(plane, (other - chroma_origin) * chroma_width, &mut self.far);
                for ((value, &near), &far) in down.iter_mut().zip(&self.near).zip(&self.far) {
                    *value = 3 * near + far;
                }
            } else {
                widen(plane, (y - chroma_origin) * chroma_width, &mut self.near);
                for (value, &near) in down.iter_mut().zip(&self.near) {
                    *value = 4 * near;
                }
            }
            // Across the same way: column 2i takes 3/4 of chroma column i
            // and 1/4 of column i-1, column 2i+1 of column i+1.
            let across = &mut self.across[component][..count];
            if planes.chroma_format == 3 {
                for (value, &down) in across.iter_mut().zip(&down[x0..x0 + count]) {
                    *value = 4 * down - mid;
                }
                continue;
            }
            let last = chroma_width - 1;
            let at = |i: usize| down[i.min(last)];
            let one = |x: usize| -> i32 {
                let i = x / 2;
                let other = if x % 2 == 0 {
                    down[i.saturating_sub(1)]
                } else {
                    at(i + 1)
                };
                3 * down[i] + other - mid
            };
            // The interior in pairs from an even column whose chroma
            // column has neighbours on both sides; the edges one by one.
            let first_pair = (x0.max(2) + 1) & !1;
            let end = x0 + count;
            let pairs_end = (last * 2).min(end) & !1;
            if first_pair >= pairs_end {
                for (offset, value) in across.iter_mut().enumerate() {
                    *value = one(x0 + offset);
                }
                continue;
            }
            for x in x0..first_pair {
                across[x - x0] = one(x);
            }
            let columns = &down[first_pair / 2 - 1..pairs_end / 2 + 1];
            for (pair, window) in across[first_pair - x0..pairs_end - x0]
                .chunks_exact_mut(2)
                .zip(columns.windows(3))
            {
                let centre = 3 * window[1] - mid;
                pair[0] = centre + window[0];
                pair[1] = centre + window[2];
            }
            for x in pairs_end..end {
                across[x - x0] = one(x);
            }
        }
    }
}

/// `out.len()` samples from `start`, widened.
fn widen(samples: &Samples, start: usize, out: &mut [i32]) {
    let end = start + out.len();
    match samples {
        Samples::Eight(samples) => {
            for (value, &sample) in out.iter_mut().zip(&samples[start..end]) {
                *value = i32::from(sample);
            }
        }
        Samples::Deep(samples) => {
            for (value, &sample) in out.iter_mut().zip(&samples[start..end]) {
                *value = i32::from(sample);
            }
        }
    }
}
