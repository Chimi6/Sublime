//! The image hub: one pixel buffer every raster format reads into and
//! writes from. Eight bits per channel, rows top to bottom, no padding.

/// The channels a pixel has.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorType {
    #[default]
    Gray,
    GrayAlpha,
    Rgb,
    Rgba,
}

impl ColorType {
    pub fn channels(self) -> usize {
        match self {
            ColorType::Gray => 1,
            ColorType::GrayAlpha => 2,
            ColorType::Rgb => 3,
            ColorType::Rgba => 4,
        }
    }

    pub fn has_alpha(self) -> bool {
        matches!(self, ColorType::GrayAlpha | ColorType::Rgba)
    }

    pub fn label(self) -> &'static str {
        match self {
            ColorType::Gray => "gray",
            ColorType::GrayAlpha => "gray+alpha",
            ColorType::Rgb => "rgb",
            ColorType::Rgba => "rgba",
        }
    }
}

/// A decoded image.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub color: ColorType,
    /// `height` rows of `width * channels` bytes.
    pub pixels: Vec<u8>,
}

/// The largest image the hub will hold: a guard against a header that
/// asks for more memory than any picture needs.
pub const MAX_PIXELS: u64 = 1 << 31;

impl Image {
    pub fn new(width: u32, height: u32, color: ColorType) -> Image {
        let size = width as usize * height as usize * color.channels();
        Image {
            width,
            height,
            color,
            pixels: vec![0; size],
        }
    }

    pub fn stride(&self) -> usize {
        self.width as usize * self.color.channels()
    }

    pub fn row(&self, y: u32) -> &[u8] {
        let stride = self.stride();
        let start = y as usize * stride;
        &self.pixels[start..start + stride]
    }

    /// True when the dimensions fit the hub's guard.
    pub fn dimensions_fit(width: u32, height: u32, channels: usize) -> bool {
        width > 0
            && height > 0
            && (width as u64)
                .saturating_mul(height as u64)
                .saturating_mul(channels as u64)
                <= MAX_PIXELS
    }
}
