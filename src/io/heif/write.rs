//! Writing HEIC: rows to YCbCr 4:2:0 (BT.601, full range, chroma from
//! each 2x2 block's average, as libheif does by default), coded by the
//! HEVC encoder here, in the boxes libheif writes: `ftyp`, then `meta`
//! (the handler, primary item, locations, item infos, and properties:
//! `hvcC`, `colr`, `ispe`, `pixi`), then `mdat`. Alpha goes in an
//! auxiliary picture referring to the primary; the ICC profile in a
//! `colr` box and Exif in an item of its own.

use std::io::Write;

use crate::image::ColorType;
use crate::io::hevc::encode::{self, Settings};
use crate::io::png::RowSink;

/// The quality `heif-enc` and this writer take by default.
pub const DEFAULT_QUALITY: u8 = 50;

/// The QP of a quality from 1 to 100 (libheif's scale for x265: the
/// constant rate factor is half the distance from 100).
pub fn qp_of(quality: u8) -> i32 {
    ((100 - i32::from(quality.clamp(1, 100))) + 1) / 2
}

/// A HEIC written from rows: they are held as YCbCr planes, then coded.
pub struct HeicRows<'a> {
    sink: &'a mut dyn Write,
    quality: u8,
    width: usize,
    height: usize,
    color: ColorType,
    /// Rows are 16-bit (coded at 10 bits).
    deep: bool,
    /// Rows come as JFIF YCbCr through `ycbcr_row`.
    ycbcr: bool,
    icc_profile: Option<Vec<u8>>,
    exif: Option<Vec<u8>>,
    settings: Option<Settings>,
    planes: [Vec<u16>; 3],
    alpha: Option<Vec<u16>>,
    /// The even row's RGB (at the coded bit depth's scale, times 16
    /// fractional bits' worth kept as i64 sums) waiting for its pair.
    pending: Vec<[i64; 3]>,
    row: usize,
}

