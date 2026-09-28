//! TrueType and OpenType fonts: the tables a converter needs to set text
//! in a font and embed it. Character to glyph (cmap formats 4 and 12),
//! advances (hmtx), the metrics a PDF font descriptor carries (head,
//! hhea, OS/2, post), the family name, and, for fonts with TrueType
//! outlines (glyf), a subset holding only the glyphs used.
//!
//! A collection (`.ttc`) holds several faces; each is opened by index.
//! Fonts with CFF outlines (`OTTO`) are read for their metrics and
//! coverage but not subset.

#[cfg(not(target_arch = "wasm32"))]
pub mod system;

use std::collections::BTreeSet;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontError(pub String);

impl std::fmt::Display for FontError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

fn fail(message: impl Into<String>) -> FontError {
    FontError(message.into())
}

fn u16_at(data: &[u8], at: usize) -> Result<u16, FontError> {
    data.get(at..at + 2)
        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
        .ok_or_else(|| fail("font cut short"))
}

fn i16_at(data: &[u8], at: usize) -> Result<i16, FontError> {
    u16_at(data, at).map(|value| value as i16)
}

fn u32_at(data: &[u8], at: usize) -> Result<u32, FontError> {
    data.get(at..at + 4)
        .map(|quad| u32::from_be_bytes([quad[0], quad[1], quad[2], quad[3]]))
        .ok_or_else(|| fail("font cut short"))
}

/// A table's tag, offset, and length.
type TableEntry = ([u8; 4], usize, usize);

/// Where each table of a face lies in the file.
fn table_directory(data: &[u8], face: u32) -> Result<(u32, Vec<TableEntry>), FontError> {
    let mut start = 0usize;
    if data.starts_with(b"ttcf") {
        let count = u32_at(data, 8)?;
        if face >= count {
            return Err(fail(format!(
                "the collection has {count} faces; there is no face {face}"
            )));
        }
        start = u32_at(data, 12 + 4 * face as usize)? as usize;
    }
    let version = u32_at(data, start)?;
    if !matches!(version, 0x0001_0000 | 0x7472_7565 | 0x4F54_544F) {
        return Err(fail("not a TrueType or OpenType font"));
    }
    let count = u16_at(data, start + 4)? as usize;
    let mut tables = Vec::with_capacity(count);
    for index in 0..count {
        let entry = start + 12 + 16 * index;
        let tag: [u8; 4] = data
            .get(entry..entry + 4)
            .and_then(|tag| tag.try_into().ok())
            .ok_or_else(|| fail("font cut short"))?;
        let offset = u32_at(data, entry + 8)? as usize;
        let length = u32_at(data, entry + 12)? as usize;
        tables.push((tag, offset, length));
    }
    Ok((version, tables))
}

/// A face's character map: the best Unicode subtable, as its bytes.
#[derive(Clone)]
struct CharMap {
    format: u16,
    table: Vec<u8>,
}

impl CharMap {
    /// Picks the subtable to use from a `cmap` table: full Unicode
    /// (format 12) before the basic plane (format 4).
    fn from_cmap(cmap: &[u8]) -> Result<CharMap, FontError> {
        let count = u16_at(cmap, 2)? as usize;
        let mut best: Option<(u8, usize)> = None;
        for index in 0..count {
            let record = 4 + 8 * index;
            let platform = u16_at(cmap, record)?;
            let encoding = u16_at(cmap, record + 2)?;
            let offset = u32_at(cmap, record + 4)? as usize;
            let format = u16_at(cmap, offset).unwrap_or(0);
            let rank = match (platform, encoding, format) {
                (3, 10, 12) | (0, 4, 12) | (0, 6, 12) => 3,
                (3, 1, 4) | (0, 3, 4) | (0, 1, 4) | (0, 0, 4) => 2,
                (0, _, 12) => 2,
                (0, _, 4) => 1,
                _ => 0,
            };
            if rank > 0 && best.is_none_or(|(known, _)| rank > known) {
                best = Some((rank, offset));
            }
        }
        let (_, offset) = best.ok_or_else(|| fail("the font has no Unicode character map"))?;
        let format = u16_at(cmap, offset)?;
        let length = if format == 12 {
            u32_at(cmap, offset + 4)? as usize
        } else {
            u16_at(cmap, offset + 2)? as usize
        };
        let table = cmap
            .get(offset..(offset + length).min(cmap.len()))
            .ok_or_else(|| fail("character map cut short"))?
            .to_vec();
        Ok(CharMap { format, table })
    }

