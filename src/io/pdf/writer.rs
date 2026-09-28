//! Writes PDF a page at a time: objects go out in file order with their
//! offsets recorded, a stream's length is an object written after the
//! stream (so no stream is held), and the page tree, catalog, and cross
//! reference table close the file. A page is one image at its own size
//! (72 points per inch over the resolution the image records, a pixel a
//! point when it records none).

use std::io::{self, Write};

use crate::image::ColorType;
use crate::io::pdf::PdfError;
use crate::io::png::RowSink;
use crate::io::png::writer::FilteredZlib;

/// The catalog and the page tree have fixed numbers; the page tree is
/// written last, when its pages are known.
const CATALOG: u32 = 1;
const PAGES: u32 = 2;

/// A PDF being written to `sink`.
pub struct PdfDocument<'a> {
    sink: &'a mut dyn Write,
    written: u64,
    /// The byte offset of each object, by number (index 0 unused).
    offsets: Vec<u64>,
    pages: Vec<u32>,
}

impl<'a> PdfDocument<'a> {
    /// Starts a document: the header and its binary marker line.
    pub fn new(sink: &'a mut dyn Write) -> io::Result<PdfDocument<'a>> {
        let mut document = PdfDocument {
            sink,
            written: 0,
            offsets: vec![0, 0, 0],
            pages: Vec::new(),
        };
        document.put(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")?;
        Ok(document)
    }

    fn put(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.sink.write_all(bytes)?;
        self.written += bytes.len() as u64;
        Ok(())
    }

    fn allocate(&mut self) -> u32 {
        self.offsets.push(0);
        (self.offsets.len() - 1) as u32
    }

    fn begin(&mut self, number: u32) -> io::Result<()> {
        self.offsets[number as usize] = self.written;
        self.put(format!("{number} 0 obj\n").as_bytes())
    }

    /// A whole object: its dictionary and, when given, its stream.
    fn object(&mut self, number: u32, dictionary: &str, stream: Option<&[u8]>) -> io::Result<()> {
        self.begin(number)?;
        self.put(dictionary.as_bytes())?;
        if let Some(stream) = stream {
            self.put(b"\nstream\n")?;
            self.put(stream)?;
            self.put(b"\nendstream")?;
        }
        self.put(b"\nendobj\n")
    }

    /// The page object and its content for an image object already
    /// written: the image fills a page of its own size.
    fn page(
        &mut self,
        image: u32,
        (width, height): (u32, u32),
        density: Option<(f64, f64)>,
    ) -> io::Result<()> {
        let content_number = self.allocate();
        let page_number = self.allocate();
        let (width, height) = match density {
            Some((across, down)) => (
                points(f64::from(width) * 72.0 / across),
                points(f64::from(height) * 72.0 / down),
            ),
            None => (width.to_string(), height.to_string()),
        };
        let content = format!("q {width} 0 0 {height} 0 0 cm /Im0 Do Q");
        self.object(
            content_number,
            &format!("<< /Length {} >>", content.len()),
            Some(content.as_bytes()),
        )?;
        self.object(
            page_number,
            &format!(
                "<< /Type /Page /Parent {PAGES} 0 R /MediaBox [0 0 {width} {height}] /Resources << /XObject << /Im0 {image} 0 R >> >> /Contents {content_number} 0 R >>"
            ),
            None,
        )?;
        self.pages.push(page_number);
        Ok(())
    }

    /// A page of pixels, filled as a `RowSink`.
    pub fn image_page(&mut self) -> PdfPage<'_, 'a> {
        PdfPage {
            document: self,
            state: None,
            density: None,
        }
    }

    /// A page of a JPEG, embedded unchanged (`/DCTDecode`).
    pub fn jpeg_page(&mut self, jpeg: &[u8]) -> Result<(), PdfError> {
        let info = jpeg_info(jpeg)?;
        let color_space = match info.components {
            1 => "/DeviceGray",
            3 => "/DeviceRGB",
            _ => "/DeviceCMYK",
        };
        // Adobe's CMYK JPEGs store their inks inverted.
        let decode = if info.components == 4 && info.adobe {
            " /Decode [1 0 1 0 1 0 1 0]"
        } else {
            ""
        };
        let image = self.allocate();
        let io = |error: io::Error| PdfError(format!("writing the PDF: {error}"));
        self.object(
            image,
            &format!(
                "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace {color_space} /BitsPerComponent 8 /Filter /DCTDecode{decode} /Length {} >>",
                info.width,
                info.height,
                jpeg.len()
            ),
            Some(jpeg),
        )
        .map_err(io)?;
        self.page(image, (info.width, info.height), info.density)
            .map_err(io)
    }

    /// Closes the document: the page tree, the catalog, the cross
    /// reference table, and the trailer.
    pub fn finish(mut self) -> io::Result<()> {
        let kids: Vec<String> = self
            .pages
            .iter()
            .map(|page| format!("{page} 0 R"))
            .collect();
        let tree = format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            kids.join(" "),
            self.pages.len()
        );
        self.object(PAGES, &tree, None)?;
        self.object(
            CATALOG,
            &format!("<< /Type /Catalog /Pages {PAGES} 0 R >>"),
            None,
        )?;
        let table_at = self.written;
        let count = self.offsets.len();
        let mut table = format!("xref\n0 {count}\n0000000000 65535 f \n");
        for offset in &self.offsets[1..] {
            table.push_str(&format!("{offset:010} 00000 n \n"));
        }
        table.push_str(&format!(
            "trailer\n<< /Size {count} /Root {CATALOG} 0 R >>\nstartxref\n{table_at}\n%%EOF\n"
        ));
        self.put(table.as_bytes())?;
        self.sink.flush()
    }

    /// The number of pages written so far.
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }
}

