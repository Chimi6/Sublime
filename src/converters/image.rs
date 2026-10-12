//! Every pair between the raster formats: one reader into the image hub,
//! one writer out of it. A pair is a `ImagePair` value naming its two
//! formats, its fidelity, and the reader's notes to report.

use std::io::{Read, Write};

use std::sync::OnceLock;

use crate::converter::{ConvertError, Converter, Fidelity, FidelityKind, Input, Location, Tier};
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::bmp::{BmpError, BmpRows, BmpRowsError, read_bmp_rows, read_bmp_rows_seekable};
use crate::io::heif::{HeifNotes, HeifRowsError, read_heif_rows, read_heif_rows_from};
use crate::io::ico::{IcoNotes, IcoRows, read_ico_rows};
use crate::io::jpeg::{DEFAULT_QUALITY, JpegError, JpegNotes, JpegRows, read_jpeg_rows};
use crate::io::netpbm::{Kind, NetpbmNotes, NetpbmRows, read_netpbm_rows};
use crate::io::pdf::{PdfDocument, PdfNotes, page_jpeg, read_pdf_rows};
use crate::io::png::{PngError, PngNotes, PngRows, RowSink, RowsError, read_png_rows};
use crate::io::qoi::{QoiRows, read_qoi_rows};
use crate::io::tga::{TgaRows, read_tga_rows};
use crate::io::tiff::{TiffNotes, TiffRows, read_tiff_rows};
use crate::io::webp::{WebpNotes, WebpRows, read_webp_rows};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Bmp,
    Jpeg,
    Webp,
    Qoi,
    Netpbm(Kind),
    Tga,
    Ico,
    Tiff,
    Pdf,
    Heic,
}

pub struct ImagePair {
    pub name: &'static str,
    pub from: &'static Format,
    pub to: &'static Format,
    pub read: ImageFormat,
    pub write: ImageFormat,
    pub fidelity: Fidelity,
}

impl Converter for ImagePair {
    fn name(&self) -> &'static str {
        self.name
    }

    fn from(&self) -> &'static Format {
        self.from
    }

    fn to(&self) -> &'static Format {
        self.to
    }

    /// Images set on a PDF page have no text, so no `--font`.
    fn options(&self) -> Vec<crate::format::Setting> {
        let mut options = self.from.read_options.to_vec();
        if self.to.id != "pdf" {
            options.extend_from_slice(self.to.write_options);
        }
        options
    }

    fn fidelity(&self) -> Fidelity {
        self.fidelity.clone()
    }

    fn tier(&self) -> Tier {
        Tier::Native
    }

    fn convert(
        &self,
        mut input: Input<'_>,
        output: &mut dyn Write,
        context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let quality = context.options.quality.unwrap_or(DEFAULT_QUALITY);
        let effort = context.options.effort.unwrap_or_default();
        // Every pair streams: the reader hands rows to the writer and no
        // image is held beyond what a format itself needs (a bottom-up
        // BMP's pixel data, a WebP's bitstream, an interlaced PNG).
        match self.write {
            ImageFormat::Png => {
                let mut rows = PngRows::new(output);
                read_rows(self.read, &mut input, &mut rows, self.name, context)
            }
            ImageFormat::Bmp => {
                let mut rows = BmpRows::new(output);
                read_rows(self.read, &mut input, &mut rows, self.name, context)
            }
            // A PDF page whose image is a plain JPEG gives its bytes.
            ImageFormat::Jpeg if self.read == ImageFormat::Pdf => {
                let mut pdf = Vec::new();
                input.read_to_end(&mut pdf)?;
                let embedded = page_jpeg(&pdf, context.options.page).map_err(|failure| {
                    ConvertError::Malformed {
                        location: Location::default(),
                        message: failure.0,
                    }
                })?;
                if let Some(jpeg) = embedded {
                    output.write_all(&jpeg)?;
                    return Ok(());
                }
                let mut rows = JpegRows::new(output, quality);
                let mut bytes = &pdf[..];
                let mut again = Input::Stream(&mut bytes);
                read_rows(self.read, &mut again, &mut rows, self.name, context)
            }
            ImageFormat::Jpeg => {
                let mut rows = JpegRows::new(output, quality);
                read_rows(self.read, &mut input, &mut rows, self.name, context)
            }
            ImageFormat::Webp => {
                let mut rows = WebpRows::new(output).with_effort(effort);
                read_rows(self.read, &mut input, &mut rows, self.name, context)
            }
            ImageFormat::Qoi => {
                let mut rows = QoiRows::new(output);
                read_rows(self.read, &mut input, &mut rows, self.name, context)
            }
            ImageFormat::Netpbm(kind) => {
                let mut rows = NetpbmRows::new(output, kind);
                read_rows(self.read, &mut input, &mut rows, self.name, context)
            }
            ImageFormat::Tga => {
                let mut rows = TgaRows::new(output);
                read_rows(self.read, &mut input, &mut rows, self.name, context)
            }
            ImageFormat::Ico => {
                let mut rows = IcoRows::new(output);
                read_rows(self.read, &mut input, &mut rows, self.name, context)
            }
            ImageFormat::Tiff => {
                let mut rows = TiffRows::new(output);
                read_rows(self.read, &mut input, &mut rows, self.name, context)
            }
            ImageFormat::Pdf => {
                let mut document = PdfDocument::new(output)?;
                if self.read == ImageFormat::Jpeg {
                    let mut jpeg = Vec::new();
                    input.read_to_end(&mut jpeg)?;
                    document
                        .jpeg_page(&jpeg)
                        .map_err(|failure| ConvertError::Malformed {
                            location: Location::default(),
                            message: failure.0,
                        })?;
                } else {
                    let mut page = document.image_page();
                    read_rows(self.read, &mut input, &mut page, self.name, context)?;
                }
                document.finish()?;
                Ok(())
            }
            ImageFormat::Heic => {
                let quality = context
                    .options
                    .quality
                    .unwrap_or(crate::io::heif::write::DEFAULT_QUALITY);
                let mut rows = crate::io::heif::write::HeicRows::new(output, quality)
                    .with_effort(context.options.effort.unwrap_or_default());
                read_rows(self.read, &mut input, &mut rows, self.name, context)
            }
        }
    }
}