    /// The glyph for a character; `None` for the missing glyph (0).
    fn glyph(&self, char: char) -> Option<u16> {
        let code = u32::from(char);
        let table = &self.table;
        let glyph = if self.format == 12 {
            let groups = u32_at(table, 12).ok()? as usize;
            let (mut low, mut high) = (0usize, groups);
            let mut found = 0;
            while low < high {
                let middle = (low + high) / 2;
                let at = 16 + 12 * middle;
                let start = u32_at(table, at).ok()?;
                let end = u32_at(table, at + 4).ok()?;
                if code < start {
                    high = middle;
                } else if code > end {
                    low = middle + 1;
                } else {
                    found = u32_at(table, at + 8).ok()? + (code - start);
                    break;
                }
            }
            found
        } else {
            if code > 0xFFFF {
                return None;
            }
            let code = code as u16;
            let segments = (u16_at(table, 6).ok()? / 2) as usize;
            let ends = 14;
            let starts = ends + 2 * segments + 2;
            let deltas = starts + 2 * segments;
            let ranges = deltas + 2 * segments;
            let (mut low, mut high) = (0usize, segments);
            while low < high {
                let middle = (low + high) / 2;
                if u16_at(table, ends + 2 * middle).ok()? < code {
                    low = middle + 1;
                } else {
                    high = middle;
                }
            }
            if low >= segments {
                return None;
            }
            let start = u16_at(table, starts + 2 * low).ok()?;
            if code < start {
                return None;
            }
            let delta = u16_at(table, deltas + 2 * low).ok()?;
            let range = u16_at(table, ranges + 2 * low).ok()?;
            let value = if range == 0 {
                code.wrapping_add(delta)
            } else {
                let at = ranges + 2 * low + range as usize + 2 * (code - start) as usize;
                let raw = u16_at(table, at).ok()?;
                if raw == 0 { 0 } else { raw.wrapping_add(delta) }
            };
            u32::from(value)
        };
        u16::try_from(glyph).ok().filter(|glyph| *glyph != 0)
    }
}

/// Which characters a font file's faces cover, read from their character
/// maps alone: finding a font for a script does not read whole files.
pub struct Coverage {
    map: CharMap,
    /// Whether the face has TrueType outlines (and so can be subset).
    pub glyf: bool,
}

impl Coverage {
    /// Reads the character map of `face` from a font's leading bytes and
    /// a reader for the rest (`read_at(offset, length)`).
    pub fn probe(
        head: &[u8],
        face: u32,
        read_at: &mut dyn FnMut(usize, usize) -> Option<Vec<u8>>,
    ) -> Result<Coverage, FontError> {
        let (_, tables) = table_directory(head, face)?;
        let (_, offset, length) = tables
            .iter()
            .find(|(tag, _, _)| tag == b"cmap")
            .copied()
            .ok_or_else(|| fail("the font has no character map"))?;
        let cmap = read_at(offset, length).ok_or_else(|| fail("character map cut short"))?;
        Ok(Coverage {
            map: CharMap::from_cmap(&cmap)?,
            glyf: tables.iter().any(|(tag, _, _)| tag == b"glyf"),
        })
    }

    pub fn covers(&self, char: char) -> bool {
        self.map.glyph(char).is_some()
    }
}

/// One face of a font, parsed for setting and embedding text.
pub struct Font {
    data: Arc<Vec<u8>>,
    tables: Vec<TableEntry>,
    map: CharMap,
    pub units_per_em: u16,
    pub glyph_count: u16,
    long_loca: bool,
    horizontal_metrics: u16,
    pub ascent: i16,
    pub descent: i16,
    pub cap_height: i16,
    pub bbox: [i16; 4],
    pub italic_angle: f64,
    pub weight: u16,
    pub fixed_pitch: bool,
    pub family: String,
}

