//! CRC-32 (IEEE, reflected), as ZIP, PNG, and gzip use it.

const fn make_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut index = 0usize;
    while index < 256 {
        let mut value = index as u32;
        let mut bit = 0;
        while bit < 8 {
            value = if value & 1 == 1 {
                0xEDB8_8320 ^ (value >> 1)
            } else {
                value >> 1
            };
            bit += 1;
        }
        table[index] = value;
        index += 1;
    }
    table
}

const TABLE: [u32; 256] = make_table();

/// Continues a checksum: pass `0` to start, and the previous result to
/// extend it over more bytes.
pub fn crc32_update(previous: u32, bytes: &[u8]) -> u32 {
    let mut value = !previous;
    for byte in bytes {
        let index = ((value ^ u32::from(*byte)) & 0xFF) as usize;
        value = TABLE[index] ^ (value >> 8);
    }
    !value
}

pub fn crc32(bytes: &[u8]) -> u32 {
    crc32_update(0, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_values() {
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32_update(crc32(b"1234"), b"56789"), 0xCBF4_3926);
    }
}
