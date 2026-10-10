//! The bitstream around the coded picture: Exp-Golomb fields, NAL units
//! with emulation prevention, the parameter sets, and the slice header
//! with its wavefront entry points (H.265 7.3).

use super::Settings;

/// RBSP bits, most significant first.
#[derive(Default)]
pub struct Rbsp {
    pub bytes: Vec<u8>,
    current: u32,
    count: u32,
}

impl Rbsp {
    pub fn bits(&mut self, value: u32, count: u32) {
        for index in (0..count).rev() {
            self.current = (self.current << 1) | ((value >> index) & 1);
            self.count += 1;
            if self.count == 8 {
                self.bytes.push(self.current as u8);
                self.current = 0;
                self.count = 0;
            }
        }
    }

    pub fn flag(&mut self, value: bool) {
        self.bits(u32::from(value), 1);
    }

    pub fn ue(&mut self, value: u32) {
        let coded = u64::from(value) + 1;
        let length = 64 - coded.leading_zeros();
        self.bits(0, length - 1);
        for index in (0..length).rev() {
            self.bits(((coded >> index) & 1) as u32, 1);
        }
    }

    pub fn se(&mut self, value: i32) {
        let mapped = if value > 0 {
            (2 * value - 1) as u32
        } else {
            (-2 * value) as u32
        };
        self.ue(mapped);
    }

    /// `rbsp_trailing_bits()` or `byte_alignment()`: a one, zeros to the byte.
    pub fn trailing(&mut self) {
        self.bits(1, 1);
        while self.count != 0 {
            self.bits(0, 1);
        }
    }
}

pub const VPS: u8 = 32;
pub const SPS: u8 = 33;
pub const PPS: u8 = 34;
/// An IDR picture (IDR_N_LP): no leading pictures.
pub const IDR: u8 = 20;

/// A NAL unit: its two-byte header, then the payload with an emulation
/// prevention byte before any 0, 1, 2, or 3 that follows two zeros.
pub fn nal(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + payload.len() / 64 + 2);
    out.push(kind << 1);
    out.push(1);
    escape_into(payload, &mut out, &mut 0);
    out
}

/// Appends `payload` escaped; `zeros` carries the run of zeros across
/// calls. Returns how many prevention bytes went in.
pub fn escape_into(payload: &[u8], out: &mut Vec<u8>, zeros: &mut u32) -> usize {
    let mut added = 0;
    for &byte in payload {
        if *zeros >= 2 && byte <= 3 {
            out.push(3);
            added += 1;
            *zeros = 0;
        }
        out.push(byte);
        *zeros = if byte == 0 { *zeros + 1 } else { 0 };
    }
    added
}

/// `profile_tier_level(1, 0)`: Main (or Main 10 above 8 bits) at the
/// level the picture size needs.
fn profile_tier_level(rbsp: &mut Rbsp, settings: &Settings) {
    let profile = if settings.bit_depth > 8 { 2 } else { 1 };
    rbsp.bits(0, 2); // profile space
    rbsp.bits(0, 1); // main tier
    rbsp.bits(profile, 5);
    // Compatible with its own profile, and Main 10 decoders take Main.
    let mut compatibility = 1u32 << (31 - profile);
    compatibility |= 1 << (31 - 2);
    rbsp.bits(compatibility, 32);
    rbsp.bits(1, 1); // progressive source
    rbsp.bits(0, 1); // interlaced
    rbsp.bits(0, 1); // non-packed constraint
    rbsp.bits(1, 1); // frame only
    // The 43 reserved bits, with Main 10's constraint flags (one picture
    // only, intra) left clear, and the inbld bit.
    rbsp.bits(0, 32);
    rbsp.bits(0, 12);
    rbsp.bits(level_idc(settings.width * settings.height), 8);
}

/// The lowest level whose largest picture holds `samples` (Table A.8),
/// times 30; the largest level past that.
fn level_idc(samples: usize) -> u32 {
    const LEVELS: [(usize, u32); 9] = [
        (36_864, 30),
        (122_880, 60),
        (245_760, 63),
        (552_960, 90),
        (983_040, 93),
        (2_228_224, 120),
        (8_912_896, 150),
        (35_651_584, 180),
        (usize::MAX, 186),
    ];
    LEVELS
        .iter()
        .find(|&&(largest, _)| samples <= largest)
        .map_or(186, |&(_, idc)| idc)
}

