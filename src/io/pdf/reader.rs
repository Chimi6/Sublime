//! Reads a page's image out of a PDF: the largest image the page draws
//! (an image object in its resources, one inside a form it uses, or an
//! image inline in its content), decoded into the image hub. Vector
//! pages are not rendered.

use std::io::Read;

use crate::image::{ColorType, Image};
use crate::io::jpeg::read_jpeg;
use crate::io::pdf::PdfError;
use crate::io::pdf::document::{Document, find};
use crate::io::pdf::filter;
use crate::io::pdf::object::{Dictionary, Object, Parser};
use crate::io::png::{PngError, RowSink, RowsError};

fn fail(message: impl Into<String>) -> PdfError {
    PdfError(message.into())
}

/// What reading left out.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PdfNotes {
    pub pages: usize,
    pub page: usize,
    /// Other images on the chosen page, not read.
    pub other_images: usize,
    pub sixteen_bit: bool,
    pub cmyk: bool,
}

/// An image found on a page: its dictionary and its raw data.
struct Found {
    dictionary: Dictionary,
    raw: Vec<u8>,
    area: u64,
}

impl Found {
    fn new(dictionary: Dictionary, raw: Vec<u8>) -> Found {
        let side = |key: &[u8]| {
            dictionary
                .get(key)
                .and_then(Object::as_integer)
                .unwrap_or(0)
                .max(0) as u64
        };
        let area = side(b"Width").max(side(b"W")) * side(b"Height").max(side(b"H"));
        Found {
            dictionary,
            raw,
            area,
        }
    }
}

/// Every image the page's resources and content hold.
fn images(
    document: &Document<'_>,
    resources: Option<&Dictionary>,
    content: &[u8],
    depth: usize,
    out: &mut Vec<Found>,
) -> Result<(), PdfError> {
    if depth > 8 {
        return Ok(());
    }
    if let Some(objects) = resources.and_then(|resources| resources.get(b"XObject")) {
        if let Some(objects) = document.resolve(objects)?.as_dictionary() {
            for (_, reference) in &objects.0 {
                let Object::Stream(dictionary, range) = document.resolve(reference)? else {
                    continue;
                };
                match dictionary.get(b"Subtype").and_then(Object::as_name) {
                    Some(b"Image") => {
                        let raw = document.bytes()[range].to_vec();
                        out.push(Found::new(dictionary, raw));
                    }
                    Some(b"Form") => {
                        let form_resources = match dictionary.get(b"Resources") {
                            Some(inner) => document.resolve(inner)?.as_dictionary().cloned(),
                            None => resources.cloned(),
                        };
                        let content = filter::decode(&dictionary, &document.bytes()[range])
                            .unwrap_or_default();
                        images(document, form_resources.as_ref(), &content, depth + 1, out)?;
                    }
                    _ => {}
                }
            }
        }
    }
    inline_images(content, out);
    Ok(())
}

/// Images inline in a content stream: `BI` keys and values `ID` data `EI`.
fn inline_images(content: &[u8], out: &mut Vec<Found>) {
    let mut at = 0;
    while let Some(found) = find(content, b"BI", at) {
        at = found + 2;
        let starts = found == 0 || content[found - 1].is_ascii_whitespace();
        let ends = content
            .get(found + 2)
            .is_none_or(|byte| byte.is_ascii_whitespace());
        if !starts || !ends {
            continue;
        }
        let mut parser = Parser::new(content, found + 2);
        let mut entries = Vec::new();
        loop {
            if parser.keyword(b"ID") {
                break;
            }
            let Ok(Object::Name(key)) = parser.object() else {
                return;
            };
            let Ok(value) = parser.object() else {
                return;
            };
            // Inline images abbreviate their keys and names.
            let key = match key.as_slice() {
                b"W" => b"Width".to_vec(),
                b"H" => b"Height".to_vec(),
                b"BPC" => b"BitsPerComponent".to_vec(),
                b"CS" => b"ColorSpace".to_vec(),
                b"D" => b"Decode".to_vec(),
                b"IM" => b"ImageMask".to_vec(),
                b"F" => b"Filter".to_vec(),
                b"DP" => b"DecodeParms".to_vec(),
                other => other.to_vec(),
            };
            let value = match value {
                Object::Name(name) => Object::Name(match name.as_slice() {
                    b"G" => b"DeviceGray".to_vec(),
                    b"RGB" => b"DeviceRGB".to_vec(),
                    b"CMYK" => b"DeviceCMYK".to_vec(),
                    b"I" => b"Indexed".to_vec(),
                    b"AHx" => b"ASCIIHexDecode".to_vec(),
                    b"A85" => b"ASCII85Decode".to_vec(),
                    b"Fl" => b"FlateDecode".to_vec(),
                    b"RL" => b"RunLengthDecode".to_vec(),
                    b"DCT" => b"DCTDecode".to_vec(),
                    _ => name,
                }),
                other => other,
            };
            entries.push((key, value));
        }
        let start = parser.at + 1;
        let Some(end) = find_inline_end(content, start) else {
            return;
        };
        at = end + 2;
        out.push(Found::new(
            Dictionary(entries),
            content[start..end].to_vec(),
        ));
    }
}