fn rows_error(error: RowsError) -> ConvertError {
    match error {
        RowsError::Png(error) => error.into(),
        RowsError::Io(error) => error.into(),
    }
}

/// The reader for an image format, by id, when there is one.
pub fn reader_for(format_id: &str) -> Option<ImageFormat> {
    CODECS
        .iter()
        .find(|codec| codec.format.id == format_id && codec.read.is_some())
        .map(|codec| codec.kind)
}

/// Reads `format` from the input into `sink`, reporting what the reader
/// dropped under `name`.
pub fn read_image(
    format: ImageFormat,
    input: &mut Input<'_>,
    sink: &mut dyn RowSink,
    name: &'static str,
    context: &mut Context<'_>,
) -> Result<(), ConvertError> {
    read_rows(format, input, sink, name, context)
}

/// Reads `format` from the input into `sink`, reporting what the reader
/// dropped.
fn read_rows(
    format: ImageFormat,
    input: &mut Input<'_>,
    sink: &mut dyn RowSink,
    name: &'static str,
    context: &mut Context<'_>,
) -> Result<(), ConvertError> {
    match format {
        ImageFormat::Png => {
            let notes = read_png_rows(input, sink).map_err(rows_error)?;
            report_png_notes(notes, name, context);
        }
        ImageFormat::Bmp => match match input {
            Input::Rewindable(file) => read_bmp_rows_seekable(*file, sink),
            Input::Stream(stream) => read_bmp_rows(*stream, sink),
        } {
            Ok(()) => {}
            Err(BmpRowsError::Bmp(error)) => return Err(error.into()),
            Err(BmpRowsError::Io(error)) => return Err(error.into()),
        },
        ImageFormat::Jpeg => {
            let notes = read_jpeg_rows(input, sink).map_err(rows_error)?;
            report_jpeg_notes(notes, context);
        }
        ImageFormat::Webp => {
            let notes = read_webp_rows(input, sink).map_err(rows_error)?;
            report_webp_notes(notes, context);
        }
        ImageFormat::Qoi => read_qoi_rows(input, sink).map_err(rows_error)?,
        ImageFormat::Tga => read_tga_rows(input, sink).map_err(rows_error)?,
        ImageFormat::Ico => {
            let notes = read_ico_rows(input, sink).map_err(rows_error)?;
            report_ico_notes(notes, context);
        }
        ImageFormat::Pdf => {
            let notes = read_pdf_rows(input, sink, context.options.page).map_err(rows_error)?;
            report_pdf_notes(notes, name, context);
        }
        ImageFormat::Tiff => {
            let notes = read_tiff_rows(input, sink).map_err(rows_error)?;
            report_tiff_notes(notes, name, context);
        }
        ImageFormat::Netpbm(_) => {
            let notes = read_netpbm_rows(input, sink).map_err(rows_error)?;
            report_netpbm_notes(notes, name, context);
        }
        ImageFormat::Heic => {
            // The container's items sit anywhere in the file: a file is
            // read an item at a time, a stream whole.
            let read = match input {
                Input::Rewindable(reader) => read_heif_rows_from(&mut **reader, sink),
                Input::Stream(stream) => {
                    let mut file = Vec::new();
                    stream.read_to_end(&mut file)?;
                    read_heif_rows(&file, sink)
                }
            };
            let notes = read.map_err(|error| match error {
                HeifRowsError::Heif(error) => {
                    let unsupported = error.0.contains("not supported") || error.0.contains("only");
                    if unsupported {
                        ConvertError::Unsupported(error.to_string())
                    } else {
                        ConvertError::Malformed {
                            location: Location::default(),
                            message: error.to_string(),
                        }
                    }
                }
                HeifRowsError::Io(error) => error.into(),
            })?;
            report_heif_notes(notes, name, context);
        }
    }
    Ok(())
}

