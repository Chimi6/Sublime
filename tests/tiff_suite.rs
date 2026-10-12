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

/// A sink taking 16-bit rows, keeping them.
#[derive(Default)]
struct Deep {
    deep: bool,
    pixels: Vec<u8>,
}

impl sublime::io::png::RowSink for Deep {
    fn accept_deep(&mut self, _color: ColorType) -> bool {
        self.deep = true;
        true
    }

    fn start(&mut self, _width: u32, _height: u32, _color: ColorType) -> std::io::Result<()> {
        Ok(())
    }

    fn row(&mut self, pixels: &[u8]) -> std::io::Result<()> {
        self.pixels.extend_from_slice(pixels);
        Ok(())
    }
}

#[test]
fn sixteen_bit_files_go_on_at_sixteen_bits() {
    for name in ["L-16bit", "RGB-16bit", "RGBA-16bit"] {
        let bytes = fs::read(dir("").join(format!("{name}.tif"))).expect("fixture");
        let mut sink = Deep::default();
        let notes = sublime::io::tiff::read_tiff_rows(&mut &bytes[..], &mut sink).expect(name);
        assert!(sink.deep && !notes.sixteen_bit, "{name}");
        let eight = pix(dir("").join(format!("{name}.pix")));
        let high: Vec<u8> = sink.pixels.iter().step_by(2).copied().collect();
        assert_eq!(high, eight.pixels, "{name}");
        assert!(
            sink.pixels.iter().skip(1).step_by(2).any(|&low| low != 0),
            "{name}"
        );
    }
    // Written at 16 bits and read back unchanged.
    let bytes = fs::read(dir("").join("RGB-16bit.tif")).expect("fixture");
    let mut first = Deep::default();
    sublime::io::tiff::read_tiff_rows(&mut &bytes[..], &mut first).expect("read");
    let image = read_tiff(&bytes).expect("eight-bit read");
    let mut written = Vec::new();
    {
        use sublime::io::png::RowSink;
        let mut rows = sublime::io::tiff::TiffRows::new(&mut written);
        assert!(rows.accept_deep(ColorType::Rgb));
        rows.start(image.width, image.height, ColorType::Rgb)
            .expect("start");
        for row in first.pixels.chunks_exact(image.width as usize * 6) {
            rows.row(row).expect("row");
        }
    }
    let mut second = Deep::default();
    sublime::io::tiff::read_tiff_rows(&mut &written[..], &mut second).expect("read back");
    assert_eq!(first.pixels, second.pixels);
}
