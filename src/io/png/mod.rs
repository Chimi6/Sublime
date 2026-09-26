//! PNG: a reader into the image hub and a writer from it.

pub mod reader;
pub mod writer;

pub use reader::{PngError, PngNotes, RowSink, RowsError, read_png, read_png_from, read_png_rows};
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
    // Lane masks: the even bytes of a word and the odd bytes, each in a
    // sixteen-bit lane.
    const EVEN: u64 = 0x00ff_00ff_00ff_00ff;
    // Byte indexes as weights: the even bytes of a word (0, 2, 4, 6) and
    // the odd bytes (1, 3, 5, 7), laid out so that a 64-bit multiply puts
    // the dot product of the lanes and the weights in the top lane: the
    // coefficient of 2^48 in the product is l0*w3 + l1*w2 + l2*w1 + l3*w0.
    // Lanes accumulate over eight words with weights up to eight, so
    // every lane's own sum stays under 2^16 and nothing carries up.
    const EVEN_WEIGHTS: u64 = 0x0000_0002_0004_0006;
    const ODD_WEIGHTS: u64 = 0x0001_0003_0005_0007;
    const ONES: u64 = 0x0001_0001_0001_0001;
    let mut a = previous & 0xffff;
    let mut b = previous >> 16;
    for run in bytes.chunks(RUN) {
        // Words are taken in blocks of eight. Lanes accumulate the even
        // and odd bytes, and a second pair accumulates them weighted by
        // the words still to come, so the block's contribution to `b`
        // comes out of three dot products and no per-word chain longer
        // than an add.
        const BLOCK: usize = 64;
        let mut blocks = run.chunks_exact(BLOCK);
        for block in &mut blocks {
            let mut even_lanes: u64 = 0;
            let mut odd_lanes: u64 = 0;
            let mut weighted_lanes: u64 = 0;
            for (index, word) in block.chunks_exact(8).enumerate() {
                let word = u64::from_le_bytes([
                    word[0], word[1], word[2], word[3], word[4], word[5], word[6], word[7],
                ]);
                let even = word & EVEN;
                let odd = (word >> 8) & EVEN;
                even_lanes += even;
                odd_lanes += odd;
                weighted_lanes += (even + odd) * (8 - index as u64);
            }
            // Byte i (0 to 7) of the word with `n` words after it in the
            // block weighs 8(n + 1) - i: the weighted lanes give the
            // first part, the index dot products the part to subtract.
            let sum = ((even_lanes + odd_lanes).wrapping_mul(ONES) >> 48) as u32;
            // The weighted lanes can sum past sixteen bits, so they are
            // added lane by lane rather than through the multiply.
            let by_word = (weighted_lanes & 0xffff) as u32
                + ((weighted_lanes >> 16) & 0xffff) as u32
                + ((weighted_lanes >> 32) & 0xffff) as u32
                + (weighted_lanes >> 48) as u32;
            let in_word = (even_lanes.wrapping_mul(EVEN_WEIGHTS) >> 48) as u32
                + (odd_lanes.wrapping_mul(ODD_WEIGHTS) >> 48) as u32;
            b += (BLOCK as u32) * a + 8 * by_word - in_word;
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
        // Bytes at the top of the range push every lane to its bound.
        let bright = vec![255u8; 6000];
        let mut a: u32 = 1;
        let mut b: u32 = 0;
        for byte in &bright {
            a = (a + u32::from(*byte)) % 65_521;
            b = (b + a) % 65_521;
        }
        assert_eq!(adler32(&bright), (b << 16) | a);
    }
}
