//! The PDF reader: every fixture (Pillow's, ImageMagick's, Ghostscript's
//! object streams and inline images, and built cases: pages, forms,
//! 16-bit, Decode, filter chains, CMYK, palettes, stencil masks, broken
//! cross references) opens to its page's image (`.pix` beside it; a name
//! ending `-pageN` is read at page N); our writer's PDFs read back to
//! their images; a JPEG comes out as its own bytes; and unsupported or
//! broken files are refused.

use std::fs;
use std::path::PathBuf;

use sublime::image::{ColorType, Image};
use sublime::io::pdf::{PdfDocument, page_jpeg, read_pdf};
use sublime::io::png::{RowSink, read_png};

fn dir(sub: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/pdf")
        .join(sub)
}

fn files(sub: &str) -> Vec<(String, Vec<u8>)> {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir(sub))
        .expect("fixture dir")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "pdf"))
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

fn page_of(name: &str) -> Option<u32> {
    name.rsplit_once("-page")
        .and_then(|(_, page)| page.parse().ok())
}

fn pix(name: &str) -> Image {
    let bytes = fs::read(dir("").join(format!("{name}.pix"))).expect("pix");
    let newline = bytes
        .iter()
        .position(|byte| *byte == b'\n')
        .expect("header");
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
fn every_fixture_opens_to_its_pages_image() {
    let mut checked = 0;
    for (name, bytes) in files("") {
        let (image, _) =
            read_pdf(&bytes, page_of(&name)).unwrap_or_else(|error| panic!("{name}: {error}"));
        let want = pix(&name);
        assert_eq!(
            (image.width, image.height, image.color),
            (want.width, want.height, want.color),
            "{name}"
        );
        assert!(image.pixels == want.pixels, "{name}: pixels differ");
        checked += 1;
    }
    assert_eq!(checked, 24);
}

#[test]
fn our_pdfs_read_back_to_their_images() {
    for path in [
        "tests/fixtures/png/basn2c08.png",
        "tests/fixtures/png/basn6a08.png",
        "tests/fixtures/png/basn0g08.png",
        "tests/fixtures/png/basn4a08.png",
    ] {
        let (image, _) =
            read_png(&fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path)).unwrap())
                .unwrap();
        let mut pdf = Vec::new();
        let mut document = PdfDocument::new(&mut pdf).unwrap();
        {
            let mut page = document.image_page();
            page.start(image.width, image.height, image.color).unwrap();
            for y in 0..image.height {
                page.row(image.row(y)).unwrap();
            }
        }
        document.finish().unwrap();
        let (back, _) = read_pdf(&pdf, None).unwrap_or_else(|error| panic!("{path}: {error}"));
        assert_eq!(
            (back.width, back.height, back.color),
            (image.width, image.height, image.color),
            "{path}"
        );
        assert!(back.pixels == image.pixels, "{path}: pixels differ");
    }
}

#[test]
fn a_jpeg_image_comes_out_as_its_own_bytes() {
    let jpeg = fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/jpeg/photo-200x130-q75-420.jpg"),
    )
    .unwrap();
    let mut pdf = Vec::new();
    let mut document = PdfDocument::new(&mut pdf).unwrap();
    document.jpeg_page(&jpeg).unwrap();
    document.finish().unwrap();
    assert_eq!(
        page_jpeg(&pdf, None).unwrap().as_deref(),
        Some(jpeg.as_slice())
    );
    // A PNG page has no JPEG to copy.
    let (image, _) = read_pdf(&fs::read(dir("").join("form.pdf")).unwrap(), None).unwrap();
    assert_eq!(image.width, 33);
    assert_eq!(
        page_jpeg(&fs::read(dir("").join("form.pdf")).unwrap(), None).unwrap(),
        None
    );
}

#[test]
fn unsupported_and_broken_files_are_refused() {
    let invalid = files("invalid");
    assert_eq!(invalid.len(), 6);
    for (name, bytes) in invalid {
        assert!(
            read_pdf(&bytes, page_of(&name)).is_err(),
            "{name} was accepted"
        );
    }
}
