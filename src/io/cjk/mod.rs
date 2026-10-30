//! Japanese text: Adobe-Japan1 glyph ids as text, the Shift_JIS, EUC-JP,
//! and JIS byte encodings decoded as the WHATWG Encoding Standard does
//! (what browsers do), and CJK radicals read as the ideographs they stand
//! for. The tables are generated (`scripts/gen-cjk-tables.py`).

mod tables;

/// Appends the text of an Adobe-Japan1 CID; false when it has none.
pub fn japan1(cid: u32, out: &mut String) -> bool {
    let Ok(index) = usize::try_from(cid) else {
        return false;
    };
    match tables::JAPAN1.get(index) {
        Some(&code) if code != 0 => {
            if let Some(ch) = char::from_u32(u32::from(code)) {
                out.push(ch);
                return true;
            }
            false
        }
        Some(_) => {
            let Ok(position) =
                tables::JAPAN1_OTHER.binary_search_by_key(&(index as u16), |entry| entry.0)
            else {
                return false;
            };
            let start = if position == 0 {
                0
            } else {
                usize::from(tables::JAPAN1_OTHER[position - 1].1)
            };
            let end = usize::from(tables::JAPAN1_OTHER[position].1);
            out.push_str(&tables::JAPAN1_OTHER_TEXT[start..end]);
            true
        }
        None => false,
    }
}

/// The character at a JIS X 0208 pointer (row times 94 plus cell, from
/// zero), the NEC and IBM extensions included.
pub fn jis0208(pointer: usize) -> Option<char> {
    let code = *tables::JIS0208.get(pointer)?;
    if code == 0 {
        return None;
    }
    char::from_u32(u32::from(code))
}

/// One character of Shift_JIS (Windows-31J) from the front of `bytes`:
/// the character, or `None` for bytes that mean nothing, and how many
/// bytes it took.
pub fn shift_jis_char(bytes: &[u8]) -> (Option<char>, usize) {
    let Some(&lead) = bytes.first() else {
        return (None, 0);
    };
    match lead {
        0x00..=0x80 => (Some(char::from(lead)), 1),
        // Half-width katakana.
        0xA1..=0xDF => (char::from_u32(0xFF61 + u32::from(lead - 0xA1)), 1),
        0x81..=0x9F | 0xE0..=0xFC => {
            let Some(&trail) = bytes.get(1) else {
                return (None, 1);
            };
            if !matches!(trail, 0x40..=0x7E | 0x80..=0xFC) {
                // An ASCII byte after a bad lead is itself.
                return (None, if trail < 0x80 { 1 } else { 2 });
            }
            let lead_offset = if lead < 0xA0 { 0x81 } else { 0xC1 };
            let trail_offset = if trail < 0x7F { 0x40 } else { 0x41 };
            let pointer = usize::from(lead - lead_offset) * 188 + usize::from(trail - trail_offset);
            // The user-defined area maps to the Private Use Area.
            if (8836..=10715).contains(&pointer) {
                return (char::from_u32(0xE000 + pointer as u32 - 8836), 2);
            }
            (jis0208(pointer), 2)
        }
        _ => (None, 1),
    }
}

/// How many bytes the Shift_JIS character starting with `lead` takes.
pub fn shift_jis_length(lead: u8) -> usize {
    if matches!(lead, 0x81..=0x9F | 0xE0..=0xFC) {
        2
    } else {
        1
    }
}

/// One character of EUC-JP from the front of `bytes`, and its length.
/// JIS X 0212 (three bytes from 0x8F) is not in the tables: `None`.
pub fn euc_jp_char(bytes: &[u8]) -> (Option<char>, usize) {
    let Some(&lead) = bytes.first() else {
        return (None, 0);
    };
    match lead {
        0x00..=0x7F => (Some(char::from(lead)), 1),
        0x8E => match bytes.get(1) {
            Some(&kana @ 0xA1..=0xDF) => (char::from_u32(0xFF61 + u32::from(kana - 0xA1)), 2),
            _ => (None, 1),
        },
        0x8F => (None, 3.min(bytes.len())),
        0xA1..=0xFE => match bytes.get(1) {
            Some(&trail @ 0xA1..=0xFE) => (
                jis0208(usize::from(lead - 0xA1) * 94 + usize::from(trail - 0xA1)),
                2,
            ),
            _ => (None, 1),
        },
        _ => (None, 1),
    }
}