impl Font {
    pub fn parse(data: Arc<Vec<u8>>, face: u32) -> Result<Font, FontError> {
        let (_, tables) = table_directory(&data, face)?;
        let table = |tag: &[u8; 4]| -> Result<&[u8], FontError> {
            let (_, offset, length) = tables
                .iter()
                .find(|(known, _, _)| known == tag)
                .copied()
                .ok_or_else(|| {
                    fail(format!(
                        "the font has no {} table",
                        String::from_utf8_lossy(tag)
                    ))
                })?;
            data.get(offset..offset + length)
                .ok_or_else(|| fail("font table outside the file"))
        };
        let head = table(b"head")?;
        let hhea = table(b"hhea")?;
        let maxp = table(b"maxp")?;
        let map = CharMap::from_cmap(table(b"cmap")?)?;
        let units_per_em = u16_at(head, 18)?.max(16);
        let bbox = [
            i16_at(head, 36)?,
            i16_at(head, 38)?,
            i16_at(head, 40)?,
            i16_at(head, 42)?,
        ];
        let long_loca = i16_at(head, 50)? == 1;
        let (mut ascent, mut descent) = (i16_at(hhea, 4)?, i16_at(hhea, 6)?);
        let horizontal_metrics = u16_at(hhea, 34)?;
        let glyph_count = u16_at(maxp, 4)?;
        let mut cap_height = ascent;
        let mut weight = 400;
        if let Ok(os2) = table(b"OS/2") {
            weight = u16_at(os2, 4).unwrap_or(400);
            if let (Ok(typo_ascent), Ok(typo_descent)) = (i16_at(os2, 68), i16_at(os2, 70))
                && typo_ascent > 0
            {
                ascent = typo_ascent;
                descent = typo_descent;
            }
            if u16_at(os2, 0).unwrap_or(0) >= 2
                && let Ok(height) = i16_at(os2, 88)
                && height > 0
            {
                cap_height = height;
            }
        }
        let (italic_angle, fixed_pitch) = match table(b"post") {
            Ok(post) => (
                f64::from(i16_at(post, 4).unwrap_or(0))
                    + f64::from(u16_at(post, 6).unwrap_or(0)) / 65536.0,
                u32_at(post, 12).unwrap_or(0) != 0,
            ),
            Err(_) => (0.0, false),
        };
        let family = table(b"name")
            .ok()
            .and_then(family_name)
            .unwrap_or_default();
        Ok(Font {
            tables,
            map,
            units_per_em,
            glyph_count,
            long_loca,
            horizontal_metrics,
            ascent,
            descent,
            cap_height,
            bbox,
            italic_angle,
            weight,
            fixed_pitch,
            family,
            data,
        })
    }

    fn table(&self, tag: &[u8; 4]) -> Option<&[u8]> {
        let (_, offset, length) = self
            .tables
            .iter()
            .find(|(known, _, _)| known == tag)
            .copied()?;
        self.data.get(offset..offset + length)
    }

    /// Whether the face has TrueType outlines (and so can be subset).
    pub fn has_glyf(&self) -> bool {
        self.table(b"glyf").is_some() && self.table(b"loca").is_some()
    }

    /// The glyph for a character; `None` when the font lacks it.
    pub fn glyph(&self, char: char) -> Option<u16> {
        self.map.glyph(char)
    }

    /// A glyph's advance in font units.
    pub fn advance(&self, glyph: u16) -> u16 {
        let Some(hmtx) = self.table(b"hmtx") else {
            return self.units_per_em / 2;
        };
        let index = glyph.min(self.horizontal_metrics.saturating_sub(1));
        u16_at(hmtx, 4 * index as usize).unwrap_or(self.units_per_em / 2)
    }

    /// Converts font units to thousandths of an em, as PDF widths are.
    pub fn to_thousandths(&self, units: f64) -> f64 {
        units * 1000.0 / f64::from(self.units_per_em)
    }

    /// A glyph's outline bytes in `glyf`.
    fn glyph_data(&self, glyph: u16) -> Option<&[u8]> {
        let loca = self.table(b"loca")?;
        let glyf = self.table(b"glyf")?;
        let index = glyph as usize;
        let (start, end) = if self.long_loca {
            (
                u32_at(loca, 4 * index).ok()? as usize,
                u32_at(loca, 4 * index + 4).ok()? as usize,
            )
        } else {
            (
                u16_at(loca, 2 * index).ok()? as usize * 2,
                u16_at(loca, 2 * index + 2).ok()? as usize * 2,
            )
        };
        if end <= start {
            return Some(&[]);
        }
        glyf.get(start..end)
    }

