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

/// A sink taking 16-bit rows, keeping them.
#[derive(Default)]
struct Deep {
    color: Option<ColorType>,
    deep: bool,
    pixels: Vec<u8>,
}

impl sublime::io::png::RowSink for Deep {
    fn accept_deep(&mut self, _color: ColorType) -> bool {
        self.deep = true;
        true
    }

    fn start(&mut self, _width: u32, _height: u32, color: ColorType) -> std::io::Result<()> {
        self.color = Some(color);
        Ok(())
    }

    fn row(&mut self, pixels: &[u8]) -> std::io::Result<()> {
        self.pixels.extend_from_slice(pixels);
        Ok(())
    }
}

#[test]
fn sixteen_bit_files_go_on_at_sixteen_bits() {
    // Gray, gray+alpha, RGB, RGBA at 16 bits, not interlaced.
    for name in ["basn0g16", "basn4a16", "basn2c16", "basn6a16"] {
        let bytes = fs::read(fixture_dir("").join(format!("{name}.png"))).expect("fixture");
        let mut sink = Deep::default();
        let notes = sublime::io::png::read_png_rows(&mut &bytes[..], &mut sink).expect(name);
        assert!(sink.deep && !notes.sixteen_bit, "{name}");
        // The high bytes are the 8-bit decode; the low bytes are kept.
        let eight = expected(name);
        let high: Vec<u8> = sink.pixels.iter().step_by(2).copied().collect();
        assert_eq!(high, eight.pixels, "{name}");
        assert!(
            sink.pixels.iter().skip(1).step_by(2).any(|&low| low != 0),
            "{name}"
        );
    }
}
