//! HEVC (H.265) intra picture decoding, for HEIF images: the parameter
//! sets, CABAC, the coding tree, intra prediction, the inverse transforms,
//! and the deblocking and sample adaptive offset filters, for 8- to 12-bit
//! monochrome, 4:2:0, 4:2:2, and 4:4:4 pictures of intra slices (every
//! still image profile). Predicted (P and B) slices are refused.

mod bits;
mod cabac;
mod decode;
mod filter;
mod intra;
mod params;
mod residual;
mod sample;
mod tables;
mod transform;

pub use decode::{DoneBand, Picture, Workspace};
pub use params::VuiColour;
pub use sample::Samples;

use std::collections::HashMap;

use bits::Nal;
use params::{Pps, SliceHeader, Sps};

/// Why a bitstream does not decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HevcError(pub String);

impl HevcError {
    pub fn new(message: &str) -> HevcError {
        HevcError(message.to_string())
    }
}

impl std::fmt::Display for HevcError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "HEVC: {}", self.0)
    }
}

impl std::error::Error for HevcError {}

const VPS: u8 = 32;
const SPS: u8 = 33;
const PPS: u8 = 34;

/// Decodes the first picture of a sequence of NAL units (each without a
/// start code or length prefix).
pub fn decode_picture<'a>(nals: impl IntoIterator<Item = &'a [u8]>) -> Result<Picture, HevcError> {
    let threads = std::thread::available_parallelism().map_or(1, |count| count.get());
    decode_picture_in(nals, &mut Workspace::default(), threads)
}