fn report_png_notes(notes: PngNotes, name: &'static str, context: &mut Context<'_>) {
    if notes.sixteen_bit {
        context.loss(
            name,
            Location::default(),
            "16-bit samples reduced to 8 bits",
        );
    }
    for chunk in notes.dropped_chunks {
        let kind = String::from_utf8_lossy(&chunk).to_string();
        context.warning(format!("{kind} chunk dropped (metadata is not carried)"));
    }
}

fn report_jpeg_notes(notes: JpegNotes, context: &mut Context<'_>) {
    for name in notes.dropped {
        context.warning(format!("{name} segment dropped (metadata is not carried)"));
    }
}

fn report_netpbm_notes(notes: NetpbmNotes, name: &'static str, context: &mut Context<'_>) {
    if notes.maxval > 255 && !notes.deep {
        context.loss(
            name,
            Location::default(),
            "16-bit samples reduced to 8 bits",
        );
    }
}

fn report_ico_notes(notes: IcoNotes, context: &mut Context<'_>) {
    if notes.others > 0 {
        context.warning(format!(
            "the largest image is kept and {} other sizes are dropped",
            notes.others
        ));
    }
    if notes.cursor {
        context.warning("the cursor's hotspot is dropped".to_string());
    }
}

fn report_tiff_notes(notes: TiffNotes, name: &'static str, context: &mut Context<'_>) {
    if notes.other_pages > 0 {
        context.warning(format!(
            "the first page is kept and {} more are dropped",
            notes.other_pages
        ));
    }
    if notes.sixteen_bit {
        context.loss(
            name,
            Location::default(),
            "16-bit samples reduced to 8 bits",
        );
    }
    if notes.cmyk {
        context.loss(
            name,
            Location::default(),
            "CMYK converted to RGB without a color profile",
        );
    }
}

fn report_pdf_notes(notes: PdfNotes, name: &'static str, context: &mut Context<'_>) {
    if notes.pages > 1 && context.options.page.is_none() {
        context.warning(format!(
            "page 1 of {} is read (choose another with --page)",
            notes.pages
        ));
    }
    if notes.other_images > 0 {
        context.warning(format!(
            "the page's largest image is kept and {} other images are dropped",
            notes.other_images
        ));
    }
    if notes.sixteen_bit {
        context.loss(
            name,
            Location::default(),
            "16-bit samples reduced to 8 bits",
        );
    }
    if notes.cmyk {
        context.loss(
            name,
            Location::default(),
            "CMYK converted to RGB without a color profile",
        );
    }
}

