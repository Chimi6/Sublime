//! CFF outlines (the `CFF ` table of an OpenType font) subset for a PDF:
//! a CID-keyed font (every CJK font: Hiragino, Noto Sans CJK, Source Han)
//! keeps only the glyphs a document uses and the subroutines they call.
//!
//! Nothing is renumbered: an unused glyph's charstring becomes a bare
//! `endchar` and an unused subroutine a bare `return`, each in its place,
//! so glyph ids, subroutine numbers, and the font's FDSelect stay as they
//! were. The charset is rewritten so that every glyph's CID is its glyph
//! id, which lets a PDF address glyphs by id through `Identity-H`, as it
//! does TrueType fonts. The subroutines a glyph calls are found by
//! walking its Type 2 charstring (counting stem hints to step over hint
//! masks), so the subset holds exactly what drawing the glyphs needs.

use std::collections::BTreeSet;

use super::{FontError, fail};

/// A CFF INDEX: its items' byte ranges in the table.
struct Index {
    items: Vec<std::ops::Range<usize>>,
    end: usize,
}

fn read_index(data: &[u8], at: usize) -> Result<Index, FontError> {
    let count = usize::from(u16::from_be_bytes([
        *data.get(at).ok_or_else(|| fail("CFF INDEX cut short"))?,
        *data
            .get(at + 1)
            .ok_or_else(|| fail("CFF INDEX cut short"))?,
    ]));
    if count == 0 {
        return Ok(Index {
            items: Vec::new(),
            end: at + 2,
        });
    }
    let size = usize::from(
        *data
            .get(at + 2)
            .ok_or_else(|| fail("CFF INDEX cut short"))?,
    );
    if !(1..=4).contains(&size) {
        return Err(fail("CFF INDEX offset size out of range"));
    }
    let offsets_at = at + 3;
    let offset = |index: usize| -> Result<usize, FontError> {
        let start = offsets_at + index * size;
        let bytes = data
            .get(start..start + size)
            .ok_or_else(|| fail("CFF INDEX cut short"))?;
        Ok(bytes
            .iter()
            .fold(0usize, |value, byte| value << 8 | usize::from(*byte)))
    };
    let data_start = offsets_at + (count + 1) * size - 1;
    let mut items = Vec::with_capacity(count);
    for index in 0..count {
        let (start, end) = (offset(index)?, offset(index + 1)?);
        if end < start || data_start + end > data.len() {
            return Err(fail("CFF INDEX offsets out of range"));
        }
        items.push(data_start + start..data_start + end);
    }
    let end = data_start + offset(count)?;
    Ok(Index { items, end })
}

fn write_index(items: &[&[u8]], out: &mut Vec<u8>) {
    out.extend_from_slice(&(items.len() as u16).to_be_bytes());
    if items.is_empty() {
        return;
    }
    let total: usize = items.iter().map(|item| item.len()).sum::<usize>() + 1;
    let size: usize = match total {
        0..=0xFF => 1,
        0x100..=0xFFFF => 2,
        0x1_0000..=0xFF_FFFF => 3,
        _ => 4,
    };
    out.push(size as u8);
    let mut offset = 1usize;
    let put = |value: usize, out: &mut Vec<u8>| {
        out.extend_from_slice(&value.to_be_bytes()[std::mem::size_of::<usize>() - size..]);
    };
    put(offset, out);
    for item in items {
        offset += item.len();
        put(offset, out);
    }
    for item in items {
        out.extend_from_slice(item);
    }
}

/// A DICT operand.
#[derive(Debug, Clone, PartialEq)]
enum Operand {
    Integer(i32),
    /// A real, kept as its encoded bytes (nibbles), which are copied out.
    Real(Vec<u8>),
}

impl Operand {
    fn integer(&self) -> Option<i32> {
        match self {
            Operand::Integer(value) => Some(*value),
            Operand::Real(_) => None,
        }
    }
}

/// A DICT: its entries in order, each operator (12 x as 1200 + x) with
/// its operands.
type Dict = Vec<(u16, Vec<Operand>)>;

