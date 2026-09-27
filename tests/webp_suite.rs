//! The WebP reader against files Pillow's libwebp wrote: every image
//! decodes to the pixels libwebp decodes (`.pix` beside it), and every
//! corrupt file is refused.

use std::fs;
use std::path::PathBuf;

use sublime::image::{ColorType, Image};
use sublime::io::webp::read_webp;

fn fixture_dir(sub: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/webp")
        .join(sub)
}

fn fixtures(sub: &str) -> Vec<(String, Vec<u8>)> {
    let mut paths: Vec<PathBuf> = fs::read_dir(fixture_dir(sub))
        .expect("fixture dir")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "webp"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no webp fixtures in {sub:?}");
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
    let bytes = fs::read(fixture_dir("").join(format!("{name}.pix"))).expect("pix beside the webp");
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

fn check(filter: impl Fn(&str) -> bool) -> usize {
    let mut checked = 0;
    for (name, webp) in fixtures("") {
        if !filter(&name) {
            continue;
        }
        let (image, _) = read_webp(&webp).unwrap_or_else(|error| panic!("{name}: {error}"));
        let want = expected(&name);
        assert_eq!(
            (image.width, image.height, image.color),
            (want.width, want.height, want.color),
            "{name}"
        );
        if image.pixels != want.pixels {
            let mut count = 0usize;
            let mut worst = 0u8;
            let mut first = None;
            for (index, (a, b)) in image.pixels.iter().zip(&want.pixels).enumerate() {
                let difference = a.abs_diff(*b);
                if difference > 0 {
                    count += 1;
                    first.get_or_insert(index);
                }
                worst = worst.max(difference);
            }
            panic!(
                "{name}: {count} of {} bytes differ, worst by {worst}, first at byte {first:?}",
                want.pixels.len()
            );
        }
        checked += 1;
    }
    checked
}

#[test]
fn every_lossless_image_decodes_to_the_pixels_libwebp_decodes() {
    let checked = check(|name| name.contains("lossless"));
    assert!(checked > 100, "only {checked} lossless images checked");
}

#[test]
fn every_corrupt_file_is_refused() {
    for (name, webp) in fixtures("invalid") {
        assert!(read_webp(&webp).is_err(), "{name} was accepted");
    }
}

#[test]
fn every_lossy_image_decodes_to_the_pixels_libwebp_decodes() {
    let checked = check(|name| name.contains("lossy"));
    assert!(checked > 90, "only {checked} lossy images checked");
}

#[test]
#[ignore]
fn list_lossy_mismatches() {
    for (name, webp) in fixtures("") {
        if !name.contains("lossy") {
            continue;
        }
        let want = expected(&name);
        match read_webp(&webp) {
            Err(error) => eprintln!("ERR  {name}: {error}"),
            Ok((image, _)) => {
                let count = image
                    .pixels
                    .iter()
                    .zip(&want.pixels)
                    .filter(|(a, b)| a != b)
                    .count();
                let first = image
                    .pixels
                    .iter()
                    .zip(&want.pixels)
                    .position(|(a, b)| a != b);
                let channels = want.color.channels();
                let where_ = first.map(|i| {
                    (
                        i / channels % want.width as usize,
                        i / channels / want.width as usize,
                    )
                });
                eprintln!(
                    "{} {name}: {count} differ, first (x, y) {where_:?}",
                    if count == 0 { "ok  " } else { "BAD " }
                );
            }
        }
    }
}

#[test]
fn every_image_survives_our_lossless_writer_and_reader() {
    use sublime::io::webp::write_webp;
    let mut checked = 0;
    for (name, webp) in fixtures("") {
        let (image, _) = read_webp(&webp).unwrap_or_else(|error| panic!("{name}: {error}"));
        let mut encoded = Vec::new();
        write_webp(&image, &mut encoded).unwrap_or_else(|error| panic!("{name}: {error}"));
        let (back, _) =
            read_webp(&encoded).unwrap_or_else(|error| panic!("{name} (ours): {error}"));
        assert_eq!(
            (back.width, back.height),
            (image.width, image.height),
            "{name}"
        );
        let want: Vec<u8> = if back.color == image.color {
            image.pixels.clone()
        } else {
            // Our writer marks alpha only when a pixel is not opaque.
            image
                .pixels
                .chunks_exact(4)
                .flat_map(|p| [p[0], p[1], p[2]])
                .collect()
        };
        if back.pixels != want {
            let count = back
                .pixels
                .iter()
                .zip(&want)
                .filter(|(a, b)| a != b)
                .count();
            let first = back
                .pixels
                .iter()
                .zip(&want)
                .position(|(a, b)| a != b)
                .unwrap_or(0);
            let channels = back.color.channels();
            panic!(
                "{name}: {count} of {} bytes changed through our writer, first at pixel ({}, {}): ours {:?} want {:?}",
                want.len(),
                first / channels % back.width as usize,
                first / channels / back.width as usize,
                &back.pixels[first / channels * channels..first / channels * channels + channels],
                &want[first / channels * channels..first / channels * channels + channels]
            );
        }
        checked += 1;
    }
    assert!(checked > 200);
}
