//! Writes PDF a page at a time: objects go out in file order with their
//! offsets recorded, a stream's length is an object written after the
//! stream (so no stream is held), and the page tree, catalog, and cross
//! reference table close the file. A page is one image at its own size
//! (72 points per inch over the resolution the image records, a pixel a
//! point when it records none).

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};
use std::sync::Arc;

use crate::image::ColorType;
use crate::io::deflate::deflate;
use crate::io::font::{Font, FontError};
use crate::io::pdf::PdfError;
use crate::io::png::writer::FilteredZlib;
use crate::io::png::{RowSink, adler32};

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
    /// The standard fonts text pages have used, by their object number.
    fonts: Vec<(StandardFont, u32)>,
    /// The document's title, for its information dictionary.
    title: Option<String>,
    /// Fonts embedded for text the standard fonts cannot set.
    embedded: Vec<Embedded>,
}

/// An embedded font: its Type0 object number (reserved when first used,
/// written at the end, when every glyph a page used is known) and the
/// glyphs used with the text each stands for.
struct Embedded {
    number: u32,
    font: Arc<Font>,
    used: BTreeMap<u16, char>,
}

/// A base-14 font: every viewer has it, so nothing is embedded. Text in
/// it is WinAnsi bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StandardFont {
    Helvetica,
    HelveticaBold,
    HelveticaOblique,
    HelveticaBoldOblique,
    Courier,
    CourierBold,
}

impl StandardFont {
    pub fn base_name(self) -> &'static str {
        match self {
            StandardFont::Helvetica => "Helvetica",
            StandardFont::HelveticaBold => "Helvetica-Bold",
            StandardFont::HelveticaOblique => "Helvetica-Oblique",
            StandardFont::HelveticaBoldOblique => "Helvetica-BoldOblique",
            StandardFont::Courier => "Courier",
            StandardFont::CourierBold => "Courier-Bold",
        }
    }

    /// The resource name a page's content uses for it.
    pub fn resource(self) -> &'static str {
        match self {
            StandardFont::Helvetica => "F1",
            StandardFont::HelveticaBold => "F2",
            StandardFont::HelveticaOblique => "F3",
            StandardFont::HelveticaBoldOblique => "F4",
            StandardFont::Courier => "F5",
            StandardFont::CourierBold => "F6",
        }
    }
}

/// A page of text: its size in points, its content stream, the fonts
/// the content names, and its links as rectangles with their targets.
pub struct TextPage<'c> {
    pub width: f64,
    pub height: f64,
    pub content: &'c [u8],
    pub fonts: &'c [StandardFont],
    /// Embedded fonts the content names (`/E1`, `/E2`, ...), by index.
    pub embedded: &'c [usize],
    pub links: &'c [([f64; 4], String)],
}