fn read_dict(data: &[u8]) -> Result<Dict, FontError> {
    let mut entries = Vec::new();
    let mut operands = Vec::new();
    let mut at = 0;
    while at < data.len() {
        let byte = data[at];
        match byte {
            0..=21 => {
                let operator = if byte == 12 {
                    at += 1;
                    1200 + u16::from(*data.get(at).ok_or_else(|| fail("CFF DICT cut short"))?)
                } else {
                    u16::from(byte)
                };
                at += 1;
                entries.push((operator, std::mem::take(&mut operands)));
            }
            28 => {
                let value = i16::from_be_bytes([
                    *data.get(at + 1).ok_or_else(|| fail("CFF DICT cut short"))?,
                    *data.get(at + 2).ok_or_else(|| fail("CFF DICT cut short"))?,
                ]);
                operands.push(Operand::Integer(i32::from(value)));
                at += 3;
            }
            29 => {
                let bytes = data
                    .get(at + 1..at + 5)
                    .ok_or_else(|| fail("CFF DICT cut short"))?;
                operands.push(Operand::Integer(i32::from_be_bytes([
                    bytes[0], bytes[1], bytes[2], bytes[3],
                ])));
                at += 5;
            }
            30 => {
                let start = at;
                at += 1;
                loop {
                    let nibbles = *data.get(at).ok_or_else(|| fail("CFF DICT cut short"))?;
                    at += 1;
                    if nibbles & 0x0F == 0x0F || nibbles >> 4 == 0x0F {
                        break;
                    }
                }
                operands.push(Operand::Real(data[start..at].to_vec()));
            }
            32..=246 => {
                operands.push(Operand::Integer(i32::from(byte) - 139));
                at += 1;
            }
            247..=250 => {
                let next = i32::from(*data.get(at + 1).ok_or_else(|| fail("CFF DICT cut short"))?);
                operands.push(Operand::Integer((i32::from(byte) - 247) * 256 + next + 108));
                at += 2;
            }
            251..=254 => {
                let next = i32::from(*data.get(at + 1).ok_or_else(|| fail("CFF DICT cut short"))?);
                operands.push(Operand::Integer(
                    -(i32::from(byte) - 251) * 256 - next - 108,
                ));
                at += 2;
            }
            _ => return Err(fail("CFF DICT holds a reserved byte")),
        }
    }
    Ok(entries)
}

/// A DICT encoded; every integer an offset could be is written in the
/// fixed five-byte form, so a DICT's size does not depend on where things
/// end up.
fn write_dict(dict: &Dict) -> Vec<u8> {
    let mut out = Vec::new();
    for (operator, operands) in dict {
        for operand in operands {
            match operand {
                Operand::Integer(value) => {
                    out.push(29);
                    out.extend_from_slice(&value.to_be_bytes());
                }
                Operand::Real(bytes) => out.extend_from_slice(bytes),
            }
        }
        if *operator >= 1200 {
            out.push(12);
            out.push((operator - 1200) as u8);
        } else {
            out.push(*operator as u8);
        }
    }
    out
}

fn get(dict: &Dict, operator: u16) -> Option<&[Operand]> {
    dict.iter()
        .find(|(known, _)| *known == operator)
        .map(|(_, operands)| operands.as_slice())
}

fn set(dict: &mut Dict, operator: u16, operands: Vec<Operand>) {
    match dict.iter_mut().find(|(known, _)| *known == operator) {
        Some(entry) => entry.1 = operands,
        None => dict.push((operator, operands)),
    }
}

const CHARSET: u16 = 15;
const CHAR_STRINGS: u16 = 17;
const PRIVATE: u16 = 18;
const SUBRS: u16 = 19;
const ROS: u16 = 1230;
const CHARSTRING_TYPE: u16 = 1206;
const FD_ARRAY: u16 = 1236;
const FD_SELECT: u16 = 1237;

