//! Standard base64 (RFC 4648, with padding), for binary values in JSON.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

const INVALID: u8 = 0xFF;
const PAD: u8 = 0xFE;

const fn make_decode_table() -> [u8; 256] {
    let mut table = [INVALID; 256];
    let mut index = 0usize;
    while index < 64 {
        table[ALPHABET[index] as usize] = index as u8;
        index += 1;
    }
    table[b'=' as usize] = PAD;
    table
}

const DECODE: [u8; 256] = make_decode_table();

/// Appends the encoding to a byte buffer (the output is ASCII).
pub fn encode_into(bytes: &[u8], out: &mut Vec<u8>) {
    out.reserve(bytes.len().div_ceil(3) * 4);
    let mut chunks = bytes.chunks_exact(3);
    for chunk in &mut chunks {
        let triple = (u32::from(chunk[0]) << 16) | (u32::from(chunk[1]) << 8) | u32::from(chunk[2]);
        out.extend_from_slice(&[
            ALPHABET[(triple >> 18) as usize & 63],
            ALPHABET[(triple >> 12) as usize & 63],
            ALPHABET[(triple >> 6) as usize & 63],
            ALPHABET[triple as usize & 63],
        ]);
    }
    let rest = chunks.remainder();
    if rest.is_empty() {
        return;
    }
    let second = rest.get(1).copied().unwrap_or(0);
    let triple = (u32::from(rest[0]) << 16) | (u32::from(second) << 8);
    out.extend_from_slice(&[
        ALPHABET[(triple >> 18) as usize & 63],
        ALPHABET[(triple >> 12) as usize & 63],
        if rest.len() > 1 {
            ALPHABET[(triple >> 6) as usize & 63]
        } else {
            b'='
        },
        b'=',
    ]);
}

pub fn encode(bytes: &[u8], out: &mut String) {
    let mut buffer = Vec::new();
    encode_into(bytes, &mut buffer);
    // The alphabet is ASCII.
    out.push_str(std::str::from_utf8(&buffer).unwrap_or(""));
}

/// Decodes into `out`; `None` on any byte outside the alphabet or a bad
/// length.
pub fn decode(text: &str, out: &mut Vec<u8>) -> Option<()> {
    let bytes = text.as_bytes();
    if bytes.len() % 4 != 0 {
        return None;
    }
    out.reserve(bytes.len() / 4 * 3);
    let mut chunks = bytes.chunks_exact(4);
    let last = chunks.next_back();
    for chunk in chunks {
        let a = DECODE[chunk[0] as usize];
        let b = DECODE[chunk[1] as usize];
        let c = DECODE[chunk[2] as usize];
        let d = DECODE[chunk[3] as usize];
        if (a | b | c | d) >= PAD {
            return None;
        }
        let triple =
            (u32::from(a) << 18) | (u32::from(b) << 12) | (u32::from(c) << 6) | u32::from(d);
        out.extend_from_slice(&[(triple >> 16) as u8, (triple >> 8) as u8, triple as u8]);
    }
    let chunk = match last {
        Some(chunk) => chunk,
        None => return Some(()),
    };
    let a = DECODE[chunk[0] as usize];
    let b = DECODE[chunk[1] as usize];
    let c = DECODE[chunk[2] as usize];
    let d = DECODE[chunk[3] as usize];
    if a >= PAD || b >= PAD || c == INVALID || d == INVALID || (c == PAD && d != PAD) {
        return None;
    }
    let triple =
        (u32::from(a) << 18) | (u32::from(b) << 12) | (u32::from(c & 63) << 6) | u32::from(d & 63);
    out.push((triple >> 16) as u8);
    if c != PAD {
        out.push((triple >> 8) as u8);
    }
    if d != PAD {
        out.push(triple as u8);
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        for input in [
            &b""[..],
            b"f",
            b"fo",
            b"foo",
            b"foob",
            b"fooba",
            b"foobar",
            &[0, 255, 128, 7],
        ] {
            let mut text = String::new();
            encode(input, &mut text);
            let mut back = Vec::new();
            decode(&text, &mut back).unwrap();
            assert_eq!(back, input);
        }
        let mut text = String::new();
        encode(b"foob", &mut text);
        assert_eq!(text, "Zm9vYg==");
        assert!(decode("Zm9v!g==", &mut Vec::new()).is_none());
        assert!(decode("Zm9vY===", &mut Vec::new()).is_none());
        assert!(decode("Zm9", &mut Vec::new()).is_none());
    }
}
