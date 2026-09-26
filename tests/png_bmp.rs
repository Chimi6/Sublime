//! The image pair on the command line: every suite image goes PNG to BMP
//! to PNG through the converters and comes back as the same pixels, the
//! written BMP reads back in Pillow's terms (checked by the BMP round
//! trip), and a file without an extension is detected by its magic.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use sublime::converter::{ConvertError, ConvertOptions, Converter, Input};
use sublime::converters::image::{BMP_TO_PNG, PNG_TO_BMP};
use sublime::event::{Context, NullSink};
use sublime::image::ColorType;
use sublime::io::png::read_png;

fn convert(converter: &dyn Converter, bytes: &[u8]) -> Result<Vec<u8>, ConvertError> {
    let options = ConvertOptions::default();
    let mut sink = NullSink;
    let mut context = Context::new(&mut sink, &options);
    let mut output = Vec::new();
    let mut cursor = std::io::Cursor::new(bytes);
    converter.convert(Input::Rewindable(&mut cursor), &mut output, &mut context)?;
    Ok(output)
}

fn suite() -> Vec<(String, Vec<u8>)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/png");
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "png"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            (
                path.file_stem().unwrap().to_string_lossy().to_string(),
                fs::read(&path).unwrap(),
            )
        })
        .collect()
}

#[test]
fn every_suite_image_survives_png_to_bmp_to_png() {
    for (name, png) in suite() {
        let (original, _) = read_png(&png).unwrap();
        let bmp = convert(&PNG_TO_BMP, &png).unwrap_or_else(|error| panic!("{name}: {error}"));
        let back = convert(&BMP_TO_PNG, &bmp).unwrap_or_else(|error| panic!("{name}: {error}"));
        let (again, _) = read_png(&back).unwrap();
        // BMP has no gray: gray comes back as RGB with equal channels.
        let expected_color = match original.color {
            ColorType::Gray | ColorType::Rgb => ColorType::Rgb,
            ColorType::GrayAlpha | ColorType::Rgba => ColorType::Rgba,
        };
        assert_eq!(again.color, expected_color, "{name}");
        let channels = original.color.channels();
        for (index, cell) in original.pixels.chunks(channels).enumerate() {
            let got =
                &again.pixels[index * again.color.channels()..(index + 1) * again.color.channels()];
            let want: Vec<u8> = match original.color {
                ColorType::Gray => vec![cell[0], cell[0], cell[0]],
                ColorType::GrayAlpha => vec![cell[0], cell[0], cell[0], cell[1]],
                _ => cell.to_vec(),
            };
            assert_eq!(got, want.as_slice(), "{name} pixel {index}");
        }
    }
}

#[test]
fn an_image_without_an_extension_is_detected_by_its_magic() {
    let dir = std::env::temp_dir().join(format!("sublime-test-{}-magic", std::process::id()));
    let _ = fs::create_dir_all(&dir);
    let png =
        fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/png/basn2c08.png"))
            .unwrap();
    let bare = dir.join("picture");
    fs::write(&bare, &png).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_sublime"))
        .args(["-q", "convert", bare.to_str().unwrap(), "--to", "bmp"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.starts_with(b"BM"));
    let _ = fs::remove_dir_all(&dir);
}