/// The subroutine bias Type 2 charstrings add to an index.
fn bias(count: usize) -> i32 {
    if count < 1240 {
        107
    } else if count < 33900 {
        1131
    } else {
        32768
    }
}

/// What walking a charstring tracks: the operand stack (its numbers, for
/// the subroutine a call names), the stem hints seen (for the bytes a
/// hint mask takes), and which subroutines were called.
struct Walk<'a> {
    data: &'a [u8],
    global: &'a [std::ops::Range<usize>],
    local: &'a [std::ops::Range<usize>],
    used_global: &'a mut BTreeSet<usize>,
    used_local: &'a mut BTreeSet<usize>,
    stack: Vec<i32>,
    stems: usize,
    done: bool,
}

impl Walk<'_> {
    fn run(&mut self, range: std::ops::Range<usize>, depth: usize) -> Result<(), FontError> {
        if depth > 10 {
            return Err(fail("CFF subroutines nest too deep"));
        }
        let code = &self.data[range];
        let mut at = 0;
        while at < code.len() && !self.done {
            let byte = code[at];
            match byte {
                32..=246 => {
                    self.stack.push(i32::from(byte) - 139);
                    at += 1;
                }
                247..=250 => {
                    let next = i32::from(*code.get(at + 1).unwrap_or(&0));
                    self.stack.push((i32::from(byte) - 247) * 256 + next + 108);
                    at += 2;
                }
                251..=254 => {
                    let next = i32::from(*code.get(at + 1).unwrap_or(&0));
                    self.stack.push(-(i32::from(byte) - 251) * 256 - next - 108);
                    at += 2;
                }
                28 => {
                    let value = i16::from_be_bytes([
                        *code.get(at + 1).unwrap_or(&0),
                        *code.get(at + 2).unwrap_or(&0),
                    ]);
                    self.stack.push(i32::from(value));
                    at += 3;
                }
                255 => {
                    // A 16.16 fixed number: its integer part is enough here.
                    let value = i32::from_be_bytes([
                        *code.get(at + 1).unwrap_or(&0),
                        *code.get(at + 2).unwrap_or(&0),
                        *code.get(at + 3).unwrap_or(&0),
                        *code.get(at + 4).unwrap_or(&0),
                    ]);
                    self.stack.push(value >> 16);
                    at += 5;
                }
                // hstem, vstem, hstemhm, vstemhm.
                1 | 3 | 18 | 23 => {
                    self.stems += self.stack.len() / 2;
                    self.stack.clear();
                    at += 1;
                }
                // hintmask, cntrmask: stems first (an implied vstem), then
                // a byte for every eight stems.
                19 | 20 => {
                    self.stems += self.stack.len() / 2;
                    self.stack.clear();
                    at += 1 + self.stems.div_ceil(8);
                }
                10 | 29 => {
                    let local = byte == 10;
                    let Some(number) = self.stack.pop() else {
                        return Err(fail("CFF subroutine call with no number"));
                    };
                    let subrs = if local { self.local } else { self.global };
                    let index = number + bias(subrs.len());
                    let Some(subr) = usize::try_from(index)
                        .ok()
                        .and_then(|index| subrs.get(index).cloned())
                    else {
                        return Err(fail("CFF subroutine number out of range"));
                    };
                    if local {
                        self.used_local.insert(index as usize);
                    } else {
                        self.used_global.insert(index as usize);
                    }
                    self.run(subr, depth + 1)?;
                    at += 1;
                }
                11 => return Ok(()),
                14 => {
                    self.done = true;
                    return Ok(());
                }
                12 => {
                    self.stack.clear();
                    at += 2;
                }
                _ => {
                    self.stack.clear();
                    at += 1;
                }
            }
        }
        Ok(())
    }
}

