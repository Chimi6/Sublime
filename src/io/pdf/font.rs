//! A font as text extraction needs it: how its strings split into codes,
//! what each code means in Unicode, and how wide each code is.
//!
//! Unicode comes from the font's ToUnicode CMap when it has one (most
//! producers write one), else, for a simple font, from its encoding: a
//! base encoding (Standard, WinAnsi, MacRoman, or the Symbol and Zapf
//! Dingbats built-ins) with `/Differences` glyph names looked up in the
//! glyph list or read as `uniXXXX`/`uXXXX`. A composite (Type0) font is
//! read through its encoding CMap: `Identity` and embedded CMaps give
//! glyph ids (CIDs), which read as text through the font's character
//! collection when it is Adobe-Japan1; the Unicode CMaps (`UniJIS-UCS2-H`
//! and the like, for every CJK collection) give Unicode itself; and the
//! Shift_JIS, EUC-JP, and JIS CMaps give Japanese bytes, decoded. CJK
//! radicals a ToUnicode map gives for ideographs read as the ideographs.

use std::collections::HashMap;

use crate::io::pdf::PdfError;
use crate::io::pdf::document::Document;
use crate::io::pdf::lexer::{Lexer, Token};
use crate::io::pdf::object::{Dictionary, Object};
use crate::io::pdf::tables::{self, Widths};

pub struct Font {
    /// A Type0 font: its codes come from its encoding CMap.
    composite: bool,
    /// How a composite font's codes are split and what they are.
    code_map: CodeMap,
    /// The composite font's glyphs are in the Adobe-Japan1 collection.
    japan1: bool,
    /// The composite font writes top to bottom (a `-V` CMap).
    pub vertical: bool,
    to_unicode: Option<ToUnicode>,
    /// A simple font's code -> Unicode, 0 where the code means nothing.
    encoding: [u16; 256],
    /// Codes the encoding maps to more than one character (ligatures).
    spelled: HashMap<u8, &'static str>,
    widths: FontWidths,
    /// The glyph space to text space scale: 1/1000 except Type3 fonts.
    scale: f64,
    /// How much larger the glyphs are drawn than the font size says: 1
    /// except for a Type3 font whose matrix is not the usual 1/1000 em.
    pub size_factor: f64,
    pub bold: bool,
    pub italic: bool,
    pub name: String,
}

