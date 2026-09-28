//! The ICO and CUR reader and the ICO writer: every icon opens to its
//! largest, deepest entry's pixels (PNG and BMP entries at 1 to 32 bits,
//! AND masks, alpha-less 32-bit entries, cursors; `.pix` beside each, in
//! RGBA); the writer's icons hold the standard sizes that fit with the
//! source itself as the largest; corrupt files are refused.

use std::fs;
use std::path::PathBuf;

use sublime::image::{ColorType, Image};
use sublime::io::ico::{entries, read_ico, write_ico};

fn dir(sub: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/ico")
        .join(sub)
}

fn files(sub: &str) -> Vec<(String, Vec<u8>)> {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir(sub))
        .expect("fixture dir")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext == "ico" || ext == "cur")
        })
        .collect();
    paths.sort();
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

fn rgba(image: &Image) -> Vec<u8> {
    let channels = image.color.channels();
    image
        .pixels
        .chunks_exact(channels)
        .flat_map(|cell| match image.color {
            ColorType::Gray => [cell[0], cell[0], cell[0], 255],
            ColorType::GrayAlpha => [cell[0], cell[0], cell[0], cell[1]],
            ColorType::Rgb => [cell[0], cell[1], cell[2], 255],
            ColorType::Rgba => [cell[0], cell[1], cell[2], cell[3]],
        })
        .collect()
}

fn pix(name: &str) -> (u32, u32, Vec<u8>) {
    let bytes = fs::read(dir("").join(format!("{name}.pix"))).expect("pix");
    let newline = bytes
        .iter()
        .position(|byte| *byte == b'\n')
        .expect("header");
    let header = String::from_utf8_lossy(&bytes[..newline]).to_string();
    let fields: Vec<&str> = header.split(' ').collect();
    (
        fields[1].parse().unwrap(),
        fields[2].parse().unwrap(),
        bytes[newline + 1..].to_vec(),
    )
}

#[test]
fn every_icon_opens_to_its_largest_deepest_entry() {
    let mut checked = 0;
    for (name, bytes) in files("") {
        let image = read_ico(&bytes).unwrap_or_else(|error| panic!("{name}: {error}"));
        let (width, height, want) = pix(&name);
        assert_eq!((image.width, image.height), (width, height), "{name}");
        assert!(rgba(&image) == want, "{name}: pixels differ");
        checked += 1;
    }
    assert_eq!(checked, 46);
}

fn image(width: u32, height: u32, color: ColorType, seed: u32) -> Image {
    let count = (width * height) as usize * color.channels();
    Image {
        width,
        height,
        color,
        pixels: (0..count as u32)
            .map(|at| (at.wrapping_mul(seed) >> 3) as u8)
            .collect(),
    }
}

#[test]
fn written_icons_hold_the_sizes_that_fit_and_the_source_itself() {
    for (width, height, sizes) in [
        (48, 48, vec![(16, 16), (24, 24), (32, 32), (48, 48)]),
        (
            100,
            100,
            vec![(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (100, 100)],
        ),
        (
            300,
            200,
            vec![
                (16, 11),
                (24, 16),
                (32, 21),
                (48, 32),
                (64, 43),
                (128, 85),
                (256, 171),
            ],
        ),
        (10, 40, vec![(4, 16), (6, 24), (8, 32), (10, 40)]),
    ] {
        for color in [ColorType::Rgba, ColorType::Rgb, ColorType::Gray] {
            let source = image(width, height, color, 2_654_435_761);
            let mut written = Vec::new();
            write_ico(&source, &mut written).unwrap();
            let listed: Vec<(u32, u32)> = entries(&written).unwrap();
            assert_eq!(listed, sizes, "{width}x{height} {color:?}");
            let opened = read_ico(&written).unwrap();
            let largest = sizes[sizes.len() - 1];
            assert_eq!((opened.width, opened.height), largest);
            if width <= 256 && height <= 256 {
                assert!(
                    rgba(&opened) == rgba(&source),
                    "{width}x{height} {color:?}: the source changed"
                );
            }
        }
    }
}

#[test]
fn a_constant_color_stays_constant_at_every_size() {
    let source = Image {
        width: 300,
        height: 300,
        color: ColorType::Rgba,
        pixels: [30, 140, 220, 200].repeat(300 * 300),
    };
    let mut written = Vec::new();
    write_ico(&source, &mut written).unwrap();
    let opened = read_ico(&written).unwrap();
    assert!(
        rgba(&opened)
            .chunks_exact(4)
            .all(|cell| cell == [30, 140, 220, 200])
    );
}

#[test]
fn every_corrupt_file_is_refused() {
    let invalid = files("invalid");
    assert_eq!(invalid.len(), 6);
    for (name, bytes) in invalid {
        assert!(read_ico(&bytes).is_err(), "{name} was accepted");
    }
}
