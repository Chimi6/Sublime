//! The HEIC reader against files Apple's encoder (sips) and x265 (through
//! libheif) wrote: every picture decodes to the very samples libheif
//! decodes (their checksums below), and the rows come out turned,
//! mirrored, and cropped as the file says, with its profile and Exif.

use std::fs;
use std::path::PathBuf;

use sublime::image::ColorType;
use sublime::io::heif::{decode_planes, read_heif_rows};
use sublime::io::png::{PngRows, RowSink};

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/heic")
        .join(format!("{name}.heic"));
    fs::read(path).expect("read fixture")
}

/// FNV-1a over the bytes, as the checksums were made.
fn fnv(bytes: impl Iterator<Item = u8>) -> u64 {
    bytes.fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Each fixture: the planes' width, height, chroma format, bit depth,
/// whether it has alpha, and the checksum of libheif's samples (luma,
/// Cb, Cr, alpha; bytes at 8 bits, little-endian words above).
const PLANES: [(&str, &str, u64); 7] = [
    ("apple-tiny", "64 64 1 8 0", 0x68bd_f994_004b_ff48),
    ("apple-odd", "518 334 1 8 0", 0x2aa1_7d4a_ca79_187b),
    ("apple-photo", "1600 1200 1 8 0", 0x4bd5_453f_c91f_f7d2),
    ("x265-tiny", "64 64 1 8 0", 0x70b3_08a6_6313_3192),
    ("x265-alpha", "64 64 1 8 1", 0x249c_7d01_57eb_24e5),
    ("x265-odd-10bit", "518 334 1 10 0", 0x1b1a_19a5_3c0f_f7b1),
    ("x265-444", "300 300 3 8 0", 0xc0b2_26e0_7b2d_acc1),
];

#[test]
fn every_picture_decodes_to_libheifs_samples() {
    for (name, header, checksum) in PLANES {
        let (planes, alpha) = decode_planes(&fixture(name)).expect(name);
        let found = format!(
            "{} {} {} {} {}",
            planes.width,
            planes.height,
            planes.chroma_format,
            planes.bit_depth,
            u8::from(alpha.is_some())
        );
        assert_eq!(found, header, "{name}");
        let mut samples = planes
            .planes
            .iter()
            .map(|plane| (plane, planes.bit_depth))
            .collect::<Vec<_>>();
        if let Some(alpha) = &alpha {
            samples.push((&alpha.planes[0], alpha.bit_depth));
        }
        let bytes = samples.into_iter().flat_map(|(plane, depth)| {
            (0..plane.len()).flat_map(move |index| {
                let sample = plane.get(index);
                if depth > 8 {
                    sample.to_le_bytes().into_iter().take(2)
                } else {
                    [sample as u8, 0].into_iter().take(1)
                }
            })
        });
        assert_eq!(fnv(bytes), checksum, "{name}");
    }
}

/// Rows kept whole, with what the reader handed over beside them.
#[derive(Default)]
struct Rows {
    width: usize,
    height: usize,
    color: Option<ColorType>,
    pixels: Vec<u8>,
    profile: Option<Vec<u8>>,
    exif: Option<Vec<u8>>,
}

impl RowSink for Rows {
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

fn rows(name: &str) -> Rows {
    let mut rows = Rows::default();
    read_heif_rows(&fixture(name), &mut rows).expect(name);
    rows
}

impl Rows {
    fn pixel(&self, x: usize, y: usize) -> &[u8] {
        let channels = self.color.expect("started").channels();
        let at = (y * self.width + x) * channels;
        &self.pixels[at..at + channels]
    }
}

#[test]
fn the_clean_aperture_crops_and_alpha_comes_through() {
    let odd = rows("apple-odd");
    assert_eq!((odd.width, odd.height), (517, 333));
    assert_eq!(odd.color, Some(ColorType::Rgb));
    let alpha = rows("x265-alpha");
    assert_eq!(alpha.color, Some(ColorType::Rgba));
    assert_eq!((alpha.width, alpha.height), (32, 32));
}

#[test]
fn rotation_and_mirroring_move_the_pixels() {
    let still = rows("turn0");
    let (width, height) = (still.width, still.height);
    // A quarter turn anticlockwise: the right column becomes the top row.
    let turned = rows("turn1");
    assert_eq!((turned.width, turned.height), (height, width));
    for y in 0..turned.height {
        for x in 0..turned.width {
            assert_eq!(
                turned.pixel(x, y),
                still.pixel(width - 1 - y, x),
                "turned ({x}, {y})"
            );
        }
    }
    // Axis 0 flips top to bottom, axis 1 left to right (as libheif reads them).
    let flipped = rows("mirror0");
    let mirrored = rows("mirror1");
    for y in 0..height {
        for x in 0..width {
            assert_eq!(
                flipped.pixel(x, y),
                still.pixel(x, height - 1 - y),
                "flipped ({x}, {y})"
            );
            assert_eq!(
                mirrored.pixel(x, y),
                still.pixel(width - 1 - x, y),
                "mirrored ({x}, {y})"
            );
        }
    }
}

#[test]
fn png_carries_the_profile_and_exif() {
    let mut png = Vec::new();
    let mut sink = PngRows::new(&mut png);
    read_heif_rows(&fixture("apple-tiny"), &mut sink).expect("apple-tiny");
    let chunk_at = |kind: &[u8]| png.windows(4).position(|window| window == kind);
    let (header, profile, exif, data) = (
        chunk_at(b"IHDR").expect("IHDR"),
        chunk_at(b"iCCP").expect("iCCP"),
        chunk_at(b"eXIf").expect("eXIf"),
        chunk_at(b"IDAT").expect("IDAT"),
    );
    assert!(header < profile && profile < data && exif < data);
    let source = rows("apple-tiny");
    let profile = source.profile.expect("a profile");
    assert!(
        profile.windows(4).any(|window| window == b"acsp"),
        "an ICC profile"
    );
    let exif = source.exif.expect("Exif");
    assert!(
        exif.starts_with(b"MM\0*") || exif.starts_with(b"II*\0"),
        "TIFF Exif"
    );
}

#[test]
fn damaged_files_are_refused_not_panicked_on() {
    let whole = fixture("x265-odd-10bit");
    for cut in [0, 8, 100, whole.len() / 2, whole.len() - 1] {
        let mut sink = Rows::default();
        assert!(
            read_heif_rows(&whole[..cut], &mut sink).is_err(),
            "cut at {cut}"
        );
    }
}
