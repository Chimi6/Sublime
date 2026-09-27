//! TIFF <-> PNG against the `image` crate's TIFF decoder (the `tiff`
//! crate) with the `png` crate at its default level, and for TIFF output
//! the `tiff` crate with deflate and the horizontal predictor, as ours
//! writes. The TIFF inputs are ImageMagick's LZW with the predictor.

use std::fs::File;
use std::io::{BufReader, BufWriter, Read};

use crate::common::run_ours;
use sublime::converters::image as image_pairs;
use tiff::encoder::{Compression, DeflateLevel, Predictor, TiffEncoder, colortype};

pub fn run(mode: &str, args: &[String]) -> Result<(), String> {
    match (mode, args) {
        ("ours-tiff-png", [input, output]) => run_ours(image_pairs::pair("tiff", "png"), input, output),
        ("ours-png-tiff", [input, output]) => run_ours(image_pairs::pair("png", "tiff"), input, output),
        ("crates-tiff-png", [input, output]) => crates_tiff_to_png(input, output),
        ("crates-png-tiff", [input, output]) => crates_png_to_tiff(input, output),
        ("pixels", [input]) => pixels(input),
        _ => Err("tiff-png modes: ours-tiff-png, ours-png-tiff, crates-tiff-png, crates-png-tiff <in> <out>, pixels <tiff>".to_string()),
    }
}

fn load(input: &str) -> Result<image::DynamicImage, String> {
    let mut bytes = Vec::new();
    File::open(input)
        .and_then(|mut file| file.read_to_end(&mut bytes))
        .map_err(|error| error.to_string())?;
    // Without the crate's default memory limit, which refuses the stock
    // photograph.
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Tiff);
    reader.no_limits();
    reader.decode().map_err(|error| error.to_string())
}

fn pixels(input: &str) -> Result<(), String> {
    println!("{}", load(input)?.as_bytes().len());
    Ok(())
}

fn crates_tiff_to_png(input: &str, output: &str) -> Result<(), String> {
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

fn crates_png_to_tiff(input: &str, output: &str) -> Result<(), String> {
    let decoder = png::Decoder::new(BufReader::new(File::open(input).map_err(|error| error.to_string())?));
    let mut reader = decoder.read_info().map_err(|error| error.to_string())?;
    let mut data = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut data).map_err(|error| error.to_string())?;
    data.truncate(info.buffer_size());
    let file = BufWriter::new(File::create(output).map_err(|error| error.to_string())?);
    let mut encoder = TiffEncoder::new(file)
        .map_err(|error| error.to_string())?
        .with_compression(Compression::Deflate(DeflateLevel::Balanced))
        .with_predictor(Predictor::Horizontal);
    let result = match info.color_type {
        png::ColorType::Rgba => encoder.write_image::<colortype::RGBA8>(info.width, info.height, &data),
        png::ColorType::Rgb => encoder.write_image::<colortype::RGB8>(info.width, info.height, &data),
        other => return Err(format!("unsupported color {other:?}")),
    };
    result.map_err(|error| error.to_string())
}
