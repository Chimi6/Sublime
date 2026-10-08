//! Exif orientation (TIFF tag 274): the eight ways a camera's stored
//! pixels are turned or mirrored for display, applied to rows on their way
//! to a sink, and set back to 1 in the Exif carried beside them.

use crate::image::ColorType;
use crate::io::png::RowSink;

/// A sink in front of another that turns or mirrors the image as an Exif
/// orientation says. The rows are held until the last arrives, then
/// handed on upright; orientation 1 passes them straight through.
pub struct Oriented<'a> {
    sink: &'a mut dyn RowSink,
    orientation: u16,
    width: usize,
    height: usize,
    channels: usize,
    pixels: Vec<u8>,
}

impl<'a> Oriented<'a> {
    pub fn new(sink: &'a mut dyn RowSink, orientation: u16) -> Oriented<'a> {
        Oriented {
            sink,
            orientation: if (1..=8).contains(&orientation) {
                orientation
            } else {
                1
            },
            width: 0,
            height: 0,
            channels: 0,
            pixels: Vec::new(),
        }
    }

    /// Whether the displayed image is the stored one turned a quarter, so
    /// its width and height trade places.
    fn transposed(&self) -> bool {
        self.orientation >= 5
    }

    /// The stored pixel shown at (x, y) of the displayed image.
    fn source(&self, x: usize, y: usize) -> (usize, usize) {
        let (w, h) = (self.width, self.height);
        match self.orientation {
            2 => (w - 1 - x, y),
            3 => (w - 1 - x, h - 1 - y),
            4 => (x, h - 1 - y),
            5 => (y, x),
            6 => (y, h - 1 - x),
            7 => (w - 1 - y, h - 1 - x),
            8 => (w - 1 - y, x),
            _ => (x, y),
        }
    }

    fn hand_on(&mut self) -> std::io::Result<()> {
        let (width, height) = if self.transposed() {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        };
        let channels = self.channels;
        let mut row = vec![0u8; width * channels];
        for y in 0..height {
            for (x, pixel) in row.chunks_exact_mut(channels).enumerate() {
                let (sx, sy) = self.source(x, y);
                let at = (sy * self.width + sx) * channels;
                pixel.copy_from_slice(&self.pixels[at..at + channels]);
            }
            self.sink.row(&row)?;
        }
        self.pixels = Vec::new();
        Ok(())
    }
}

impl RowSink for Oriented<'_> {
    fn start(&mut self, width: u32, height: u32, color: ColorType) -> std::io::Result<()> {
        self.width = width as usize;
        self.height = height as usize;
        self.channels = color.channels();
        if self.orientation != 1 {
            self.pixels = Vec::with_capacity(self.width * self.height * self.channels);
        }
        if self.transposed() {
            self.sink.start(height, width, color)
        } else {
            self.sink.start(width, height, color)
        }
    }

    fn row(&mut self, pixels: &[u8]) -> std::io::Result<()> {
        if self.orientation == 1 {
            return self.sink.row(pixels);
        }
        self.pixels.extend_from_slice(pixels);
        if self.pixels.len() == self.width * self.height * self.channels {
            self.hand_on()?;
        }
        Ok(())
    }

    fn density(&mut self, across: f64, down: f64) {
        if self.transposed() {
            self.sink.density(down, across);
        } else {
            self.sink.density(across, down);
        }
    }

    fn icc_profile(&mut self, profile: &[u8]) -> bool {
        self.sink.icc_profile(profile)
    }

    fn exif(&mut self, exif: &[u8]) -> bool {
        self.sink.exif(exif)
    }
}

/// The orientation tag (274) of an Exif TIFF structure's first directory.
pub fn exif_orientation(tiff: &[u8]) -> Option<u16> {
    let at = orientation_entry(tiff)?;
    let big = tiff.starts_with(b"MM");
    let bytes: [u8; 2] = tiff.get(at + 8..at + 10)?.try_into().ok()?;
    Some(if big {
        u16::from_be_bytes(bytes)
    } else {
        u16::from_le_bytes(bytes)
    })
}

/// Sets an Exif TIFF structure's orientation, if it has one, to 1: the
/// pixels beside it are upright already.
pub fn reset_exif_orientation(tiff: &mut [u8]) {
    if let Some(at) = orientation_entry(tiff) {
        let one = if tiff.starts_with(b"MM") {
            [0, 1]
        } else {
            [1, 0]
        };
        tiff[at + 8..at + 10].copy_from_slice(&one);
    }
}

/// Where the orientation entry (a SHORT) sits in the first directory.
fn orientation_entry(tiff: &[u8]) -> Option<usize> {
    let big = match tiff.get(..2)? {
        b"MM" => true,
        b"II" => false,
        _ => return None,
    };
    let u16_at = |at: usize| -> Option<u16> {
        let bytes: [u8; 2] = tiff.get(at..at + 2)?.try_into().ok()?;
        Some(if big {
            u16::from_be_bytes(bytes)
        } else {
            u16::from_le_bytes(bytes)
        })
    };
    let bytes: [u8; 4] = tiff.get(4..8)?.try_into().ok()?;
    let directory = if big {
        u32::from_be_bytes(bytes)
    } else {
        u32::from_le_bytes(bytes)
    } as usize;
    let count = u16_at(directory)?;
    (0..usize::from(count))
        .map(|entry| directory + 2 + entry * 12)
        .find(|&at| u16_at(at) == Some(274) && u16_at(at + 2) == Some(3))
        .filter(|&at| at + 10 <= tiff.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::png::reader::Collect;

    /// A 3 by 2 image whose pixels count 0 to 5, through an orientation.
    fn oriented(orientation: u16) -> (u32, u32, Vec<u8>) {
        let mut collect = Collect::default();
        let mut sink = Oriented::new(&mut collect, orientation);
        sink.start(3, 2, ColorType::Gray).unwrap();
        sink.row(&[0, 1, 2]).unwrap();
        sink.row(&[3, 4, 5]).unwrap();
        let image = collect.image;
        (image.width, image.height, image.pixels)
    }

    #[test]
    fn every_orientation_shows_the_image_upright() {
        assert_eq!(oriented(1), (3, 2, vec![0, 1, 2, 3, 4, 5]));
        assert_eq!(oriented(2), (3, 2, vec![2, 1, 0, 5, 4, 3]));
        assert_eq!(oriented(3), (3, 2, vec![5, 4, 3, 2, 1, 0]));
        assert_eq!(oriented(4), (3, 2, vec![3, 4, 5, 0, 1, 2]));
        assert_eq!(oriented(5), (2, 3, vec![0, 3, 1, 4, 2, 5]));
        // 6: turned a quarter clockwise, the left column on top.
        assert_eq!(oriented(6), (2, 3, vec![3, 0, 4, 1, 5, 2]));
        assert_eq!(oriented(7), (2, 3, vec![5, 2, 4, 1, 3, 0]));
        // 8: a quarter anticlockwise, the right column on top.
        assert_eq!(oriented(8), (2, 3, vec![2, 5, 1, 4, 0, 3]));
    }

    #[test]
    fn orientation_is_read_and_reset_in_either_byte_order() {
        let mut little = b"II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0".to_vec();
        assert_eq!(exif_orientation(&little), Some(6));
        reset_exif_orientation(&mut little);
        assert_eq!(exif_orientation(&little), Some(1));
        let mut big = b"MM\0*\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01\0\x06\0\0".to_vec();
        assert_eq!(exif_orientation(&big), Some(6));
        reset_exif_orientation(&mut big);
        assert_eq!(&big[18..20], &[0, 1]);
    }
}
