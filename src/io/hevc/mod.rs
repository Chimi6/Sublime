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

pub use decode::{Picture, Workspace};
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
    let mut decoder = decode::Decoder::reusing(sps.clone(), pps.clone(), workspace, threads)?;
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