/// Whether CFF table bytes (its first few kilobytes suffice) hold a
/// CID-keyed font: its Top DICT has the ROS operator.
pub fn is_cid_keyed(cff: &[u8]) -> bool {
    let parse = || -> Result<bool, FontError> {
        let header_size = usize::from(*cff.get(2).ok_or_else(|| fail("short"))?);
        let names = read_index(cff, header_size)?;
        let tops = read_index(cff, names.end)?;
        let top = tops.items.first().ok_or_else(|| fail("no Top DICT"))?;
        Ok(get(&read_dict(&cff[top.clone()])?, ROS).is_some())
    };
    parse().unwrap_or(false)
}

/// The Registry, Ordering, and Supplement a CID-keyed font names.
pub fn registry_ordering(cff: &[u8]) -> Option<(String, String, i32)> {
    let header_size = usize::from(*cff.get(2)?);
    let names = read_index(cff, header_size).ok()?;
    let tops = read_index(cff, names.end).ok()?;
    let strings = read_index(cff, tops.end).ok()?;
    let top = read_dict(&cff[tops.items.first()?.clone()]).ok()?;
    let ros = get(&top, ROS)?;
    let string = |sid: i32| -> Option<String> {
        let index = usize::try_from(sid).ok()?.checked_sub(391)?;
        let range = strings.items.get(index)?.clone();
        Some(String::from_utf8_lossy(&cff[range]).into_owned())
    };
    Some((
        string(ros.first()?.integer()?)?,
        string(ros.get(1)?.integer()?)?,
        ros.get(2)?.integer()?,
    ))
}