/// What a page needs while its rows stream.
struct Streaming {
    width: u32,
    height: u32,
    color: ColorType,
    image: u32,
    length: u32,
    written: u64,
    stream: FilteredZlib,
    /// The alpha channel's own stream, kept in memory (a soft mask is
    /// its own object, after the color stream).
    alpha: Option<(FilteredZlib, Vec<u8>)>,
    color_row: Vec<u8>,
    alpha_row: Vec<u8>,
    rows_left: u32,
}

/// One page of pixels, filled as a `RowSink`.
pub struct PdfPage<'d, 'a> {
    document: &'d mut PdfDocument<'a>,
    state: Option<Streaming>,
    /// Pixels per inch the image records, when it records them.
    density: Option<(f64, f64)>,
}

impl PdfPage<'_, '_> {
    fn finish(&mut self) -> io::Result<()> {
        let Some(mut state) = self.state.take() else {
            return Ok(());
        };
        let tail = state.stream.finish().to_vec();
        self.document.put(&tail)?;
        state.written += tail.len() as u64;
        self.document.put(b"\nendstream\nendobj\n")?;
        self.document
            .object(state.length, &state.written.to_string(), None)?;
        if let Some((mut alpha, mut bytes)) = state.alpha.take() {
            bytes.extend_from_slice(alpha.finish());
            let mask = self.smask_number(state.image);
            self.document.object(
                mask,
                &format!(
                    "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode /DecodeParms << /Predictor 15 /Colors 1 /BitsPerComponent 8 /Columns {} >> /Length {} >>",
                    state.width,
                    state.height,
                    state.width,
                    bytes.len()
                ),
                Some(&bytes),
            )?;
        }
        self.document
            .page(state.image, (state.width, state.height), self.density)
    }

    /// The soft mask's number: allocated right after the image's length.
    fn smask_number(&self, image: u32) -> u32 {
        image + 2
    }
}

impl RowSink for PdfPage<'_, '_> {
    fn density(&mut self, across: f64, down: f64) {
        // A resolution far outside what scanners and screens write
        // (1 to 10,000 per inch) is a broken header, not a page size.
        let plausible = |value: f64| (1.0..=10_000.0).contains(&value);
        if plausible(across) && plausible(down) {
            self.density = Some((across, down));
        }
    }

    fn start(&mut self, width: u32, height: u32, color: ColorType) -> io::Result<()> {
        let (colors, space, alpha) = match color {
            ColorType::Gray => (1, "/DeviceGray", false),
            ColorType::GrayAlpha => (1, "/DeviceGray", true),
            ColorType::Rgb => (3, "/DeviceRGB", false),
            ColorType::Rgba => (3, "/DeviceRGB", true),
        };
        let image = self.document.allocate();
        let length = self.document.allocate();
        let mask = if alpha {
            let number = self.document.allocate();
            format!(" /SMask {number} 0 R")
        } else {
            String::new()
        };
        self.document.begin(image)?;
        self.document.put(
            format!(
                "<< /Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace {space} /BitsPerComponent 8 /Filter /FlateDecode /DecodeParms << /Predictor 15 /Colors {colors} /BitsPerComponent 8 /Columns {width} >> /Length {length} 0 R{mask} >>\nstream\n"
            )
            .as_bytes(),
        )?;
        let width_usize = width as usize;
        self.state = Some(Streaming {
            width,
            height,
            color,
            image,
            length,
            written: 0,
            stream: FilteredZlib::new(width_usize * colors, colors),
            alpha: alpha.then(|| (FilteredZlib::new(width_usize, 1), Vec::new())),
            color_row: vec![0; width_usize * colors],
            alpha_row: vec![0; width_usize],
            rows_left: height,
        });
        if height == 0 {
            self.finish()?;
        }
        Ok(())
    }

