//! The HEIC writer: images written and read back by the reader here
//! (which is bit-exact with libheif's decoding), at their size, with
//! their alpha, depth, profile, and Exif, and close to the source.

use sublime::image::ColorType;
use sublime::io::heif::read_heif_rows;
use sublime::io::heif::write::HeicRows;
use sublime::io::png::RowSink;

/// Rows kept whole, with what came beside them.
#[derive(Default)]
struct Rows {
    width: usize,
    height: usize,
    color: Option<ColorType>,
    pixels: Vec<u8>,
    profile: Option<Vec<u8>>,
    exif: Option<Vec<u8>>,
    deep: bool,
    takes_deep: bool,
}

impl RowSink for Rows {
    fn accept_deep(&mut self, _color: ColorType) -> bool {
        self.deep = self.takes_deep;
        self.deep
    }

    fn start(&mut self, width: u32, height: u32, color: ColorType) -> std::io::Result<()> {
        self.width = width as usize;
        self.height = height as usize;
        self.color = Some(color);
        Ok(())
    }

    fn row(&mut self, pixels: &[u8]) -> std::io::Result<()> {
        self.pixels.extend_from_slice(pixels);
        Ok(())
    }

    fn icc_profile(&mut self, profile: &[u8]) -> bool {
        self.profile = Some(profile.to_vec());
        true
    }

    fn exif(&mut self, exif: &[u8]) -> bool {
        self.exif = Some(exif.to_vec());
        true
    }
}

/// A test picture: gradients, edges, and a little noise, in `color`.
fn picture(width: usize, height: usize, color: ColorType) -> Vec<u8> {
    let channels = color.channels();
    let mut seed = 7u32;
    let mut pixels = Vec::with_capacity(width * height * channels);
    for y in 0..height {
        for x in 0..width {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            let noise = ((seed >> 16) % 7) as i32 - 3;
            let edge = if (x / 9 + y / 5) % 4 == 0 { 50 } else { 0 };
            let base = |phase: usize| -> u8 {
                (((x * 3 + y * 2 + phase) % 180) as i32 + edge + noise).clamp(0, 255) as u8
            };
            let rgb = [base(0), base(40), base(90)];
            match color {
                ColorType::Gray => pixels.push(rgb[0]),
                ColorType::GrayAlpha => {
                    pixels.extend_from_slice(&[rgb[0], (x * 255 / width) as u8])
                }
                ColorType::Rgb => pixels.extend_from_slice(&rgb),
                ColorType::Rgba => {
                    pixels.extend_from_slice(&rgb);
                    pixels.push(((x + y) * 255 / (width + height)) as u8);
                }
            }
        }
    }
    pixels
}

/// Writes `pixels` as HEIC at `quality`, with a profile and Exif when given.
fn write(
    width: usize,
    height: usize,
    color: ColorType,
    pixels: &[u8],
    quality: u8,
    extras: (Option<&[u8]>, Option<&[u8]>),
) -> Vec<u8> {
    let mut file = Vec::new();
    let mut rows = HeicRows::new(&mut file, quality);
    if let Some(profile) = extras.0 {
        assert!(rows.icc_profile(profile));
    }
    if let Some(exif) = extras.1 {
        assert!(rows.exif(exif));
    }
    rows.start(width as u32, height as u32, color)
        .expect("start");
    let stride = width * color.channels();
    for row in pixels.chunks_exact(stride) {
        rows.row(row).expect("row");
    }
    file
}

fn read(file: &[u8]) -> Rows {
    let mut rows = Rows::default();
    read_heif_rows(file, &mut rows).expect("reads back");
    rows
}

/// The BT.601 luma of RGB pixels (or gray ones), as f64.
fn luma(pixels: &[u8], channels: usize) -> Vec<f64> {
    pixels
        .chunks_exact(channels)
        .map(|p| {
            if channels < 3 {
                f64::from(p[0])
            } else {
                0.299 * f64::from(p[0]) + 0.587 * f64::from(p[1]) + 0.114 * f64::from(p[2])
            }
        })
        .collect()
}

/// PSNR of the luma of two pictures, in dB: what 4:2:0 coding keeps.
fn luma_psnr(a: &[u8], a_channels: usize, b: &[u8], b_channels: usize) -> f64 {
    let (a, b) = (luma(a, a_channels), luma(b, b_channels));
    assert_eq!(a.len(), b.len());
    let mean = a.iter().zip(&b).map(|(x, y)| (x - y).powi(2)).sum::<f64>() / a.len() as f64;
    if mean == 0.0 {
        f64::INFINITY
    } else {
        10.0 * (255.0f64 * 255.0 / mean).log10()
    }
}

/// PSNR of two 8-bit sample runs, in dB.
fn psnr(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len());
    let squares: f64 = a
        .iter()
        .zip(b)
        .map(|(&x, &y)| (f64::from(x) - f64::from(y)).powi(2))
        .sum();
    let mean = squares / a.len() as f64;
    if mean == 0.0 {
        f64::INFINITY
    } else {
        10.0 * (255.0f64 * 255.0 / mean).log10()
    }
}