/// How many bytes the EUC-JP character starting with `lead` takes.
pub fn euc_jp_length(lead: u8) -> usize {
    match lead {
        0x8E | 0xA1..=0xFE => 2,
        0x8F => 3,
        _ => 1,
    }
}

/// A two-byte JIS X 0208 code as ISO-2022-JP writes it (0x2121 to 0x7E7E).
pub fn jis_code(code: u32) -> Option<char> {
    let (row, cell) = ((code >> 8) & 0xFF, code & 0xFF);
    if !(0x21..=0x7E).contains(&row) || !(0x21..=0x7E).contains(&cell) {
        return None;
    }
    jis0208(((row - 0x21) * 94 + (cell - 0x21)) as usize)
}

/// Shift_JIS bytes as text, a bad sequence as U+FFFD.
pub fn decode_shift_jis(bytes: &[u8]) -> String {
    decode_with(bytes, shift_jis_char)
}

/// EUC-JP bytes as text, a bad sequence as U+FFFD.
pub fn decode_euc_jp(bytes: &[u8]) -> String {
    decode_with(bytes, euc_jp_char)
}

fn decode_with(bytes: &[u8], next: fn(&[u8]) -> (Option<char>, usize)) -> String {
    let mut out = String::with_capacity(bytes.len() * 3 / 2);
    let mut at = 0;
    while at < bytes.len() {
        let (ch, length) = next(&bytes[at..]);
        out.push(ch.unwrap_or('\u{FFFD}'));
        at += length.max(1);
    }
    out
}

/// The unified ideograph a CJK radical or stroke stands for, or the
/// character itself.
pub fn unify_radical(ch: char) -> char {
    let code = u32::from(ch);
    if !(0x2E80..=0x2FDF).contains(&code) && !(0x31C0..=0x31EF).contains(&code) {
        return ch;
    }
    match tables::RADICALS.binary_search_by_key(&code, |pair| pair.0) {
        Ok(index) => char::from_u32(tables::RADICALS[index].1).unwrap_or(ch),
        Err(_) => ch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test-only: Shift_JIS by searching the table (slow, exact).
    fn encode_shift_jis(text: &str) -> Vec<u8> {
        let mut out = Vec::new();
        for ch in text.chars() {
            if (ch as u32) < 0x80 {
                out.push(ch as u8);
                continue;
            }
            if ('\u{FF61}'..='\u{FF9F}').contains(&ch) {
                out.push((ch as u32 - 0xFF61 + 0xA1) as u8);
                continue;
            }
            let pointer = tables::JIS0208
                .iter()
                .position(|code| u32::from(*code) == ch as u32)
                .expect("in JIS X 0208");
            let lead = pointer / 188;
            let trail = pointer % 188;
            out.push((lead + if lead < 0x1F { 0x81 } else { 0xC1 }) as u8);
            out.push((trail + if trail < 0x3F { 0x40 } else { 0x41 }) as u8);
        }
        out
    }

    #[test]
    fn decodes_shift_jis_like_a_browser() {
        let bytes = encode_shift_jis("日本語ｶﾅ、English");
        assert_eq!(decode_shift_jis(&bytes), "日本語ｶﾅ、English");
    }

    #[test]
    fn decodes_euc_jp_and_jis_codes() {
        // 日 is row 38 cell 92: EUC-JP C6 FC, JIS 0x467C.
        assert_eq!(decode_euc_jp(&[0xC6, 0xFC, 0x41, 0x8E, 0xB6]), "日Aｶ");
        assert_eq!(jis_code(0x467C), Some('日'));
        assert_eq!(jis_code(0x2020), None);
    }

    #[test]
    fn reads_japan1_cids() {
        let mut text = String::new();
        // CID 1 is the space; 7641 is beyond the BMP.
        assert!(japan1(1, &mut text));
        assert_eq!(text, " ");
        text.clear();
        assert!(japan1(7641, &mut text));
        assert_eq!(text, "\u{28cdd}");
        assert!(!japan1(999_999, &mut text));
    }

    #[test]
    fn radicals_read_as_ideographs() {
        assert_eq!(unify_radical('\u{2F47}'), '日');
        assert_eq!(unify_radical('日'), '日');
    }
}
