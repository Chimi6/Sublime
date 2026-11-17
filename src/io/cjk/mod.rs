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

/// ISO-2022-JP bytes as text: ASCII, JIS X 0201 Roman and katakana, and
/// JIS X 0208, switched by their escape sequences.
pub fn decode_iso_2022_jp(bytes: &[u8]) -> String {
    #[derive(Clone, Copy, PartialEq)]
    enum Mode {
        Ascii,
        Katakana,
        Jis0208,
    }
    let mut out = String::with_capacity(bytes.len());
    let mut mode = Mode::Ascii;
    let mut at = 0;
    while at < bytes.len() {
        let byte = bytes[at];
        if byte == 0x1B {
            let switched = match (bytes.get(at + 1), bytes.get(at + 2)) {
                (Some(b'('), Some(b'B' | b'J')) => Some(Mode::Ascii),
                (Some(b'('), Some(b'I')) => Some(Mode::Katakana),
                (Some(b'$'), Some(b'@' | b'B')) => Some(Mode::Jis0208),
                _ => None,
            };
            if let Some(switched) = switched {
                mode = switched;
                at += 3;
                continue;
            }
        }
        if byte == b'\n' || byte == b'\r' || mode == Mode::Ascii {
            out.push(char::from(byte));
            at += 1;
            continue;
        }
        if mode == Mode::Katakana {
            let kana = (0x21..=0x5F)
                .contains(&byte)
                .then(|| char::from_u32(0xFF61 + u32::from(byte - 0x21)))
                .flatten();
            out.push(kana.unwrap_or('\u{FFFD}'));
            at += 1;
            continue;
        }
        let code = u32::from(byte) << 8 | u32::from(bytes.get(at + 1).copied().unwrap_or(0));
        out.push(jis_code(code).unwrap_or('\u{FFFD}'));
        at += 2;
    }
    out
}

/// A Japanese encoding text may be in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    ShiftJis,
    EucJp,
    Iso2022Jp,
}

impl Encoding {
    /// The encoding a charset name (`Shift_JIS`, `EUC-JP`, ...) names.
    pub fn named(name: &str) -> Option<Encoding> {
        match name.trim().to_ascii_lowercase().as_str() {
            "shift_jis" | "shift-jis" | "sjis" | "x-sjis" | "windows-31j" | "cp932"
            | "ms_kanji" | "csshiftjis" | "ms932" => Some(Encoding::ShiftJis),
            "euc-jp" | "euc_jp" | "x-euc-jp" | "eucjp" | "cseucpkdfmtjapanese" => {
                Some(Encoding::EucJp)
            }
            "iso-2022-jp" | "csiso2022jp" | "jis" => Some(Encoding::Iso2022Jp),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Encoding::ShiftJis => "Shift_JIS",
            Encoding::EucJp => "EUC-JP",
            Encoding::Iso2022Jp => "ISO-2022-JP",
        }
    }

    /// The bytes as text in this encoding, a bad sequence as U+FFFD.
    pub fn decode(self, bytes: &[u8]) -> String {
        match self {
            Encoding::ShiftJis => decode_shift_jis(bytes),
            Encoding::EucJp => decode_euc_jp(bytes),
            Encoding::Iso2022Jp => decode_iso_2022_jp(bytes),
        }
    }
}

/// The Japanese encoding a document's first bytes declare: an HTML or
/// CSS `charset`, or an XML declaration's `encoding`.
pub fn declared(head: &[u8]) -> Option<Encoding> {
    let head = String::from_utf8_lossy(&head[..head.len().min(2048)]).to_ascii_lowercase();
    for key in ["charset=", "encoding="] {
        let mut from = 0;
        while let Some(found) = head[from..].find(key) {
            let at = from + found + key.len();
            let value: String = head[at..]
                .trim_start_matches(['"', '\'', ' '])
                .chars()
                .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
                .collect();
            if let Some(encoding) = Encoding::named(&value) {
                return Some(encoding);
            }
            from = at;
        }
    }
    None
}

/// Whether `bytes` switch to a Japanese character set the way ISO-2022-JP
/// does (ESC `$B`, ESC `$@`, or ESC `(J`). The encoding is 7-bit, so such
/// bytes can be valid UTF-8 and must be looked for there too.
pub fn has_iso_2022_jp_escape(bytes: &[u8]) -> bool {
    bytes
        .windows(3)
        .any(|window| matches!(window, b"\x1b$B" | b"\x1b$@" | b"\x1b(J"))
}