impl<'a> HeicRows<'a> {
    pub fn new(sink: &'a mut dyn Write, quality: u8) -> HeicRows<'a> {
        HeicRows {
            sink,
            quality,
            width: 0,
            height: 0,
            color: ColorType::Rgb,
            deep: false,
            ycbcr: false,
            icc_profile: None,
            exif: None,
            settings: None,
            planes: [Vec::new(), Vec::new(), Vec::new()],
            alpha: None,
            pending: Vec::new(),
            row: 0,
        }
    }

    fn bit_depth(&self) -> u32 {
        if self.deep { 10 } else { 8 }
    }

    /// An input sample at the coded depth's scale, in 16 fractional bits.
    #[inline(always)]
    fn scaled(&self, value: u32) -> i64 {
        if self.deep {
            // 16 bits to 10: times 1023/65535.
            (i64::from(value) * 1023 * 65536 + 32767) / 65535
        } else {
            i64::from(value) << 16
        }
    }

    /// Luma and the RGB of one pixel, at the coded scale (16 fraction bits).
    fn pixel(&self, pixels: &[u8], x: usize) -> ([i64; 3], Option<u32>) {
        let channels = self.color.channels();
        let sample = |index: usize| -> u32 {
            if self.deep {
                let at = (x * channels + index) * 2;
                u32::from(u16::from_be_bytes([pixels[at], pixels[at + 1]]))
            } else {
                u32::from(pixels[x * channels + index])
            }
        };
        let (rgb, alpha) = match self.color {
            ColorType::Gray => ([sample(0); 3], None),
            ColorType::GrayAlpha => ([sample(0); 3], Some(sample(1))),
            ColorType::Rgb => ([sample(0), sample(1), sample(2)], None),
            ColorType::Rgba => ([sample(0), sample(1), sample(2)], Some(sample(3))),
        };
        (
            [
                self.scaled(rgb[0]),
                self.scaled(rgb[1]),
                self.scaled(rgb[2]),
            ],
            alpha,
        )
    }

    fn finish(&mut self) -> std::io::Result<()> {
        let Some(settings) = self.settings.clone() else {
            return Ok(());
        };
        let (cw, ch) = (settings.coded_width, settings.coded_height);
        pad(&mut self.planes[0], self.width, self.height, cw, ch);
        let (chroma_width, chroma_height) = (self.width.div_ceil(2), self.height.div_ceil(2));
        pad(
            &mut self.planes[1],
            chroma_width,
            chroma_height,
            cw / 2,
            ch / 2,
        );
        pad(
            &mut self.planes[2],
            chroma_width,
            chroma_height,
            cw / 2,
            ch / 2,
        );
        let planes = std::mem::take(&mut self.planes);
        let picture = encode::encode(&settings, planes);
        let alpha = self.alpha.take().map(|mut alpha| {
            pad(&mut alpha, self.width, self.height, cw, ch);
            let mid = 1u16 << (settings.bit_depth - 1);
            let chroma = vec![mid; (cw / 2) * (ch / 2)];
            let mut alpha_settings = settings.clone();
            // Alpha's own levels: full range, no colour.
            alpha_settings.colour = (2, 2, 2);
            let coded = encode::encode(&alpha_settings, [alpha, chroma.clone(), chroma]);
            (coded, alpha_settings)
        });
        let file = container(
            &settings,
            (self.width, self.height),
            &picture,
            alpha.as_ref().map(|(coded, settings)| (coded, settings)),
            self.icc_profile.as_deref(),
            self.exif.as_deref(),
        );
        self.sink.write_all(&file)?;
        self.sink.flush()
    }
}

/// Fills a plane of `coded_width` by `coded_height`, whose first
/// `width` by `height` samples are the picture's, out to its edges,
/// repeating the last column and row.
fn pad(plane: &mut [u16], width: usize, height: usize, coded_width: usize, coded_height: usize) {
    for row in plane.chunks_exact_mut(coded_width).take(height) {
        let last = row[width - 1];
        row[width..].fill(last);
    }
    let (filled, rest) = plane.split_at_mut(height * coded_width);
    let last = &filled[(height - 1) * coded_width..];
    for row in rest
        .chunks_exact_mut(coded_width)
        .take(coded_height - height)
    {
        row.copy_from_slice(last);
    }
}

/// BT.601 luma and chroma weights in 16 fractional bits.
const KR: i64 = 19595;
const KG: i64 = 38470;
const KB: i64 = 7471;
const CB: [i64; 3] = [-11059, -21709, 32768];
const CR: [i64; 3] = [32768, -27439, -5329];

impl RowSink for HeicRows<'_> {
    fn icc_profile(&mut self, profile: &[u8]) -> bool {
        self.icc_profile = Some(profile.to_vec());
        true
    }

    fn exif(&mut self, exif: &[u8]) -> bool {
        self.exif = Some(exif.to_vec());
        true
    }

    fn accept_deep(&mut self, _color: ColorType) -> bool {
        self.deep = true;
        true
    }

    fn accept_ycbcr(&mut self) -> bool {
        self.ycbcr = true;
        true
    }

    fn start(&mut self, width: u32, height: u32, color: ColorType) -> std::io::Result<()> {
        if width == 0 || height == 0 {
            return Err(std::io::Error::other("an empty image"));
        }
        if width > 16_384 || height > 16_384 {
            return Err(std::io::Error::other(
                "HEIC is written up to 16384 pixels a side",
            ));
        }
        self.width = width as usize;
        self.height = height as usize;
        self.color = color;
        // 4:2:0 crops to even sizes only: an odd side is coded one longer
        // and the clean aperture takes the extra column or row off.
        let settings = Settings::new(
            self.width + self.width % 2,
            self.height + self.height % 2,
            self.bit_depth(),
            qp_of(self.quality),
        );
        // Planes at the coded size, filled row by row and padded after.
        let (cw, ch) = (settings.coded_width, settings.coded_height);
        self.planes = [
            vec![0; cw * ch],
            vec![0; (cw / 2) * (ch / 2)],
            vec![0; (cw / 2) * (ch / 2)],
        ];
        if color.has_alpha() {
            self.alpha = Some(vec![0; cw * ch]);
        }
        self.pending = vec![[0; 3]; self.width.div_ceil(2)];
        self.settings = Some(settings);
        Ok(())
    }