fn report_heif_notes(notes: HeifNotes, name: &'static str, context: &mut Context<'_>) {
    if let Some(depth) = notes.deep {
        context.loss(
            name,
            Location::default(),
            format!("{depth}-bit samples reduced to 8 bits"),
        );
    }
    if notes.profile_dropped {
        context.warning(
            "the color profile is dropped (the output format does not carry one here)".to_string(),
        );
    }
    if notes.exif_dropped {
        context.warning(
            "the Exif metadata is dropped (the output format does not carry it here)".to_string(),
        );
    }
    if notes.other_images > 0 {
        context.warning(format!(
            "the primary image is read and {} other images are dropped",
            notes.other_images
        ));
    }
}

fn report_webp_notes(notes: WebpNotes, context: &mut Context<'_>) {
    for name in notes.dropped {
        context.warning(format!("{name} chunk dropped (metadata is not carried)"));
    }
    if notes.frames_dropped > 0 {
        context.warning(format!(
            "an animation: the first frame is kept and {} more are dropped",
            notes.frames_dropped
        ));
    }
}

impl From<JpegError> for ConvertError {
    fn from(error: JpegError) -> Self {
        let unsupported = error.0.contains("not supported");
        if unsupported {
            ConvertError::Unsupported(error.0)
        } else {
            ConvertError::Malformed {
                location: Location::default(),
                message: error.0,
            }
        }
    }
}

impl From<PngError> for ConvertError {
    fn from(error: PngError) -> Self {
        ConvertError::Malformed {
            location: Location::default(),
            message: error.0,
        }
    }
}

impl From<BmpError> for ConvertError {
    fn from(error: BmpError) -> Self {
        let unsupported = error.0.contains("not supported");
        if unsupported {
            ConvertError::Unsupported(error.0)
        } else {
            ConvertError::Malformed {
                location: Location::default(),
                message: error.0,
            }
        }
    }
}

/// One raster format: what its reader and its writer lose. Every pair
/// between two codecs is generated from this table, its fidelity the
/// worse of the reader's and the writer's with both texts.
struct Codec {
    format: &'static Format,
    kind: ImageFormat,
    /// What the reader loses; `None` for a format only written.
    read: Option<Fidelity>,
    /// What the writer loses; `None` for a format only read.
    write: Option<Fidelity>,
}

const JPEG_LOSS: &str = "JPEG is lossy: the image is re-encoded at the quality given (85 by default, 4:2:0 chroma below 90) and alpha is flattened onto white";

