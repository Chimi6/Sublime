//! Bits of an HEVC bitstream: NAL units with their emulation prevention
//! bytes removed (the RBSP), read MSB first with the Exp-Golomb codes of
//! the parameter sets and slice headers (H.265 7.2, 9.2).

use super::HevcError;

/// A NAL unit: its type and its payload as an RBSP (the two header bytes
/// and every emulation prevention byte, the `03` of `00 00 03`, gone).
pub struct Nal<'a> {
    pub kind: u8,
    pub rbsp: std::borrow::Cow<'a, [u8]>,
}

impl<'a> Nal<'a> {
    /// A NAL unit from its bytes, header included.
    pub fn parse(bytes: &'a [u8]) -> Result<Nal<'a>, HevcError> {
        if bytes.len() < 2 {
            return Err(HevcError::new("a NAL unit shorter than its header"));
        }
        let kind = (bytes[0] >> 1) & 0x3F;
        let payload = &bytes[2..];
        // Most payloads hold no emulation prevention byte: borrowed whole.
        if !payload.windows(3).any(|window| window == [0, 0, 3]) {
            return Ok(Nal {
                kind,
                rbsp: std::borrow::Cow::Borrowed(payload),
            });
        }
        let mut rbsp = Vec::with_capacity(bytes.len());
        let mut zeros = 0;
        for &byte in &bytes[2..] {
            if zeros >= 2 && byte == 3 {
                zeros = 0;
                continue;
            }
            zeros = if byte == 0 { zeros + 1 } else { 0 };
            rbsp.push(byte);
        }
        Ok(Nal {
            kind,
            rbsp: std::borrow::Cow::Owned(rbsp),
        })
    }
}

/// Reads bits MSB first.
pub struct BitReader<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> BitReader<'a> {
        BitReader { data, position: 0 }
    }

    pub fn bit(&mut self) -> Result<u32, HevcError> {
        let byte = self
            .data
            .get(self.position >> 3)
            .ok_or_else(|| HevcError::new("a header that runs past its NAL unit"))?;
        let bit = (byte >> (7 - (self.position & 7))) & 1;
        self.position += 1;
        Ok(u32::from(bit))
    }

    pub fn flag(&mut self) -> Result<bool, HevcError> {
        Ok(self.bit()? == 1)
    }

    /// `count` bits (up to 32) as a number.
    pub fn bits(&mut self, count: u32) -> Result<u32, HevcError> {
        let mut value = 0u32;
        for _ in 0..count {
            value = (value << 1) | self.bit()?;
        }
        Ok(value)
    }

    pub fn skip(&mut self, count: usize) -> Result<(), HevcError> {
        if (self.position + count).div_ceil(8) > self.data.len() {
            return Err(HevcError::new("a header that runs past its NAL unit"));
        }
        self.position += count;
        Ok(())
    }

    /// `ue(v)`: an unsigned Exp-Golomb code.
    pub fn ue(&mut self) -> Result<u32, HevcError> {
        let mut zeros = 0;
        while self.bit()? == 0 {
            zeros += 1;
            if zeros > 31 {
                return Err(HevcError::new("an Exp-Golomb code over 32 bits"));
            }
        }
        if zeros == 0 {
            return Ok(0);
        }
        let rest = self.bits(zeros)?;
        Ok(((1u64 << zeros) - 1 + u64::from(rest)).min(u64::from(u32::MAX)) as u32)
    }

    /// `se(v)`: a signed Exp-Golomb code.
    pub fn se(&mut self) -> Result<i32, HevcError> {
        let code = self.ue()?;
        let magnitude = code.div_ceil(2) as i32;
        Ok(if code & 1 == 1 { magnitude } else { -magnitude })
    }

    /// The byte where the data after a byte-aligned header begins.
    pub fn byte_position(&self) -> usize {
        self.position.div_ceil(8)
    }
}