impl<'a> PdfDocument<'a> {
    /// Starts a document: the header and its binary marker line.
    pub fn new(sink: &'a mut dyn Write) -> io::Result<PdfDocument<'a>> {
        let mut document = PdfDocument {
            sink,
            written: 0,
            offsets: vec![0, 0, 0],
            pages: Vec::new(),
            fonts: Vec::new(),
            title: None,
            embedded: Vec::new(),
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
        for index in 0..self.embedded.len() {
            self.write_embedded(index)?;
        }
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
        let info = match self.title.take() {
            Some(title) => {
                let number = self.allocate();
                self.object(
                    number,
                    &format!("<< /Title {} /Producer (Sublime) >>", text_string(&title)),
                    None,
                )?;
                format!(" /Info {number} 0 R")
            }
            None => String::new(),
        };
        let table_at = self.written;
        let count = self.offsets.len();
        let mut table = format!("xref\n0 {count}\n0000000000 65535 f \n");
        for offset in &self.offsets[1..] {
            table.push_str(&format!("{offset:010} 00000 n \n"));
        }
        table.push_str(&format!(
            "trailer\n<< /Size {count} /Root {CATALOG} 0 R{info} >>\nstartxref\n{table_at}\n%%EOF\n"
        ));
        self.put(table.as_bytes())?;
        self.sink.flush()
    }

    /// Embeds a font (once): the index its pages name it by, as `/E<n+1>`.
    pub fn embed(&mut self, font: Arc<Font>) -> usize {
        if let Some(index) = self
            .embedded
            .iter()
            .position(|known| Arc::ptr_eq(&known.font, &font))
        {
            return index;
        }
        let number = self.allocate();
        self.embedded.push(Embedded {
            number,
            font,
            used: BTreeMap::new(),
        });
        self.embedded.len() - 1
    }

    /// Records that a page set `glyph` of embedded font `index` for `text`.
    pub fn use_glyph(&mut self, index: usize, glyph: u16, text: char) {
        if let Some(embedded) = self.embedded.get_mut(index) {
            embedded.used.entry(glyph).or_insert(text);
        }
    }

    /// Writes an embedded font: the subset program, its descriptor, the
    /// CID font with the used glyphs' widths, the ToUnicode map, and the
    /// Type0 font at its reserved number.
    fn write_embedded(&mut self, index: usize) -> io::Result<()> {
        let font = self.embedded[index].font.clone();
        let used = self.embedded[index].used.clone();
        let number = self.embedded[index].number;
        let invalid = |error: FontError| io::Error::new(io::ErrorKind::InvalidData, error.0);
        let glyphs: BTreeSet<u16> = used.keys().copied().collect();
        let program = font.subset(&glyphs).map_err(invalid)?;
        let name = subset_name(&font.family, &glyphs);
        let mut compressed = vec![0x78, 0x9c];
        deflate(&program, &mut compressed);
        compressed.extend_from_slice(&adler32(&program).to_be_bytes());
        let file = self.allocate();
        self.object(
            file,
            &format!(
                "<< /Length {} /Length1 {} /Filter /FlateDecode >>",
                compressed.len(),
                program.len()
            ),
            Some(&compressed),
        )?;
        let scale = |units: i16| number_text(font.to_thousandths(f64::from(units)));
        let mut flags = 32;
        if font.fixed_pitch {
            flags |= 1;
        }
        if font.italic_angle != 0.0 {
            flags |= 64;
        }
        let descriptor = self.allocate();
        self.object(
            descriptor,
            &format!(
                "<< /Type /FontDescriptor /FontName /{name} /Flags {flags} /FontBBox [{} {} {} {}] /ItalicAngle {} /Ascent {} /Descent {} /CapHeight {} /StemV {} /FontFile2 {file} 0 R >>",
                scale(font.bbox[0]),
                scale(font.bbox[1]),
                scale(font.bbox[2]),
                scale(font.bbox[3]),
                number_text(font.italic_angle),
                scale(font.ascent),
                scale(font.descent),
                scale(font.cap_height),
                if font.weight >= 600 { 140 } else { 80 }
            ),
            None,
        )?;
        let mut widths = String::new();
        for glyph in &glyphs {
            let width = font.to_thousandths(f64::from(font.advance(*glyph)));
            widths.push_str(&format!("{glyph} [{}] ", number_text(width)));
        }
        let cid_font = self.allocate();
        self.object(
            cid_font,
            &format!(
                "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /{name} /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor {descriptor} 0 R /CIDToGIDMap /Identity /DW 1000 /W [{widths}] >>"
            ),
            None,
        )?;
        let mut cmap = String::from(
            "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
        );
        let entries: Vec<(&u16, &char)> = used.iter().collect();
        for chunk in entries.chunks(100) {
            cmap.push_str(&format!("{} beginbfchar\n", chunk.len()));
            for (glyph, text) in chunk {
                let mut units = [0u16; 2];
                let hex: String = text
                    .encode_utf16(&mut units)
                    .iter()
                    .map(|unit| format!("{unit:04X}"))
                    .collect();
                cmap.push_str(&format!("<{glyph:04X}> <{hex}>\n"));
            }
            cmap.push_str("endbfchar\n");
        }
        cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
        let to_unicode = self.allocate();
        self.object(
            to_unicode,
            &format!("<< /Length {} >>", cmap.len()),
            Some(cmap.as_bytes()),
        )?;
        self.object(
            number,
            &format!(
                "<< /Type /Font /Subtype /Type0 /BaseFont /{name} /Encoding /Identity-H /DescendantFonts [{cid_font} 0 R] /ToUnicode {to_unicode} 0 R >>"
            ),
            None,
        )
    }

    /// Sets the title the document's information dictionary carries.
    pub fn set_title(&mut self, title: &str) {
        self.title = Some(title.to_string());
    }

    /// A page of text, its content deflated.
    pub fn text_page(&mut self, page: &TextPage<'_>) -> io::Result<()> {
        let mut resources = String::new();
        for font in page.fonts {
            let number = match self.fonts.iter().find(|(known, _)| known == font) {
                Some((_, number)) => *number,
                None => {
                    let number = self.allocate();
                    self.object(
                        number,
                        &format!(
                            "<< /Type /Font /Subtype /Type1 /BaseFont /{} /Encoding /WinAnsiEncoding >>",
                            font.base_name()
                        ),
                        None,
                    )?;
                    self.fonts.push((*font, number));
                    number
                }
            };
            resources.push_str(&format!("/{} {number} 0 R ", font.resource()));
        }
        for index in page.embedded {
            if let Some(embedded) = self.embedded.get(*index) {
                resources.push_str(&format!("/E{} {} 0 R ", index + 1, embedded.number));
            }
        }
        let mut compressed = vec![0x78, 0x9c];
        deflate(page.content, &mut compressed);
        compressed.extend_from_slice(&adler32(page.content).to_be_bytes());
        let content = self.allocate();
        self.object(
            content,
            &format!("<< /Length {} /Filter /FlateDecode >>", compressed.len()),
            Some(&compressed),
        )?;
        let mut annotations = Vec::new();
        for (rect, target) in page.links {
            let number = self.allocate();
            self.object(
                number,
                &format!(
                    "<< /Type /Annot /Subtype /Link /Rect [{} {} {} {}] /Border [0 0 0] /A << /S /URI /URI {} >> >>",
                    number_text(rect[0]),
                    number_text(rect[1]),
                    number_text(rect[2]),
                    number_text(rect[3]),
                    literal(target.as_bytes())
                ),
                None,
            )?;
            annotations.push(format!("{number} 0 R"));
        }
        let annots = if annotations.is_empty() {
            String::new()
        } else {
            format!(" /Annots [{}]", annotations.join(" "))
        };
        let page_number = self.allocate();
        self.object(
            page_number,
            &format!(
                "<< /Type /Page /Parent {PAGES} 0 R /MediaBox [0 0 {} {}] /Resources << /Font << {resources}>> >> /Contents {content} 0 R{annots} >>",
                number_text(page.width),
                number_text(page.height)
            ),
            None,
        )?;
        self.pages.push(page_number);
        Ok(())
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

/// A subset font's name: six capital letters from the glyphs it holds,
/// a plus, and the family without spaces or delimiters.
fn subset_name(family: &str, glyphs: &BTreeSet<u16>) -> String {
    let mut hash: u32 = 2_166_136_261;
    for glyph in glyphs {
        for byte in glyph.to_be_bytes() {
            hash = (hash ^ u32::from(byte)).wrapping_mul(16_777_619);
        }
    }
    let mut tag = String::new();
    for _ in 0..6 {
        tag.push((b'A' + (hash % 26) as u8) as char);
        hash /= 26;
    }
    let mut family: String = family
        .chars()
        .filter(|char| char.is_ascii_alphanumeric())
        .collect();
    if family.is_empty() {
        family.push_str("Font");
    }
    format!("{tag}+{family}")
}

/// A number for a PDF dictionary: to a hundredth, without trailing zeros.
pub fn number_text(value: f64) -> String {
    points(value)
}

/// A literal string with its delimiters escaped and bytes outside
/// printable ASCII as octal.
pub fn literal(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() + 2);
    out.push('(');
    for &byte in bytes {
        match byte {
            b'(' | b')' | b'\\' => {
                out.push('\\');
                out.push(byte as char);
            }
            0x20..=0x7e => out.push(byte as char),
            _ => out.push_str(&format!("\\{byte:03o}")),
        }
    }
    out.push(')');
    out
}

/// A text string: PDFDocEncoding when ASCII, else UTF-16BE with its mark.
fn text_string(text: &str) -> String {
    if text.is_ascii() {
        return literal(text.as_bytes());
    }
    let mut bytes = vec![0xfe, 0xff];
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }
    literal(&bytes)
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
