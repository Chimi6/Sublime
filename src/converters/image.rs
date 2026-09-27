//! Every pair between the raster formats: one reader into the image hub,
//! one writer out of it. A pair is a `ImagePair` value naming its two
//! formats, its fidelity, and the reader's notes to report.

use std::io::Write;

use std::sync::OnceLock;

use crate::converter::{ConvertError, Converter, Fidelity, FidelityKind, Input, Location, Tier};
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::bmp::{BmpError, BmpRows, BmpRowsError, read_bmp_rows};
use crate::io::ico::{IcoNotes, IcoRows, read_ico_rows};
use crate::io::jpeg::{DEFAULT_QUALITY, JpegError, JpegNotes, JpegRows, read_jpeg_rows};
use crate::io::netpbm::{Kind, NetpbmNotes, NetpbmRows, read_netpbm_rows};
use crate::io::png::{PngError, PngNotes, PngRows, RowSink, RowsError, read_png_rows};
use crate::io::qoi::{QoiRows, read_qoi_rows};
use crate::io::tga::{TgaRows, read_tga_rows};
use crate::io::tiff::{TiffNotes, TiffRows, read_tiff_rows};
use crate::io::webp::{Effort, WebpNotes, WebpRows, read_webp_rows};

#[derive(Clone, Copy)]
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
        let effort = Effort::from_quality(context.options.quality);
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
        }
    }
}

fn rows_error(error: RowsError) -> ConvertError {
    match error {
        RowsError::Png(error) => error.into(),
        RowsError::Io(error) => error.into(),
    }
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
        ImageFormat::Bmp => match read_bmp_rows(input, sink) {
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
        ImageFormat::Tiff => {
            let notes = read_tiff_rows(input, sink).map_err(rows_error)?;
            report_tiff_notes(notes, name, context);
        }
        ImageFormat::Netpbm(_) => {
            let notes = read_netpbm_rows(input, sink).map_err(rows_error)?;
            report_netpbm_notes(notes, name, context);
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
    if let Some(orientation) = notes.orientation {
        context.warning(format!(
            "Exif orientation {orientation} is not applied: the pixels are as stored, and a viewer would rotate them"
        ));
    }
}

fn report_netpbm_notes(notes: NetpbmNotes, name: &'static str, context: &mut Context<'_>) {
    if notes.maxval > 255 {
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
    read: Fidelity,
    /// What the writer loses; `None` for a format only read.
    write: Option<Fidelity>,
}

const JPEG_LOSS: &str = "JPEG is lossy: the image is re-encoded at the quality given (85 by default, 4:2:0 chroma below 90) and alpha is flattened onto white";

static CODECS: [Codec; 13] = [
    Codec {
        format: &formats::PNG,
        kind: ImageFormat::Png,
        read: Fidelity::Conditional(
            "16-bit samples become 8-bit, and metadata (gamma, color profile, text) is dropped",
        ),
        write: Some(Fidelity::Lossless),
    },
    Codec {
        format: &formats::BMP,
        kind: ImageFormat::Bmp,
        read: Fidelity::Lossless,
        write: Some(Fidelity::Lossless),
    },
    Codec {
        format: &formats::JPEG,
        kind: ImageFormat::Jpeg,
        read: Fidelity::Conditional(
            "pixels as decoded (Exif orientation is reported, not applied); metadata (Exif, ICC, comments) is dropped",
        ),
        write: Some(Fidelity::Lossy(JPEG_LOSS)),
    },
    Codec {
        format: &formats::WEBP,
        kind: ImageFormat::Webp,
        read: Fidelity::Conditional(
            "pixels as decoded (a lossy WebP decodes exactly as libwebp does); an animation keeps its first frame; metadata (ICC, Exif, XMP) is dropped",
        ),
        write: Some(Fidelity::Lossless),
    },
    Codec {
        format: &formats::QOI,
        kind: ImageFormat::Qoi,
        read: Fidelity::Lossless,
        write: Some(Fidelity::Lossless),
    },
    Codec {
        format: &formats::PBM,
        kind: ImageFormat::Netpbm(Kind::Pbm),
        read: Fidelity::Lossless,
        write: Some(Fidelity::Lossy(
            "black and white: color becomes luma, alpha is flattened onto white, and gray below half is black",
        )),
    },
    Codec {
        format: &formats::PGM,
        kind: ImageFormat::Netpbm(Kind::Pgm),
        read: Fidelity::Conditional(NETPBM_READ),
        write: Some(Fidelity::Conditional(
            "color becomes luma and alpha is flattened onto white",
        )),
    },
    Codec {
        format: &formats::PPM,
        kind: ImageFormat::Netpbm(Kind::Ppm),
        read: Fidelity::Conditional(NETPBM_READ),
        write: Some(Fidelity::Conditional("alpha is flattened onto white")),
    },
    Codec {
        format: &formats::PAM,
        kind: ImageFormat::Netpbm(Kind::Pam),
        read: Fidelity::Conditional(NETPBM_READ),
        write: Some(Fidelity::Lossless),
    },
    Codec {
        format: &formats::TGA,
        kind: ImageFormat::Tga,
        read: Fidelity::Conditional(
            "the ID field and any TGA 2.0 extension area (thumbnail, author, dates) are dropped",
        ),
        write: Some(Fidelity::Lossless),
    },
    Codec {
        format: &formats::TIFF,
        kind: ImageFormat::Tiff,
        read: Fidelity::Conditional(
            "the first page is read; 16-bit samples become 8-bit, CMYK becomes RGB, and metadata (resolution, EXIF, ICC, XMP) is dropped",
        ),
        write: Some(Fidelity::Lossless),
    },
    Codec {
        format: &formats::ICO,
        kind: ImageFormat::Ico,
        read: Fidelity::Conditional(
            "the largest image is read and the icon's other sizes are dropped",
        ),
        write: Some(Fidelity::Conditional(
            "the image is the icon's largest size (scaled down to 256 pixels if larger), with the standard smaller sizes added, scaled by area averaging",
        )),
    },
    Codec {
        format: &formats::CUR,
        kind: ImageFormat::Ico,
        read: Fidelity::Conditional(
            "the largest image is read; the cursor's other sizes and its hotspot are dropped",
        ),
        write: None,
    },
];

const NETPBM_READ: &str = "samples wider than 8 bits (maxval over 255) are scaled to 8 bits";

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
                    fidelity: combine(&from.read, write),
                });
            }
        }
        pairs
    })
}

/// The pair from one format id to another (`pair("png", "bmp")`).
pub fn pair(from: &str, to: &str) -> &'static ImagePair {
    pairs()
        .iter()
        .find(|pair| pair.from.id == from && pair.to.id == to)
        .unwrap_or_else(|| panic!("no image pair {from} -> {to}"))
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
        let readers = CODECS.len();
        let writers = CODECS.iter().filter(|codec| codec.write.is_some()).count();
        assert_eq!(pairs().len(), readers * writers - writers);
    }
}