/// The CFF table of a CID-keyed font holding only `glyphs` (and glyph 0),
/// every glyph's CID set to its glyph id.
pub fn subset(cff: &[u8], glyphs: &BTreeSet<u16>) -> Result<Vec<u8>, FontError> {
    let header_size = usize::from(*cff.get(2).ok_or_else(|| fail("CFF cut short"))?);
    let names = read_index(cff, header_size)?;
    let tops = read_index(cff, names.end)?;
    let strings = read_index(cff, tops.end)?;
    let global = read_index(cff, strings.end)?;
    let top_range = tops
        .items
        .first()
        .ok_or_else(|| fail("CFF has no Top DICT"))?
        .clone();
    let mut top = read_dict(&cff[top_range])?;
    if get(&top, ROS).is_none() {
        return Err(fail("only CID-keyed CFF fonts are subset"));
    }
    if get(&top, CHARSTRING_TYPE)
        .and_then(|operands| operands.first()?.integer())
        .unwrap_or(2)
        != 2
    {
        return Err(fail("CFF charstrings are not Type 2"));
    }
    let offset_of = |dict: &Dict, operator: u16| -> Result<usize, FontError> {
        get(dict, operator)
            .and_then(|operands| operands.last()?.integer())
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| fail("CFF DICT lacks an offset"))
    };
    let char_strings = read_index(cff, offset_of(&top, CHAR_STRINGS)?)?;
    let glyph_count = char_strings.items.len();
    let fd_select_at = offset_of(&top, FD_SELECT)?;
    let fd_array = read_index(cff, offset_of(&top, FD_ARRAY)?)?;
    // Each font DICT, its Private DICT, and that DICT's local subroutines.
    struct FontDict {
        dict: Dict,
        private: Dict,
        local: Vec<std::ops::Range<usize>>,
        used: BTreeSet<usize>,
    }
    let mut fonts = Vec::with_capacity(fd_array.items.len());
    for range in &fd_array.items {
        let dict = read_dict(&cff[range.clone()])?;
        let private_operands =
            get(&dict, PRIVATE).ok_or_else(|| fail("CFF font DICT lacks a Private DICT"))?;
        let size = private_operands
            .first()
            .and_then(Operand::integer)
            .unwrap_or(0)
            .max(0) as usize;
        let offset = private_operands
            .get(1)
            .and_then(Operand::integer)
            .unwrap_or(0)
            .max(0) as usize;
        let private_bytes = cff
            .get(offset..offset + size)
            .ok_or_else(|| fail("CFF Private DICT outside the table"))?;
        let private = read_dict(private_bytes)?;
        let local = match get(&private, SUBRS).and_then(|operands| operands.first()?.integer()) {
            Some(relative) => read_index(cff, offset + relative.max(0) as usize)?.items,
            None => Vec::new(),
        };
        fonts.push(FontDict {
            dict,
            private,
            local,
            used: BTreeSet::new(),
        });
    }
    let font_of = |glyph: usize| -> Result<usize, FontError> {
        let format = *cff
            .get(fd_select_at)
            .ok_or_else(|| fail("CFF FDSelect cut short"))?;
        match format {
            0 => Ok(usize::from(
                *cff.get(fd_select_at + 1 + glyph)
                    .ok_or_else(|| fail("CFF FDSelect cut short"))?,
            )),
            3 => {
                let count = usize::from(u16::from_be_bytes([
                    cff[fd_select_at + 1],
                    cff[fd_select_at + 2],
                ]));
                let mut chosen = 0;
                for range in 0..count {
                    let at = fd_select_at + 3 + range * 3;
                    let first = usize::from(u16::from_be_bytes([cff[at], cff[at + 1]]));
                    if first > glyph {
                        break;
                    }
                    chosen = usize::from(cff[at + 2]);
                }
                Ok(chosen)
            }
            _ => Err(fail("CFF FDSelect format not supported")),
        }
    };
    // The glyphs kept, and every subroutine they call.
    let mut keep: BTreeSet<usize> = glyphs
        .iter()
        .map(|glyph| usize::from(*glyph))
        .filter(|glyph| *glyph < glyph_count)
        .collect();
    keep.insert(0);
    let mut used_global = BTreeSet::new();
    for glyph in &keep {
        let font = font_of(*glyph)?;
        let Some(fd) = fonts.get_mut(font) else {
            return Err(fail("CFF FDSelect names a font that is not there"));
        };
        let mut walk = Walk {
            data: cff,
            global: &global.items,
            local: &fd.local,
            used_global: &mut used_global,
            used_local: &mut fd.used,
            stack: Vec::new(),
            stems: 0,
            done: false,
        };
        walk.run(char_strings.items[*glyph].clone(), 0)?;
    }
    // Rebuilt pieces.
    const ENDCHAR: &[u8] = &[14];
    const RETURN: &[u8] = &[11];
    let pick = |items: &[std::ops::Range<usize>],
                used: &dyn Fn(usize) -> bool,
                blank: &'static [u8]|
     -> Vec<Vec<u8>> {
        items
            .iter()
            .enumerate()
            .map(|(index, range)| {
                if used(index) {
                    cff[range.clone()].to_vec()
                } else {
                    blank.to_vec()
                }
            })
            .collect()
    };
    let char_string_data = pick(&char_strings.items, &|index| keep.contains(&index), ENDCHAR);
    let global_data = pick(&global.items, &|index| used_global.contains(&index), RETURN);
    let mut charset = vec![2u8];
    if glyph_count > 1 {
        charset.extend_from_slice(&1u16.to_be_bytes());
        charset.extend_from_slice(&((glyph_count - 2) as u16).to_be_bytes());
    }
    let fd_select_bytes = {
        let format = cff[fd_select_at];
        let length = if format == 0 {
            1 + glyph_count
        } else {
            let count = usize::from(u16::from_be_bytes([
                cff[fd_select_at + 1],
                cff[fd_select_at + 2],
            ]));
            3 + count * 3 + 2
        };
        cff.get(fd_select_at..fd_select_at + length)
            .ok_or_else(|| fail("CFF FDSelect cut short"))?
            .to_vec()
    };
    let mut private_blocks: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    for fd in &mut fonts {
        let local_data = pick(&fd.local, &|index| fd.used.contains(&index), RETURN);
        let mut local_index = Vec::new();
        if !fd.local.is_empty() {
            let refs: Vec<&[u8]> = local_data.iter().map(Vec::as_slice).collect();
            write_index(&refs, &mut local_index);
            // Subrs points just past the Private DICT, whose size is fixed.
            set(&mut fd.private, SUBRS, vec![Operand::Integer(0)]);
            let size = write_dict(&fd.private).len();
            set(&mut fd.private, SUBRS, vec![Operand::Integer(size as i32)]);
        }
        private_blocks.push((write_dict(&fd.private), local_index));
    }
    // Lay out: header, Name, Top DICT, String, global Subrs, then the
    // charset, FDSelect, CharStrings, FDArray, and the Private DICTs.
    let header = cff[..header_size].to_vec();
    let mut name_index = Vec::new();
    write_index(
        &names
            .items
            .iter()
            .map(|range| &cff[range.clone()])
            .collect::<Vec<_>>(),
        &mut name_index,
    );
    let mut string_index = Vec::new();
    write_index(
        &strings
            .items
            .iter()
            .map(|range| &cff[range.clone()])
            .collect::<Vec<_>>(),
        &mut string_index,
    );
    let mut global_index = Vec::new();
    write_index(
        &global_data.iter().map(Vec::as_slice).collect::<Vec<_>>(),
        &mut global_index,
    );
    let mut char_string_index = Vec::new();
    write_index(
        &char_string_data
            .iter()
            .map(Vec::as_slice)
            .collect::<Vec<_>>(),
        &mut char_string_index,
    );
    // Offsets are written five bytes wide: set placeholders, measure,
    // then fill in.
    for operator in [CHARSET, CHAR_STRINGS, FD_SELECT, FD_ARRAY] {
        set(&mut top, operator, vec![Operand::Integer(0)]);
    }
    let top_index_size = {
        let mut index = Vec::new();
        write_index(&[&write_dict(&top)], &mut index);
        index.len()
    };
    let mut font_dicts: Vec<Dict> = fonts.iter().map(|fd| fd.dict.clone()).collect();
    for dict in &mut font_dicts {
        set(
            dict,
            PRIVATE,
            vec![Operand::Integer(0), Operand::Integer(0)],
        );
    }
    let fd_array_size = {
        let encoded: Vec<Vec<u8>> = font_dicts.iter().map(write_dict).collect();
        let mut index = Vec::new();
        write_index(
            &encoded.iter().map(Vec::as_slice).collect::<Vec<_>>(),
            &mut index,
        );
        index.len()
    };
    let start = header.len() + name_index.len();
    let after_top =
        |top_index_size: usize| start + top_index_size + string_index.len() + global_index.len();
    let charset_at = after_top(top_index_size);
    let fd_select_new = charset_at + charset.len();
    let char_strings_at = fd_select_new + fd_select_bytes.len();
    let fd_array_at = char_strings_at + char_string_index.len();
    let mut private_at = fd_array_at + fd_array_size;
    set(&mut top, CHARSET, vec![Operand::Integer(charset_at as i32)]);
    set(
        &mut top,
        FD_SELECT,
        vec![Operand::Integer(fd_select_new as i32)],
    );
    set(
        &mut top,
        CHAR_STRINGS,
        vec![Operand::Integer(char_strings_at as i32)],
    );
    set(
        &mut top,
        FD_ARRAY,
        vec![Operand::Integer(fd_array_at as i32)],
    );
    let mut top_index = Vec::new();
    write_index(&[&write_dict(&top)], &mut top_index);
    // The five-byte form keeps the size; check rather than assume.
    if top_index.len() != top_index_size {
        return Err(fail("CFF Top DICT changed size when its offsets were set"));
    }
    for (dict, (private, local)) in font_dicts.iter_mut().zip(&private_blocks) {
        set(
            dict,
            PRIVATE,
            vec![
                Operand::Integer(private.len() as i32),
                Operand::Integer(private_at as i32),
            ],
        );
        private_at += private.len() + local.len();
    }
    let encoded: Vec<Vec<u8>> = font_dicts.iter().map(write_dict).collect();
    let mut fd_array_index = Vec::new();
    write_index(
        &encoded.iter().map(Vec::as_slice).collect::<Vec<_>>(),
        &mut fd_array_index,
    );
    let mut out = Vec::with_capacity(private_at);
    out.extend_from_slice(&header);
    out.extend_from_slice(&name_index);
    out.extend_from_slice(&top_index);
    out.extend_from_slice(&string_index);
    out.extend_from_slice(&global_index);
    out.extend_from_slice(&charset);
    out.extend_from_slice(&fd_select_bytes);
    out.extend_from_slice(&char_string_index);
    out.extend_from_slice(&fd_array_index);
    for (private, local) in &private_blocks {
        out.extend_from_slice(private);
        out.extend_from_slice(local);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexes_round_trip() {
        let mut out = Vec::new();
        write_index(&[b"ab", b"", b"cde"], &mut out);
        let index = read_index(&out, 0).expect("reads");
        let items: Vec<&[u8]> = index
            .items
            .iter()
            .map(|range| &out[range.clone()])
            .collect();
        assert_eq!(items, vec![&b"ab"[..], b"", b"cde"]);
        assert_eq!(index.end, out.len());
    }

    #[test]
    fn dicts_round_trip_with_wide_integers() {
        let dict: Dict = vec![
            (CHAR_STRINGS, vec![Operand::Integer(123_456)]),
            (
                ROS,
                vec![
                    Operand::Integer(391),
                    Operand::Integer(392),
                    Operand::Integer(6),
                ],
            ),
            (
                PRIVATE,
                vec![Operand::Integer(-5), Operand::Integer(70_000)],
            ),
        ];
        assert_eq!(read_dict(&write_dict(&dict)).expect("reads"), dict);
    }

    /// The CFF table of the Noto Sans CJK fixture.
    fn fixture_cff() -> Vec<u8> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/fonts/NotoSansCJKjp-Subset.otf"
        );
        let data = std::fs::read(path).expect("the fixture font");
        let font = crate::io::font::Font::parse(std::sync::Arc::new(data), 0).expect("parses");
        font.table(b"CFF ").expect("a CFF table").to_vec()
    }

    #[test]
    fn a_subset_keeps_used_glyphs_whole_and_blanks_the_rest() {
        let cff = fixture_cff();
        assert!(is_cid_keyed(&cff));
        let kept: BTreeSet<u16> = [3, 5].into_iter().collect();
        let cut_cff = subset(&cff, &kept).expect("subsets");
        let read = |data: &[u8]| -> (Vec<Vec<u8>>, usize) {
            let header_size = usize::from(data[2]);
            let names = read_index(data, header_size).unwrap();
            let tops = read_index(data, names.end).unwrap();
            let top = read_dict(&data[tops.items[0].clone()]).unwrap();
            let at = get(&top, CHAR_STRINGS).unwrap()[0].integer().unwrap() as usize;
            let index = read_index(data, at).unwrap();
            let strings = index
                .items
                .iter()
                .map(|range| data[range.clone()].to_vec())
                .collect();
            let strings_index = read_index(data, tops.end).unwrap();
            let global = read_index(data, strings_index.end).unwrap();
            (strings, global.items.len())
        };
        let (original, original_globals) = read(&cff);
        let (cut, cut_globals) = read(&cut_cff);
        assert_eq!(cut.len(), original.len(), "glyph ids keep their places");
        assert_eq!(
            cut_globals, original_globals,
            "subroutine numbers keep theirs"
        );
        for glyph in 0..original.len() {
            if glyph == 0 || kept.contains(&(glyph as u16)) {
                assert_eq!(cut[glyph], original[glyph], "glyph {glyph} kept whole");
            } else {
                assert_eq!(cut[glyph], vec![14], "glyph {glyph} blanked");
            }
        }
        assert!(cut_cff.len() < cff.len());
        // The subset is itself a CID-keyed font that subsets again.
        assert!(is_cid_keyed(&cut_cff));
        subset(&cut_cff, &kept).expect("subsets again");
    }

    #[test]
    fn biases_follow_the_subroutine_count() {
        assert_eq!(bias(100), 107);
        assert_eq!(bias(2000), 1131);
        assert_eq!(bias(40_000), 32768);
    }
}