    /// A TrueType font holding only `glyphs` (and the missing glyph and
    /// every component a composite glyph uses), each at its own index, so
    /// glyph ids need no renumbering. The tables a PDF viewer needs to
    /// draw TrueType outlines are kept: head, hhea, maxp, hmtx, loca,
    /// glyf, and the hinting tables cvt, fpgm, prep.
    pub fn subset(&self, glyphs: &BTreeSet<u16>) -> Result<Vec<u8>, FontError> {
        if !self.has_glyf() {
            return Err(fail("only fonts with TrueType outlines are subset"));
        }
        let mut keep: BTreeSet<u16> = glyphs
            .iter()
            .copied()
            .filter(|glyph| *glyph < self.glyph_count)
            .collect();
        keep.insert(0);
        // Follow composite glyphs to their components.
        let mut pending: Vec<u16> = keep.iter().copied().collect();
        while let Some(glyph) = pending.pop() {
            for component in components(self.glyph_data(glyph).unwrap_or(&[])) {
                if component < self.glyph_count && keep.insert(component) {
                    pending.push(component);
                }
            }
        }
        let mut glyf = Vec::new();
        let mut loca = Vec::with_capacity(4 * (self.glyph_count as usize + 1));
        for glyph in 0..self.glyph_count {
            loca.extend_from_slice(&(glyf.len() as u32).to_be_bytes());
            if keep.contains(&glyph) {
                glyf.extend_from_slice(self.glyph_data(glyph).unwrap_or(&[]));
                while glyf.len() % 4 != 0 {
                    glyf.push(0);
                }
            }
        }
        loca.extend_from_slice(&(glyf.len() as u32).to_be_bytes());
        let mut head = self
            .table(b"head")
            .ok_or_else(|| fail("no head table"))?
            .to_vec();
        if head.len() < 54 {
            return Err(fail("head table cut short"));
        }
        head[8..12].copy_from_slice(&[0, 0, 0, 0]);
        head[50..52].copy_from_slice(&1i16.to_be_bytes());
        let mut tables: Vec<([u8; 4], Vec<u8>)> =
            vec![(*b"glyf", glyf), (*b"head", head), (*b"loca", loca)];
        for tag in [b"cvt ", b"fpgm", b"hhea", b"hmtx", b"maxp", b"prep"] {
            if let Some(table) = self.table(tag) {
                tables.push((*tag, table.to_vec()));
            }
        }
        tables.sort_by_key(|table| table.0);
        Ok(write_sfnt(tables))
    }
}

/// The glyphs a composite glyph is built from.
fn components(glyph: &[u8]) -> Vec<u16> {
    let mut out = Vec::new();
    if glyph.len() < 10 || i16_at(glyph, 0).unwrap_or(0) >= 0 {
        return out;
    }
    let mut at = 10;
    while let (Ok(flags), Ok(component)) = (u16_at(glyph, at), u16_at(glyph, at + 2)) {
        out.push(component);
        at += 4;
        at += if flags & 0x0001 != 0 { 4 } else { 2 };
        if flags & 0x0008 != 0 {
            at += 2;
        } else if flags & 0x0040 != 0 {
            at += 4;
        } else if flags & 0x0080 != 0 {
            at += 8;
        }
        if flags & 0x0020 == 0 {
            break;
        }
    }
    out
}

fn checksum(bytes: &[u8]) -> u32 {
    bytes
        .chunks(4)
        .map(|chunk| {
            let mut quad = [0u8; 4];
            quad[..chunk.len()].copy_from_slice(chunk);
            u32::from_be_bytes(quad)
        })
        .fold(0u32, u32::wrapping_add)
}

/// Writes tables (sorted by tag) as a TrueType file, with checksums and
/// the head table's adjustment.
fn write_sfnt(tables: Vec<([u8; 4], Vec<u8>)>) -> Vec<u8> {
    let count = tables.len() as u16;
    let mut power = 1u16;
    let mut exponent = 0u16;
    while power * 2 <= count {
        power *= 2;
        exponent += 1;
    }
    let mut out = Vec::new();
    out.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    out.extend_from_slice(&count.to_be_bytes());
    out.extend_from_slice(&(power * 16).to_be_bytes());
    out.extend_from_slice(&exponent.to_be_bytes());
    out.extend_from_slice(&(count * 16 - power * 16).to_be_bytes());
    let mut offset = 12 + 16 * tables.len();
    let mut head_at = None;
    for (tag, data) in &tables {
        out.extend_from_slice(tag);
        out.extend_from_slice(&checksum(data).to_be_bytes());
        out.extend_from_slice(&(offset as u32).to_be_bytes());
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        if tag == b"head" {
            head_at = Some(offset);
        }
        offset += data.len().div_ceil(4) * 4;
    }
    for (_, data) in &tables {
        out.extend_from_slice(data);
        while out.len() % 4 != 0 {
            out.push(0);
        }
    }
    if let Some(at) = head_at {
        let adjustment = 0xB1B0_AFBAu32.wrapping_sub(checksum(&out));
        out[at + 8..at + 12].copy_from_slice(&adjustment.to_be_bytes());
    }
    out
}

