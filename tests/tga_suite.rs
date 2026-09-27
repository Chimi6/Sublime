//! The TGA reader and writer: every layout (gray, gray and alpha,
//! palette, RGB, RGBA; raw and RLE; both orientations and right to left;
//! 15, 16, 24, and 32 bits; RLE packets across rows) reads to the pixels
//! it means (`.pix` beside it); the writer produces Pillow's RLE bytes
//! (top-down, with the TGA 2.0 footer); corrupt files are refused.

use std::fs;
use std::path::PathBuf;

use sublime::image::{ColorType, Image};
use sublime::io::tga::{read_tga, write_tga};

fn dir(sub: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/tga")
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
    for (name, bytes) in files("", "tga") {
        let image = read_tga(&bytes).unwrap_or_else(|error| panic!("{name}: {error}"));
        let want = pix(dir("").join(format!("{name}.pix")));
        assert_eq!(
            (image.width, image.height, image.color),
            (want.width, want.height, want.color),
            "{name}"
        );
        assert!(image.pixels == want.pixels, "{name}: pixels differ");
        checked += 1;
    }
    assert_eq!(checked, 247);
}

#[test]
fn the_writer_writes_pillows_rle_bytes() {
    for (name, theirs) in files("written", "tga") {
        let source = pix(dir("written").join(format!("{name}.pix")));
        let mut ours = Vec::new();
        write_tga(&source, &mut ours).unwrap();
        assert!(
            ours == theirs,
            "{name}: {} bytes against Pillow's {}",
            ours.len(),
            theirs.len()
        );
    }
}

#[test]
fn every_corrupt_file_is_refused() {
    let invalid = files("invalid", "tga");
    assert_eq!(invalid.len(), 6);
    for (name, bytes) in invalid {
        assert!(read_tga(&bytes).is_err(), "{name} was accepted");
    }
}