/// Decodes the first picture, its large buffers from `workspace` (one per
/// thread decoding a grid's tiles). A picture of one slice segment with
/// wavefront rows decodes its rows on up to `threads` threads.
pub fn decode_picture_in<'a>(
    nals: impl IntoIterator<Item = &'a [u8]>,
    workspace: &mut Workspace,
    threads: usize,
) -> Result<Picture, HevcError> {
    let mut sps_sets: HashMap<u32, Sps> = HashMap::new();
    let mut pps_sets: HashMap<u32, Pps> = HashMap::new();
    let mut previous: Option<SliceHeader> = None;
    // The first picture's slice segments, each with its parameter sets.
    let mut segments: Vec<(Nal<'a>, SliceHeader, Sps, Pps)> = Vec::new();
    for bytes in nals {
        let nal = Nal::parse(bytes)?;
        match nal.kind {
            VPS => {}
            SPS => {
                let sps = Sps::parse(&nal.rbsp)?;
                sps_sets.insert(sps.id, sps);
            }
            PPS => {
                let pps = Pps::parse(&nal.rbsp)?;
                pps_sets.insert(pps.id, pps);
            }
            kind if kind <= 21 && !(10..=15).contains(&kind) => {
                let lookup = |id: u32| {
                    let pps = pps_sets.get(&id)?;
                    let sps = sps_sets.get(&pps.sps_id)?;
                    Some((sps.clone(), pps.clone()))
                };
                let (header, sps, pps) =
                    SliceHeader::parse(&nal.rbsp, kind, lookup, previous.as_ref())?;
                if header.first_in_picture && !segments.is_empty() {
                    // The next picture: the first is done.
                    break;
                }
                if !header.dependent {
                    previous = Some(header.clone());
                }
                segments.push((nal, header, sps, pps));
            }
            _ => {}
        }
    }
    let Some((_, _, sps, pps)) = segments.first() else {
        return Err(HevcError::new("no picture in the bitstream"));
    };
    let mut decoder = decode::Decoder::reusing(sps.clone(), pps.clone(), workspace, threads, true)?;
    let wavefront_rows = segments.len() == 1
        && threads > 1
        && decoder.pps.entropy_sync
        && !decoder.pps.tiles
        && decoder.height_ctbs > 1
        && segments[0].1.address == 0
        && segments[0].1.entry_points.len() + 1 == decoder.height_ctbs;
    if wavefront_rows {
        let (nal, header, _, _) = segments.remove(0);
        // Entry points count bytes as sent: each row's start in the RBSP.
        let mut raw = nal.raw_position(header.data_offset);
        let mut starts = vec![header.data_offset];
        for &size in &header.entry_points {
            raw += size as usize;
            starts.push(nal.rbsp_position(raw));
        }
        decoder.slice_in_rows(header, &nal.rbsp, &starts, threads)?;
    } else {
        for (nal, header, _, _) in segments {
            decoder.slice(header, &nal.rbsp)?;
        }
    }
    if !decoder.complete() {
        return Err(HevcError::new(
            "a picture whose data ends before its last block",
        ));
    }
    Ok(decoder.finish_into(workspace))
}

/// A picture's size and format, told before its first band.
#[derive(Clone, Debug)]
pub struct Shape {
    pub width: usize,
    pub height: usize,
    /// 0 monochrome, 1 4:2:0, 2 4:2:2, 3 4:4:4.
    pub chroma_format: u32,
    pub bit_depth_luma: u32,
    pub bit_depth_chroma: u32,
    /// The conformance window: left, right, top, bottom, in luma samples.
    pub crop: [u32; 4],
    pub vui_colour: Option<VuiColour>,
}

/// Decodes the first picture and hands it on a CTB row at a time, each
/// band filtered and in order, after `shape`. A picture of one slice
/// segment with wavefront rows decodes its rows on up to `threads` threads
/// into a few band buffers and never holds the whole picture; any other is
/// decoded whole and handed on from that.
pub fn decode_picture_by_bands<'a>(
    nals: impl IntoIterator<Item = &'a [u8]>,
    threads: usize,
    shape: &mut dyn FnMut(&Shape) -> Result<(), HevcError>,
    band: &mut dyn FnMut(&DoneBand) -> Result<(), HevcError>,
) -> Result<(), HevcError> {
    let mut sps_sets: HashMap<u32, Sps> = HashMap::new();
    let mut pps_sets: HashMap<u32, Pps> = HashMap::new();
    let mut previous: Option<SliceHeader> = None;
    let mut segments: Vec<(Nal<'a>, SliceHeader, Sps, Pps)> = Vec::new();
    let mut units: Vec<&'a [u8]> = Vec::new();
    for bytes in nals {
        units.push(bytes);
        let nal = Nal::parse(bytes)?;
        match nal.kind {
            SPS => {
                let sps = Sps::parse(&nal.rbsp)?;
                sps_sets.insert(sps.id, sps);
            }
            PPS => {
                let pps = Pps::parse(&nal.rbsp)?;
                pps_sets.insert(pps.id, pps);
            }
            kind if kind <= 21 && !(10..=15).contains(&kind) => {
                let lookup = |id: u32| {
                    let pps = pps_sets.get(&id)?;
                    let sps = sps_sets.get(&pps.sps_id)?;
                    Some((sps.clone(), pps.clone()))
                };
                let (header, sps, pps) =
                    SliceHeader::parse(&nal.rbsp, kind, lookup, previous.as_ref())?;
                if header.first_in_picture && !segments.is_empty() {
                    break;
                }
                if !header.dependent {
                    previous = Some(header.clone());
                }
                segments.push((nal, header, sps, pps));
            }
            _ => {}
        }
    }
    let Some((_, first, sps, pps)) = segments.first() else {
        return Err(HevcError::new("no picture in the bitstream"));
    };
    let ctb = 1usize << sps.log2_ctb;
    let height_ctbs = (sps.height as usize).div_ceil(ctb);
    let streams = segments.len() == 1
        && threads > 1
        && pps.entropy_sync
        && !pps.tiles
        && height_ctbs > 1
        && first.address == 0
        && first.entry_points.len() + 1 == height_ctbs;
    if !streams {
        let picture = decode_picture_in(units, &mut Workspace::default(), threads)?;
        shape(&Shape {
            width: picture.width,
            height: picture.height,
            chroma_format: picture.chroma_format,
            bit_depth_luma: picture.bit_depth_luma,
            bit_depth_chroma: picture.bit_depth_chroma,
            crop: picture.crop,
            vui_colour: picture.vui_colour,
        })?;
        let sub_y = u32::from(picture.chroma_format == 1);
        let mut top = 0;
        while top < picture.height {
            let rows = ctb.min(picture.height - top);
            let planes = picture
                .planes
                .iter()
                .enumerate()
                .map(|(component, samples)| {
                    let shift = if component == 0 { 0 } else { sub_y };
                    let stride = picture.sizes[component].0;
                    samples.slice_of((top >> shift) * stride, (rows >> shift) * stride)
                })
                .collect();
            band(&DoneBand { top, rows, planes })?;
            top += rows;
        }
        return Ok(());
    }
    let (nal, header, sps, pps) = segments.remove(0);
    let mut decoder =
        decode::Decoder::reusing(sps.clone(), pps, &mut Workspace::default(), threads, false)?;
    shape(&Shape {
        width: decoder.width,
        height: decoder.height,
        chroma_format: decoder.chroma,
        bit_depth_luma: sps.bit_depth_luma,
        bit_depth_chroma: sps.bit_depth_chroma,
        crop: sps.crop,
        vui_colour: sps.vui_colour,
    })?;
    let mut raw = nal.raw_position(header.data_offset);
    let mut starts = vec![header.data_offset];
    for &size in &header.entry_points {
        raw += size as usize;
        starts.push(nal.rbsp_position(raw));
    }
    decoder.stream_in_rows(header, &nal.rbsp, &starts, threads, band)?;
    if !decoder.complete() {
        return Err(HevcError::new(
            "a picture whose data ends before its last block",
        ));
    }
    Ok(())
}

/// A picture decoded by bands and put back together: what the streamed
/// path hands on, as one picture (for checking it against the whole one).
#[doc(hidden)]
pub fn decode_picture_assembled<'a>(
    nals: impl IntoIterator<Item = &'a [u8]>,
    threads: usize,
) -> Result<Picture, HevcError> {
    let assembled = std::cell::RefCell::new(None::<Picture>);
    let mut shape_of = |shape: &Shape| -> Result<(), HevcError> {
        let (sub_x, sub_y) = match shape.chroma_format {
            1 => (1, 1),
            2 => (1, 0),
            _ => (0, 0),
        };
        let mut sizes = vec![(shape.width, shape.height)];
        if shape.chroma_format != 0 {
            sizes.push((shape.width >> sub_x, shape.height >> sub_y));
            sizes.push((shape.width >> sub_x, shape.height >> sub_y));
        }
        let planes = sizes
            .iter()
            .enumerate()
            .map(|(component, &(w, h))| {
                let depth = if component == 0 {
                    shape.bit_depth_luma
                } else {
                    shape.bit_depth_chroma
                };
                Samples::new(depth, w * h)
            })
            .collect();
        *assembled.borrow_mut() = Some(Picture {
            width: shape.width,
            height: shape.height,
            chroma_format: shape.chroma_format,
            bit_depth_luma: shape.bit_depth_luma,
            bit_depth_chroma: shape.bit_depth_chroma,
            planes,
            sizes,
            crop: shape.crop,
            vui_colour: shape.vui_colour,
        });
        Ok(())
    };
    let mut band_of = |band: &DoneBand| -> Result<(), HevcError> {
        let mut slot = assembled.borrow_mut();
        let Some(picture) = slot.as_mut() else {
            return Ok(());
        };
        let sub_y = u32::from(picture.chroma_format == 1);
        for (component, samples) in band.planes.iter().enumerate() {
            let shift = if component == 0 { 0 } else { sub_y };
            let stride = picture.sizes[component].0;
            let count = (band.rows >> shift) * stride;
            picture.planes[component].copy_from((band.top >> shift) * stride, samples, 0, count);
        }
        Ok(())
    };
    decode_picture_by_bands(nals, threads, &mut shape_of, &mut band_of)?;
    assembled
        .into_inner()
        .ok_or_else(|| HevcError::new("no picture"))
}
