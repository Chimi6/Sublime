//! JPEG <-> PNG against the `image` crate (zune-jpeg to decode, its own
//! baseline encoder, 4:2:2 by default) and the `jpeg-encoder` crate (a
//! mozjpeg port, 4:2:0 below quality 90) for the encode side. Sizes at
//! the same quality number mean little when subsampling differs, so the
//! harness also measures every output's PSNR against the source pixels.

use std::fs::File;
use std::io::{BufReader, BufWriter, Cursor, Read};

use sublime::converters::image::{JPEG_TO_PNG, PNG_TO_JPEG};

use crate::common::run_ours;

pub fn run(mode: &str, args: &[String]) -> Result<(), String> {
    match (mode, args) {
        ("gen-photo", [side, path]) => generate(side, path, true),
        ("gen-flat", [side, path]) => generate(side, path, false),
        ("ours-jpeg-png", [input, output]) => run_ours(&JPEG_TO_PNG, input, output),
        ("ours-png-jpeg", [input, output]) => run_ours(&PNG_TO_JPEG, input, output),
        ("crates-jpeg-png", [input, output]) => crates_jpeg_to_png(input, output),
        ("crates-png-jpeg", [input, output]) => crates_png_to_jpeg(input, output),
        ("crates-png-jpeg-encoder", [input, output]) => crate_jpeg_encoder(input, output),
        ("pixels", [input]) => pixel_bytes(input),
        ("psnr", [original, candidate]) => psnr(original, candidate),
        ("time-decode", [input]) => time_decode(input),
        ("time-encode", [input]) => time_encode(input),
        _ => Err(
            "jpeg-png modes: gen-photo|gen-flat <side> <png>, ours-jpeg-png, ours-png-jpeg, crates-jpeg-png, crates-png-jpeg, crates-png-jpeg-encoder <in> <out>, pixels <file>, psnr <original.png> <candidate>, time-decode <jpg>"
                .to_string(),
        ),
    }
}

/// The photo shape is three gradients with independent noise per
/// channel; the flat shape is blocks of solid color. Written as PNG by
/// the png crate, and as a JPEG at quality 85 (4:2:0) by jpeg-encoder,
/// so the decode direction reads a real encoder's file.
fn generate(side: &str, path: &str, photo: bool) -> Result<(), String> {
    let side: u32 = side.parse().map_err(|_| "bad side".to_string())?;
    let mut pixels = vec![0u8; side as usize * side as usize * 3];
    let mut seed: u32 = 0x9e37_79b9;
    for y in 0..side {
        for x in 0..side {
            let at = (y as usize * side as usize + x as usize) * 3;
            if photo {
                let mut noise = [0i32; 3];
                for value in noise.iter_mut() {
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    *value = (seed >> 24) as i32 % 16 - 8;
                }
                pixels[at] = ((x * 255 / side) as i32 + noise[0]).clamp(0, 255) as u8;
                pixels[at + 1] = ((y * 255 / side) as i32 + noise[1]).clamp(0, 255) as u8;
                pixels[at + 2] =
                    (((x + y) * 255 / (2 * side)) as i32 + noise[2] / 2).clamp(0, 255) as u8;
            } else {
                let block = ((x / 64) + (y / 64)) % 5;
                let color: [u8; 3] = match block {
                    0 => [220, 40, 40],
                    1 => [40, 180, 60],
                    2 => [30, 60, 220],
                    3 => [250, 250, 250],
                    _ => [20, 20, 20],
                };
                pixels[at..at + 3].copy_from_slice(&color);
            }
        }
    }
    let file = File::create(path).map_err(|error| error.to_string())?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), side, side);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
    writer.write_image_data(&pixels).map_err(|error| error.to_string())?;
    drop(writer);
    let jpeg_path = path.replace(".png", ".jpg");
    let file = File::create(&jpeg_path).map_err(|error| error.to_string())?;
    let encoder = jpeg_encoder::Encoder::new(BufWriter::new(file), 85);
    encoder
        .encode(&pixels, side as u16, side as u16, jpeg_encoder::ColorType::Rgb)
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn read_all(path: &str) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|mut file| file.read_to_end(&mut bytes))
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

fn decode_png(path: &str) -> Result<(u32, u32, Vec<u8>), String> {
    let decoder = png::Decoder::new(BufReader::new(File::open(path).map_err(|error| error.to_string())?));
    let mut reader = decoder.read_info().map_err(|error| error.to_string())?;
    let mut pixels = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut pixels).map_err(|error| error.to_string())?;
    if info.color_type != png::ColorType::Rgb {
        return Err("the benchmark PNGs are RGB".to_string());
    }
    pixels.truncate(info.buffer_size());
    Ok((info.width, info.height, pixels))
}

