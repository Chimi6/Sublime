//! TGA <-> PNG against the `image` crate's TGA codec (run-length encoded
//! both ways) with the `png` crate at its default level on the PNG side.
//! The TGA inputs are Pillow's defaults: RLE, bottom-up.

use std::fs::File;
use std::io::{BufReader, BufWriter, Read};

use crate::common::run_ours;
use sublime::converters::image as image_pairs;

pub fn run(mode: &str, args: &[String]) -> Result<(), String> {
    match (mode, args) {
        ("ours-tga-png", [input, output]) => run_ours(image_pairs::pair("tga", "png"), input, output),
        ("ours-png-tga", [input, output]) => run_ours(image_pairs::pair("png", "tga"), input, output),
        ("crates-tga-png", [input, output]) => crates_tga_to_png(input, output),
        ("crates-png-tga", [input, output]) => crates_png_to_tga(input, output),
        ("pixels", [input]) => pixels(input),
        _ => Err("tga-png modes: ours-tga-png, ours-png-tga, crates-tga-png, crates-png-tga <in> <out>, pixels <tga>".to_string()),
    }
}

fn load(input: &str) -> Result<image::DynamicImage, String> {
    let mut bytes = Vec::new();
    File::open(input)
        .and_then(|mut file| file.read_to_end(&mut bytes))
        .map_err(|error| error.to_string())?;
    image::load_from_memory_with_format(&bytes, image::ImageFormat::Tga).map_err(|error| error.to_string())
}

fn pixels(input: &str) -> Result<(), String> {
    println!("{}", load(input)?.as_bytes().len());
    Ok(())
}

fn crates_tga_to_png(input: &str, output: &str) -> Result<(), String> {
    let decoded = load(input)?;
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

fn crates_png_to_tga(input: &str, output: &str) -> Result<(), String> {
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
    let file = BufWriter::new(File::create(output).map_err(|error| error.to_string())?);
    let encoder = image::codecs::tga::TgaEncoder::new(file);
    image::ImageEncoder::write_image(encoder, &data, info.width, info.height, color)
        .map_err(|error| error.to_string())
}