    fn row(&mut self, pixels: &[u8]) -> std::io::Result<()> {
        if self.settings.is_none() || self.row >= self.height {
            return Err(std::io::Error::other("a row outside the image"));
        }
        let max = (1i64 << self.bit_depth()) - 1;
        let mid = 1i64 << (self.bit_depth() - 1);
        let chroma_width = self.width.div_ceil(2);
        let stride = self
            .settings
            .as_ref()
            .map_or(0, |settings| settings.coded_width);
        let line = self.row * stride;
        let mut sums = vec![[0i64; 3]; chroma_width];
        for x in 0..self.width {
            let (rgb, alpha) = self.pixel(pixels, x);
            let luma = (KR * rgb[0] + KG * rgb[1] + KB * rgb[2] + (1 << 31)) >> 32;
            self.planes[0][line + x] = luma.clamp(0, max) as u16;
            if let (Some(plane), Some(alpha)) = (self.alpha.as_mut(), alpha) {
                let value = if self.deep {
                    (u64::from(alpha) * 1023 + 32767) / 65535
                } else {
                    u64::from(alpha)
                };
                plane[line + x] = value as u16;
            }
            let sum = &mut sums[x / 2];
            for (total, &value) in sum.iter_mut().zip(&rgb) {
                // An odd width's last pixel counts twice.
                *total += if x + 1 == self.width && x % 2 == 0 {
                    2 * value
                } else {
                    value
                };
            }
        }
        let last = self.row + 1 == self.height;
        if self.row % 2 == 0 && !last {
            self.pending = sums;
        } else {
            if self.row % 2 == 0 {
                // An odd height's last row pairs with itself.
                self.pending = sums.clone();
            }
            let chroma_row = self.row / 2;
            for (x, (pair, upper)) in sums.iter().zip(&self.pending).enumerate() {
                let total = [pair[0] + upper[0], pair[1] + upper[1], pair[2] + upper[2]];
                let chroma = |weights: &[i64; 3]| -> u16 {
                    let value = (weights[0] * total[0]
                        + weights[1] * total[1]
                        + weights[2] * total[2]
                        + (1 << 33))
                        >> 34;
                    (value + mid).clamp(0, max) as u16
                };
                self.planes[1][chroma_row * stride / 2 + x] = chroma(&CB);
                self.planes[2][chroma_row * stride / 2 + x] = chroma(&CR);
            }
        }
        self.row += 1;
        if self.row == self.height {
            self.finish()?;
        }
        Ok(())
    }

    fn ycbcr_row(&mut self, luma: &[u8], chroma: Option<(&[u8], &[u8])>) -> std::io::Result<()> {
        if self.settings.is_none() || !self.ycbcr || self.row >= self.height {
            return Err(std::io::Error::other("a YCbCr row the sink did not take"));
        }
        let stride = self
            .settings
            .as_ref()
            .map_or(0, |settings| settings.coded_width);
        for (sample, &value) in self.planes[0][self.row * stride..]
            .iter_mut()
            .zip(&luma[..self.width])
        {
            *sample = u16::from(value);
        }
        if let Some((blue, red)) = chroma {
            let chroma_width = self.width.div_ceil(2);
            let at = (self.row / 2) * (stride / 2);
            for x in 0..chroma_width {
                self.planes[1][at + x] = u16::from(blue[x]);
                self.planes[2][at + x] = u16::from(red[x]);
            }
        }
        self.row += 1;
        if self.row == self.height {
            self.finish()?;
        }
        Ok(())
    }
}