pub fn vps(settings: &Settings) -> Vec<u8> {
    let mut rbsp = Rbsp::default();
    rbsp.bits(0, 4); // id
    rbsp.bits(1, 1); // base layer internal
    rbsp.bits(1, 1); // base layer available
    rbsp.bits(0, 6); // max layers - 1
    rbsp.bits(0, 3); // max sub-layers - 1
    rbsp.bits(1, 1); // temporal id nesting
    rbsp.bits(0xFFFF, 16);
    profile_tier_level(&mut rbsp, settings);
    rbsp.flag(true); // sub-layer ordering info present
    rbsp.ue(0); // max dec pic buffering - 1
    rbsp.ue(0); // num reorder pics
    rbsp.ue(0); // max latency increase + 1
    rbsp.bits(0, 6); // max layer id
    rbsp.ue(0); // num layer sets - 1
    rbsp.flag(false); // timing info
    rbsp.flag(false); // extension
    rbsp.trailing();
    nal(VPS, &rbsp.bytes)
}

pub fn sps(settings: &Settings) -> Vec<u8> {
    let mut rbsp = Rbsp::default();
    rbsp.bits(0, 4); // VPS id
    rbsp.bits(0, 3); // max sub-layers - 1
    rbsp.bits(1, 1); // temporal id nesting
    profile_tier_level(&mut rbsp, settings);
    rbsp.ue(0); // id
    rbsp.ue(1); // 4:2:0
    rbsp.ue(settings.coded_width as u32);
    rbsp.ue(settings.coded_height as u32);
    let (right, bottom) = (
        settings.coded_width - settings.width,
        settings.coded_height - settings.height,
    );
    rbsp.flag(right > 0 || bottom > 0);
    if right > 0 || bottom > 0 {
        // In chroma samples.
        rbsp.ue(0);
        rbsp.ue((right / 2) as u32);
        rbsp.ue(0);
        rbsp.ue((bottom / 2) as u32);
    }
    rbsp.ue(settings.bit_depth - 8);
    rbsp.ue(settings.bit_depth - 8);
    rbsp.ue(0); // log2 max POC lsb - 4
    rbsp.flag(true); // sub-layer ordering info present
    rbsp.ue(0);
    rbsp.ue(0);
    rbsp.ue(0);
    rbsp.ue(0); // min CB 8
    rbsp.ue(settings.ctb_log2 - 3);
    rbsp.ue(0); // min TB 4
    rbsp.ue(settings.ctb_log2.min(5) - 2); // max TB: 32, or the CTB
    rbsp.ue(settings.transform_depth); // inter (unused)
    rbsp.ue(settings.transform_depth);
    rbsp.flag(false); // scaling lists
    rbsp.flag(false); // AMP
    rbsp.flag(settings.sao);
    rbsp.flag(false); // PCM
    rbsp.ue(0); // short-term reference picture sets
    rbsp.flag(false); // long-term reference pictures
    rbsp.flag(false); // temporal MVP
    rbsp.flag(true); // strong intra smoothing
    rbsp.flag(true); // VUI
    vui(&mut rbsp, settings);
    rbsp.flag(false); // extensions
    rbsp.trailing();
    nal(SPS, &rbsp.bytes)
}

/// The VUI: only the video signal type, so a decoder that does not read
/// the container's colour still converts right.
fn vui(rbsp: &mut Rbsp, settings: &Settings) {
    rbsp.flag(false); // aspect ratio
    rbsp.flag(false); // overscan
    rbsp.flag(true); // video signal type
    rbsp.bits(5, 3); // unspecified video format
    rbsp.flag(settings.full_range);
    rbsp.flag(true); // colour description
    rbsp.bits(u32::from(settings.colour.0), 8);
    rbsp.bits(u32::from(settings.colour.1), 8);
    rbsp.bits(u32::from(settings.colour.2), 8);
    rbsp.flag(false); // chroma location
    rbsp.flag(false); // neutral chroma
    rbsp.flag(false); // field sequence
    rbsp.flag(false); // frame field info
    rbsp.flag(false); // default display window
    rbsp.flag(false); // timing
    rbsp.flag(false); // bitstream restriction
}

pub fn pps(settings: &Settings) -> Vec<u8> {
    let mut rbsp = Rbsp::default();
    rbsp.ue(0); // id
    rbsp.ue(0); // SPS id
    rbsp.flag(false); // dependent slices
    rbsp.flag(false); // output flag present
    rbsp.bits(0, 3); // extra slice header bits
    rbsp.flag(settings.sign_hiding);
    rbsp.flag(false); // CABAC init present
    rbsp.ue(0);
    rbsp.ue(0);
    rbsp.se(settings.qp - 26);
    rbsp.flag(false); // constrained intra prediction
    rbsp.flag(settings.transform_skip);
    rbsp.flag(settings.qp_delta_depth.is_some());
    if let Some(depth) = settings.qp_delta_depth {
        rbsp.ue(depth);
    }
    rbsp.se(settings.chroma_qp_offset);
    rbsp.se(settings.chroma_qp_offset);
    rbsp.flag(false); // slice chroma QP offsets
    rbsp.flag(false); // weighted prediction
    rbsp.flag(false); // weighted bi-prediction
    rbsp.flag(false); // transquant bypass
    rbsp.flag(false); // tiles
    rbsp.flag(true); // entropy coding sync (wavefront rows)
    rbsp.flag(false); // loop filter across slices
    let offsets = settings.deblocking_offsets != (0, 0);
    rbsp.flag(!settings.deblocking || offsets); // deblocking control present
    if !settings.deblocking || offsets {
        rbsp.flag(false); // override enabled
        rbsp.flag(!settings.deblocking); // disabled
        if settings.deblocking {
            rbsp.se(settings.deblocking_offsets.0);
            rbsp.se(settings.deblocking_offsets.1);
        }
    }
    rbsp.flag(false); // scaling list data
    rbsp.flag(false); // lists modification
    rbsp.ue(0); // parallel merge level - 2
    rbsp.flag(false); // slice header extension
    rbsp.flag(false); // extensions
    rbsp.trailing();
    nal(PPS, &rbsp.bytes)
}

