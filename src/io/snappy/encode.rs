//! Snappy block encoding. For now blocks are written as literals only,
//! which every Snappy decoder accepts; a matching compressor is a later
//! step for smaller files.

/// Encodes `data` as one block holding a single literal.
pub fn encode_literal_block(data: &[u8], out: &mut Vec<u8>) {
    crate::io::protobuf::tree::write_varint(out, data.len() as u64);
    if data.is_empty() {
        return;
    }
    let length = data.len() - 1;
    if length < 60 {
        out.push((length as u8) << 2);
    } else if length < 1 << 8 {
        out.push(60 << 2);
        out.push(length as u8);
    } else if length < 1 << 16 {
        out.push(61 << 2);
        out.extend_from_slice(&(length as u16).to_le_bytes());
    } else if length < 1 << 24 {
        out.push(62 << 2);
        out.extend_from_slice(&(length as u32).to_le_bytes()[..3]);
    } else {
        out.push(63 << 2);
        out.extend_from_slice(&(length as u32).to_le_bytes());
    }
    out.extend_from_slice(data);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::snappy::decompress_block;

    #[test]
    fn literal_blocks_decode_back() {
        for size in [0usize, 1, 59, 60, 61, 255, 256, 65535, 65536, 70000] {
            let data: Vec<u8> = (0..size).map(|index| (index * 7) as u8).collect();
            let mut block = Vec::new();
            encode_literal_block(&data, &mut block);
            let mut back = Vec::new();
            decompress_block(&block, &mut back, 1 << 20).unwrap();
            assert_eq!(back, data, "size {size}");
        }
    }
}