/// The end of inline image data: `EI` between whitespace.
fn find_inline_end(content: &[u8], from: usize) -> Option<usize> {
    let mut at = from;
    while let Some(found) = find(content, b"EI", at) {
        let before = found > from && content[found - 1].is_ascii_whitespace();
        let after = content
            .get(found + 2)
            .is_none_or(|byte| byte.is_ascii_whitespace());
        if before && after {
            let mut end = found - 1;
            while end > from && content[end - 1].is_ascii_whitespace() {
                end -= 1;
            }
            return Some(end.max(from));
        }
        at = found + 2;
    }
    None
}

/// A page's content streams, decoded and joined.
fn page_content(document: &Document<'_>, page: &Dictionary) -> Vec<u8> {
    let Some(contents) = page.get(b"Contents") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let streams: Vec<Object> = match document.resolve(contents) {
        Ok(Object::Array(items)) => items,
        Ok(stream @ Object::Stream(..)) => vec![stream],
        _ => Vec::new(),
    };
    for stream in &streams {
        if let Ok((_, data)) = document.stream_data(stream) {
            out.extend_from_slice(&data);
            out.push(b'\n');
        }
    }
    out
}

/// The components and a way to turn samples into the hub's pixels.
enum Space {
    Gray,
    Rgb,
    Cmyk,
    /// A palette: the base's components, and its colors as bytes.
    Indexed {
        base: Box<Space>,
        palette: Vec<u8>,
    },
}

impl Space {
    fn components(&self) -> usize {
        match self {
            Space::Gray => 1,
            Space::Rgb => 3,
            Space::Cmyk => 4,
            Space::Indexed { .. } => 1,
        }
    }
}