/// A box: its size and type, then its body.
fn boxed(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 8);
    out.extend_from_slice(&((body.len() + 8) as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    out
}

/// A full box: version and flags before the body.
fn full(kind: &[u8; 4], version: u8, flags: u32, body: &[u8]) -> Vec<u8> {
    let mut with_header = Vec::with_capacity(body.len() + 4);
    with_header.push(version);
    with_header.extend_from_slice(&flags.to_be_bytes()[1..]);
    with_header.extend_from_slice(body);
    boxed(kind, &with_header)
}

/// An item's data in `mdat`: its NAL units, each after a 4-byte length.
fn item_data(units: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    for unit in units {
        out.extend_from_slice(&(unit.len() as u32).to_be_bytes());
        out.extend_from_slice(unit);
    }
    out
}

/// The whole file around the coded pictures.
fn container(
    settings: &Settings,
    (width, height): (usize, usize),
    picture: &encode::Encoded,
    alpha: Option<(&encode::Encoded, &Settings)>,
    icc_profile: Option<&[u8]>,
    exif: Option<&[u8]>,
) -> Vec<u8> {
    // Items: 1 the picture, 2 its alpha, 3 its Exif.
    let mut items: Vec<(u16, [u8; 4], Vec<u8>)> = vec![(1, *b"hvc1", item_data(&[&picture.slice]))];
    if let Some((coded, _)) = alpha {
        items.push((2, *b"hvc1", item_data(&[&coded.slice])));
    }
    if let Some(exif) = exif {
        // The offset to the TIFF header, then the TIFF structure.
        let mut data = vec![0, 0, 0, 0];
        data.extend_from_slice(exif);
        items.push((3, *b"Exif", data));
    }
    let ftyp = boxed(b"ftyp", b"heic\0\0\0\0mif1heicmiaf");
    let hdlr = full(b"hdlr", 0, 0, b"\0\0\0\0pict\0\0\0\0\0\0\0\0\0\0\0\0\0");
    let pitm = full(b"pitm", 0, 0, &1u16.to_be_bytes());
    let mut infos = Vec::new();
    infos.extend_from_slice(&(items.len() as u16).to_be_bytes());
    for (id, kind, _) in &items {
        let mut body = Vec::new();
        body.extend_from_slice(&id.to_be_bytes());
        body.extend_from_slice(&0u16.to_be_bytes());
        body.extend_from_slice(kind);
        body.push(0); // empty name
        // The alpha picture is hidden: it is not an image to show.
        let flags = u32::from(*id == 2);
        infos.extend_from_slice(&full(b"infe", 2, flags, &body));
    }
    let iinf = full(b"iinf", 0, 0, &infos);
    // References: alpha is auxiliary to the picture, Exif describes it.
    let mut references = Vec::new();
    if alpha.is_some() {
        references.extend_from_slice(&boxed(b"auxl", &[0, 2, 0, 1, 0, 1]));
    }
    if exif.is_some() {
        references.extend_from_slice(&boxed(b"cdsc", &[0, 3, 0, 1, 0, 1]));
    }
    let iref = (!references.is_empty()).then(|| full(b"iref", 0, 0, &references));
    // Properties: 1 hvcC, 2 colr nclx, 3 ispe, 4 pixi, then the profile,
    // alpha's hvcC, its pixi, and its auxC.
    let mut properties: Vec<Vec<u8>> = Vec::new();
    properties.push(boxed(b"hvcC", &picture.hvcc(settings)));
    let (primaries, transfer, matrix) = settings.colour;
    let mut nclx = b"nclx".to_vec();
    nclx.extend_from_slice(&u16::from(primaries).to_be_bytes());
    nclx.extend_from_slice(&u16::from(transfer).to_be_bytes());
    nclx.extend_from_slice(&u16::from(matrix).to_be_bytes());
    nclx.push(if settings.full_range { 0x80 } else { 0 });
    properties.push(boxed(b"colr", &nclx));
    let mut ispe = Vec::new();
    ispe.extend_from_slice(&(settings.width as u32).to_be_bytes());
    ispe.extend_from_slice(&(settings.height as u32).to_be_bytes());
    properties.push(full(b"ispe", 0, 0, &ispe));
    let depth = settings.bit_depth as u8;
    properties.push(full(b"pixi", 0, 0, &[3, depth, depth, depth]));
    let mut picture_properties = vec![(1u8, true), (2, false), (3, false), (4, false)];
    if let Some(profile) = icc_profile {
        let mut prof = b"prof".to_vec();
        prof.extend_from_slice(profile);
        properties.push(boxed(b"colr", &prof));
        picture_properties.push((properties.len() as u8, false));
    }
    let mut alpha_properties = Vec::new();
    if let Some((coded, alpha_settings)) = alpha {
        properties.push(boxed(b"hvcC", &coded.hvcc(alpha_settings)));
        alpha_properties.push((properties.len() as u8, true));
        alpha_properties.push((3, false));
        properties.push(full(b"pixi", 0, 0, &[1, depth]));
        alpha_properties.push((properties.len() as u8, false));
        properties.push(full(b"auxC", 0, 0, b"urn:mpeg:hevc:2015:auxid:1\0"));
        alpha_properties.push((properties.len() as u8, true));
    }
    // The clean aperture of an odd size: the coded picture's left and top
    // `width` by `height` (centred half a sample up and left).
    if (width, height) != (settings.width, settings.height) {
        let mut clap = Vec::with_capacity(32);
        let offset = |extra: usize| -> u32 { (-(extra as i32)) as u32 };
        for value in [
            width as u32,
            1,
            height as u32,
            1,
            offset(settings.width - width),
            2,
            offset(settings.height - height),
            2,
        ] {
            clap.extend_from_slice(&value.to_be_bytes());
        }
        properties.push(boxed(b"clap", &clap));
        let index = properties.len() as u8;
        picture_properties.push((index, true));
        if alpha.is_some() {
            alpha_properties.push((index, true));
        }
    }
    let ipco = boxed(b"ipco", &properties.concat());
    let mut associations = Vec::new();
    let associated: Vec<(u16, &Vec<(u8, bool)>)> = std::iter::once((1, &picture_properties))
        .chain(alpha.map(|_| (2, &alpha_properties)))
        .collect();
    associations.extend_from_slice(&(associated.len() as u32).to_be_bytes());
    for (id, list) in associated {
        associations.extend_from_slice(&id.to_be_bytes());
        associations.push(list.len() as u8);
        for &(index, essential) in list {
            associations.push(index | if essential { 0x80 } else { 0 });
        }
    }
    let ipma = full(b"ipma", 0, 0, &associations);
    let iprp = boxed(b"iprp", &[ipco, ipma].concat());
    // Locations: 4-byte offsets and lengths, filled once `meta`'s size is
    // known (it does not depend on them).
    let iloc_for = |offsets: &[u32]| -> Vec<u8> {
        let mut body = vec![0x44, 0x00];
        body.extend_from_slice(&(items.len() as u16).to_be_bytes());
        for ((id, _, data), &offset) in items.iter().zip(offsets) {
            body.extend_from_slice(&id.to_be_bytes());
            body.extend_from_slice(&0u16.to_be_bytes()); // data reference
            body.extend_from_slice(&1u16.to_be_bytes()); // one extent
            body.extend_from_slice(&offset.to_be_bytes());
            body.extend_from_slice(&(data.len() as u32).to_be_bytes());
        }
        full(b"iloc", 0, 0, &body)
    };
    let meta_for = |iloc: Vec<u8>| -> Vec<u8> {
        let mut body = Vec::new();
        for part in [&hdlr, &pitm, &iloc, &iinf] {
            body.extend_from_slice(part);
        }
        if let Some(iref) = &iref {
            body.extend_from_slice(iref);
        }
        body.extend_from_slice(&iprp);
        full(b"meta", 0, 0, &body)
    };
    let placeholder = meta_for(iloc_for(&vec![0; items.len()]));
    let mut offset = (ftyp.len() + placeholder.len() + 8) as u32;
    let mut offsets = Vec::with_capacity(items.len());
    for (_, _, data) in &items {
        offsets.push(offset);
        offset += data.len() as u32;
    }
    let meta = meta_for(iloc_for(&offsets));
    let data: Vec<u8> = items
        .iter()
        .flat_map(|(_, _, data)| data.iter().copied())
        .collect();
    let mut file = ftyp;
    file.extend_from_slice(&meta);
    file.extend_from_slice(&boxed(b"mdat", &data));
    file
}