static CODECS: [Codec; 15] = [
    Codec {
        format: &formats::PNG,
        kind: ImageFormat::Png,
        read: Some(Fidelity::Conditional(
            "16-bit samples stay 16-bit into PNG, TIFF, and Netpbm and become 8-bit elsewhere (and from interlaced files); metadata (gamma, color profile, text) is dropped",
        )),
        write: Some(Fidelity::Lossless),
    },
    Codec {
        format: &formats::BMP,
        kind: ImageFormat::Bmp,
        read: Some(Fidelity::Lossless),
        write: Some(Fidelity::Lossless),
    },
    Codec {
        format: &formats::JPEG,
        kind: ImageFormat::Jpeg,
        read: Some(Fidelity::Conditional(
            "pixels as decoded, turned upright by their Exif orientation; the color profile and Exif are carried into PNG and JPEG and dropped elsewhere, as are XMP and comments",
        )),
        write: Some(Fidelity::Lossy(JPEG_LOSS)),
    },
    Codec {
        format: &formats::WEBP,
        kind: ImageFormat::Webp,
        read: Some(Fidelity::Conditional(
            "pixels as decoded (a lossy WebP decodes exactly as libwebp does); an animation keeps its first frame; metadata (ICC, Exif, XMP) is dropped",
        )),
        write: Some(Fidelity::Lossless),
    },
    Codec {
        format: &formats::QOI,
        kind: ImageFormat::Qoi,
        read: Some(Fidelity::Lossless),
        write: Some(Fidelity::Lossless),
    },
    Codec {
        format: &formats::PBM,
        kind: ImageFormat::Netpbm(Kind::Pbm),
        read: Some(Fidelity::Lossless),
        write: Some(Fidelity::Lossy(
            "black and white: color becomes luma, alpha is flattened onto white, and gray below half is black",
        )),
    },
    Codec {
        format: &formats::PGM,
        kind: ImageFormat::Netpbm(Kind::Pgm),
        read: Some(Fidelity::Conditional(NETPBM_READ)),
        write: Some(Fidelity::Conditional(
            "color becomes luma and alpha is flattened onto white",
        )),
    },
    Codec {
        format: &formats::PPM,
        kind: ImageFormat::Netpbm(Kind::Ppm),
        read: Some(Fidelity::Conditional(NETPBM_READ)),
        write: Some(Fidelity::Conditional("alpha is flattened onto white")),
    },
    Codec {
        format: &formats::PAM,
        kind: ImageFormat::Netpbm(Kind::Pam),
        read: Some(Fidelity::Conditional(NETPBM_READ)),
        write: Some(Fidelity::Lossless),
    },
    Codec {
        format: &formats::TGA,
        kind: ImageFormat::Tga,
        read: Some(Fidelity::Conditional(
            "the ID field and any TGA 2.0 extension area (thumbnail, author, dates) are dropped",
        )),
        write: Some(Fidelity::Lossless),
    },
    Codec {
        format: &formats::TIFF,
        kind: ImageFormat::Tiff,
        read: Some(Fidelity::Conditional(
            "the first page is read; 16-bit gray and RGB stay 16-bit into PNG, TIFF, and Netpbm and become 8-bit elsewhere; CMYK becomes RGB, and metadata (resolution, EXIF, ICC, XMP) is dropped",
        )),
        write: Some(Fidelity::Lossless),
    },
    Codec {
        format: &formats::ICO,
        kind: ImageFormat::Ico,
        read: Some(Fidelity::Conditional(
            "the largest image is read and the icon's other sizes are dropped",
        )),
        write: Some(Fidelity::Conditional(
            "the image is the icon's largest size (scaled down to 256 pixels if larger), with the standard smaller sizes added, scaled by area averaging",
        )),
    },
    Codec {
        format: &formats::PDF,
        kind: ImageFormat::Pdf,
        read: Some(Fidelity::Conditional(
            "the chosen page's largest image is read (--page, the first by default); a page's vector content is not rendered, and other pages and images are dropped",
        )),
        write: Some(Fidelity::Lossless),
    },
    Codec {
        format: &formats::HEIC,
        kind: ImageFormat::Heic,
        read: Some(Fidelity::Conditional(
            "the primary image is read with its crop, rotation, mirroring, and alpha applied; 10-bit samples become 16-bit in PNG, TIFF, and Netpbm and 8-bit elsewhere; the color profile and Exif are carried into PNG, JPEG, and HEIC and dropped elsewhere",
        )),
        write: Some(Fidelity::Lossy(
            "HEIC is lossy: the image is coded with HEVC at the quality given (50 by default, as heif-enc), 4:2:0 chroma, 16-bit samples at 10 bits; the color profile and Exif are carried",
        )),
    },
    Codec {
        format: &formats::CUR,
        kind: ImageFormat::Ico,
        read: Some(Fidelity::Conditional(
            "the largest image is read; the cursor's other sizes and its hotspot are dropped",
        )),
        write: None,
    },
];

const NETPBM_READ: &str = "samples wider than 8 bits (maxval over 255) are scaled to 16 bits into PNG, TIFF, and Netpbm and to 8 bits elsewhere";

/// The worse of two fidelities, carrying both texts when both lose.
fn combine(read: &Fidelity, write: &Fidelity) -> Fidelity {
    let text = |fidelity: &Fidelity| match fidelity {
        Fidelity::Lossless => None,
        Fidelity::Conditional(text) | Fidelity::Lossy(text) => Some(*text),
    };
    let joined: &'static str = match (text(read), text(write)) {
        (None, None) => return Fidelity::Lossless,
        (Some(one), None) | (None, Some(one)) => one,
        (Some(first), Some(second)) => {
            // Built once per pair when the table is first read.
            Box::leak(format!("{first}; {second}").into_boxed_str())
        }
    };
    if read.kind() == FidelityKind::Lossy || write.kind() == FidelityKind::Lossy {
        Fidelity::Lossy(joined)
    } else {
        Fidelity::Conditional(joined)
    }
}