fn space(document: &Document<'_>, object: &Object) -> Result<Space, PdfError> {
    let object = document.resolve(object)?;
    let name = match &object {
        Object::Name(name) => name.clone(),
        Object::Array(items) => items
            .first()
            .and_then(Object::as_name)
            .map(<[u8]>::to_vec)
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    match name.as_slice() {
        b"DeviceGray" | b"CalGray" | b"G" => Ok(Space::Gray),
        b"DeviceRGB" | b"CalRGB" | b"RGB" => Ok(Space::Rgb),
        b"DeviceCMYK" | b"CMYK" => Ok(Space::Cmyk),
        b"ICCBased" => {
            let stream = object
                .as_array()
                .and_then(|items| items.get(1))
                .ok_or_else(|| fail("bad ICCBased color space"))?;
            let components = document
                .resolve(stream)?
                .as_dictionary()
                .and_then(|d| d.get(b"N"))
                .and_then(Object::as_integer)
                .unwrap_or(3);
            match components {
                1 => Ok(Space::Gray),
                4 => Ok(Space::Cmyk),
                _ => Ok(Space::Rgb),
            }
        }
        b"Indexed" | b"I" => {
            let items = object
                .as_array()
                .ok_or_else(|| fail("bad Indexed color space"))?;
            let base = space(
                document,
                items
                    .get(1)
                    .ok_or_else(|| fail("bad Indexed color space"))?,
            )?;
            let lookup = items
                .get(3)
                .ok_or_else(|| fail("bad Indexed color space"))?;
            let palette = match document.resolve(lookup)? {
                Object::String(bytes) => bytes,
                stream @ Object::Stream(..) => document.stream_data(&stream)?.1,
                _ => return Err(fail("bad Indexed palette")),
            };
            Ok(Space::Indexed {
                base: Box::new(base),
                palette,
            })
        }
        other => Err(fail(format!(
            "the PDF color space /{} is not supported",
            String::from_utf8_lossy(other)
        ))),
    }
}

/// Decodes one found image into the hub.
fn decode(document: &Document<'_>, found: &Found, notes: &mut PdfNotes) -> Result<Image, PdfError> {
    let dictionary = &found.dictionary;
    let number = |key: &[u8]| dictionary.get(key).and_then(Object::as_integer);
    let width = number(b"Width").unwrap_or(0).max(0) as usize;
    let height = number(b"Height").unwrap_or(0).max(0) as usize;
    if width == 0 || height == 0 {
        return Err(fail("PDF image has no size"));
    }
    if filter::ends_in_dct(dictionary) {
        let jpeg = filter::decode(dictionary, &found.raw)?;
        let (image, _) =
            read_jpeg(&jpeg).map_err(|failure| fail(format!("PDF JPEG image: {}", failure.0)))?;
        return with_mask(document, dictionary, image);
    }
    let data = filter::decode(dictionary, &found.raw)?;
    let mask = matches!(dictionary.get(b"ImageMask"), Some(Object::Bool(true)));
    let bits = if mask {
        1
    } else {
        number(b"BitsPerComponent").unwrap_or(8) as usize
    };
    if !matches!(bits, 1 | 2 | 4 | 8 | 16) {
        return Err(fail(format!("PDF images of {bits} bits are not supported")));
    }
    notes.sixteen_bit |= bits == 16;
    let space = if mask {
        Space::Gray
    } else {
        space(
            document,
            dictionary
                .get(b"ColorSpace")
                .ok_or_else(|| fail("PDF image has no color space"))?,
        )?
    };
    notes.cmyk |= matches!(space, Space::Cmyk)
        || matches!(&space, Space::Indexed { base, .. } if matches!(**base, Space::Cmyk));
    let components = space.components();
    let stride = (width * components * bits).div_ceil(8);
    if data.len() < stride * height {
        return Err(fail("PDF image data cut short"));
    }
    let levels = (1u32 << bits.min(16)) - 1;
    let decode: Vec<f64> = dictionary
        .get(b"Decode")
        .and_then(Object::as_array)
        .map(|items| items.iter().filter_map(Object::as_number).collect())
        .unwrap_or_default();
    let sample = |row: &[u8], index: usize| -> u32 {
        match bits {
            16 => u32::from(u16::from_be_bytes([row[index * 2], row[index * 2 + 1]])),
            8 => u32::from(row[index]),
            _ => {
                let bit = index * bits;
                u32::from(row[bit / 8] >> (8 - bits - bit % 8)) & levels
            }
        }
    };
    let color = match &space {
        Space::Gray => ColorType::Gray,
        _ => ColorType::Rgb,
    };
    let channels = color.channels();
    let mut pixels = vec![0u8; width * height * channels];
    for y in 0..height {
        let row = &data[y * stride..(y + 1) * stride];
        for x in 0..width {
            let target = &mut pixels[(y * width + x) * channels..(y * width + x + 1) * channels];
            // A component as 0..255, through the Decode array.
            let unit = |component: usize| -> u8 {
                let raw = sample(row, x * components + component);
                // Without a Decode range, 8 bits as is and 16 by the high
                // byte, as the PNG and TIFF readers scale.
                if decode.len() < (component + 1) * 2 {
                    match bits {
                        8 => return raw as u8,
                        16 => return (raw >> 8) as u8,
                        _ => {}
                    }
                }
                let value = f64::from(raw) / f64::from(levels);
                let (low, high) = match decode.get(component * 2..component * 2 + 2) {
                    Some(pair) => (pair[0], pair[1]),
                    None => (0.0, 1.0),
                };
                ((low + value * (high - low)).clamp(0.0, 1.0) * 255.0).round() as u8
            };
            match &space {
                // A stencil mask paints (black) where its sample decodes
                // to 0, as gray does.
                Space::Gray => target[0] = unit(0),
                Space::Rgb => {
                    for (channel, value) in target.iter_mut().enumerate() {
                        *value = unit(channel);
                    }
                }
                Space::Cmyk => cmyk(&[unit(0), unit(1), unit(2), unit(3)], target),
                Space::Indexed { base, palette } => {
                    let index = sample(row, x) as usize;
                    let size = base.components();
                    let entry = palette.get(index * size..(index + 1) * size);
                    match (entry, base.as_ref()) {
                        (Some(entry), Space::Gray) => target.fill(entry[0]),
                        (Some(entry), Space::Cmyk) => cmyk(entry, target),
                        (Some(entry), _) => target.copy_from_slice(&entry[..3]),
                        (None, _) => target.fill(0),
                    }
                }
            }
        }
    }
    with_mask(
        document,
        dictionary,
        Image {
            width: width as u32,
            height: height as u32,
            color,
            pixels,
        },
    )
}

/// CMYK to RGB as Pillow converts it: round((255 - c) * (255 - k) / 255).
fn cmyk(inks: &[u8], target: &mut [u8]) {
    let black = 255 - u32::from(inks[3]);
    for (channel, value) in target.iter_mut().take(3).enumerate() {
        *value = (((255 - u32::from(inks[channel])) * black + 127) / 255) as u8;
    }
}

/// Adds a soft mask as alpha when the image has one of its own size.
fn with_mask(
    document: &Document<'_>,
    dictionary: &Dictionary,
    image: Image,
) -> Result<Image, PdfError> {
    let Some(reference) = dictionary.get(b"SMask") else {
        return Ok(image);
    };
    let Object::Stream(mask_dictionary, range) = document.resolve(reference)? else {
        return Ok(image);
    };
    let mut notes = PdfNotes::default();
    let found = Found::new(mask_dictionary, document.bytes()[range].to_vec());
    let mask = decode(document, &found, &mut notes)?;
    if (mask.width, mask.height) != (image.width, image.height) || mask.color != ColorType::Gray {
        return Ok(image);
    }
    let channels = image.color.channels();
    let color = match image.color {
        ColorType::Gray | ColorType::GrayAlpha => ColorType::GrayAlpha,
        _ => ColorType::Rgba,
    };
    let colors = channels.min(color.channels() - 1);
    let mut pixels = Vec::with_capacity(image.pixels.len() / channels * color.channels());
    for (cell, alpha) in image.pixels.chunks_exact(channels).zip(&mask.pixels) {
        pixels.extend_from_slice(&cell[..colors]);
        pixels.push(*alpha);
    }
    Ok(Image {
        width: image.width,
        height: image.height,
        color,
        pixels,
    })
}

/// The chosen page's largest image, as found (not yet decoded).
fn choose(document: &Document<'_>, page: Option<u32>) -> Result<(Found, PdfNotes), PdfError> {
    let pages = document.pages()?;
    if pages.is_empty() {
        return Err(fail("the PDF has no pages"));
    }
    let wanted = page.unwrap_or(1) as usize;
    if wanted == 0 || wanted > pages.len() {
        return Err(fail(format!(
            "the PDF has {} pages; there is no page {wanted}",
            pages.len()
        )));
    }
    let chosen = &pages[wanted - 1];
    let content = page_content(document, &chosen.dictionary);
    let mut found = Vec::new();
    images(document, chosen.resources.as_ref(), &content, 0, &mut found)?;
    let count = found.len();
    let best = found
        .into_iter()
        .enumerate()
        .max_by_key(|(index, image)| (image.area, std::cmp::Reverse(*index)))
        .map(|(_, image)| image)
        .ok_or_else(|| {
            fail(format!(
                "page {wanted} holds no image (vector pages are not rendered)"
            ))
        })?;
    Ok((
        best,
        PdfNotes {
            pages: pages.len(),
            page: wanted,
            other_images: count - 1,
            ..PdfNotes::default()
        },
    ))
}

/// The chosen page's image into `sink`.
pub fn read_pdf_rows(
    reader: &mut dyn Read,
    sink: &mut dyn RowSink,
    page: Option<u32>,
) -> Result<PdfNotes, RowsError> {
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).map_err(RowsError::Io)?;
    let to_rows = |failure: PdfError| RowsError::Png(PngError(failure.0));
    let document = Document::open(&bytes).map_err(to_rows)?;
    let (found, mut notes) = choose(&document, page).map_err(to_rows)?;
    let image = decode(&document, &found, &mut notes).map_err(to_rows)?;
    sink.start(image.width, image.height, image.color)
        .map_err(RowsError::Io)?;
    for y in 0..image.height {
        sink.row(image.row(y)).map_err(RowsError::Io)?;
    }
    Ok(notes)
}

/// The chosen page's image as a JPEG's own bytes, when it is a plain
/// DCT image (no mask): PDF to JPEG copies it rather than re-encoding.
pub fn page_jpeg(bytes: &[u8], page: Option<u32>) -> Result<Option<Vec<u8>>, PdfError> {
    let document = Document::open(bytes)?;
    let (found, _) = choose(&document, page)?;
    let dictionary = &found.dictionary;
    let plain = filter::filters(dictionary).len() == 1
        && filter::ends_in_dct(dictionary)
        && dictionary.get(b"SMask").is_none()
        && dictionary.get(b"Decode").is_none();
    if !plain {
        return Ok(None);
    }
    Ok(Some(found.raw))
}

/// A page's image as a whole image.
pub fn read_pdf(bytes: &[u8], page: Option<u32>) -> Result<(Image, PdfNotes), PdfError> {
    let document = Document::open(bytes)?;
    let (found, mut notes) = choose(&document, page)?;
    let image = decode(&document, &found, &mut notes)?;
    Ok((image, notes))
}
