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
mod tables;
mod transform;

pub use decode::Picture;
pub use params::VuiColour;

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
    let mut sps_sets: HashMap<u32, Sps> = HashMap::new();
    let mut pps_sets: HashMap<u32, Pps> = HashMap::new();
    let mut decoder: Option<decode::Decoder> = None;
    let mut previous: Option<SliceHeader> = None;
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
                if header.first_in_picture && decoder.is_some() {
                    // The next picture: the first is done.
                    break;
                }
                if decoder.is_none() {
                    decoder = Some(decode::Decoder::new(sps, pps)?);
                }
                let picture = decoder
                    .as_mut()
                    .ok_or_else(|| HevcError::new("no picture"))?;
                if !header.dependent {
                    previous = Some(header.clone());
                }
                picture.slice(header, &nal.rbsp)?;
            }
            _ => {}
        }
    }
    let decoder = decoder.ok_or_else(|| HevcError::new("no picture in the bitstream"))?;
    if !decoder.complete() {
        return Err(HevcError::new(
            "a picture whose data ends before its last block",
        ));
    }
    Ok(decoder.finish())
}
