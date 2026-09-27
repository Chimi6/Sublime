//! Images <-> PDF (the png-pdf and jpeg-pdf pairs). PNG to PDF is
//! against printpdf, set lossless (Flate, no resize, no JPEG, no gray
//! detection); PDF to PNG against lopdf's image lookup and Flate with the
//! png crate at its default level. JPEG to PDF embeds the file as a
//! DCTDecode stream with lopdf (printpdf decodes and re-encodes it, which
//! is not the same job); PDF to JPEG copies that stream back out.

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};

use crate::common::run_ours;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, dictionary};
use printpdf::{
    ImageCompression, ImageOptimizationOptions, Mm, Op, PdfPage, PdfSaveOptions, RawImage, RawImageData, RawImageFormat,
    XObjectTransform,
};
use sublime::converters::image as image_pairs;

pub fn run(mode: &str, args: &[String]) -> Result<(), String> {
    match (mode, args) {
        ("ours-png-pdf", [input, output]) => run_ours(image_pairs::pair("png", "pdf"), input, output),
        ("ours-pdf-png", [input, output]) => run_ours(image_pairs::pair("pdf", "png"), input, output),
        ("ours-jpeg-pdf", [input, output]) => run_ours(image_pairs::pair("jpeg", "pdf"), input, output),
        ("ours-pdf-jpeg", [input, output]) => run_ours(image_pairs::pair("pdf", "jpeg"), input, output),
        ("crates-png-pdf", [input, output]) => crates_png_to_pdf(input, output),
        ("crates-pdf-png", [input, output]) => crates_pdf_to_png(input, output),
        ("crates-jpeg-pdf", [input, output]) => crates_jpeg_to_pdf(input, output),
        ("crates-pdf-jpeg", [input, output]) => crates_pdf_to_jpeg(input, output),
        ("pixels", [input]) => pixels(input),
        _ => Err("pdf modes: ours-png-pdf, ours-pdf-png, ours-jpeg-pdf, ours-pdf-jpeg, crates-png-pdf, crates-pdf-png, crates-jpeg-pdf, crates-pdf-jpeg <in> <out>, pixels <png or jpeg>".to_string()),
    }
}

fn read_all(input: &str) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    File::open(input)
        .and_then(|mut file| file.read_to_end(&mut bytes))
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

fn pixels(input: &str) -> Result<(), String> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(read_all(input)?))
        .with_guessed_format()
        .map_err(|error| error.to_string())?;
    reader.no_limits();
    let decoded = reader.decode().map_err(|error| error.to_string())?;
    println!("{}", decoded.as_bytes().len());
    Ok(())
}

/// A page's size in millimetres at 72 dpi, as ours sizes it.
fn millimetres(pixels: usize) -> Mm {
    Mm(pixels as f32 * 25.4 / 72.0)
}

fn crates_png_to_pdf(input: &str, output: &str) -> Result<(), String> {
    let decoder = png::Decoder::new(BufReader::new(File::open(input).map_err(|error| error.to_string())?));
    let mut reader = decoder.read_info().map_err(|error| error.to_string())?;
    let mut data = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut data).map_err(|error| error.to_string())?;
    data.truncate(info.buffer_size());
    let data_format = match info.color_type {
        png::ColorType::Rgba => RawImageFormat::RGBA8,
        png::ColorType::Rgb => RawImageFormat::RGB8,
        png::ColorType::Grayscale => RawImageFormat::R8,
        other => return Err(format!("unsupported color {other:?}")),
    };
    let width = info.width as usize;
    let height = info.height as usize;
    let image = RawImage {
        pixels: RawImageData::U8(data),
        width,
        height,
        data_format,
        tag: Vec::new(),
    };
    let mut document = printpdf::PdfDocument::new("bench");
    let image_id = document.add_image(&image);
    let transform = XObjectTransform {
        dpi: Some(72.0),
        ..XObjectTransform::default()
    };
    let operations = vec![Op::UseXobject { id: image_id, transform }];
    let page = PdfPage::new(millimetres(width), millimetres(height), operations);
    let options = PdfSaveOptions {
        image_optimization: Some(ImageOptimizationOptions {
            quality: None,
            max_image_size: None,
            dither_greyscale: None,
            convert_to_greyscale: None,
            auto_optimize: Some(false),
            format: Some(ImageCompression::Flate),
        }),
        ..PdfSaveOptions::default()
    };
    let bytes = document.with_pages(vec![page]).save(&options, &mut Vec::new());
    std::fs::write(output, bytes).map_err(|error| error.to_string())
}

