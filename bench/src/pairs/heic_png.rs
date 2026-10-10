//! PNG -> HEIC against libheif's `heif-enc` (x265) and macOS `sips`
//! (Apple's encoder). A size at a quality number means nothing across
//! encoders, so every output is decoded by the same decoder (ours,
//! bit-exact with libheif's planes) and scored against the source:
//! PSNR over RGB and over luma, and SSIM over luma.

use sublime::image::ColorType;
use sublime::io::png::RowSink;

pub fn run(mode: &str, args: &[String]) -> Result<(), String> {
    match (mode, args) {
        ("quality", [original, candidate]) => quality(original, candidate),
        ("bd", [points, ours, other]) => bd(points, ours, other),
        _ => Err(
            "heic-png modes: quality <original.png> <candidate.heic>, bd <points> <ours> <other>"
                .to_string(),
        ),
    }
}

/// RGB rows kept whole.
#[derive(Default)]
struct Rgb {
    width: usize,
    height: usize,
    color: Option<ColorType>,
    pixels: Vec<u8>,
}

impl RowSink for Rgb {
    fn start(&mut self, width: u32, height: u32, color: ColorType) -> std::io::Result<()> {
        self.width = width as usize;
        self.height = height as usize;
        self.color = Some(color);
        Ok(())
    }

    fn row(&mut self, pixels: &[u8]) -> std::io::Result<()> {
        let channels = self.color.map_or(3, ColorType::channels);
        for pixel in pixels.chunks_exact(channels) {
            match channels {
                1 | 2 => self.pixels.extend_from_slice(&[pixel[0]; 3]),
                _ => self.pixels.extend_from_slice(&pixel[..3]),
            }
        }
        Ok(())
    }
}

fn read_rgb(path: &str) -> Result<Rgb, String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let mut rgb = Rgb::default();
    if path.ends_with(".png") {
        let mut source: &[u8] = &bytes;
        sublime::io::png::read_png_rows(&mut source, &mut rgb).map_err(|error| format!("{error:?}"))?;
    } else {
        sublime::io::heif::read_heif_rows(&bytes, &mut rgb).map_err(|error| format!("{error:?}"))?;
    }
    Ok(rgb)
}

fn luma(pixels: &[u8]) -> Vec<f64> {
    pixels
        .chunks_exact(3)
        .map(|p| 0.299 * f64::from(p[0]) + 0.587 * f64::from(p[1]) + 0.114 * f64::from(p[2]))
        .collect()
}

fn psnr(squares: f64, count: usize) -> f64 {
    let mean = squares / count as f64;
    if mean == 0.0 {
        f64::INFINITY
    } else {
        10.0 * (255.0 * 255.0 / mean).log10()
    }
}

/// SSIM over a plane (Wang et al.: an 11-tap Gaussian window of sigma
/// 1.5, constants for 8-bit samples), the mean of its map.
fn ssim(a: &[f64], b: &[f64], width: usize, height: usize) -> f64 {
    let taps: Vec<f64> = {
        let raw: Vec<f64> = (-5i32..=5)
            .map(|k| (-(f64::from(k * k)) / (2.0 * 1.5 * 1.5)).exp())
            .collect();
        let sum: f64 = raw.iter().sum();
        raw.iter().map(|value| value / sum).collect()
    };
    let blur = |values: &dyn Fn(usize) -> f64| -> Vec<f64> {
        let mut across = vec![0.0; width * height];
        for y in 0..height {
            for x in 5..width.saturating_sub(5) {
                let mut sum = 0.0;
                for (k, tap) in taps.iter().enumerate() {
                    sum += tap * values(y * width + x + k - 5);
                }
                across[y * width + x] = sum;
            }
        }
        let mut out = vec![0.0; width * height];
        for y in 5..height.saturating_sub(5) {
            for x in 5..width.saturating_sub(5) {
                let mut sum = 0.0;
                for (k, tap) in taps.iter().enumerate() {
                    sum += tap * across[(y + k - 5) * width + x];
                }
                out[y * width + x] = sum;
            }
        }
        out
    };
    let mean_a = blur(&|i| a[i]);
    let mean_b = blur(&|i| b[i]);
    let square_a = blur(&|i| a[i] * a[i]);
    let square_b = blur(&|i| b[i] * b[i]);
    let product = blur(&|i| a[i] * b[i]);
    let (c1, c2) = ((0.01f64 * 255.0).powi(2), (0.03f64 * 255.0).powi(2));
    let mut total = 0.0;
    let mut count = 0usize;
    for y in 5..height.saturating_sub(5) {
        for x in 5..width.saturating_sub(5) {
            let i = y * width + x;
            let (ma, mb) = (mean_a[i], mean_b[i]);
            let va = square_a[i] - ma * ma;
            let vb = square_b[i] - mb * mb;
            let cov = product[i] - ma * mb;
            total += ((2.0 * ma * mb + c1) * (2.0 * cov + c2))
                / ((ma * ma + mb * mb + c1) * (va + vb + c2));
            count += 1;
        }
    }
    total / count.max(1) as f64
}

