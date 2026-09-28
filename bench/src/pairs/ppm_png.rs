//! Netpbm <-> PNG against the `image` crate's PNM codec with the `png`
//! crate at its default level on the PNG side. RGB images go as PPM (P6);
//! the RGBA stock photograph goes as PAM (P7).

use std::fs::File;
use std::io::{BufReader, BufWriter, Read};

use crate::common::run_ours;
use image::codecs::pnm::{PnmEncoder, PnmSubtype, SampleEncoding};
use sublime::converters::image as image_pairs;

pub fn run(mode: &str, args: &[String]) -> Result<(), String> {
    match (mode, args) {
        ("ours-ppm-png", [input, output]) => run_ours(image_pairs::pair("ppm", "png"), input, output),
        ("ours-png-ppm", [input, output]) => run_ours(image_pairs::pair("png", "ppm"), input, output),
        ("ours-pam-png", [input, output]) => run_ours(image_pairs::pair("pam", "png"), input, output),
        ("ours-png-pam", [input, output]) => run_ours(image_pairs::pair("png", "pam"), input, output),
        ("crates-pnm-png", [input, output]) => crates_pnm_to_png(input, output),
        ("crates-png-ppm", [input, output]) => crates_png_to_pnm(input, output, false),
        ("crates-png-pam", [input, output]) => crates_png_to_pnm(input, output, true),
        _ => Err("ppm-png modes: ours-ppm-png, ours-png-ppm, ours-pam-png, ours-png-pam, crates-pnm-png, crates-png-ppm, crates-png-pam <in> <out>".to_string()),
    }
}

fn crates_pnm_to_png(input: &str, output: &str) -> Result<(), String> {
    let mut bytes = Vec::new();
    File::open(input)
        .and_then(|mut file| file.read_to_end(&mut bytes))
        .map_err(|error| error.to_string())?;
    let decoded = image::load_from_memory_with_format(&bytes, image::ImageFormat::Pnm)
        .map_err(|error| error.to_string())?;
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

fn crates_png_to_pnm(input: &str, output: &str, pam: bool) -> Result<(), String> {
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
    let subtype = if pam {
        PnmSubtype::ArbitraryMap
    } else {
        PnmSubtype::Pixmap(SampleEncoding::Binary)
    };
    let encoder = PnmEncoder::new(file).with_subtype(subtype);
    image::ImageEncoder::write_image(encoder, &data, info.width, info.height, color)
        .map_err(|error| error.to_string())
}