/// The first page's largest image XObject, taken out of the document.
fn largest_image(document: &mut Document) -> Result<Stream, String> {
    let page_id = *document.get_pages().values().next().ok_or("no pages")?;
    let images = document.get_page_images(page_id).map_err(|error| error.to_string())?;
    let largest = images
        .iter()
        .max_by_key(|image| image.width * image.height)
        .ok_or("no image on the page")?;
    let id: ObjectId = largest.id;
    take_stream(document, id)
}

fn take_stream(document: &mut Document, id: ObjectId) -> Result<Stream, String> {
    match document.objects.remove(&id) {
        Some(Object::Stream(stream)) => Ok(stream),
        _ => Err("the image is not a stream".to_string()),
    }
}

/// Flate content of an image stream. lopdf refuses to decompress image
/// streams, so the subtype is dropped first.
fn image_samples(mut stream: Stream) -> Result<Vec<u8>, String> {
    stream.dict.remove(b"Subtype");
    stream.decompressed_content().map_err(|error| error.to_string())
}

fn crates_pdf_to_png(input: &str, output: &str) -> Result<(), String> {
    let mut document = Document::load_mem(&read_all(input)?).map_err(|error| error.to_string())?;
    let stream = largest_image(&mut document)?;
    let width = stream.dict.get(b"Width").and_then(Object::as_i64).map_err(|error| error.to_string())? as u32;
    let height = stream.dict.get(b"Height").and_then(Object::as_i64).map_err(|error| error.to_string())? as u32;
    let gray = stream.dict.get(b"ColorSpace").and_then(Object::as_name).ok() == Some(b"DeviceGray".as_slice());
    let smask = stream.dict.get(b"SMask").and_then(Object::as_reference).ok();
    let color = image_samples(stream)?;
    let alpha = match smask {
        Some(id) => Some(image_samples(take_stream(&mut document, id)?)?),
        None => None,
    };
    let channels = if gray { 1 } else { 3 };
    let (color_type, data) = match alpha {
        Some(alpha) => {
            let mut joined = Vec::with_capacity(alpha.len() * (channels + 1));
            for (pixel, opacity) in color.chunks_exact(channels).zip(alpha) {
                joined.extend_from_slice(pixel);
                joined.push(opacity);
            }
            let color_type = if gray { png::ColorType::GrayscaleAlpha } else { png::ColorType::Rgba };
            (color_type, joined)
        }
        None => {
            let color_type = if gray { png::ColorType::Grayscale } else { png::ColorType::Rgb };
            (color_type, color)
        }
    };
    let file = File::create(output).map_err(|error| error.to_string())?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), width, height);
    encoder.set_color(color_type);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Default);
    let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
    writer.write_image_data(&data).map_err(|error| error.to_string())
}

fn crates_jpeg_to_pdf(input: &str, output: &str) -> Result<(), String> {
    let jpeg = read_all(input)?;
    let decoder = image::codecs::jpeg::JpegDecoder::new(std::io::Cursor::new(&jpeg)).map_err(|error| error.to_string())?;
    let (width, height) = image::ImageDecoder::dimensions(&decoder);
    let color_space = match image::ImageDecoder::color_type(&decoder) {
        image::ColorType::L8 => "DeviceGray",
        _ => "DeviceRGB",
    };
    let mut document = Document::with_version("1.5");
    let pages_id = document.new_object_id();
    let image = Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => width,
            "Height" => height,
            "ColorSpace" => color_space,
            "BitsPerComponent" => 8,
            "Filter" => "DCTDecode",
        },
        jpeg,
    );
    let image_id = document.add_object(image);
    let content = format!("q {width} 0 0 {height} 0 0 cm /Im0 Do Q");
    let content_id = document.add_object(Stream::new(Dictionary::new(), content.into_bytes()));
    let page_id = document.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "MediaBox" => vec![0.into(), 0.into(), width.into(), height.into()],
        "Resources" => dictionary! { "XObject" => dictionary! { "Im0" => image_id } },
        "Contents" => content_id,
    });
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        }),
    );
    let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    document.trailer.set("Root", catalog_id);
    let mut file = BufWriter::new(File::create(output).map_err(|error| error.to_string())?);
    document.save_to(&mut file).map_err(|error| error.to_string())?;
    file.flush().map_err(|error| error.to_string())
}

fn crates_pdf_to_jpeg(input: &str, output: &str) -> Result<(), String> {
    let mut document = Document::load_mem(&read_all(input)?).map_err(|error| error.to_string())?;
    let stream = largest_image(&mut document)?;
    std::fs::write(output, stream.content).map_err(|error| error.to_string())
}