/// Every pair between two codecs, built once.
pub fn pairs() -> &'static [ImagePair] {
    static PAIRS: OnceLock<Vec<ImagePair>> = OnceLock::new();
    PAIRS.get_or_init(|| {
        let mut pairs = Vec::new();
        for from in &CODECS {
            let Some(read) = &from.read else {
                continue;
            };
            for to in &CODECS {
                let Some(write) = &to.write else {
                    continue;
                };
                if from.format.id == to.format.id {
                    continue;
                }
                let name = format!("{}-to-{}", from.format.id, to.format.id);
                pairs.push(ImagePair {
                    name: Box::leak(name.into_boxed_str()),
                    from: from.format,
                    to: to.format,
                    read: from.kind,
                    write: to.kind,
                    // A JPEG goes into a PDF as it is: nothing is decoded.
                    fidelity: if from.kind == ImageFormat::Jpeg && to.kind == ImageFormat::Pdf {
                        Fidelity::Lossless
                    } else {
                        combine(read, write)
                    },
                });
            }
        }
        pairs
    })
}

/// Every codec's format with what its reader and its writer lose
/// (`None` where it has no reader or no writer): the hub the pairs are
/// generated from, which the format map draws.
pub fn codecs() -> impl Iterator<
    Item = (
        &'static Format,
        Option<&'static Fidelity>,
        Option<&'static Fidelity>,
    ),
> {
    CODECS
        .iter()
        .map(|codec| (codec.format, codec.read.as_ref(), codec.write.as_ref()))
}

/// The pair from one format id to another (`pair("png", "bmp")`).
pub fn pair(from: &str, to: &str) -> &'static ImagePair {
    pairs()
        .iter()
        .find(|pair| pair.from.id == from && pair.to.id == to)
        .unwrap_or_else(|| panic!("no image pair {from} -> {to}"))
}

/// Reads a picture held in memory into `sink`, its format told by its
/// first bytes (PNG, JPEG, WebP, BMP, TIFF, QOI, ICO, HEIC): the pictures a
/// document carries, set into a PDF.
pub fn decode_image(bytes: &[u8], sink: &mut dyn RowSink) -> std::io::Result<()> {
    let format = if bytes.starts_with(b"\x89PNG") {
        ImageFormat::Png
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        ImageFormat::Jpeg
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        ImageFormat::Webp
    } else if bytes.starts_with(b"BM") {
        ImageFormat::Bmp
    } else if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        ImageFormat::Tiff
    } else if bytes.starts_with(b"qoif") {
        ImageFormat::Qoi
    } else if bytes.starts_with(&[0, 0, 1, 0]) {
        ImageFormat::Ico
    } else if bytes.get(4..8) == Some(b"ftyp")
        && bytes.get(8..12).is_some_and(|brand| {
            matches!(
                brand,
                b"heic" | b"heix" | b"heim" | b"heis" | b"hevc" | b"hevx" | b"mif1" | b"msf1"
            )
        })
    {
        ImageFormat::Heic
    } else {
        return Err(std::io::Error::other(
            "an image format there is no reader for",
        ));
    };
    let options = crate::converter::ConvertOptions::default();
    let mut events = crate::event::NullSink;
    let mut context = Context::new(&mut events, &options);
    let mut source: &[u8] = bytes;
    read_rows(
        format,
        &mut Input::Stream(&mut source),
        sink,
        "image",
        &mut context,
    )
    .map_err(|error| std::io::Error::other(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_declare_their_contracts() {
        assert_eq!(pair("png", "bmp").name(), "png-to-bmp");
        assert_eq!(pair("png", "bmp").from().id, "png");
        assert_eq!(pair("bmp", "png").to().id, "png");
        assert_eq!(pair("bmp", "png").fidelity(), Fidelity::Lossless);
        assert_eq!(pair("bmp", "jpeg").fidelity(), Fidelity::Lossy(JPEG_LOSS));
        assert_eq!(pair("webp", "jpeg").fidelity().kind(), FidelityKind::Lossy);
    }

    #[test]
    fn every_codec_reaches_every_other() {
        let readers = CODECS.iter().filter(|codec| codec.read.is_some()).count();
        let writers = CODECS.iter().filter(|codec| codec.write.is_some()).count();
        let both = CODECS
            .iter()
            .filter(|codec| codec.read.is_some() && codec.write.is_some())
            .count();
        assert_eq!(pairs().len(), readers * writers - both);
    }
}