/// The family name from a `name` table: the typographic family (16) when
/// present, else the family (1), preferring Windows Unicode records.
fn family_name(name: &[u8]) -> Option<String> {
    let count = u16_at(name, 2).ok()? as usize;
    let strings = u16_at(name, 4).ok()? as usize;
    let mut best: Option<(u8, String)> = None;
    for index in 0..count {
        let record = 6 + 12 * index;
        let platform = u16_at(name, record).ok()?;
        let name_id = u16_at(name, record + 6).ok()?;
        let length = u16_at(name, record + 8).ok()? as usize;
        let offset = u16_at(name, record + 10).ok()? as usize;
        if name_id != 1 && name_id != 16 {
            continue;
        }
        let bytes = name.get(strings + offset..strings + offset + length)?;
        let text = if platform == 3 || platform == 0 {
            let units: Vec<u16> = bytes
                .chunks(2)
                .map(|pair| u16::from_be_bytes([pair[0], *pair.get(1).unwrap_or(&0)]))
                .collect();
            String::from_utf16_lossy(&units)
        } else {
            bytes.iter().map(|byte| *byte as char).collect()
        };
        let rank = u8::from(name_id == 16) * 2 + u8::from(platform == 3);
        if best.as_ref().is_none_or(|(known, _)| rank > *known) {
            best = Some((rank, text));
        }
    }
    best.map(|(_, text)| text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn droid() -> Font {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/fonts/DroidSans.ttf"
        );
        Font::parse(Arc::new(std::fs::read(path).expect("fixture")), 0).expect("parses")
    }

    #[test]
    fn the_character_map_and_metrics_read() {
        let font = droid();
        assert_eq!(font.family, "Droid Sans");
        assert_eq!(font.units_per_em, 2048);
        assert!(font.has_glyf());
        let (a, alpha, zhe) = (font.glyph('A'), font.glyph('α'), font.glyph('ж'));
        assert!(a.is_some() && alpha.is_some() && zhe.is_some());
        assert_eq!(font.glyph('\u{378}'), None);
        assert!(font.advance(a.unwrap()) > 0);
    }

    /// A subset keeps the asked-for glyphs (and glyph 0) byte for byte,
    /// empties the rest, and its checksums balance as the format requires.
    #[test]
    fn a_subset_keeps_only_its_glyphs() {
        let font = droid();
        let kept: BTreeSet<u16> = ['S', 'u', 'b']
            .iter()
            .map(|char| font.glyph(*char).unwrap())
            .collect();
        let subset = font.subset(&kept).expect("subsets");
        assert!(subset.len() * 10 < 190_000, "{} bytes", subset.len());
        assert_eq!(checksum(&subset), 0xB1B0_AFBA);
        let (_, tables) = table_directory(&subset, 0).expect("reads back");
        let tags: Vec<&[u8; 4]> = tables.iter().map(|(tag, _, _)| tag).collect();
        assert!(
            tags.windows(2).all(|pair| pair[0] < pair[1]),
            "tables in tag order"
        );
        let reread = Font {
            data: Arc::new(subset),
            tables,
            map: font.map.clone(),
            long_loca: true,
            ..droid()
        };
        for glyph in 0..font.glyph_count {
            let expected: &[u8] = if glyph == 0 || kept.contains(&glyph) {
                font.glyph_data(glyph).unwrap()
            } else {
                &[]
            };
            let got = reread.glyph_data(glyph).unwrap();
            assert_eq!(&got[..expected.len()], expected, "glyph {glyph}");
            assert!(got[expected.len()..].iter().all(|byte| *byte == 0));
        }
    }

    #[test]
    fn composites_list_their_components() {
        // A composite with two components: words arguments, then bytes.
        let mut glyph = vec![0xFF, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0];
        glyph.extend_from_slice(&[0x00, 0x21, 0x00, 0x05, 0, 1, 0, 2]);
        glyph.extend_from_slice(&[0x00, 0x00, 0x00, 0x07, 1, 2]);
        assert_eq!(components(&glyph), [5, 7]);
    }

    #[test]
    fn checksums_pad_to_whole_words() {
        assert_eq!(checksum(&[0, 0, 0, 1, 0, 0, 1]), 1 + 256);
    }
}