/// Any RGB image the `image` crate can read, as pixels.
fn decode_any(path: &str) -> Result<(u32, u32, Vec<u8>), String> {
    let decoded = image::ImageReader::open(path)
        .map_err(|error| error.to_string())?
        .decode()
        .map_err(|error| error.to_string())?
        .into_rgb8();
    let (width, height) = decoded.dimensions();
    Ok((width, height, decoded.into_raw()))
}

fn pixel_bytes(input: &str) -> Result<(), String> {
    let (width, height, _) = decode_any(input)?;
    println!("{}", width as usize * height as usize * 3);
    Ok(())
}

/// PSNR of a decoded candidate against the original PNG's pixels, in dB.
fn psnr(original: &str, candidate: &str) -> Result<(), String> {
    let (_, _, a) = decode_png(original)?;
    let (_, _, b) = decode_any(candidate)?;
    if a.len() != b.len() {
        return Err("dimensions differ".to_string());
    }
    let squares: f64 = a
        .iter()
        .zip(&b)
        .map(|(x, y)| {
            let diff = f64::from(*x) - f64::from(*y);
            diff * diff
        })
        .sum();
    let mean = squares / a.len() as f64;
    if mean == 0.0 {
        println!("inf");
    } else {
        println!("{:.2}", 10.0 * (255.0 * 255.0 / mean).log10());
    }
    Ok(())
}

fn crates_jpeg_to_png(input: &str, output: &str) -> Result<(), String> {
    let bytes = read_all(input)?;
    let decoded = image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg)
        .map_err(|error| error.to_string())?
        .into_rgb8();
    let (width, height) = decoded.dimensions();
    let file = File::create(output).map_err(|error| error.to_string())?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    // The same deflate level the png-bmp pair holds the crate to.
    encoder.set_compression(png::Compression::Default);
    let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
    writer
        .write_image_data(decoded.as_raw())
        .map_err(|error| error.to_string())
}

/// The image crate's own encoder at quality 85 (it subsamples 4:2:2).
fn crates_png_to_jpeg(input: &str, output: &str) -> Result<(), String> {
    let (width, height, pixels) = decode_png(input)?;
    let file = File::create(output).map_err(|error| error.to_string())?;
    let mut writer = BufWriter::new(file);
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut writer, 85);
    image::ImageEncoder::write_image(
        encoder,
        &pixels,
        width,
        height,
        image::ExtendedColorType::Rgb8,
    )
    .map_err(|error| error.to_string())
}

/// The jpeg-encoder crate at quality 85 (4:2:0 below 90, like ours).
fn crate_jpeg_encoder(input: &str, output: &str) -> Result<(), String> {
    let (width, height, pixels) = decode_png(input)?;
    let file = File::create(output).map_err(|error| error.to_string())?;
    let encoder = jpeg_encoder::Encoder::new(BufWriter::new(file), 85);
    encoder
        .encode(&pixels, width as u16, height as u16, jpeg_encoder::ColorType::Rgb)
        .map_err(|error| error.to_string())
}

/// Times the image crate's (zune-jpeg) decode alone, from memory.
fn time_decode(input: &str) -> Result<(), String> {
    let bytes = read_all(input)?;
    let started = std::time::Instant::now();
    let decoded = image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg)
        .map_err(|error| error.to_string())?
        .into_rgb8();
    let elapsed = started.elapsed();
    let _ = Cursor::new(decoded.as_raw());
    println!(
        "image crate decode {}x{}: {:.0} ms",
        decoded.width(),
        decoded.height(),
        elapsed.as_secs_f64() * 1000.0
    );
    Ok(())
}

/// Times the jpeg-encoder crate's encode alone, from decoded pixels in
/// memory to a buffer.
fn time_encode(input: &str) -> Result<(), String> {
    let (width, height, pixels) = decode_png(input)?;
    let started = std::time::Instant::now();
    let mut out = Vec::with_capacity(pixels.len() / 4);
    let encoder = jpeg_encoder::Encoder::new(&mut out, 85);
    encoder
        .encode(&pixels, width as u16, height as u16, jpeg_encoder::ColorType::Rgb)
        .map_err(|error| error.to_string())?;
    let elapsed = started.elapsed();
    println!(
        "jpeg-encoder encode {}x{}: {:.0} ms ({} bytes)",
        width,
        height,
        elapsed.as_secs_f64() * 1000.0,
        out.len()
    );
    Ok(())
}
