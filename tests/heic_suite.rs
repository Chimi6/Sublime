//! The HEIC reader against files Apple's encoder (sips) and x265 (through
//! libheif) wrote: every picture decodes to the very samples libheif
//! decodes (their checksums below), and the rows come out turned,
//! mirrored, and cropped as the file says, with its profile and Exif.

use std::fs;
use std::path::PathBuf;

use sublime::image::ColorType;
use sublime::io::heif::rgb::Converter;
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
    /// Offers this sink takes.
    takes_deep: bool,
    takes_ycbcr: bool,
    deep: bool,
    /// YCbCr rows as given: luma, and the chroma rows.
    luma: Vec<u8>,
    chroma: Vec<u8>,
}

impl RowSink for Rows {
    fn accept_deep(&mut self, _color: ColorType) -> bool {
        self.deep = self.takes_deep;
        self.deep
    }

    fn accept_ycbcr(&mut self) -> bool {
        self.takes_ycbcr
    }

    fn ycbcr_row(&mut self, luma: &[u8], chroma: Option<(&[u8], &[u8])>) -> std::io::Result<()> {
        self.luma.extend_from_slice(luma);
        if let Some((blue, red)) = chroma {
            self.chroma.extend_from_slice(blue);
            self.chroma.extend_from_slice(red);
        }
        Ok(())
    }

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
fn ten_bit_pictures_go_on_at_sixteen_bits_to_a_sink_that_takes_them() {
    let eight = rows("x265-odd-10bit");
    let mut deep = Rows {
        takes_deep: true,
        ..Rows::default()
    };
    read_heif_rows(&fixture("x265-odd-10bit"), &mut deep).expect("deep");
    assert!(deep.deep);
    assert_eq!(deep.pixels.len(), eight.pixels.len() * 2);
    // Each 16-bit sample, scaled down, is the 8-bit sample within one.
    let mut fine = 0;
    for (pair, &byte) in deep.pixels.chunks_exact(2).zip(&eight.pixels) {
        let value = u32::from(u16::from_be_bytes([pair[0], pair[1]]));
        let scaled = (value * 255 + 32_767) / 65_535;
        assert!(scaled.abs_diff(u32::from(byte)) <= 1);
        // Two 10-bit levels apart fall in one 8-bit level.
        fine += usize::from(value % 257 != 0);
    }
    assert!(fine > deep.pixels.len() / 4, "the low bits carry detail");
}

#[test]
fn a_jpeg_sink_takes_the_pictures_own_ycbcr() {
    // An 8-bit 4:2:0 BT.601 full-range picture (Apple's), uncropped.
    let (planes, _) = decode_planes(&fixture("apple-tiny")).expect("planes");
    let mut rows = Rows {
        takes_ycbcr: true,
        ..Rows::default()
    };
    read_heif_rows(&fixture("apple-tiny"), &mut rows).expect("ycbcr");
    assert!(rows.pixels.is_empty(), "no RGB rows");
    let luma: Vec<u8> = (0..planes.planes[0].len())
        .map(|index| planes.planes[0].get(index) as u8)
        .collect();
    assert_eq!(rows.luma, luma);
    assert_eq!(rows.chroma.len(), 2 * planes.planes[1].len());
    // The first chroma row: Cb then Cr.
    let width = planes.sizes[1].0;
    for x in 0..width {
        assert_eq!(rows.chroma[x], planes.planes[1].get(x) as u8);
        assert_eq!(rows.chroma[width + x], planes.planes[2].get(x) as u8);
    }
}

#[test]
fn a_grid_streamed_by_bands_matches_its_whole_canvas() {
    // 1600 by 1200 in 512-row tiles: three bands, two seams.
    let streamed = rows("apple-photo");
    let (planes, _) = decode_planes(&fixture("apple-photo")).expect("planes");
    let mut converter = Converter::new(&planes, 6, true, false);
    let mut row = vec![0u8; planes.width * 3];
    for y in 0..planes.height {
        converter.row(y, 0, &mut row);
        let stride = planes.width * 3;
        assert_eq!(
            &streamed.pixels[y * stride..(y + 1) * stride],
            &row[..],
            "row {y}"
        );
    }
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
