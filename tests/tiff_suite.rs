//! The TIFF reader and writer: every layout ImageMagick writes that we
//! read (strips and tiles; none, LZW, deflate, PackBits; the horizontal
//! predictor at 8 and 16 bits; 1 to 16-bit gray, min-is-white, palette,
//! RGB, RGBA, CMYK; planar; big-endian; the first of several pages; and
//! associated alpha) reads to the pixels it means (`.pix` beside it);
//! every image survives our writer and reader; corrupt and unsupported
//! files are refused.

use std::fs;
use std::path::PathBuf;

use sublime::image::{ColorType, Image};
use sublime::io::tiff::{read_tiff, write_tiff};

fn dir(sub: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/tiff")
        .join(sub)
}

fn files(sub: &str, extension: &str) -> Vec<(String, Vec<u8>)> {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir(sub))
        .expect("fixture dir")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == extension))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no .{extension} fixtures in {sub:?}");
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

fn pix(path: PathBuf) -> Image {
    let bytes = fs::read(&path).unwrap_or_else(|_| panic!("{path:?}"));
    let newline = bytes
        .iter()
        .position(|byte| *byte == b'\n')
        .expect("header line");
    let header = String::from_utf8_lossy(&bytes[..newline]).to_string();
    let fields: Vec<&str> = header.split(' ').collect();
    let color = match fields[3] {
        "gray" => ColorType::Gray,
        "grayalpha" => ColorType::GrayAlpha,
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
fn every_layout_reads_to_the_pixels_it_means() {
    let mut checked = 0;
    for (name, bytes) in files("", "tif") {
        let image = read_tiff(&bytes).unwrap_or_else(|error| panic!("{name}: {error}"));
        let want = pix(dir("").join(format!("{name}.pix")));
        assert_eq!(
            (image.width, image.height, image.color),
            (want.width, want.height, want.color),
            "{name}"
        );
        assert!(image.pixels == want.pixels, "{name}: pixels differ");
        checked += 1;
    }
    assert_eq!(checked, 139);
}

#[test]
fn every_image_survives_our_writer_and_reader() {
    for (name, bytes) in files("", "tif") {
        let image = read_tiff(&bytes).unwrap();
        let mut written = Vec::new();
        write_tiff(&image, &mut written).unwrap();
        let back = read_tiff(&written).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(
            (back.width, back.height, back.color),
            (image.width, image.height, image.color),
            "{name}"
        );
        assert!(
            back.pixels == image.pixels,
            "{name}: our writer changed the pixels"
        );
    }
}

#[test]
fn every_corrupt_file_is_refused() {
    let invalid = files("invalid", "tif");
    assert_eq!(invalid.len(), 6);
    for (name, bytes) in invalid {
        assert!(read_tiff(&bytes).is_err(), "{name} was accepted");
    }
}
