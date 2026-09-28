//! The JPEG reader against files Pillow (libjpeg-turbo) wrote in every
//! baseline shape we read: every image decodes to the pixels Pillow
//! decodes (`.pix` beside it), and every corrupt image is refused.

use std::fs;
use std::path::PathBuf;

use sublime::image::{ColorType, Image};
use sublime::io::jpeg::{read_jpeg, write_jpeg};

fn fixture_dir(sub: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/jpeg")
        .join(sub)
}

fn fixtures(sub: &str) -> Vec<(String, Vec<u8>)> {
    let mut paths: Vec<PathBuf> = fs::read_dir(fixture_dir(sub))
        .expect("fixture dir")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "jpg"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no jpeg fixtures in {sub:?}");
    paths
        .into_iter()
        .map(|path| {
            let name = path
                .file_stem()
                .expect("stem")
                .to_string_lossy()
                .to_string();
            (name, fs::read(&path).expect("read fixture"))
        })
        .collect()
}

fn expected(name: &str) -> Image {
    let bytes = fs::read(fixture_dir("").join(format!("{name}.pix"))).expect("pix beside the jpg");
    let newline = bytes
        .iter()
        .position(|byte| *byte == b'\n')
        .expect("header line");
    let header = String::from_utf8_lossy(&bytes[..newline]).to_string();
    let fields: Vec<&str> = header.split(' ').collect();
    let color = match fields[3] {
        "g" => ColorType::Gray,
        _ => ColorType::Rgb,
    };
    Image {
        width: fields[1].parse().unwrap(),
        height: fields[2].parse().unwrap(),
        color,
        pixels: bytes[newline + 1..].to_vec(),
    }
}

#[test]
fn every_image_decodes_to_the_pixels_pillow_decodes() {
    let mut checked = 0;
    for (name, jpg) in fixtures("") {
        let (image, _) = read_jpeg(&jpg).unwrap_or_else(|error| panic!("{name}: {error}"));
        let want = expected(&name);
        assert_eq!(
            (image.width, image.height, image.color),
            (want.width, want.height, want.color),
            "{name}"
        );
        if image.pixels != want.pixels {
            let (mut worst, mut count) = (0u8, 0usize);
            let mut first = None;
            for (index, (a, b)) in image.pixels.iter().zip(&want.pixels).enumerate() {
                let diff = a.abs_diff(*b);
                if diff > 0 {
                    count += 1;
                    if first.is_none() {
                        first = Some(index);
                    }
                }
                worst = worst.max(diff);
            }
            panic!(
                "{name}: {count} of {} bytes differ, worst by {worst}, first at byte {:?}",
                want.pixels.len(),
                first
            );
        }
        checked += 1;
    }
    assert!(checked > 100, "only {checked} images checked");
}

#[test]
fn every_corrupt_image_is_refused() {
    for (name, jpg) in fixtures("invalid") {
        assert!(read_jpeg(&jpg).is_err(), "{name} was accepted");
    }
}

/// Peak signal-to-noise ratio of `candidate` against `original`, in dB.
fn psnr(original: &[u8], candidate: &[u8]) -> f64 {
    let squares: f64 = original
        .iter()
        .zip(candidate)
        .map(|(a, b)| {
            let diff = f64::from(*a) - f64::from(*b);
            diff * diff
        })
        .sum();
    let mean = squares / original.len() as f64;
    if mean == 0.0 {
        return f64::INFINITY;
    }
    10.0 * (255.0 * 255.0 / mean).log10()
}

#[test]
fn our_writer_round_trips_through_our_reader_at_every_quality() {
    // A smooth source (the decoded gradient) gets closer to itself as
    // quality rises; a noisy one is bounded by chroma subsampling below
    // 90 and must still clear a floor.
    let smooth = expected("gradient-200x130-q75-420");
    // Two runs of qualities, one per subsampling (4:2:0 below 90, 4:4:4
    // from 90): the ratio rises within each.
    for qualities in [[10u8, 50, 85], [90, 95, 100]] {
        let mut last = 0.0;
        for quality in qualities {
            let mut encoded = Vec::new();
            write_jpeg(&smooth, &mut encoded, quality).unwrap();
            let (back, _) =
                read_jpeg(&encoded).unwrap_or_else(|error| panic!("q{quality}: {error}"));
            assert_eq!(
                (back.width, back.height, back.color),
                (smooth.width, smooth.height, smooth.color)
            );
            let ratio = psnr(&smooth.pixels, &back.pixels);
            assert!(
                ratio > last,
                "q{quality}: {ratio:.1} dB is not above the quality below's {last:.1}"
            );
            last = ratio;
        }
        assert!(
            last > 45.0,
            "q{} only reached {last:.1} dB on a smooth image",
            qualities[2]
        );
    }
    let noisy = expected("photo-200x130-q100-444");
    for quality in [50u8, 85, 100] {
        let mut encoded = Vec::new();
        write_jpeg(&noisy, &mut encoded, quality).unwrap();
        let (back, _) = read_jpeg(&encoded).unwrap();
        let ratio = psnr(&noisy.pixels, &back.pixels);
        assert!(ratio > 28.0, "q{quality}: {ratio:.1} dB on a noisy image");
    }
}

#[test]
fn gray_and_alpha_images_write_as_gray_and_flattened_rgb() {
    let gray = expected("photo-64x64-gray-q75");
    let mut encoded = Vec::new();
    write_jpeg(&gray, &mut encoded, 85).unwrap();
    let (back, _) = read_jpeg(&encoded).unwrap();
    assert_eq!(back.color, ColorType::Gray);
    assert!(psnr(&gray.pixels, &back.pixels) > 35.0);

    // Half-transparent red over white reads back as pink.
    let rgba = Image {
        width: 16,
        height: 16,
        color: ColorType::Rgba,
        pixels: [255u8, 0, 0, 128].repeat(256),
    };
    let mut encoded = Vec::new();
    write_jpeg(&rgba, &mut encoded, 95).unwrap();
    let (back, _) = read_jpeg(&encoded).unwrap();
    assert_eq!(back.color, ColorType::Rgb);
    let pixel = &back.pixels[..3];
    assert!(
        pixel[0] > 240 && (120..=136).contains(&pixel[1]) && (120..=136).contains(&pixel[2]),
        "{pixel:?}"
    );
}