#[test]
fn pictures_come_back_at_their_size_and_close() {
    for (width, height) in [(64, 64), (37, 29), (200, 136), (1, 1), (2, 3), (130, 7)] {
        let pixels = picture(width, height, ColorType::Rgb);
        let file = write(width, height, ColorType::Rgb, &pixels, 90, (None, None));
        let back = read(&file);
        assert_eq!(
            (back.width, back.height),
            (width, height),
            "{width}x{height}"
        );
        assert_eq!(back.color, Some(ColorType::Rgb));
        let quality = luma_psnr(&pixels, 3, &back.pixels, 3);
        assert!(quality > 38.0, "{width}x{height}: {quality:.1} dB");
    }
}

#[test]
fn a_lower_quality_is_a_smaller_file() {
    let pixels = picture(160, 120, ColorType::Rgb);
    let sizes: Vec<usize> = [20u8, 50, 80, 95]
        .iter()
        .map(|&quality| write(160, 120, ColorType::Rgb, &pixels, quality, (None, None)).len())
        .collect();
    assert!(sizes.windows(2).all(|pair| pair[0] < pair[1]), "{sizes:?}");
}

#[test]
fn alpha_comes_back() {
    for color in [ColorType::Rgba, ColorType::GrayAlpha] {
        let pixels = picture(48, 40, color);
        let file = write(48, 40, color, &pixels, 90, (None, None));
        let back = read(&file);
        assert_eq!((back.width, back.height), (48, 40));
        assert_eq!(
            back.color.map(ColorType::has_alpha),
            Some(true),
            "{color:?}"
        );
        let channels = back.color.expect("started").channels();
        let alpha: Vec<u8> = pixels
            .chunks_exact(color.channels())
            .map(|p| p[p.len() - 1])
            .collect();
        let back_alpha: Vec<u8> = back
            .pixels
            .chunks_exact(channels)
            .map(|p| p[channels - 1])
            .collect();
        let quality = psnr(&alpha, &back_alpha);
        assert!(quality > 35.0, "{color:?} alpha: {quality:.1} dB");
    }
}

#[test]
fn gray_comes_back_gray_in_rgb() {
    let pixels = picture(40, 30, ColorType::Gray);
    let file = write(40, 30, ColorType::Gray, &pixels, 90, (None, None));
    let back = read(&file);
    let gray: Vec<u8> = back.pixels.chunks_exact(3).map(|p| p[1]).collect();
    assert!(
        back.pixels
            .chunks_exact(3)
            .all(|p| p[0].abs_diff(p[1]) <= 2 && p[2].abs_diff(p[1]) <= 2)
    );
    assert!(psnr(&pixels, &gray) > 35.0);
}

#[test]
fn sixteen_bit_rows_are_coded_at_ten_bits() {
    let (width, height) = (64usize, 48usize);
    let mut file = Vec::new();
    let mut rows = HeicRows::new(&mut file, 95);
    assert!(rows.accept_deep(ColorType::Rgb));
    rows.start(width as u32, height as u32, ColorType::Rgb)
        .expect("start");
    let mut source = Vec::new();
    for y in 0..height {
        let mut row = Vec::with_capacity(width * 6);
        for x in 0..width {
            for channel in 0..3 {
                // Smooth ramps at 16 bits, a different slope a channel:
                // the low bits matter.
                let value = (x * 700 + y * 300 + channel * 9000).min(65535) as u16;
                row.extend_from_slice(&value.to_be_bytes());
                source.push(value);
            }
        }
        rows.row(&row).expect("row");
    }
    drop(rows);
    let mut back = Rows {
        takes_deep: true,
        ..Rows::default()
    };
    read_heif_rows(&file, &mut back).expect("reads back");
    assert!(back.deep, "a 10-bit picture offers 16-bit rows");
    let decoded: Vec<u16> = back
        .pixels
        .chunks_exact(2)
        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
        .collect();
    assert_eq!(decoded.len(), source.len());
    let squares: f64 = source
        .iter()
        .zip(&decoded)
        .map(|(&a, &b)| (f64::from(a) - f64::from(b)).powi(2))
        .sum();
    let quality = 10.0 * (65535.0f64 * 65535.0 / (squares / source.len() as f64)).log10();
    assert!(quality > 45.0, "{quality:.1} dB at 16 bits");
}

#[test]
fn the_profile_and_exif_are_carried() {
    let pixels = picture(32, 32, ColorType::Rgb);
    let profile = b"a profile's bytes, kept as they are".to_vec();
    // A minimal big-endian TIFF with no entries.
    let exif = vec![b'M', b'M', 0, 42, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0];
    let file = write(
        32,
        32,
        ColorType::Rgb,
        &pixels,
        80,
        (Some(&profile), Some(&exif)),
    );
    let back = read(&file);
    assert_eq!(back.profile.as_deref(), Some(&profile[..]));
    assert_eq!(back.exif.as_deref(), Some(&exif[..]));
}
