//! The QOI reader and writer against files Pillow wrote with the qoi.h
//! algorithm: every image decodes to the pixels Pillow decodes (`.pix`
//! beside it), our writer produces the same bytes from those pixels,
//! and every corrupt file is refused.

use std::fs;
use std::path::PathBuf;

use sublime::image::{ColorType, Image};
use sublime::io::qoi::{read_qoi, write_qoi};

fn fixture_dir(sub: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/qoi")
        .join(sub)
}

fn fixtures(sub: &str) -> Vec<(String, Vec<u8>)> {
    let mut paths: Vec<PathBuf> = fs::read_dir(fixture_dir(sub))
        .expect("fixture dir")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "qoi"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no qoi fixtures in {sub:?}");
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
    let bytes = fs::read(fixture_dir("").join(format!("{name}.pix"))).expect("pix beside the qoi");
    let newline = bytes
        .iter()
        .position(|byte| *byte == b'\n')
        .expect("header line");
    let header = String::from_utf8_lossy(&bytes[..newline]).to_string();
    let fields: Vec<&str> = header.split(' ').collect();
    let color = match fields[3] {
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
fn every_image_decodes_to_the_pixels_pillow_decodes() {
    let mut checked = 0;
    for (name, qoi) in fixtures("") {
        let image = read_qoi(&qoi).unwrap_or_else(|error| panic!("{name}: {error}"));
        let want = expected(&name);
        assert_eq!(
            (image.width, image.height, image.color),
            (want.width, want.height, want.color),
            "{name}"
        );
        assert!(image.pixels == want.pixels, "{name}: pixels differ");
        checked += 1;
    }
    assert_eq!(checked, 60);
}

#[test]
fn our_writer_writes_the_bytes_pillow_wrote() {
    for (name, qoi) in fixtures("") {
        let mut ours = Vec::new();
        write_qoi(&expected(&name), &mut ours).unwrap_or_else(|error| panic!("{name}: {error}"));
        // Byte 13, the informative colorspace: Pillow writes 1 (linear),
        // ours 0 (sRGB, which the pixels are).
        assert_eq!(ours[13], 0, "{name}");
        let (ours_rest, theirs_rest) = ((&ours[..13], &ours[14..]), (&qoi[..13], &qoi[14..]));
        assert!(
            ours_rest == theirs_rest,
            "{name}: {} bytes against Pillow's {}",
            ours.len(),
            qoi.len()
        );
    }
}

#[test]
fn every_corrupt_file_is_refused() {
    let invalid = fixtures("invalid");
    assert_eq!(invalid.len(), 6);
    for (name, qoi) in invalid {
        assert!(read_qoi(&qoi).is_err(), "{name} was accepted");
    }
}
