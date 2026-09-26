//! PNG <-> BMP: generated photo-like and flat images; ours against the
//! `png` crate (decode and encode) and the `image` crate's BMP codec.

use std::fs::File;
use std::io::{BufReader, BufWriter, Write};

use sublime::converters::image::{BMP_TO_PNG, PNG_TO_BMP};

use crate::common::run_ours;

pub fn run(mode: &str, args: &[String]) -> Result<(), String> {
    match (mode, args) {
        ("gen-photo", [side, path]) => generate(side, path, true),
        ("gen-flat", [side, path]) => generate(side, path, false),
        ("ours-png-bmp", [input, output]) => run_ours(&PNG_TO_BMP, input, output),
        ("ours-bmp-png", [input, output]) => run_ours(&BMP_TO_PNG, input, output),
        ("crates-png-bmp", [input, output]) => crates_png_to_bmp(input, output),
        ("crates-bmp-png", [input, output]) => {
            crates_bmp_to_png(input, output, png::Compression::Default)
        }
        ("crates-bmp-png-fast", [input, output]) => {
            crates_bmp_to_png(input, output, png::Compression::Fast)
        }
        ("pixels", [input]) => pixel_bytes(input),
        ("time-decode", [input]) => time_decode(input),
        _ => Err(
            "png-bmp modes: gen-photo <side> <path> | gen-flat <side> <path> | ours-png-bmp <in> <out> | ours-bmp-png <in> <out> | crates-png-bmp <in> <out> | crates-bmp-png <in> <out> | pixels <png>"
                .to_string(),
        ),
    }
}

/// A square RGBA image written by the `png` crate: `photo` is a smooth
/// gradient with noise (what filters and deflate work hardest on), `flat`
/// is blocks of solid color with hard edges.
fn generate(side: &str, path: &str, photo: bool) -> Result<(), String> {
    let side: u32 = side.parse().map_err(|_| "bad side".to_string())?;
    let mut pixels = vec![0u8; side as usize * side as usize * 4];
    let mut seed: u32 = 0x9e37_79b9;
    for y in 0..side {
        for x in 0..side {
            let at = (y as usize * side as usize + x as usize) * 4;
            if photo {
                // Independent noise per channel, as a sensor gives; one
                // shared draw made every channel a function of the same
                // value, and the filters' residuals then repeated in
                // ways no real photo does.
                let mut noise = [0i32; 3];
                for value in noise.iter_mut() {
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    *value = (seed >> 24) as i32 % 16 - 8;
                }
                pixels[at] = ((x * 255 / side) as i32 + noise[0]).clamp(0, 255) as u8;
                pixels[at + 1] = ((y * 255 / side) as i32 + noise[1]).clamp(0, 255) as u8;
                pixels[at + 2] =
                    (((x + y) * 255 / (2 * side)) as i32 + noise[2] / 2).clamp(0, 255) as u8;
                pixels[at + 3] = 255;
            } else {
                let block = ((x / 64) + (y / 64)) % 5;
                let color: [u8; 4] = match block {
                    0 => [220, 40, 40, 255],
                    1 => [40, 180, 60, 255],
                    2 => [30, 60, 220, 255],
                    3 => [250, 250, 250, 255],
                    _ => [20, 20, 20, 128],
                };
                pixels[at..at + 4].copy_from_slice(&color);
            }
        }
    }
    let file = File::create(path).map_err(|error| error.to_string())?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), side, side);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
    writer
        .write_image_data(&pixels)
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// The decoded byte count of a PNG, for the throughput denominator.
fn pixel_bytes(input: &str) -> Result<(), String> {
    let file = File::open(input).map_err(|error| error.to_string())?;
    let decoder = png::Decoder::new(BufReader::new(file));
    let reader = decoder.read_info().map_err(|error| error.to_string())?;
    let info = reader.info();
    let bytes = info.width as u64
        * info.height as u64
        * info.color_type.samples() as u64
        * (info.bit_depth as u64 / 8).max(1);
    println!("{bytes}");
    Ok(())
}

/// Times the png crate's decode alone, from memory, for a phase split.
fn time_decode(input: &str) -> Result<(), String> {
    let bytes = std::fs::read(input).map_err(|error| error.to_string())?;
    let started = std::time::Instant::now();
    let decoder = png::Decoder::new(&bytes[..]);
    let mut reader = decoder.read_info().map_err(|error| error.to_string())?;
    let mut pixels = vec![0u8; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut pixels)
        .map_err(|error| error.to_string())?;
    let elapsed = started.elapsed();
    println!(
        "png crate decode {}x{}: {:.0} ms",
        info.width,
        info.height,
        elapsed.as_secs_f64() * 1000.0
    );
    Ok(())
}

fn crates_png_to_bmp(input: &str, output: &str) -> Result<(), String> {
    let file = File::open(input).map_err(|error| error.to_string())?;
    let decoder = png::Decoder::new(BufReader::new(file));
    let mut reader = decoder.read_info().map_err(|error| error.to_string())?;
    let mut buffer = vec![0u8; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|error| error.to_string())?;
    let color = match info.color_type {
        png::ColorType::Rgba => image::ExtendedColorType::Rgba8,
        png::ColorType::Rgb => image::ExtendedColorType::Rgb8,
        png::ColorType::Grayscale => image::ExtendedColorType::L8,
        png::ColorType::GrayscaleAlpha => image::ExtendedColorType::La8,
        other => return Err(format!("unsupported color {other:?}")),
    };
    let out = File::create(output).map_err(|error| error.to_string())?;
    let mut writer = BufWriter::new(out);
    let encoder = image::codecs::bmp::BmpEncoder::new(&mut writer);
    image::ImageEncoder::write_image(
        encoder,
        &buffer[..info.buffer_size()],
        info.width,
        info.height,
        color,
    )
    .map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())
}

fn crates_bmp_to_png(input: &str, output: &str, level: png::Compression) -> Result<(), String> {
    let decoded = image::ImageReader::open(input)
        .map_err(|error| error.to_string())?
        .decode()
        .map_err(|error| error.to_string())?;
    let rgba = decoded.to_rgba8();
    let file = File::create(output).map_err(|error| error.to_string())?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), rgba.width(), rgba.height());
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(level);
    let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
    writer
        .write_image_data(rgba.as_raw())
        .map_err(|error| error.to_string())
}
