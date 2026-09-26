//! The PNG reader against PngSuite (Willem van Schaik's reference images,
//! `tests/fixtures/png/PngSuite.LICENSE`): every valid image decodes to
//! the pixels Pillow decodes (`.pix` beside it), every corrupt image is
//! refused, and every decoded image survives our writer and reader
//! unchanged.

use std::fs;
use std::path::PathBuf;

use sublime::image::{ColorType, Image};
use sublime::io::png::{read_png, write_png};

fn fixture_dir(sub: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/png")
        .join(sub)
}

fn fixtures(sub: &str) -> Vec<(String, Vec<u8>)> {
    let mut paths: Vec<PathBuf> = fs::read_dir(fixture_dir(sub))
        .expect("fixture dir")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "png"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no png fixtures in {sub:?}");
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

/// `PIX <width> <height> <color>\n` then the pixels.
fn expected(name: &str) -> Image {
    let bytes = fs::read(fixture_dir("").join(format!("{name}.pix"))).expect("pix beside the png");
    let newline = bytes
        .iter()
        .position(|byte| *byte == b'\n')
        .expect("header line");
    let header = String::from_utf8_lossy(&bytes[..newline]).to_string();
    let fields: Vec<&str> = header.split(' ').collect();
    let color = match fields[3] {
        "g" => ColorType::Gray,
        "ga" => ColorType::GrayAlpha,
        "rgb" => ColorType::Rgb,
        _ => ColorType::Rgba,
    };
    Image {
        width: fields[1].parse().unwrap(),
        height: fields[2].parse().unwrap(),
        color,
        pixels: bytes[newline + 1..].to_vec(),
    }
}

#[test]
fn every_suite_image_decodes_to_the_pixels_pillow_decodes() {
    for (name, png) in fixtures("") {
        let (image, _) = read_png(&png).unwrap_or_else(|error| panic!("{name}: {error}"));
        let want = expected(&name);
        assert_eq!(
            (image.width, image.height, image.color),
            (want.width, want.height, want.color),
            "{name}"
        );
        if image.pixels != want.pixels {
            let first = image
                .pixels
                .iter()
                .zip(&want.pixels)
                .position(|(a, b)| a != b)
                .unwrap_or(0);
            panic!(
                "{name}: pixels differ first at byte {first} (ours {:?}, pillow {:?})",
                &image.pixels[first..(first + 8).min(image.pixels.len())],
                &want.pixels[first..(first + 8).min(want.pixels.len())]
            );
        }
    }
}

#[test]
fn every_corrupt_suite_image_is_refused() {
    for (name, png) in fixtures("invalid") {
        assert!(read_png(&png).is_err(), "{name} was accepted");
    }
}

#[test]
fn every_suite_image_survives_our_writer_and_reader() {
    for (name, png) in fixtures("") {
        let (image, _) = read_png(&png).unwrap();
        let mut encoded = Vec::new();
        write_png(&image, &mut encoded).unwrap();
        let (again, _) = read_png(&encoded)
            .unwrap_or_else(|error| panic!("{name}: our PNG does not read back: {error}"));
        assert_eq!(again, image, "{name}");
    }
}
