//! The Netpbm reader and writers: every form (P1 to P7, maxvals from 1 to
//! 65535, ASCII with comments, every PAM depth) reads to the 8-bit pixels
//! it means (`.pix` beside it); the PPM, PGM, and PBM writers produce
//! Pillow's bytes; PAM round-trips every image; corrupt files are refused.

use std::fs;
use std::path::PathBuf;

use sublime::image::{ColorType, Image};
use sublime::io::netpbm::{Kind, read_netpbm, write_netpbm};

fn dir(sub: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/netpbm")
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
fn every_form_reads_to_the_pixels_it_means() {
    let mut checked = 0;
    for (name, bytes) in files("", "pnm") {
        let image = read_netpbm(&bytes).unwrap_or_else(|error| panic!("{name}: {error}"));
        let want = pix(dir("").join(format!("{name}.pix")));
        assert_eq!(
            (image.width, image.height, image.color),
            (want.width, want.height, want.color),
            "{name}"
        );
        assert!(image.pixels == want.pixels, "{name}: pixels differ");
        checked += 1;
    }
    assert_eq!(checked, 140);
}

#[test]
fn ppm_pgm_and_pbm_writers_write_pillows_bytes() {
    for (name, _) in files("written", "pix") {
        let source = pix(dir("written").join(format!("{name}.pix")));
        for (kind, extension) in [(Kind::Ppm, "ppm"), (Kind::Pgm, "pgm"), (Kind::Pbm, "pbm")] {
            let mut ours = Vec::new();
            write_netpbm(&source, &mut ours, kind).unwrap();
            let theirs = fs::read(dir("written").join(format!("{name}.{extension}"))).unwrap();
            assert!(
                ours == theirs,
                "{name}.{extension}: ours differs from Pillow's"
            );
        }
    }
}

#[test]
fn pam_round_trips_every_image() {
    for (name, bytes) in files("", "pnm") {
        let image = read_netpbm(&bytes).unwrap();
        let mut written = Vec::new();
        write_netpbm(&image, &mut written, Kind::Pam).unwrap();
        let back = read_netpbm(&written).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(
            (back.width, back.height, back.color),
            (image.width, image.height, image.color),
            "{name}"
        );
        assert!(
            back.pixels == image.pixels,
            "{name}: PAM changed the pixels"
        );
    }
}

#[test]
fn every_corrupt_file_is_refused() {
    let invalid = files("invalid", "pnm");
    assert_eq!(invalid.len(), 8);
    for (name, bytes) in invalid {
        assert!(read_netpbm(&bytes).is_err(), "{name} was accepted");
    }
}