    fn row(&mut self, pixels: &[u8]) -> io::Result<()> {
        let Some(state) = self.state.as_mut() else {
            return Ok(());
        };
        state.rows_left -= 1;
        let color_row: &[u8] = match state.color {
            ColorType::Gray | ColorType::Rgb => pixels,
            ColorType::GrayAlpha | ColorType::Rgba => {
                let channels = state.color.channels();
                let colors = channels - 1;
                for ((target, alpha), cell) in state
                    .color_row
                    .chunks_exact_mut(colors)
                    .zip(state.alpha_row.iter_mut())
                    .zip(pixels.chunks_exact(channels))
                {
                    target.copy_from_slice(&cell[..colors]);
                    *alpha = cell[colors];
                }
                if let Some((stream, bytes)) = state.alpha.as_mut() {
                    stream.row(&state.alpha_row);
                    if let Some(part) = stream.take_part() {
                        bytes.extend_from_slice(part);
                    }
                }
                &state.color_row
            }
        };
        state.stream.row(color_row);
        let last = state.rows_left == 0;
        if !last {
            if let Some(part) = state.stream.take_part() {
                let part = part.to_vec();
                state.written += part.len() as u64;
                self.document.put(&part)?;
            }
        }
        if last {
            self.finish()?;
        }
        Ok(())
    }
}

/// What a PDF needs from a JPEG's header.
pub struct JpegInfo {
    pub width: u32,
    pub height: u32,
    pub components: u8,
    /// An Adobe APP14 segment is present (its CMYK is stored inverted).
    pub adobe: bool,
    /// Pixels per inch from the JFIF header, when it gives them.
    pub density: Option<(f64, f64)>,
}

/// A page dimension in points, to a hundredth, without trailing zeros.
fn points(value: f64) -> String {
    let text = format!("{value:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Reads a JPEG's frame header (and whether it carries Adobe's marker).
pub fn jpeg_info(jpeg: &[u8]) -> Result<JpegInfo, PdfError> {
    let fail = |message: &str| PdfError(message.to_string());
    if !jpeg.starts_with(&[0xff, 0xd8]) {
        return Err(fail("not a JPEG"));
    }
    let mut at = 2;
    let mut adobe = false;
    let mut density = None;
    loop {
        while at < jpeg.len() && jpeg[at] != 0xff {
            at += 1;
        }
        while at < jpeg.len() && jpeg[at] == 0xff {
            at += 1;
        }
        let Some(&marker) = jpeg.get(at) else {
            return Err(fail("JPEG has no frame header"));
        };
        at += 1;
        if matches!(marker, 0xd8 | 0x01 | 0xd0..=0xd7) {
            continue;
        }
        let length = jpeg
            .get(at..at + 2)
            .map(|pair| usize::from(u16::from_be_bytes([pair[0], pair[1]])))
            .ok_or_else(|| fail("JPEG cut short"))?;
        let segment = jpeg
            .get(at + 2..at + length)
            .ok_or_else(|| fail("JPEG cut short"))?;
        match marker {
            0xee if segment.starts_with(b"Adobe") => adobe = true,
            // JFIF: units (1 per inch, 2 per centimetre), then densities.
            0xe0 if segment.starts_with(b"JFIF\0") && segment.len() >= 12 => {
                let across = f64::from(u16::from_be_bytes([segment[8], segment[9]]));
                let down = f64::from(u16::from_be_bytes([segment[10], segment[11]]));
                let scale = match segment[7] {
                    1 => Some(1.0),
                    2 => Some(2.54),
                    _ => None,
                };
                let plausible = |value: f64| (1.0..=10_000.0).contains(&value);
                if let Some(scale) = scale {
                    density = Some((across * scale, down * scale))
                        .filter(|(x, y)| plausible(*x) && plausible(*y));
                }
            }
            0xc0..=0xcf if !matches!(marker, 0xc4 | 0xc8 | 0xcc) => {
                if segment.len() < 6 {
                    return Err(fail("JPEG frame header cut short"));
                }
                let height = u32::from(u16::from_be_bytes([segment[1], segment[2]]));
                let width = u32::from(u16::from_be_bytes([segment[3], segment[4]]));
                let components = segment[5];
                if width == 0 || height == 0 || !matches!(components, 1 | 3 | 4) {
                    return Err(fail("JPEG frame is not one a PDF can hold"));
                }
                return Ok(JpegInfo {
                    width,
                    height,
                    components,
                    adobe,
                    density,
                });
            }
            0xda | 0xd9 => return Err(fail("JPEG has no frame header")),
            _ => {}
        }
        at += length;
    }
}