/// The picture's one slice: its header, then the substreams of its CTB
/// rows, each entry point counting the prevention bytes inside it.
pub fn slice(settings: &Settings, substreams: &[Vec<u8>]) -> Vec<u8> {
    // The data is escaped apart from the header: the header ends in its
    // alignment's one bit, so no run of zeros crosses into the data.
    let mut data = Vec::with_capacity(substreams.iter().map(Vec::len).sum::<usize>() * 101 / 100);
    let mut zeros = 0;
    let mut offsets = Vec::with_capacity(substreams.len());
    for substream in substreams {
        let start = data.len();
        escape_into(substream, &mut data, &mut zeros);
        offsets.push((data.len() - start) as u32);
    }
    offsets.pop();
    let mut rbsp = Rbsp::default();
    rbsp.flag(true); // first slice segment in the picture
    rbsp.flag(false); // no output of prior pictures
    rbsp.ue(0); // PPS id
    rbsp.ue(2); // I slice
    if settings.sao {
        rbsp.flag(true);
        rbsp.flag(true);
    }
    rbsp.se(0); // slice QP delta
    rbsp.ue(offsets.len() as u32);
    if !offsets.is_empty() {
        let largest = offsets.iter().copied().max().unwrap_or(1).max(1);
        let length = 32 - (largest - 1).leading_zeros().min(31);
        let length = length.max(1);
        rbsp.ue(length - 1);
        for &offset in &offsets {
            rbsp.bits(offset - 1, length);
        }
    }
    rbsp.trailing();
    let mut out = Vec::with_capacity(rbsp.bytes.len() + data.len() + 2);
    out.push(IDR << 1);
    out.push(1);
    let mut zeros = 0;
    escape_into(&rbsp.bytes, &mut out, &mut zeros);
    out.extend_from_slice(&data);
    out
}

/// The `hvcC` record (ISO/IEC 14496-15 8.3.3) of the parameter sets.
pub fn hvcc(settings: &Settings, vps: &[u8], sps: &[u8], pps: &[u8]) -> Vec<u8> {
    let profile = if settings.bit_depth > 8 { 2u8 } else { 1 };
    let mut out = vec![1u8, profile];
    let compatibility = (1u32 << (31 - profile)) | (1 << (31 - 2));
    out.extend_from_slice(&compatibility.to_be_bytes());
    // progressive and frame-only, the rest of the 48 constraint bits clear
    out.extend_from_slice(&[0x90, 0, 0, 0, 0, 0]);
    out.push(level_idc(settings.width * settings.height) as u8);
    out.extend_from_slice(&[0xF0, 0x00]); // min spatial segmentation 0
    out.push(0xFC); // parallelism unknown
    out.push(0xFC | 1); // 4:2:0
    out.push(0xF8 | (settings.bit_depth - 8) as u8);
    out.push(0xF8 | (settings.bit_depth - 8) as u8);
    out.extend_from_slice(&[0, 0]); // average frame rate
    // constant frame rate 0, one temporal layer, nested, 4-byte lengths
    out.push(0x0F);
    out.push(3);
    for (kind, unit) in [(VPS, vps), (SPS, sps), (PPS, pps)] {
        out.push(0x80 | kind);
        out.extend_from_slice(&1u16.to_be_bytes());
        out.extend_from_slice(&(unit.len() as u16).to_be_bytes());
        out.extend_from_slice(unit);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exp_golomb_codes() {
        let mut rbsp = Rbsp::default();
        for value in [0, 1, 2, 3, 7] {
            rbsp.ue(value);
        }
        rbsp.se(-1);
        rbsp.trailing();
        // 1 010 011 00100 0001000 011 1 + alignment
        assert_eq!(rbsp.bytes, vec![0b1010_0110, 0b0100_0001, 0b0000_1110]);
    }

    #[test]
    fn emulation_prevention() {
        let out = nal(SPS, &[0, 0, 1, 0, 0, 0, 5, 0, 0]);
        assert_eq!(out[2..], [0, 0, 3, 1, 0, 0, 3, 0, 5, 0, 0]);
    }
}
