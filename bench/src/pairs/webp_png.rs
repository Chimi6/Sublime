//! WebP <-> PNG against the `image` crate (image-webp to decode, its
//! lossless encoder to encode) with the `png` crate on the PNG side. The
//! WebP inputs are written by libwebp (through Pillow) so the decode
//! direction reads a real encoder's files, lossless and lossy.

use std::fs::File;
use std::io::{BufReader, BufWriter, Read};

use crate::common::run_ours;
use sublime::converters::image as image_pairs;

pub fn run(mode: &str, args: &[String]) -> Result<(), String> {
    match (mode, args) {
        ("ours-webp-png", [input, output]) => run_ours(image_pairs::pair("webp", "png"), input, output),
        ("ours-png-webp", [input, output]) => run_ours(image_pairs::pair("png", "webp"), input, output),
        ("crates-webp-png", [input, output]) => crates_webp_to_png(input, output),
        ("crates-png-webp", [input, output]) => crates_png_to_webp(input, output),
        ("pixels", [input]) => pixels(input),
        ("time-decode", [input]) => time_decode(input),
        _ => Err("webp-png modes: ours-webp-png, ours-png-webp, crates-webp-png, crates-png-webp <in> <out>, pixels <file>".to_string()),
    }
}

fn read_all(path: &str) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|mut file| file.read_to_end(&mut bytes))
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

fn pixels(input: &str) -> Result<(), String> {
    let decoded = image::ImageReader::open(input)
        .map_err(|error| error.to_string())?
        .with_guessed_format()
        .map_err(|error| error.to_string())?
        .decode()
        .map_err(|error| error.to_string())?;
    println!("{}", decoded.as_bytes().len());
    Ok(())
}

fn crates_webp_to_png(input: &str, output: &str) -> Result<(), String> {
    let bytes = read_all(input)?;
    let decoded = image::load_from_memory_with_format(&bytes, image::ImageFormat::WebP).map_err(|error| error.to_string())?;
    let (width, height) = (decoded.width(), decoded.height());
    let (color, data) = if decoded.color().has_alpha() {
        (png::ColorType::Rgba, decoded.into_rgba8().into_raw())
    } else {
        (png::ColorType::Rgb, decoded.into_rgb8().into_raw())
    };
    let file = File::create(output).map_err(|error| error.to_string())?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), width, height);
    encoder.set_color(color);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Default);
    let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
    writer.write_image_data(&data).map_err(|error| error.to_string())
}

fn crates_png_to_webp(input: &str, output: &str) -> Result<(), String> {
    let decoder = png::Decoder::new(BufReader::new(File::open(input).map_err(|error| error.to_string())?));
    let mut reader = decoder.read_info().map_err(|error| error.to_string())?;
    let mut data = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut data).map_err(|error| error.to_string())?;
    data.truncate(info.buffer_size());
    let color = match info.color_type {
        png::ColorType::Rgba => image::ExtendedColorType::Rgba8,
        png::ColorType::Rgb => image::ExtendedColorType::Rgb8,
        other => return Err(format!("unsupported color {other:?}")),
    };
    let file = File::create(output).map_err(|error| error.to_string())?;
    let encoder = image::codecs::webp::WebPEncoder::new_lossless(BufWriter::new(file));
    image::ImageEncoder::write_image(encoder, &data, info.width, info.height, color).map_err(|error| error.to_string())
}

/// Times the image crate's (image-webp) decode alone, from memory.
fn time_decode(input: &str) -> Result<(), String> {
    let bytes = read_all(input)?;
    let started = std::time::Instant::now();
    let decoded = image::load_from_memory_with_format(&bytes, image::ImageFormat::WebP).map_err(|error| error.to_string())?;
    println!("image-webp decode {}x{}: {:.0} ms", decoded.width(), decoded.height(), started.elapsed().as_secs_f64() * 1000.0);
    Ok(())
}
