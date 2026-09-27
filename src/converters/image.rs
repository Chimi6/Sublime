//! Every pair between the raster formats: one reader into the image hub,
//! one writer out of it. A pair is a `ImagePair` value naming its two
//! formats, its fidelity, and the reader's notes to report.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Fidelity, Input, Location, Tier};
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::bmp::{BmpError, BmpRows, BmpRowsError, read_bmp_rows};
use crate::io::jpeg::{DEFAULT_QUALITY, JpegError, JpegNotes, JpegRows, read_jpeg_rows};
use crate::io::png::{PngError, PngNotes, PngRows, RowSink, RowsError, read_png_rows};
use crate::io::webp::{WebpNotes, WebpRows, read_webp_rows};

#[derive(Clone, Copy)]
pub enum ImageFormat {
    Png,
    Bmp,
    Jpeg,
    Webp,
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
                let mut rows = WebpRows::new(output);
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

pub static PNG_TO_BMP: ImagePair = ImagePair {
    name: "png-to-bmp",
    from: &formats::PNG,
    to: &formats::BMP,
    read: ImageFormat::Png,
    write: ImageFormat::Bmp,
    fidelity: Fidelity::Conditional(
        "16-bit samples become 8-bit, gray becomes RGB, and metadata (gamma, color profile, text) is dropped",
    ),
};

pub static BMP_TO_PNG: ImagePair = ImagePair {
    name: "bmp-to-png",
    from: &formats::BMP,
    to: &formats::PNG,
    read: ImageFormat::Bmp,
    write: ImageFormat::Png,
    fidelity: Fidelity::Lossless,
};

const JPEG_LOSS: &str = "JPEG is lossy: the image is re-encoded at the quality given (85 by default, 4:2:0 chroma below 90), alpha is flattened onto white, and metadata is dropped";
const JPEG_DECODE_NOTE: &str = "pixels as decoded (Exif orientation is reported, not applied); metadata (Exif, ICC, comments) is dropped";

pub static JPEG_TO_PNG: ImagePair = ImagePair {
    name: "jpeg-to-png",
    from: &formats::JPEG,
    to: &formats::PNG,
    read: ImageFormat::Jpeg,
    write: ImageFormat::Png,
    fidelity: Fidelity::Conditional(JPEG_DECODE_NOTE),
};

pub static JPEG_TO_BMP: ImagePair = ImagePair {
    name: "jpeg-to-bmp",
    from: &formats::JPEG,
    to: &formats::BMP,
    read: ImageFormat::Jpeg,
    write: ImageFormat::Bmp,
    fidelity: Fidelity::Conditional(JPEG_DECODE_NOTE),
};

pub static PNG_TO_JPEG: ImagePair = ImagePair {
    name: "png-to-jpeg",
    from: &formats::PNG,
    to: &formats::JPEG,
    read: ImageFormat::Png,
    write: ImageFormat::Jpeg,
    fidelity: Fidelity::Lossy(JPEG_LOSS),
};

pub static BMP_TO_JPEG: ImagePair = ImagePair {
    name: "bmp-to-jpeg",
    from: &formats::BMP,
    to: &formats::JPEG,
    read: ImageFormat::Bmp,
    write: ImageFormat::Jpeg,
    fidelity: Fidelity::Lossy(JPEG_LOSS),
};

const WEBP_DECODE_NOTE: &str = "pixels as decoded (a lossy WebP decodes exactly as libwebp does); an animation keeps its first frame; metadata (ICC, Exif, XMP) is dropped";
const TO_WEBP_NOTE: &str =
    "lossless WebP of the 8-bit pixels: 16-bit samples become 8-bit and metadata is dropped";

pub static WEBP_TO_PNG: ImagePair = ImagePair {
    name: "webp-to-png",
    from: &formats::WEBP,
    to: &formats::PNG,
    read: ImageFormat::Webp,
    write: ImageFormat::Png,
    fidelity: Fidelity::Conditional(WEBP_DECODE_NOTE),
};

pub static WEBP_TO_BMP: ImagePair = ImagePair {
    name: "webp-to-bmp",
    from: &formats::WEBP,
    to: &formats::BMP,
    read: ImageFormat::Webp,
    write: ImageFormat::Bmp,
    fidelity: Fidelity::Conditional(WEBP_DECODE_NOTE),
};

pub static WEBP_TO_JPEG: ImagePair = ImagePair {
    name: "webp-to-jpeg",
    from: &formats::WEBP,
    to: &formats::JPEG,
    read: ImageFormat::Webp,
    write: ImageFormat::Jpeg,
    fidelity: Fidelity::Lossy(JPEG_LOSS),
};

pub static PNG_TO_WEBP: ImagePair = ImagePair {
    name: "png-to-webp",
    from: &formats::PNG,
    to: &formats::WEBP,
    read: ImageFormat::Png,
    write: ImageFormat::Webp,
    fidelity: Fidelity::Conditional(TO_WEBP_NOTE),
};

pub static BMP_TO_WEBP: ImagePair = ImagePair {
    name: "bmp-to-webp",
    from: &formats::BMP,
    to: &formats::WEBP,
    read: ImageFormat::Bmp,
    write: ImageFormat::Webp,
    fidelity: Fidelity::Lossless,
};

pub static JPEG_TO_WEBP: ImagePair = ImagePair {
    name: "jpeg-to-webp",
    from: &formats::JPEG,
    to: &formats::WEBP,
    read: ImageFormat::Jpeg,
    write: ImageFormat::Webp,
    fidelity: Fidelity::Conditional(JPEG_DECODE_NOTE),
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_declare_their_contracts() {
        assert_eq!(PNG_TO_BMP.name(), "png-to-bmp");
        assert_eq!(PNG_TO_BMP.from().id, "png");
        assert_eq!(BMP_TO_PNG.to().id, "png");
        assert_eq!(BMP_TO_PNG.fidelity(), Fidelity::Lossless);
    }
}
