//! ICO <-> PNG against the `image` crate: its ICO decoder (the largest
//! entry) with the `png` crate at its default level, and for icons the
//! same seven standard sizes ours writes, each a `thumbnail` of the
//! source encoded as a PNG entry.

use std::fs::File;
use std::io::{BufReader, BufWriter, Read};

use crate::common::run_ours;
use image::codecs::ico::{IcoEncoder, IcoFrame};
use sublime::converters::image as image_pairs;

pub fn run(mode: &str, args: &[String]) -> Result<(), String> {
    match (mode, args) {
        ("ours-ico-png", [input, output]) => run_ours(image_pairs::pair("ico", "png"), input, output),
        ("ours-png-ico", [input, output]) => run_ours(image_pairs::pair("png", "ico"), input, output),
        ("crates-ico-png", [input, output]) => crates_ico_to_png(input, output),
        ("crates-png-ico", [input, output]) => crates_png_to_ico(input, output),
        _ => Err("ico-png modes: ours-ico-png, ours-png-ico, crates-ico-png, crates-png-ico <in> <out>".to_string()),
    }
}

fn crates_ico_to_png(input: &str, output: &str) -> Result<(), String> {
    let mut bytes = Vec::new();
    File::open(input)
        .and_then(|mut file| file.read_to_end(&mut bytes))
        .map_err(|error| error.to_string())?;
    let decoded = image::load_from_memory_with_format(&bytes, image::ImageFormat::Ico)
        .map_err(|error| error.to_string())?
        .into_rgba8();
    let (width, height) = decoded.dimensions();
    let file = File::create(output).map_err(|error| error.to_string())?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Default);
    let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
    writer.write_image_data(&decoded.into_raw()).map_err(|error| error.to_string())
}

fn crates_png_to_ico(input: &str, output: &str) -> Result<(), String> {
    let decoder = png::Decoder::new(BufReader::new(File::open(input).map_err(|error| error.to_string())?));
    let mut reader = decoder.read_info().map_err(|error| error.to_string())?;
    let mut data = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut data).map_err(|error| error.to_string())?;
    data.truncate(info.buffer_size());
    let source = match info.color_type {
        png::ColorType::Rgba => image::DynamicImage::ImageRgba8(
            image::RgbaImage::from_raw(info.width, info.height, data).ok_or("bad size")?,
        ),
        png::ColorType::Rgb => image::DynamicImage::ImageRgb8(
            image::RgbImage::from_raw(info.width, info.height, data).ok_or("bad size")?,
        ),
        other => return Err(format!("unsupported color {other:?}")),
    };
    let longer = info.width.max(info.height);
    let mut frames = Vec::new();
    for size in [16u32, 24, 32, 48, 64, 128, 256] {
        if size > longer {
            continue;
        }
        let small = source.thumbnail(size, size).into_rgba8();
        let (width, height) = small.dimensions();
        frames.push(
            IcoFrame::as_png(small.as_raw(), width, height, image::ExtendedColorType::Rgba8)
                .map_err(|error| error.to_string())?,
        );
    }
    let file = BufWriter::new(File::create(output).map_err(|error| error.to_string())?);
    IcoEncoder::new(file).encode_images(&frames).map_err(|error| error.to_string())
}
