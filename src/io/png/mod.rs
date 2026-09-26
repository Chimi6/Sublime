//! PNG: a reader into the image hub and a writer from it.

pub mod reader;
pub mod writer;

pub use reader::{PngError, PngNotes, read_png, read_png_from};
pub use writer::write_png;

/// The zlib stream around PNG's deflate data: a two-byte header and an
/// Adler-32 trailer. `adler32_update` carries the sum across pieces;
/// pass `1` to start.
pub fn adler32_update(previous: u32, bytes: &[u8]) -> u32 {
    const MODULUS: u32 = 65_521;
    // The most bytes the sums can take before a reduction, as zlib
    // derives it; the sums are the same integers whatever the order of
    // the additions, so the bound holds for the blocked form too.
    const RUN: usize = 5552;
    const BLOCK: usize = 32;
    const LANES: usize = 8;
    let mut a = previous & 0xffff;
    let mut b = previous >> 16;
    for run in bytes.chunks(RUN) {
        let mut blocks = run.chunks_exact(BLOCK);
        for block in &mut blocks {
            // Over a block, `a` gains the plain sum and `b` gains the
            // old `a` once per byte plus each byte weighted by how many
            // bytes follow it (inclusive). Eight lanes of each sum, as
            // arrays the compiler keeps in vector registers.
            let mut sums = [0u32; LANES];
            let mut weighted = [0u32; LANES];
            for (group, bytes) in block.chunks_exact(LANES).enumerate() {
                let weight_base = (BLOCK - group * LANES) as u32;
                for lane in 0..LANES {
                    let value = u32::from(bytes[lane]);
                    sums[lane] += value;
                    weighted[lane] += value * (weight_base - lane as u32);
                }
            }
            let sum: u32 = sums.iter().sum();
            let weighted: u32 = weighted.iter().sum();
            b += BLOCK as u32 * a + weighted;
            a += sum;
        }
        for byte in blocks.remainder() {
            a += u32::from(*byte);
            b += a;
        }
        a %= MODULUS;
        b %= MODULUS;
    }
    (b << 16) | a
}

pub fn adler32(bytes: &[u8]) -> u32 {
    adler32_update(1, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adler_matches_the_byte_loop_at_every_split() {
        let text: Vec<u8> = (0..20_000u32).map(|i| (i * 7 + i / 3) as u8).collect();
        let mut a: u32 = 1;
        let mut b: u32 = 0;
        for byte in &text {
            a = (a + u32::from(*byte)) % 65_521;
            b = (b + a) % 65_521;
        }
        let expected = (b << 16) | a;
        assert_eq!(adler32(&text), expected);
        for split in [0usize, 1, 31, 32, 33, 5551, 5552, 5553, 12_345, 19_999] {
            assert_eq!(
                adler32_update(adler32(&text[..split]), &text[split..]),
                expected
            );
        }
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }
}
