//! LZW as TIFF and PDF use it: codes from 9 to 12 bits, most significant
//! bit first, 256 clearing the table and 257 ending the data. With
//! `early_change` (TIFF always, PDF by default) the code width grows one
//! code before the table fills.

/// Decodes `input`, stopping at the end code, at `limit` bytes when one
/// is given, or when the input runs out after at least one code.
pub fn decode(input: &[u8], limit: Option<usize>, early_change: bool) -> Result<Vec<u8>, String> {
    const CLEAR: usize = 256;
    const END: usize = 257;
    let mut out = Vec::with_capacity(limit.unwrap_or(input.len() * 3));
    // Each code: its prefix code, last byte, first byte, and length.
    let mut prefix = vec![0u16; 4096];
    let mut last = vec![0u8; 4096];
    let mut first = vec![0u8; 4096];
    let mut length = vec![0u16; 4096];
    for code in 0..256 {
        last[code] = code as u8;
        first[code] = code as u8;
        length[code] = 1;
    }
    let (mut next, mut width) = (258usize, 9u32);
    let mut previous: Option<usize> = None;
    let (mut buffer, mut count, mut at) = (0u32, 0u32, 0usize);
    let emit = |out: &mut Vec<u8>, code: usize, prefix: &[u16], last: &[u8], length: &[u16]| {
        let size = usize::from(length[code]);
        let start = out.len();
        out.resize(start + size, 0);
        let mut cursor = code;
        for slot in out[start..].iter_mut().rev() {
            *slot = last[cursor];
            cursor = usize::from(prefix[cursor]);
        }
    };
    let early = usize::from(early_change);
    while limit.is_none_or(|limit| out.len() < limit) {
        while count < width {
            let Some(byte) = input.get(at) else {
                // Data that ends without the end code ends here.
                if limit.is_none() && previous.is_some() {
                    return Ok(out);
                }
                return Err("LZW data cut short".to_string());
            };
            buffer = (buffer << 8) | u32::from(*byte);
            count += 8;
            at += 1;
        }
        let code = ((buffer >> (count - width)) & ((1 << width) - 1)) as usize;
        count -= width;
        if code == END {
            break;
        }
        if code == CLEAR {
            next = 258;
            width = 9;
            previous = None;
            continue;
        }
        match previous {
            None => {
                if code >= 256 {
                    return Err("bad LZW code".to_string());
                }
                out.push(code as u8);
            }
            Some(before) => {
                let added = if code < next {
                    emit(&mut out, code, &prefix, &last, &length);
                    first[code]
                } else if code == next {
                    let head = first[before];
                    emit(&mut out, before, &prefix, &last, &length);
                    out.push(head);
                    head
                } else {
                    return Err("bad LZW code".to_string());
                };
                if next < 4096 {
                    prefix[next] = before as u16;
                    last[next] = added;
                    first[next] = first[before];
                    length[next] = length[before] + 1;
                    next += 1;
                }
            }
        }
        previous = Some(code);
        if next + early >= (1 << width) && width < 12 {
            width += 1;
        }
    }
    if let Some(limit) = limit {
        out.truncate(limit);
    }
    Ok(out)
}