/// The Japanese encoding bytes that are not UTF-8 are most plausibly in:
/// ISO-2022-JP when its escape is there, else Shift_JIS or EUC-JP,
/// whichever reads with fewer bad sequences (then fewer half-width
/// katakana, what EUC-JP turns into when misread as Shift_JIS). Either
/// must read with fewer than one bad sequence in a hundred characters,
/// and with most of its two-byte characters (seven in ten) in JIS rows 1
/// to 47: punctuation, kana, and the common (Level 1) kanji, where real
/// Japanese lives; Western bytes misread as Japanese land in the rare
/// rows instead, and are not taken for it. A sequence cut off at the end
/// of `bytes` (a sample of a longer input) does not count against it.
pub fn sniff(bytes: &[u8]) -> Option<Encoding> {
    if has_iso_2022_jp_escape(bytes) {
        return Some(Encoding::Iso2022Jp);
    }
    let shift_jis = tally(bytes, Encoding::ShiftJis);
    let euc = tally(bytes, Encoding::EucJp);
    let best = if (euc.bad, euc.half_kana) < (shift_jis.bad, shift_jis.half_kana) {
        euc
    } else {
        shift_jis
    };
    let pairs = best.common + best.rare;
    // Kana: Japanese text has hiragana or katakana in it, and EUC-KR or GBK
    // text read as Japanese has almost none (the Hangul and Hanzi land in
    // the kanji rows), so at least one in twenty of the two-byte characters
    // (half-width katakana count too).
    let plausible = best.bad * 100 < best.characters.max(1)
        && pairs > 0
        && best.common * 10 >= pairs * 7
        && best.kana + best.half_kana > 0
        && (best.kana + best.half_kana) * 20 >= pairs;
    plausible.then_some(best.encoding)
}

/// How bytes read in one encoding: characters, bad sequences, half-width
/// katakana, two-byte characters in the common rows and the rest, and the
/// hiragana and katakana among them.
#[derive(Clone, Copy)]
struct Tally {
    encoding: Encoding,
    characters: usize,
    bad: usize,
    half_kana: usize,
    common: usize,
    rare: usize,
    kana: usize,
}

fn tally(bytes: &[u8], encoding: Encoding) -> Tally {
    // Rows 1 to 47 of JIS X 0208: pointers below 47 times 94.
    const COMMON_ROWS: usize = 47 * 94;
    let mut tally = Tally {
        encoding,
        characters: 0,
        bad: 0,
        half_kana: 0,
        common: 0,
        rare: 0,
        kana: 0,
    };
    let mut at = 0;
    while at < bytes.len() {
        let rest = &bytes[at..];
        let (ch, length) = match encoding {
            Encoding::ShiftJis => shift_jis_char(rest),
            _ => euc_jp_char(rest),
        };
        let length = length.max(1);
        let cut =
            at + length > bytes.len() || (ch.is_none() && length == 1 && at + 1 == bytes.len());
        tally.characters += 1;
        match ch {
            None if !cut => tally.bad += 1,
            None => {}
            Some(ch) if ('\u{FF61}'..='\u{FF9F}').contains(&ch) => tally.half_kana += 1,
            Some(ch) if length == 2 => {
                if matches!(ch, 'ぁ'..='ゖ' | 'ァ'..='ヺ' | 'ー') {
                    tally.kana += 1;
                }
                let pointer = match encoding {
                    Encoding::ShiftJis => {
                        let (lead, trail) = (rest[0], rest[1]);
                        let lead_offset = if lead < 0xA0 { 0x81 } else { 0xC1 };
                        let trail_offset = if trail < 0x7F { 0x40 } else { 0x41 };
                        usize::from(lead - lead_offset) * 188 + usize::from(trail - trail_offset)
                    }
                    _ => {
                        usize::from(rest[0] - 0xA1) * 94 + usize::from(rest[1].saturating_sub(0xA1))
                    }
                };
                if pointer < COMMON_ROWS {
                    tally.common += 1;
                } else {
                    tally.rare += 1;
                }
            }
            Some(_) => {}
        }
        at += length;
    }
    tally
}

/// A reader that turns Shift_JIS or EUC-JP bytes into UTF-8 as they are
/// read, carrying a character cut between two reads over to the next.
pub struct Utf8Reader<R> {
    inner: R,
    encoding: Encoding,
    /// Bytes read but not yet decoded: the start of a cut character.
    pending: Vec<u8>,
    /// Decoded UTF-8 not yet handed out.
    decoded: Vec<u8>,
    at: usize,
    done: bool,
}

impl<R: std::io::Read> Utf8Reader<R> {
    pub fn new(inner: R, encoding: Encoding) -> Utf8Reader<R> {
        Utf8Reader {
            inner,
            encoding,
            pending: Vec::new(),
            decoded: Vec::new(),
            at: 0,
            done: false,
        }
    }

