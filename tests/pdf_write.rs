//! The PDF writer: every cross-reference offset lands on its object, each
//! page holds its image at the image's size, the pixels inflate and
//! un-predict to the source's (the alpha to the soft mask), and a JPEG
//! is embedded byte for byte.

use std::fs;
use std::path::PathBuf;

use sublime::image::{ColorType, Image};
use sublime::io::deflate::inflate;
use sublime::io::pdf::PdfDocument;
use sublime::io::png::{RowSink, read_png};

fn fixture(path: &str) -> Vec<u8> {
    fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path)).expect(path)
}

fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    haystack[from..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|at| at + from)
}

fn number_after(bytes: &[u8], key: &[u8], from: usize) -> usize {
    let at = find(bytes, key, from).expect("key") + key.len();
    let text: String = bytes[at..]
        .iter()
        .skip_while(|byte| **byte == b' ')
        .take_while(|byte| byte.is_ascii_digit())
        .map(|byte| *byte as char)
        .collect();
    text.parse().expect("number")
}

/// The xref table's offsets, checked against the objects they point at.
fn offsets(pdf: &[u8]) -> Vec<usize> {
    let start = number_after(pdf, b"startxref\n", 0);
    assert_eq!(&pdf[start..start + 4], b"xref");
    let count = number_after(pdf, b"xref\n0 ", start);
    let mut table = find(pdf, b"f \n", start).expect("free entry") + 3;
    let mut offsets = vec![0];
    for number in 1..count {
        let offset: usize = String::from_utf8_lossy(&pdf[table..table + 10])
            .parse()
            .unwrap();
        let header = format!("{number} 0 obj");
        assert_eq!(
            &pdf[offset..offset + header.len()],
            header.as_bytes(),
            "object {number}"
        );
        offsets.push(offset);
        table += 20;
    }
    offsets
}

/// An object's stream bytes, its length read directly or through a
/// reference.
fn stream(pdf: &[u8], offsets: &[usize], number: usize) -> Vec<u8> {
    let at = offsets[number];
    let dictionary_end = find(pdf, b"stream\n", at).unwrap();
    let dictionary = &pdf[at..dictionary_end];
    let length_at = find(dictionary, b"/Length ", 0).unwrap() + 8;
    let rest = String::from_utf8_lossy(&dictionary[length_at..]).to_string();
    let words: Vec<&str> = rest.split_whitespace().collect();
    let length: usize = if words.len() > 2 && words[1] == "0" && words[2].starts_with('R') {
        let target = offsets[words[0].parse::<usize>().unwrap()];
        number_after(pdf, b"obj\n", target)
    } else {
        words[0].trim_end_matches(">>").parse().unwrap()
    };
    let data = dictionary_end + 7;
    pdf[data..data + length].to_vec()
}

/// A zlib stream of PNG-predicted rows back to pixels.
fn unpredict(stream: &[u8], width: usize, channels: usize, height: usize) -> Vec<u8> {
    let mut raw = Vec::new();
    inflate(&stream[2..], &mut raw, usize::MAX).unwrap();
    let stride = width * channels;
    let mut out = vec![0u8; stride * height];
    for y in 0..height {
        let filter = raw[y * (stride + 1)];
        let line = &raw[y * (stride + 1) + 1..(y + 1) * (stride + 1)];
        for x in 0..stride {
            let left = if x >= channels {
                out[y * stride + x - channels]
            } else {
                0
            };
            let up = if y > 0 { out[(y - 1) * stride + x] } else { 0 };
            let corner = if y > 0 && x >= channels {
                out[(y - 1) * stride + x - channels]
            } else {
                0
            };
            let predicted = match filter {
                0 => 0,
                1 => left,
                2 => up,
                3 => ((u16::from(left) + u16::from(up)) / 2) as u8,
                _ => {
                    let p = i16::from(left) + i16::from(up) - i16::from(corner);
                    let (pa, pb, pc) = (
                        (p - i16::from(left)).abs(),
                        (p - i16::from(up)).abs(),
                        (p - i16::from(corner)).abs(),
                    );
                    if pa <= pb && pa <= pc {
                        left
                    } else if pb <= pc {
                        up
                    } else {
                        corner
                    }
                }
            };
            out[y * stride + x] = line[x].wrapping_add(predicted);
        }
    }
    out
}

fn write_one(image: &Image) -> Vec<u8> {
    let mut out = Vec::new();
    let mut document = PdfDocument::new(&mut out).unwrap();
    {
        let mut page = document.image_page();
        page.start(image.width, image.height, image.color).unwrap();
        for y in 0..image.height {
            page.row(image.row(y)).unwrap();
        }
    }
    document.finish().unwrap();
    out
}

#[test]
fn a_page_of_pixels_holds_the_image_exactly() {
    for path in [
        "tests/fixtures/png/basn2c08.png",
        "tests/fixtures/png/basn6a08.png",
        "tests/fixtures/png/basn0g08.png",
        "tests/fixtures/png/basn4a08.png",
    ] {
        let (image, _) = read_png(&fixture(path)).unwrap();
        let pdf = write_one(&image);
        let offsets = offsets(&pdf);
        // Objects: catalog, pages, then the image, its length, its mask.
        let (width, height) = (image.width as usize, image.height as usize);
        let channels = image.color.channels();
        let colors = match image.color {
            ColorType::Gray | ColorType::GrayAlpha => 1,
            _ => 3,
        };
        let color = unpredict(&stream(&pdf, &offsets, 3), width, colors, height);
        let want: Vec<u8> = image
            .pixels
            .chunks_exact(channels)
            .flat_map(|cell| cell[..colors].to_vec())
            .collect();
        assert!(color == want, "{path}: color differs");
        if channels > colors {
            let mask = unpredict(&stream(&pdf, &offsets, 5), width, 1, height);
            let alpha: Vec<u8> = image
                .pixels
                .chunks_exact(channels)
                .map(|cell| cell[colors])
                .collect();
            assert!(mask == alpha, "{path}: alpha differs");
        }
        let text = String::from_utf8_lossy(&pdf);
        assert!(
            text.contains(&format!("/MediaBox [0 0 {width} {height}]")),
            "{path}"
        );
        assert!(text.contains("/Count 1"), "{path}");
    }
}

#[test]
fn a_jpeg_is_embedded_byte_for_byte() {
    let jpeg = fixture("tests/fixtures/jpeg/photo-200x130-q75-420.jpg");
    let mut out = Vec::new();
    let mut document = PdfDocument::new(&mut out).unwrap();
    document.jpeg_page(&jpeg).unwrap();
    document.jpeg_page(&jpeg).unwrap();
    document.finish().unwrap();
    let offsets = offsets(&out);
    assert!(stream(&out, &offsets, 3) == jpeg);
    let text = String::from_utf8_lossy(&out);
    assert!(text.contains("/Filter /DCTDecode"));
    assert!(text.contains("/Count 2"));
    assert!(text.contains("/MediaBox [0 0 200 130]"));
}
