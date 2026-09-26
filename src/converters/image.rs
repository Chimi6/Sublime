//! Every pair between the raster formats: one reader into the image hub,
//! one writer out of it. A pair is a `ImagePair` value naming its two
//! formats, its fidelity, and the reader's notes to report.

use std::io::{Read, Write};

use crate::converter::{ConvertError, Converter, Fidelity, Input, Location, Tier};
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::image::Image;
use crate::io::bmp::{BmpError, BmpRows, BmpRowsError, read_bmp, read_bmp_rows, write_bmp};
use crate::io::png::{
    PngError, PngNotes, PngRows, RowsError, read_png_from, read_png_rows, write_png,
};

#[derive(Clone, Copy)]
pub enum ImageFormat {
    Png,
    Bmp,
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
        if matches!(
            (self.read, self.write),
            (ImageFormat::Png, ImageFormat::Bmp)
        ) {
            // Row by row: no image is held, and the BMP is written
            // top-down as rows come out of the unfilter.
            let mut rows = BmpRows::new(output);
            let notes = match read_png_rows(&mut input, &mut rows) {
                Ok(notes) => notes,
                Err(RowsError::Png(error)) => return Err(error.into()),
                Err(RowsError::Io(error)) => return Err(error.into()),
            };
            report_png_notes(notes, self.name, context);
            return Ok(());
        }
        if matches!(
            (self.read, self.write),
            (ImageFormat::Bmp, ImageFormat::Png)
        ) {
            // Row by row: a top-down BMP streams, a bottom-up one is held
            // once as file bytes; the PNG writer needs only the row above.
            let mut rows = PngRows::new(output);
            return match read_bmp_rows(&mut input, &mut rows) {
                Ok(()) => Ok(()),
                Err(BmpRowsError::Bmp(error)) => Err(error.into()),
                Err(BmpRowsError::Io(error)) => Err(error.into()),
            };
        }
        let image = read(self.read, &mut input, self.name, context)?;
        write(self.write, &image, output)?;
        output.flush()?;
        Ok(())
    }
}

/// PNG streams from the input; BMP, being uncompressed and random
/// access, is read whole.
fn read(
    format: ImageFormat,
    input: &mut Input<'_>,
    name: &'static str,
    context: &mut Context<'_>,
) -> Result<Image, ConvertError> {
    match format {
        ImageFormat::Png => {
            let (image, notes) = read_png_from(input)?;
            report_png_notes(notes, name, context);
            Ok(image)
        }
        ImageFormat::Bmp => {
            let mut bytes = Vec::new();
            input.read_to_end(&mut bytes)?;
            Ok(read_bmp(&bytes)?)
        }
    }
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

fn write(format: ImageFormat, image: &Image, output: &mut dyn Write) -> Result<(), ConvertError> {
    match format {
        ImageFormat::Png => write_png(image, output)?,
        ImageFormat::Bmp => write_bmp(image, output)?,
    }
    Ok(())
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