    fn refill(&mut self) -> std::io::Result<()> {
        let mut chunk = vec![0u8; 64 * 1024];
        let count = self.inner.read(&mut chunk)?;
        if count == 0 {
            self.done = true;
        }
        self.pending.extend_from_slice(&chunk[..count]);
        // Decode up to the last whole character; keep a cut one.
        let mut end = self.pending.len();
        if !self.done {
            let mut at = 0;
            let mut last_whole = 0;
            while at < self.pending.len() {
                let length = match self.encoding {
                    Encoding::ShiftJis => shift_jis_length(self.pending[at]),
                    _ => euc_jp_length(self.pending[at]),
                };
                if at + length > self.pending.len() {
                    break;
                }
                at += length;
                last_whole = at;
            }
            end = last_whole;
        }
        let text = self.encoding.decode(&self.pending[..end]);
        self.pending.drain(..end);
        self.decoded.clear();
        self.decoded.extend_from_slice(text.as_bytes());
        self.at = 0;
        Ok(())
    }
}

impl<R: std::io::Read> std::io::Read for Utf8Reader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        while self.at >= self.decoded.len() {
            if self.done {
                return Ok(0);
            }
            self.refill()?;
        }
        let count = buffer.len().min(self.decoded.len() - self.at);
        buffer[..count].copy_from_slice(&self.decoded[self.at..self.at + count]);
        self.at += count;
        Ok(count)
    }
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
    fn decodes_iso_2022_jp() {
        // ESC $ B, 日 (0x467C), ESC ( B, then ASCII.
        let bytes = [0x1B, b'$', b'B', 0x46, 0x7C, 0x1B, b'(', b'B', b'!'];
        assert_eq!(decode_iso_2022_jp(&bytes), "日!");
    }

    #[test]
    fn iso_2022_jp_escapes_are_found_in_seven_bit_text() {
        assert!(has_iso_2022_jp_escape(b"a\x1b$Bb\x1b(Bc"));
        assert!(has_iso_2022_jp_escape(b"\x1b$@x"));
        assert!(has_iso_2022_jp_escape(b"\x1b(J"));
        assert!(!has_iso_2022_jp_escape(b"plain \x1b[0m text"));
    }

    #[test]
    fn euc_kr_and_gbk_are_not_japanese() {
        // "안녕하세요 세계" in EUC-KR; "你好，世界" in GBK.
        let korean = [
            0xBE, 0xC8, 0xB3, 0xE7, 0xC7, 0xCF, 0xBC, 0xBC, 0xBF, 0xE4, 0x20, 0xBC, 0xBC, 0xB0,
            0xE8,
        ];
        let chinese = [0xC4, 0xE3, 0xBA, 0xC3, 0xA3, 0xAC, 0xCA, 0xC0, 0xBD, 0xE7];
        assert_eq!(sniff(&korean), None);
        assert_eq!(sniff(&chinese), None);
    }

    #[test]
    fn tells_shift_jis_from_euc_jp() {
        let text = "日本語の文章です。ひらがなとカタカナ。";
        let shift_jis = encode_shift_jis(text);
        assert_eq!(sniff(&shift_jis), Some(Encoding::ShiftJis));
        // EUC-JP: each JIS X 0208 pointer as two bytes from 0xA1.
        let mut euc = Vec::new();
        for ch in text.chars() {
            let pointer = tables::JIS0208
                .iter()
                .position(|code| u32::from(*code) == ch as u32)
                .expect("in JIS X 0208");
            euc.push((pointer / 94 + 0xA1) as u8);
            euc.push((pointer % 94 + 0xA1) as u8);
        }
        assert_eq!(sniff(&euc), Some(Encoding::EucJp));
        assert_eq!(decode_euc_jp(&euc), text);
        // Latin-1 bytes are not Japanese.
        assert_eq!(
            sniff(
                "café crème brûlée"
                    .as_bytes()
                    .iter()
                    .map(|b| if *b > 0x7F { 0xE9 } else { *b })
                    .collect::<Vec<u8>>()
                    .as_slice()
            ),
            None
        );
        assert_eq!(
            declared(br#"<meta charset="Shift_JIS">"#),
            Some(Encoding::ShiftJis)
        );
    }

    #[test]
    fn decodes_a_stream_cut_between_bytes() {
        use std::io::Read;
        let bytes = encode_shift_jis("日本語ｶﾅ、English");
        // One byte at a time: every two-byte character is cut.
        struct Trickle<'a>(&'a [u8]);
        impl Read for Trickle<'_> {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                let Some((first, rest)) = self.0.split_first() else {
                    return Ok(0);
                };
                buffer[0] = *first;
                self.0 = rest;
                Ok(1)
            }
        }
        let mut text = String::new();
        Utf8Reader::new(Trickle(&bytes), Encoding::ShiftJis)
            .read_to_string(&mut text)
            .expect("reads");
        assert_eq!(text, "日本語ｶﾅ、English");
    }

    #[test]
    fn radicals_read_as_ideographs() {
        assert_eq!(unify_radical('\u{2F47}'), '日');
        assert_eq!(unify_radical('日'), '日');
    }
}
