//! QOI <-> PNG against the `qoi` crate (the one the `image` crate uses)
//! with the `png` crate at its default level on the PNG side. The QOI
//! inputs are written by Pillow (the qoi.h algorithm).

use std::fs::File;
use std::io::{BufReader, BufWriter, Read};

use crate::common::run_ours;
use sublime::converters::image as image_pairs;

pub fn run(mode: &str, args: &[String]) -> Result<(), String> {
    match (mode, args) {
        ("ours-qoi-png", [input, output]) => run_ours(image_pairs::pair("qoi", "png"), input, output),
        ("ours-png-qoi", [input, output]) => run_ours(image_pairs::pair("png", "qoi"), input, output),
        ("crates-qoi-png", [input, output]) => crates_qoi_to_png(input, output),
        ("crates-png-qoi", [input, output]) => crates_png_to_qoi(input, output),
        ("pixels", [input]) => pixels(input),
        _ => Err("qoi-png modes: ours-qoi-png, ours-png-qoi, crates-qoi-png, crates-png-qoi <in> <out>, pixels <qoi>".to_string()),
    }
}

fn read_all(path: &str) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|mut file| file.read_to_end(&mut bytes))
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

/// Decoded pixel bytes of a QOI file.
fn pixels(input: &str) -> Result<(), String> {
    let bytes = read_all(input)?;
    let (_, data) = qoi::decode_to_vec(&bytes).map_err(|error| error.to_string())?;
    println!("{}", data.len());
    Ok(())
}

fn crates_qoi_to_png(input: &str, output: &str) -> Result<(), String> {
    let bytes = read_all(input)?;
    let (header, data) = qoi::decode_to_vec(&bytes).map_err(|error| error.to_string())?;
    let color = match header.channels {
        qoi::Channels::Rgb => png::ColorType::Rgb,
        qoi::Channels::Rgba => png::ColorType::Rgba,
    };
    let file = File::create(output).map_err(|error| error.to_string())?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), header.width, header.height);
    encoder.set_color(color);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Default);
    let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
    writer.write_image_data(&data).map_err(|error| error.to_string())
}

fn crates_png_to_qoi(input: &str, output: &str) -> Result<(), String> {
    let decoder = png::Decoder::new(BufReader::new(File::open(input).map_err(|error| error.to_string())?));
    let mut reader = decoder.read_info().map_err(|error| error.to_string())?;
    let mut data = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut data).map_err(|error| error.to_string())?;
    data.truncate(info.buffer_size());
    match info.color_type {
        png::ColorType::Rgba | png::ColorType::Rgb => {}
        other => return Err(format!("unsupported color {other:?}")),
    }
    let encoded = qoi::encode_to_vec(&data, info.width, info.height).map_err(|error| error.to_string())?;
    std::fs::write(output, encoded).map_err(|error| error.to_string())
}