/// Prints PSNR over RGB, PSNR over luma, and SSIM over luma.
fn quality(original: &str, candidate: &str) -> Result<(), String> {
    let a = read_rgb(original)?;
    let b = read_rgb(candidate)?;
    if (a.width, a.height) != (b.width, b.height) || a.pixels.len() != b.pixels.len() {
        return Err("dimensions differ".to_string());
    }
    let squares: f64 = a
        .pixels
        .iter()
        .zip(&b.pixels)
        .map(|(&x, &y)| (f64::from(x) - f64::from(y)).powi(2))
        .sum();
    let (la, lb) = (luma(&a.pixels), luma(&b.pixels));
    let luma_squares: f64 = la.iter().zip(&lb).map(|(x, y)| (x - y).powi(2)).sum();
    println!(
        "{:.3} {:.3} {:.5}",
        psnr(squares, a.pixels.len()),
        psnr(luma_squares, la.len()),
        ssim(&la, &lb, a.width, a.height)
    );
    Ok(())
}

/// One coded image's point: its size and its scores.
struct Point {
    bytes: f64,
    psnr_rgb: f64,
    psnr_y: f64,
    ssim_db: f64,
    /// SSIMULACRA2, when the line has it.
    ssimulacra2: Option<f64>,
}

/// The mean log-rate difference of `curve` against `reference` over the
/// quality range both cover, the rate interpolated piecewise linearly in
/// its log (Bjontegaard's measure, without the cubic fit): negative when
/// `curve` takes fewer bytes for the same quality.
fn bd_rate(reference: &[(f64, f64)], curve: &[(f64, f64)]) -> Option<f64> {
    let sorted = |points: &[(f64, f64)]| {
        let mut points = points.to_vec();
        points.sort_by(|a, b| a.1.total_cmp(&b.1));
        points
    };
    let (reference, curve) = (sorted(reference), sorted(curve));
    let low = reference[0].1.max(curve[0].1);
    let high = reference[reference.len() - 1].1.min(curve[curve.len() - 1].1);
    if high <= low {
        return None;
    }
    let log_rate = |points: &[(f64, f64)], quality: f64| -> f64 {
        for pair in points.windows(2) {
            let ((r0, q0), (r1, q1)) = (pair[0], pair[1]);
            if q0 <= quality && quality <= q1 && q1 > q0 {
                let t = (quality - q0) / (q1 - q0);
                return r0.ln() * (1.0 - t) + r1.ln() * t;
            }
        }
        points[points.len() - 1].0.ln()
    };
    let steps = 200;
    let total: f64 = (0..=steps)
        .map(|step| {
            let quality = low + (high - low) * f64::from(step) / f64::from(steps);
            log_rate(&curve, quality) - log_rate(&reference, quality)
        })
        .sum();
    Some((total / f64::from(steps + 1)).exp() - 1.0)
}

/// Prints the BD-rates of encoder `ours` against `other` at equal PSNR
/// over luma, SSIM (in dB), PSNR over RGB, and, when every line has it,
/// SSIMULACRA2, each the mean over the images both coded, from a file of
/// `encoder image bytes psnr_rgb psnr_y ssim [ssimulacra2]` lines.
fn bd(points: &str, ours: &str, other: &str) -> Result<(), String> {
    let text = std::fs::read_to_string(points).map_err(|error| error.to_string())?;
    let mut curves: std::collections::BTreeMap<(String, String), Vec<Point>> =
        std::collections::BTreeMap::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let (encoder, image, bytes, psnr_rgb, psnr_y, ssim, ssimulacra2) = match fields[..] {
            [encoder, image, bytes, psnr_rgb, psnr_y, ssim] => {
                (encoder, image, bytes, psnr_rgb, psnr_y, ssim, None)
            }
            [encoder, image, bytes, psnr_rgb, psnr_y, ssim, s2] => {
                (encoder, image, bytes, psnr_rgb, psnr_y, ssim, Some(s2))
            }
            _ => continue,
        };
        let number = |text: &str| text.parse::<f64>().map_err(|error| error.to_string());
        let ssim = number(ssim)?;
        let ssimulacra2 = ssimulacra2.map(number).transpose()?;
        curves
            .entry((encoder.to_string(), image.to_string()))
            .or_default()
            .push(Point {
                bytes: number(bytes)?,
                psnr_rgb: number(psnr_rgb)?,
                psnr_y: number(psnr_y)?,
                ssim_db: -10.0 * (1.0 - ssim).max(1e-12).log10(),
                ssimulacra2,
            });
    }
    let images: Vec<String> = curves
        .keys()
        .filter(|(encoder, _)| encoder == ours)
        .map(|(_, image)| image.clone())
        .collect();
    let mut out = Vec::new();
    let perceptual = curves.values().flatten().all(|point| point.ssimulacra2.is_some());
    let mut metrics: Vec<fn(&Point) -> f64> = vec![
        |p: &Point| p.psnr_y,
        |p: &Point| p.ssim_db,
        |p: &Point| p.psnr_rgb,
    ];
    if perceptual {
        metrics.push(|p: &Point| p.ssimulacra2.unwrap_or(0.0));
    }
    for metric in metrics {
        let mut values = Vec::new();
        for image in &images {
            let (Some(a), Some(b)) = (
                curves.get(&(other.to_string(), image.clone())),
                curves.get(&(ours.to_string(), image.clone())),
            ) else {
                continue;
            };
            let pairs = |points: &Vec<Point>| -> Vec<(f64, f64)> {
                points.iter().map(|p| (p.bytes, metric(p))).collect()
            };
            if let Some(value) = bd_rate(&pairs(a), &pairs(b)) {
                values.push(value);
            }
        }
        if values.is_empty() {
            return Err("no curves in common".to_string());
        }
        out.push(100.0 * values.iter().sum::<f64>() / values.len() as f64);
    }
    let text: Vec<String> = out.iter().map(|value| format!("{value:+.1}")).collect();
    println!("{}", text.join(" "));
    Ok(())
}