enum FontWidths {
    /// `/FirstChar` and `/Widths`, with `/MissingWidth` for the rest.
    Simple {
        first: u32,
        widths: Vec<f64>,
        missing: f64,
    },
    /// A core font without `/Widths`: its AFM widths by Unicode.
    Core(&'static Widths),
    /// Every glyph one width (Courier).
    Fixed(f64),
    /// A CID font's `/W` ranges and `/DW`.
    Cid {
        ranges: Vec<(u32, u32, f64)>,
        singles: HashMap<u32, f64>,
        default: f64,
    },
}

/// One code of a string: its value, its advance in text space (before
/// the font size), and whether it is the single-byte space (code 32) that
/// word spacing applies to.
pub struct Code {
    pub code: u32,
    pub advance: f64,
    pub word_space: bool,
}

impl Font {
    pub fn load(document: &Document<'_>, dictionary: &Dictionary) -> Font {
        let name_of = |key: &[u8]| {
            dictionary
                .get(key)
                .and_then(Object::as_name)
                .map(|name| String::from_utf8_lossy(name).into_owned())
                .unwrap_or_default()
        };
        let subtype = name_of(b"Subtype");
        let base = name_of(b"BaseFont");
        // A subset prefix (ABCDEF+) is not part of the family name.
        let family = match base.split_once('+') {
            Some((prefix, rest)) if prefix.len() == 6 => rest.to_string(),
            _ => base.clone(),
        };
        let lower = family.to_ascii_lowercase();
        let descriptor = dictionary
            .get(b"FontDescriptor")
            .and_then(|object| document.resolve(object).ok())
            .and_then(|object| object.as_dictionary().cloned());
        let flags = descriptor
            .as_ref()
            .and_then(|descriptor| descriptor.get(b"Flags"))
            .and_then(Object::as_integer)
            .unwrap_or(0);
        let weight = descriptor
            .as_ref()
            .and_then(|descriptor| descriptor.get(b"FontWeight"))
            .and_then(Object::as_number)
            .unwrap_or(0.0);
        let bold = lower.contains("bold")
            || lower.contains("black")
            || lower.contains("heavy")
            || lower.contains("semibold")
            || weight >= 600.0
            || flags & (1 << 18) != 0;
        let italic = lower.contains("italic") || lower.contains("oblique") || flags & (1 << 6) != 0;
        let to_unicode = dictionary
            .get(b"ToUnicode")
            .and_then(|object| document.stream_data(object).ok())
            .map(|(_, data)| ToUnicode::parse(&data));
        let composite = subtype == "Type0";
        let (code_map, vertical) = if composite {
            code_map(document, dictionary)
        } else {
            (CodeMap::Unknown, false)
        };
        let japan1 = composite && is_japan1(document, dictionary, &code_map);
        // A Type3 font's matrix maps its glyph space to text space: its
        // first entry scales the widths, its fourth the glyphs' height.
        let matrix: Vec<f64> = if subtype == "Type3" {
            dictionary
                .get(b"FontMatrix")
                .and_then(|object| document.resolve(object).ok())
                .and_then(|object| {
                    object
                        .as_array()
                        .map(|items| items.iter().filter_map(Object::as_number).collect())
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let size_factor = match matrix.get(3) {
            Some(height) if *height != 0.0 => (height / 0.001).abs(),
            _ => 1.0,
        };
        let (widths, scale) = if composite {
            (cid_widths(document, dictionary), 0.001)
        } else {
            let scale = matrix
                .first()
                .copied()
                .filter(|value| *value != 0.0)
                .unwrap_or(0.001);
            (
                simple_widths(document, dictionary, descriptor.as_ref(), &lower, scale),
                scale,
            )
        };
        let (encoding, spelled) = if composite {
            ([0u16; 256], HashMap::new())
        } else {
            simple_encoding(document, dictionary, &lower)
        };
        Font {
            composite,
            code_map,
            japan1,
            vertical,
            to_unicode,
            encoding,
            spelled,
            widths,
            scale,
            size_factor,
            bold,
            italic,
            name: family,
        }
    }

    /// The codes a string holds, in order.
    pub fn codes(&self, bytes: &[u8], out: &mut Vec<Code>) {
        out.clear();
        let mut at = 0;
        while at < bytes.len() {
            let rest = &bytes[at..];
            let length = if self.composite {
                match &self.code_map {
                    CodeMap::Unknown => match &self.to_unicode {
                        Some(map) if !map.spaces.is_empty() => map.code_length(rest),
                        _ => 2,
                    },
                    known => known.code_length(rest),
                }
            } else {
                match &self.to_unicode {
                    Some(map) if !map.spaces.is_empty() => map.code_length(rest),
                    _ => 1,
                }
            };
            let length = length.min(bytes.len() - at).max(1);
            let code = bytes[at..at + length]
                .iter()
                .fold(0u32, |value, byte| value << 8 | u32::from(*byte));
            out.push(Code {
                code,
                advance: self.width(code) * self.scale,
                word_space: length == 1 && code == 32,
            });
            at += length;
        }
    }

    /// Appends the Unicode text of a code; nothing when it has none.
    pub fn text(&self, code: u32, out: &mut String) {
        let start = out.len();
        self.raw_text(code, out);
        // A radical in place of the ideograph it is drawn as.
        if out[start..]
            .chars()
            .any(|ch| crate::io::cjk::unify_radical(ch) != ch)
        {
            let unified: String = out[start..]
                .chars()
                .map(crate::io::cjk::unify_radical)
                .collect();
            out.truncate(start);
            out.push_str(&unified);
        }
    }

    fn raw_text(&self, code: u32, out: &mut String) {
        if let Some(map) = &self.to_unicode
            && map.text(code, out)
        {
            return;
        }
        if self.composite {
            if self.code_map.text(code, out) {
                return;
            }
            if self.japan1
                && let Some(cid) = self.code_map.cid(code)
            {
                crate::io::cjk::japan1(cid, out);
            }
            return;
        }
        if code > 255 {
            return;
        }
        let byte = code as u8;
        if let Some(text) = self.spelled.get(&byte) {
            out.push_str(text);
            return;
        }
        let value = self.encoding[usize::from(byte)];
        if value >= 0x20
            && let Some(char) = char::from_u32(u32::from(value))
        {
            out.push(char);
        }
    }

    /// A code's width in glyph space.
    fn width(&self, code: u32) -> f64 {
        match &self.widths {
            FontWidths::Simple {
                first,
                widths,
                missing,
            } => code
                .checked_sub(*first)
                .and_then(|index| widths.get(index as usize))
                .copied()
                .unwrap_or(*missing),
            FontWidths::Core(table) => {
                let unicode = if code < 256 {
                    self.encoding[code as usize]
                } else {
                    0
                };
                let width = table
                    .pairs
                    .binary_search_by_key(&unicode, |pair| pair.0)
                    .map(|index| table.pairs[index].1)
                    .unwrap_or(table.default);
                f64::from(width)
            }
            FontWidths::Fixed(width) => *width,
            FontWidths::Cid {
                ranges,
                singles,
                default,
            } => {
                // Widths are by CID; an encoding whose CIDs are not known
                // here (Shift_JIS, Unicode) is half or full width by text.
                let Some(cid) = self.code_map.cid(code).or(match self.code_map {
                    CodeMap::Unknown => Some(code),
                    _ => None,
                }) else {
                    let mut text = String::new();
                    self.code_map.text(code, &mut text);
                    let half = text.chars().next().is_some_and(|ch| {
                        (' '..='~').contains(&ch) || ('\u{FF61}'..='\u{FF9F}').contains(&ch)
                    });
                    return if half { 500.0 } else { *default };
                };
                singles.get(&cid).copied().unwrap_or_else(|| {
                    ranges
                        .iter()
                        .find(|(low, high, _)| (*low..=*high).contains(&cid))
                        .map_or(*default, |range| range.2)
                })
            }
        }
    }
}

/// A composite font's encoding: how its strings split into codes, and
/// what a code is (a CID, or text in some encoding).
enum CodeMap {
    /// `Identity-H`/`-V`: two-byte codes that are the CIDs.
    Identity,
    /// A Unicode CMap (`UniJIS-UCS2-H`, `UniGB-UTF16-H`, ...): codes that
    /// are text in one of Unicode's forms.
    Unicode(UnicodeForm),
    /// The Shift_JIS CMaps (`90ms-RKSJ-H` and the other `RKSJ` ones).
    ShiftJis,
    /// The EUC-JP CMaps (`EUC-H`, `EUC-V`).
    EucJp,
    /// The ISO-2022-JP CMaps (`H`, `V`): two-byte JIS X 0208 codes.
    Jis,
    /// A CMap embedded in the file: its code space and code -> CID, with
    /// the predefined CMap it builds on (`usecmap`), if any.
    Embedded(Box<EmbeddedCMap>),
    /// Anything else: two-byte codes taken as CIDs only for widths.
    Unknown,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UnicodeForm {
    Ucs2,
    Utf16,
    Utf8,
    Utf32,
}

struct EmbeddedCMap {
    /// (length in bytes, low, high)
    spaces: Vec<(usize, u32, u32)>,
    singles: HashMap<u32, u32>,
    /// (low, high, first CID)
    ranges: Vec<(u32, u32, u32)>,
    parent: CodeMap,
}

impl CodeMap {
    /// The predefined CMap a name gives, and whether it writes vertically.
    fn named(name: &str) -> (CodeMap, bool) {
        let vertical = name.ends_with("-V") || name == "V";
        let map = if name.starts_with("Identity-") {
            CodeMap::Identity
        } else if name.starts_with("Uni") {
            if name.contains("UCS2") {
                CodeMap::Unicode(UnicodeForm::Ucs2)
            } else if name.contains("UTF16") {
                CodeMap::Unicode(UnicodeForm::Utf16)
            } else if name.contains("UTF8") {
                CodeMap::Unicode(UnicodeForm::Utf8)
            } else if name.contains("UTF32") {
                CodeMap::Unicode(UnicodeForm::Utf32)
            } else {
                CodeMap::Unknown
            }
        } else if name.contains("RKSJ") {
            CodeMap::ShiftJis
        } else if name.starts_with("EUC-") {
            CodeMap::EucJp
        } else if matches!(name, "H" | "V") {
            CodeMap::Jis
        } else {
            CodeMap::Unknown
        };
        (map, vertical)
    }

    /// How many bytes the code at the front of `bytes` takes.
    fn code_length(&self, bytes: &[u8]) -> usize {
        let lead = bytes.first().copied().unwrap_or(0);
        match self {
            CodeMap::Identity | CodeMap::Jis | CodeMap::Unknown => 2,
            CodeMap::Unicode(UnicodeForm::Ucs2) => 2,
            CodeMap::Unicode(UnicodeForm::Utf16) => {
                if (0xD8..=0xDB).contains(&lead) {
                    4
                } else {
                    2
                }
            }
            CodeMap::Unicode(UnicodeForm::Utf8) => match lead {
                0xF0..=0xF7 => 4,
                0xE0..=0xEF => 3,
                0xC0..=0xDF => 2,
                _ => 1,
            },
            CodeMap::Unicode(UnicodeForm::Utf32) => 4,
            CodeMap::ShiftJis => crate::io::cjk::shift_jis_length(lead),
            CodeMap::EucJp => crate::io::cjk::euc_jp_length(lead),
            CodeMap::Embedded(map) => {
                for length in 1..=4 {
                    if bytes.len() < length {
                        break;
                    }
                    let value = be(&bytes[..length]);
                    if map
                        .spaces
                        .iter()
                        .any(|(size, low, high)| *size == length && (*low..=*high).contains(&value))
                    {
                        return length;
                    }
                }
                if map.spaces.is_empty() {
                    map.parent.code_length(bytes)
                } else {
                    map.spaces.iter().map(|space| space.0).min().unwrap_or(2)
                }
            }
        }
    }

    /// The CID a code selects, when the encoding says.
    fn cid(&self, code: u32) -> Option<u32> {
        match self {
            CodeMap::Identity => Some(code),
            CodeMap::Embedded(map) => map
                .singles
                .get(&code)
                .copied()
                .or_else(|| {
                    map.ranges
                        .iter()
                        .find(|(low, high, _)| (*low..=*high).contains(&code))
                        .map(|(low, _, first)| first + (code - low))
                })
                .or_else(|| map.parent.cid(code)),
            _ => None,
        }
    }

    /// Appends the text a code is in an encoding of text; false for an
    /// encoding of CIDs or a code that means nothing.
    fn text(&self, code: u32, out: &mut String) -> bool {
        let pushed = |ch: Option<char>, out: &mut String| match ch {
            Some(ch) if ch != '\0' => {
                out.push(ch);
                true
            }
            _ => false,
        };
        match self {
            CodeMap::Unicode(form) => {
                let text = match form {
                    UnicodeForm::Ucs2 => char::from_u32(code).map(String::from),
                    UnicodeForm::Utf32 => char::from_u32(code).map(String::from),
                    UnicodeForm::Utf16 => {
                        if code > 0xFFFF {
                            let units = [(code >> 16) as u16, code as u16];
                            char::decode_utf16(units)
                                .collect::<Result<String, _>>()
                                .ok()
                        } else {
                            char::from_u32(code).map(String::from)
                        }
                    }
                    UnicodeForm::Utf8 => {
                        let bytes = code.to_be_bytes();
                        let start = bytes.iter().position(|byte| *byte != 0).unwrap_or(3);
                        std::str::from_utf8(&bytes[start..])
                            .ok()
                            .map(str::to_string)
                    }
                };
                match text {
                    Some(text) if !text.is_empty() && text != "\0" => {
                        out.push_str(&text);
                        true
                    }
                    _ => false,
                }
            }
            CodeMap::ShiftJis => {
                let bytes = code_bytes(code);
                pushed(crate::io::cjk::shift_jis_char(&bytes).0, out)
            }
            CodeMap::EucJp => {
                let bytes = code_bytes(code);
                pushed(crate::io::cjk::euc_jp_char(&bytes).0, out)
            }
            CodeMap::Jis => pushed(crate::io::cjk::jis_code(code), out),
            CodeMap::Embedded(map) => map.parent.text(code, out),
            CodeMap::Identity | CodeMap::Unknown => false,
        }
    }
}

/// A code's bytes, most significant first, without leading zeros (one
/// byte for a code under 256).
fn code_bytes(code: u32) -> Vec<u8> {
    let bytes = code.to_be_bytes();
    let start = bytes.iter().position(|byte| *byte != 0).unwrap_or(3);
    bytes[start..].to_vec()
}

/// A composite font's encoding CMap: a name, or an embedded stream.
fn code_map(document: &Document<'_>, dictionary: &Dictionary) -> (CodeMap, bool) {
    let Some(encoding) = dictionary
        .get(b"Encoding")
        .and_then(|object| document.resolve(object).ok())
    else {
        return (CodeMap::Identity, false);
    };
    if let Some(name) = encoding.as_name() {
        return CodeMap::named(&String::from_utf8_lossy(name));
    }
    let Some(entry) = dictionary.get(b"Encoding") else {
        return (CodeMap::Unknown, false);
    };
    let Ok((stream, data)) = document.stream_data(entry) else {
        return (CodeMap::Unknown, false);
    };
    let mut vertical = stream
        .get(b"WMode")
        .and_then(Object::as_integer)
        .is_some_and(|mode| mode == 1);
    let mut parent = CodeMap::Unknown;
    if let Some(name) = stream.get(b"UseCMap").and_then(Object::as_name) {
        let (named, named_vertical) = CodeMap::named(&String::from_utf8_lossy(name));
        parent = named;
        vertical |= named_vertical;
    }
    let mut map = EmbeddedCMap {
        spaces: Vec::new(),
        singles: HashMap::new(),
        ranges: Vec::new(),
        parent: CodeMap::Unknown,
    };
    let mut lexer = Lexer::new(&data);
    let mut operands: Vec<Object> = Vec::new();
    while let Some(token) = lexer.next_token() {
        match token {
            Token::Operand(object) => operands.push(object),
            Token::Operator(b"endcodespacerange") => {
                for pair in operands.chunks_exact(2) {
                    if let (Object::String(low), Object::String(high)) = (&pair[0], &pair[1]) {
                        map.spaces.push((low.len().max(1), be(low), be(high)));
                    }
                }
                operands.clear();
            }
            Token::Operator(b"endcidchar") => {
                for pair in operands.chunks_exact(2) {
                    if let (Object::String(code), Some(cid)) = (&pair[0], pair[1].as_integer()) {
                        map.singles.insert(be(code), cid.max(0) as u32);
                    }
                }
                operands.clear();
            }
            Token::Operator(b"endcidrange") => {
                for triple in operands.chunks_exact(3) {
                    if let (Object::String(low), Object::String(high), Some(cid)) =
                        (&triple[0], &triple[1], triple[2].as_integer())
                    {
                        map.ranges.push((be(low), be(high), cid.max(0) as u32));
                    }
                }
                operands.clear();
            }
            Token::Operator(b"usecmap") => {
                if let Some(Object::Name(name)) = operands.last() {
                    let (named, named_vertical) = CodeMap::named(&String::from_utf8_lossy(name));
                    parent = named;
                    vertical |= named_vertical;
                }
                operands.clear();
            }
            Token::Operator(b"def") => {
                // `/WMode 1 def` in the CMap's own dictionary.
                if let [Object::Name(key), Object::Integer(value)] = operands.as_slice()
                    && key.as_slice() == b"WMode"
                {
                    vertical |= *value == 1;
                }
                operands.clear();
            }
            Token::Operator(_) => operands.clear(),
        }
    }
    map.parent = parent;
    (CodeMap::Embedded(Box::new(map)), vertical)
}

/// Whether a composite font's glyph ids are Adobe-Japan1's: its CID
/// font says so, or its CMap is one of Japan1's.
fn is_japan1(document: &Document<'_>, dictionary: &Dictionary, code_map: &CodeMap) -> bool {
    if matches!(code_map, CodeMap::ShiftJis | CodeMap::EucJp | CodeMap::Jis) {
        return true;
    }
    let ordering = dictionary
        .get(b"DescendantFonts")
        .and_then(|object| document.resolve(object).ok())
        .and_then(|object| object.as_array().and_then(|items| items.first().cloned()))
        .and_then(|object| document.resolve(&object).ok())
        .and_then(|object| object.as_dictionary().cloned())
        .and_then(|descendant| descendant.get(b"CIDSystemInfo").cloned())
        .and_then(|object| document.resolve(&object).ok())
        .and_then(|object| object.as_dictionary().cloned())
        .and_then(|info| info.get(b"Ordering").cloned())
        .and_then(|object| document.resolve(&object).ok());
    match ordering {
        Some(Object::String(text)) => text.as_slice() == b"Japan1",
        Some(Object::Name(name)) => name.as_slice() == b"Japan1",
        _ => false,
    }
}

/// Which core font's metrics a font without `/Widths` takes.
fn core_widths(lower: &str) -> Option<FontWidths> {
    let bold = lower.contains("bold");
    let italic = lower.contains("italic") || lower.contains("oblique");
    if lower.contains("courier") {
        return Some(FontWidths::Fixed(600.0));
    }
    let table = if lower.contains("times") {
        match (bold, italic) {
            (true, true) => &tables::TIMES_BOLD_ITALIC,
            (true, false) => &tables::TIMES_BOLD,
            (false, true) => &tables::TIMES_ITALIC,
            (false, false) => &tables::TIMES_ROMAN,
        }
    } else if lower.contains("helvetica") || lower.contains("arial") {
        if bold {
            &tables::HELVETICA_BOLD
        } else {
            &tables::HELVETICA
        }
    } else if lower.contains("symbol") {
        &tables::SYMBOL_WIDTHS
    } else if lower.contains("dingbats") {
        &tables::ZAPF_DINGBATS_WIDTHS
    } else {
        return None;
    };
    Some(FontWidths::Core(table))
}

fn simple_widths(
    document: &Document<'_>,
    dictionary: &Dictionary,
    descriptor: Option<&Dictionary>,
    lower: &str,
    scale: f64,
) -> FontWidths {
    let resolved = |key: &[u8]| {
        dictionary
            .get(key)
            .and_then(|object| document.resolve(object).ok())
    };
    let missing = descriptor
        .and_then(|descriptor| descriptor.get(b"MissingWidth"))
        .and_then(Object::as_number)
        .unwrap_or(0.0);
    if let Some(Object::Array(items)) = resolved(b"Widths") {
        let first = resolved(b"FirstChar")
            .and_then(|object| object.as_integer())
            .unwrap_or(0)
            .max(0) as u32;
        let widths = items
            .iter()
            .map(|item| {
                document
                    .resolve(item)
                    .ok()
                    .and_then(|object| object.as_number())
                    .unwrap_or(missing)
            })
            .collect();
        return FontWidths::Simple {
            first,
            widths,
            missing,
        };
    }
    // An unknown font without widths: half an em, a typical average, so
    // gaps between strings still read as spaces or not.
    let fallback = if missing > 0.0 { missing } else { 0.5 / scale };
    core_widths(lower).unwrap_or(FontWidths::Fixed(fallback))
}

fn cid_widths(document: &Document<'_>, dictionary: &Dictionary) -> FontWidths {
    let descendant = dictionary
        .get(b"DescendantFonts")
        .and_then(|object| document.resolve(object).ok())
        .and_then(|object| object.as_array().and_then(|items| items.first().cloned()))
        .and_then(|object| document.resolve(&object).ok())
        .and_then(|object| object.as_dictionary().cloned());
    let Some(descendant) = descendant else {
        return FontWidths::Fixed(1000.0);
    };
    let default = descendant
        .get(b"DW")
        .and_then(Object::as_number)
        .unwrap_or(1000.0);
    let mut ranges = Vec::new();
    let mut singles = HashMap::new();
    let items = descendant
        .get(b"W")
        .and_then(|object| document.resolve(object).ok())
        .and_then(|object| object.as_array().map(<[Object]>::to_vec))
        .unwrap_or_default();
    let number = |object: &Object| {
        document
            .resolve(object)
            .ok()
            .and_then(|value| value.as_number())
    };
    let mut at = 0;
    while at + 1 < items.len() {
        let Some(first) = number(&items[at]) else {
            break;
        };
        let first = first as u32;
        match document.resolve(&items[at + 1]) {
            Ok(Object::Array(widths)) => {
                for (offset, width) in widths.iter().enumerate() {
                    if let Some(width) = number(width) {
                        singles.insert(first + offset as u32, width);
                    }
                }
                at += 2;
            }
            _ => {
                let (Some(last), Some(width)) =
                    (number(&items[at + 1]), items.get(at + 2).and_then(number))
                else {
                    break;
                };
                ranges.push((first, last as u32, width));
                at += 3;
            }
        }
    }
    FontWidths::Cid {
        ranges,
        singles,
        default,
    }
}

/// A simple font's code -> Unicode table and its multi-letter codes.
fn simple_encoding(
    document: &Document<'_>,
    dictionary: &Dictionary,
    lower: &str,
) -> ([u16; 256], HashMap<u8, &'static str>) {
    let built_in: &[u16; 256] = if lower.contains("symbol") {
        &tables::SYMBOL
    } else if lower.contains("dingbats") {
        &tables::ZAPF_DINGBATS
    } else {
        &tables::STANDARD
    };
    let base_named = |name: &[u8]| -> Option<&'static [u16; 256]> {
        match name {
            b"WinAnsiEncoding" => Some(&tables::WIN_ANSI),
            b"MacRomanEncoding" => Some(&tables::MAC_ROMAN),
            b"StandardEncoding" => Some(&tables::STANDARD),
            _ => None,
        }
    };
    let encoding = dictionary
        .get(b"Encoding")
        .and_then(|object| document.resolve(object).ok());
    let mut table = *built_in;
    let mut spelled = HashMap::new();
    match encoding {
        Some(Object::Name(name)) => {
            if let Some(base) = base_named(&name) {
                table = *base;
            }
        }
        Some(Object::Dictionary(encoding)) => {
            if let Some(base) = encoding
                .get(b"BaseEncoding")
                .and_then(Object::as_name)
                .and_then(base_named)
            {
                table = *base;
            }
            let differences = encoding
                .get(b"Differences")
                .and_then(|object| document.resolve(object).ok())
                .and_then(|object| object.as_array().map(<[Object]>::to_vec))
                .unwrap_or_default();
            let mut code: i64 = 0;
            for item in differences {
                match item {
                    Object::Integer(value) => code = value,
                    Object::Name(name) => {
                        if (0..256).contains(&code) {
                            let slot = code as usize;
                            match glyph_unicode(&name) {
                                Glyph::One(value) => table[slot] = value,
                                Glyph::Spelled(text) => {
                                    spelled.insert(slot as u8, text);
                                }
                                Glyph::Unknown => table[slot] = 0,
                            }
                        }
                        code += 1;
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    (table, spelled)
}

enum Glyph {
    One(u16),
    Spelled(&'static str),
    Unknown,
}

/// A glyph name's Unicode: the glyph list, a ligature, `uniXXXX`, or
/// `uXXXX`; a suffix after a period (`a.sc`) is dropped first.
fn glyph_unicode(name: &[u8]) -> Glyph {
    let name = std::str::from_utf8(name).unwrap_or("");
    let name = name.split('.').next().unwrap_or(name);
    if let Some((_, text)) = tables::LIGATURES.iter().find(|(glyph, _)| *glyph == name) {
        return Glyph::Spelled(text);
    }
    if let Some(code) = lookup_glyph(name) {
        return Glyph::One(code);
    }
    let hex = name
        .strip_prefix("uni")
        .filter(|digits| digits.len() == 4)
        .or_else(|| {
            name.strip_prefix('u')
                .filter(|digits| (4..=6).contains(&digits.len()))
        });
    match hex.and_then(|digits| u32::from_str_radix(digits, 16).ok()) {
        Some(value) if value < 0x10000 => Glyph::One(value as u16),
        _ => Glyph::Unknown,
    }
}

/// Binary search over the run-together sorted glyph names.
fn lookup_glyph(name: &str) -> Option<u16> {
    let ends = &tables::GLYPH_ENDS;
    let names = tables::GLYPH_NAMES;
    let (mut low, mut high) = (0usize, ends.len());
    while low < high {
        let middle = (low + high) / 2;
        let start = if middle == 0 {
            0
        } else {
            usize::from(ends[middle - 1])
        };
        let candidate = &names[start..usize::from(ends[middle])];
        match candidate.cmp(name) {
            std::cmp::Ordering::Equal => return Some(tables::GLYPH_CODES[middle]),
            std::cmp::Ordering::Less => low = middle + 1,
            std::cmp::Ordering::Greater => high = middle,
        }
    }
    None
}

/// A ToUnicode CMap: code-space ranges (how long each code is) and code
/// -> text mappings.
struct ToUnicode {
    /// (length in bytes, low, high)
    spaces: Vec<(usize, u32, u32)>,
    singles: HashMap<u32, String>,
    /// (low, high, first code point): consecutive codes to consecutive
    /// code points.
    ranges: Vec<(u32, u32, Vec<u16>)>,
}

impl ToUnicode {
    fn parse(data: &[u8]) -> ToUnicode {
        let mut map = ToUnicode {
            spaces: Vec::new(),
            singles: HashMap::new(),
            ranges: Vec::new(),
        };
        let mut lexer = Lexer::new(data);
        let mut operands: Vec<Object> = Vec::new();
        while let Some(token) = lexer.next_token() {
            match token {
                Token::Operand(object) => operands.push(object),
                Token::Operator(b"endcodespacerange") => {
                    for pair in operands.chunks_exact(2) {
                        if let (Object::String(low), Object::String(high)) = (&pair[0], &pair[1]) {
                            map.spaces.push((low.len().max(1), be(low), be(high)));
                        }
                    }
                    operands.clear();
                }
                Token::Operator(b"endbfchar") => {
                    for pair in operands.chunks_exact(2) {
                        if let (Object::String(code), Object::String(text)) = (&pair[0], &pair[1]) {
                            map.singles.insert(be(code), utf16(text));
                        }
                    }
                    operands.clear();
                }
                Token::Operator(b"endbfrange") => {
                    for triple in operands.chunks_exact(3) {
                        let (Object::String(low), Object::String(high)) = (&triple[0], &triple[1])
                        else {
                            continue;
                        };
                        let (low, high) = (be(low), be(high));
                        if high < low || high - low > 0xFFFF {
                            continue;
                        }
                        match &triple[2] {
                            Object::String(first) => {
                                let units: Vec<u16> = first
                                    .chunks(2)
                                    .map(|pair| {
                                        u16::from_be_bytes([pair[0], *pair.get(1).unwrap_or(&0)])
                                    })
                                    .collect();
                                map.ranges.push((low, high, units));
                            }
                            Object::Array(texts) => {
                                for (offset, text) in texts.iter().enumerate() {
                                    if let Object::String(text) = text {
                                        map.singles.insert(low + offset as u32, utf16(text));
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    operands.clear();
                }
                Token::Operator(_) => operands.clear(),
            }
        }
        map
    }

    /// How many bytes the code at the front of `bytes` takes.
    fn code_length(&self, bytes: &[u8]) -> usize {
        for length in 1..=4 {
            if bytes.len() < length {
                break;
            }
            let value = be(&bytes[..length]);
            let fits = self
                .spaces
                .iter()
                .any(|(size, low, high)| *size == length && (*low..=*high).contains(&value));
            if fits {
                return length;
            }
        }
        self.spaces.iter().map(|space| space.0).min().unwrap_or(1)
    }

    /// Appends a code's text; false when the map does not have it.
    fn text(&self, code: u32, out: &mut String) -> bool {
        if let Some(text) = self.singles.get(&code) {
            out.push_str(text);
            return true;
        }
        self.range_text(code, out)
    }

    /// A code in a bfrange: the range's first text with its last unit
    /// advanced by the code's offset.
    fn range_text(&self, code: u32, out: &mut String) -> bool {
        let Some((low, _, first)) = self
            .ranges
            .iter()
            .find(|(low, high, _)| (*low..=*high).contains(&code))
        else {
            return false;
        };
        let mut units = first.clone();
        if let Some(last) = units.last_mut() {
            *last = last.wrapping_add((code - low) as u16);
        }
        out.extend(char::decode_utf16(units).map(|unit| unit.unwrap_or('\u{fffd}')));
        true
    }
}

fn be(bytes: &[u8]) -> u32 {
    bytes
        .iter()
        .take(4)
        .fold(0u32, |value, byte| value << 8 | u32::from(*byte))
}

fn utf16(bytes: &[u8]) -> String {
    let units = bytes
        .chunks(2)
        .map(|pair| u16::from_be_bytes([pair[0], *pair.get(1).unwrap_or(&0)]));
    char::decode_utf16(units)
        .map(|unit| unit.unwrap_or('\u{fffd}'))
        .collect()
}

/// Loads a font once per object: pages share their fonts.
#[derive(Default)]
pub struct FontCache {
    loaded: HashMap<u32, std::rc::Rc<Font>>,
}

impl FontCache {
    /// The font a page's resources name `name`.
    pub fn get(
        &mut self,
        document: &Document<'_>,
        resources: Option<&Dictionary>,
        name: &[u8],
    ) -> Result<Option<std::rc::Rc<Font>>, PdfError> {
        let Some(fonts) = resources
            .and_then(|resources| resources.get(b"Font"))
            .and_then(|object| document.resolve(object).ok())
        else {
            return Ok(None);
        };
        let Some(entry) = fonts
            .as_dictionary()
            .and_then(|fonts| fonts.get(name))
            .cloned()
        else {
            return Ok(None);
        };
        if let Object::Reference(number, _) = entry
            && let Some(font) = self.loaded.get(&number)
        {
            return Ok(Some(font.clone()));
        }
        let Some(dictionary) = document.resolve(&entry)?.as_dictionary().cloned() else {
            return Ok(None);
        };
        let font = std::rc::Rc::new(Font::load(document, &dictionary));
        if let Object::Reference(number, _) = entry {
            self.loaded.insert(number, font.clone());
        }
        Ok(Some(font))
    }
}
