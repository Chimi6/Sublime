//! Stream filters: Flate (with PNG and TIFF predictors), LZW, ASCIIHex,
//! ASCII85, and RunLength, applied in order. A DCT (JPEG) stream is left
//! as its bytes, for the JPEG reader; other image codecs are refused.

use std::borrow::Cow;

use crate::io::deflate::{Inflater, Progress, inflate};
use crate::io::pdf::PdfError;
use crate::io::pdf::object::{Dictionary, Object};
use crate::io::png::reader::unfilter_row;

/// Decoded data is capped here (a stream claiming more is refused).
const LIMIT: usize = 1 << 31;

fn fail(message: impl Into<String>) -> PdfError {
    PdfError(message.into())
}

/// The filter names and their parameters, in order.
pub fn filters(dictionary: &Dictionary) -> Vec<(Vec<u8>, Option<Dictionary>)> {
    let names: Vec<Vec<u8>> = match dictionary.get(b"Filter").or_else(|| dictionary.get(b"F")) {
        Some(Object::Name(name)) => vec![name.clone()],
        Some(Object::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_name().map(<[u8]>::to_vec))
            .collect(),
        _ => Vec::new(),
    };
    let parameters: Vec<Option<Dictionary>> = match dictionary
        .get(b"DecodeParms")
        .or_else(|| dictionary.get(b"DP"))
    {
        Some(Object::Dictionary(parameters)) => vec![Some(parameters.clone())],
        Some(Object::Array(items)) => items
            .iter()
            .map(|item| item.as_dictionary().cloned())
            .collect(),
        _ => Vec::new(),
    };
    names
        .into_iter()
        .enumerate()
        .map(|(index, name)| (name, parameters.get(index).cloned().flatten()))
        .collect()
}

/// Whether a stream's last filter is DCT (its decoded bytes are a JPEG).
pub fn ends_in_dct(dictionary: &Dictionary) -> bool {
    filters(dictionary)
        .last()
        .is_some_and(|(name, _)| name == b"DCTDecode" || name == b"DCT")
}

/// Applies every filter but a final DCT.
pub fn decode(dictionary: &Dictionary, raw: &[u8]) -> Result<Vec<u8>, PdfError> {
    let mut data = raw.to_vec();
    for (name, parameters) in filters(dictionary) {
        data = match name.as_slice() {
            b"FlateDecode" | b"Fl" => predict(flate(&data)?, parameters.as_ref())?,
            b"LZWDecode" | b"LZW" => {
                let early = parameters
                    .as_ref()
                    .and_then(|parameters| parameters.get(b"EarlyChange"))
                    .and_then(Object::as_integer)
                    .unwrap_or(1)
                    != 0;
                let decoded = crate::io::lzw::decode(&data, None, early)
                    .map_err(|failure| fail(format!("PDF {failure}")))?;
                predict(decoded, parameters.as_ref())?
            }
            b"ASCIIHexDecode" | b"AHx" => ascii_hex(&data)?,
            b"ASCII85Decode" | b"A85" => ascii85(&data)?,
            b"RunLengthDecode" | b"RL" => run_length(&data)?,
            b"DCTDecode" | b"DCT" => return Ok(data),
            other => {
                let other = String::from_utf8_lossy(other);
                return Err(fail(format!("the PDF filter /{other} is not supported")));
            }
        };
    }
    Ok(data)
}

/// Inflated output asked for per push.
const OUTPUT_STEP: usize = 1 << 16;

/// An image's decoded sample rows, one at a time. A lone Flate filter
/// (the common case) inflates and undoes its predictor row by row, so
/// neither the samples nor a copy of the stream are held; an unfiltered
/// stream is read in place; other chains decode whole first.
pub enum Samples<'a> {
    Held { data: Cow<'a, [u8]>, at: usize },
    Flate(Box<FlateRows<'a>>),
}

impl<'a> Samples<'a> {
    /// Rows of `stride` bytes from `raw` through the stream's filters.
    pub fn new(
        dictionary: &Dictionary,
        raw: Cow<'a, [u8]>,
        stride: usize,
    ) -> Result<Samples<'a>, PdfError> {
        let chain = filters(dictionary);
        if chain.is_empty() {
            return Ok(Samples::Held { data: raw, at: 0 });
        }
        if let ([(name, parameters)], Cow::Borrowed(raw)) = (chain.as_slice(), &raw) {
            let is_flate = name == b"FlateDecode" || name == b"Fl";
            if let (true, Some(predictor)) = (is_flate, Predictor::of(parameters.as_ref(), stride))
            {
                return Ok(Samples::Flate(Box::new(FlateRows::new(
                    raw, predictor, stride,
                ))));
            }
        }
        let data = decode(dictionary, &raw)?;
        Ok(Samples::Held {
            data: Cow::Owned(data),
            at: 0,
        })
    }

    /// The next row, or `None` when the data runs out.
    pub fn next_row(&mut self, stride: usize) -> Result<Option<&[u8]>, PdfError> {
        match self {
            Samples::Held { data, at } => {
                let Some(row) = data.get(*at..*at + stride) else {
                    return Ok(None);
                };
                *at += stride;
                Ok(Some(row))
            }
            Samples::Flate(rows) => rows.next_row(),
        }
    }
}

/// A stream predictor the row reader undoes.
#[derive(Clone, Copy)]
pub enum Predictor {
    None,
    /// TIFF predictor 2 at eight bits: each byte adds the one a pixel back.
    Tiff {
        unit: usize,
    },
    /// PNG predictors: a filter byte before each row.
    Png {
        unit: usize,
    },
}

impl Predictor {
    /// The predictor the parameters name, when its rows are `stride`
    /// bytes long and the row reader handles it.
    fn of(parameters: Option<&Dictionary>, stride: usize) -> Option<Predictor> {
        let Some(parameters) = parameters else {
            return Some(Predictor::None);
        };
        let number = |key: &[u8], default: i64| {
            parameters
                .get(key)
                .and_then(Object::as_integer)
                .unwrap_or(default)
        };
        let predictor = number(b"Predictor", 1);
        if predictor < 2 {
            return Some(Predictor::None);
        }
        let colors = number(b"Colors", 1).max(1) as usize;
        let bits = number(b"BitsPerComponent", 8).max(1) as usize;
        let columns = number(b"Columns", 1).max(1) as usize;
        let unit = (colors * bits).div_ceil(8).max(1);
        if (colors * bits * columns).div_ceil(8) != stride || !matches!(unit, 1..=4 | 6 | 8) {
            return None;
        }
        match predictor {
            2 if bits == 8 => Some(Predictor::Tiff { unit }),
            10.. => Some(Predictor::Png { unit }),
            _ => None,
        }
    }
}

/// Flate data inflated a row at a time, its predictor undone as it goes.
pub struct FlateRows<'a> {
    input: &'a [u8],
    inflater: Inflater,
    predictor: Predictor,
    /// Bytes per row in the stream (a PNG filter byte and the row).
    line: usize,
    previous: Vec<u8>,
    current: Vec<u8>,
}

impl<'a> FlateRows<'a> {
    fn new(data: &'a [u8], predictor: Predictor, stride: usize) -> FlateRows<'a> {
        let line = match predictor {
            Predictor::Png { .. } => stride + 1,
            _ => stride,
        };
        FlateRows {
            input: zlib_body(data),
            inflater: Inflater::new(),
            predictor,
            line,
            previous: vec![0; stride],
            current: vec![0; stride],
        }
    }

    fn next_row(&mut self) -> Result<Option<&[u8]>, PdfError> {
        while self.inflater.output().len() < self.line {
            if self.inflater.is_done() {
                return Ok(None);
            }
            let (consumed, progress) =
                self.inflater
                    .push(self.input, OUTPUT_STEP.max(self.line))
                    .map_err(|failure| fail(format!("bad PDF Flate data: {failure:?}")))?;
            self.input = &self.input[consumed..];
            if progress == Progress::NeedInput && self.input.is_empty() {
                if self.inflater.output().len() < self.line {
                    return Ok(None);
                }
                break;
            }
        }
        let line = &self.inflater.output()[..self.line];
        match self.predictor {
            Predictor::None => self.current.copy_from_slice(line),
            Predictor::Tiff { unit } => {
                self.current.copy_from_slice(line);
                for at in unit..self.current.len() {
                    self.current[at] = self.current[at].wrapping_add(self.current[at - unit]);
                }
            }
            Predictor::Png { unit } => {
                unfilter_row(line[0], &line[1..], &self.previous, unit, &mut self.current)
                    .map_err(|_| fail("bad PNG predictor row in PDF"))?;
            }
        }
        self.inflater.drain(self.line);
        std::mem::swap(&mut self.previous, &mut self.current);
        Ok(Some(&self.previous))
    }
}

/// A zlib stream's deflate data; some writers leave off the two-byte
/// header.
fn zlib_body(data: &[u8]) -> &[u8] {
    if data.len() >= 2
        && data[0] & 0x0f == 8
        && (u16::from(data[0]) << 8 | u16::from(data[1])) % 31 == 0
    {
        &data[2..]
    } else {
        data
    }
}

fn flate(data: &[u8]) -> Result<Vec<u8>, PdfError> {
    let body = zlib_body(data);
    let mut out = Vec::new();
    inflate(body, &mut out, LIMIT)
        .map_err(|failure| fail(format!("bad PDF Flate data: {failure:?}")))?;
    Ok(out)
}

/// Undoes a PNG (10 and up) or TIFF (2) predictor.
fn predict(data: Vec<u8>, parameters: Option<&Dictionary>) -> Result<Vec<u8>, PdfError> {
    let Some(parameters) = parameters else {
        return Ok(data);
    };
    let number = |key: &[u8], default: i64| {
        parameters
            .get(key)
            .and_then(Object::as_integer)
            .unwrap_or(default)
    };
    let predictor = number(b"Predictor", 1);
    if predictor < 2 {
        return Ok(data);
    }
    let colors = number(b"Colors", 1).max(1) as usize;
    let bits = number(b"BitsPerComponent", 8).max(1) as usize;
    let columns = number(b"Columns", 1).max(1) as usize;
    let unit = (colors * bits).div_ceil(8).max(1);
    let stride = (colors * bits * columns).div_ceil(8);
    if predictor == 2 {
        if bits != 8 {
            return Err(fail("PDF TIFF predictor below 8 bits is not supported"));
        }
        let mut data = data;
        for row in data.chunks_mut(stride) {
            for at in unit..row.len() {
                row[at] = row[at].wrapping_add(row[at - unit]);
            }
        }
        return Ok(data);
    }
    let mut out = Vec::with_capacity(data.len());
    let mut previous = vec![0u8; stride];
    for line in data.chunks(stride + 1) {
        if line.len() < 2 {
            break;
        }
        let (filter, bytes) = (line[0], &line[1..]);
        let mut row = vec![0u8; stride];
        row[..bytes.len()].copy_from_slice(bytes);
        for at in 0..stride {
            let left = if at >= unit { row[at - unit] } else { 0 };
            let up = previous[at];
            let corner = if at >= unit { previous[at - unit] } else { 0 };
            let predicted = match filter {
                0 => 0,
                1 => left,
                2 => up,
                3 => ((u16::from(left) + u16::from(up)) / 2) as u8,
                4 => {
                    let estimate = i16::from(left) + i16::from(up) - i16::from(corner);
                    let (to_left, to_up, to_corner) = (
                        (estimate - i16::from(left)).abs(),
                        (estimate - i16::from(up)).abs(),
                        (estimate - i16::from(corner)).abs(),
                    );
                    if to_left <= to_up && to_left <= to_corner {
                        left
                    } else if to_up <= to_corner {
                        up
                    } else {
                        corner
                    }
                }
                _ => return Err(fail("bad PNG predictor row in PDF")),
            };
            row[at] = row[at].wrapping_add(predicted);
        }
        out.extend_from_slice(&row);
        previous = row;
    }
    Ok(out)
}

fn ascii_hex(data: &[u8]) -> Result<Vec<u8>, PdfError> {
    let mut out = Vec::new();
    let mut high: Option<u8> = None;
    for &byte in data {
        let digit = match byte {
            b'>' => break,
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ if byte.is_ascii_whitespace() || byte == 0 => continue,
            _ => return Err(fail("bad ASCIIHex data in PDF")),
        };
        match high.take() {
            None => high = Some(digit),
            Some(first) => out.push((first << 4) | digit),
        }
    }
    if let Some(first) = high {
        out.push(first << 4);
    }
    Ok(out)
}

fn ascii85(data: &[u8]) -> Result<Vec<u8>, PdfError> {
    let mut out = Vec::new();
    let mut group = [0u32; 5];
    let mut filled = 0;
    let mut at = 0;
    if data.starts_with(b"<~") {
        at = 2;
    }
    while at < data.len() {
        let byte = data[at];
        at += 1;
        match byte {
            b'~' => break,
            b'z' if filled == 0 => out.extend_from_slice(&[0, 0, 0, 0]),
            b'!'..=b'u' => {
                group[filled] = u32::from(byte - b'!');
                filled += 1;
                if filled == 5 {
                    let value = group
                        .iter()
                        .try_fold(0u32, |sum, digit| sum.checked_mul(85)?.checked_add(*digit));
                    let value = value.ok_or_else(|| fail("bad ASCII85 data in PDF"))?;
                    out.extend_from_slice(&value.to_be_bytes());
                    filled = 0;
                }
            }
            _ if byte.is_ascii_whitespace() => {}
            _ => return Err(fail("bad ASCII85 data in PDF")),
        }
    }
    if filled > 1 {
        for digit in group.iter_mut().skip(filled) {
            *digit = 84;
        }
        let value = group
            .iter()
            .fold(0u64, |sum, digit| sum * 85 + u64::from(*digit)) as u32;
        out.extend_from_slice(&value.to_be_bytes()[..filled - 1]);
    }
    Ok(out)
}

fn run_length(data: &[u8]) -> Result<Vec<u8>, PdfError> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < data.len() {
        let length = data[at];
        at += 1;
        match length {
            128 => break,
            0..=127 => {
                let count = usize::from(length) + 1;
                let bytes = data
                    .get(at..at + count)
                    .ok_or_else(|| fail("RunLength data cut short in PDF"))?;
                out.extend_from_slice(bytes);
                at += count;
            }
            _ => {
                let byte = *data
                    .get(at)
                    .ok_or_else(|| fail("RunLength data cut short in PDF"))?;
                at += 1;
                out.extend(std::iter::repeat_n(byte, 257 - usize::from(length)));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_filters_decode() {
        assert_eq!(ascii_hex(b"48 65 6C6c 6F>").unwrap(), b"Hello");
        assert_eq!(ascii85(b"<~87cURD]i,\"Ebo80~>").unwrap(), b"Hello World!");
        assert_eq!(ascii85(b"z~>").unwrap(), vec![0, 0, 0, 0]);
    }

    /// Flate streams read a row at a time give what decoding them whole
    /// gives, for both predictor kinds.
    #[test]
    fn streamed_rows_match_the_whole_decode() {
        let parse = |text: &[u8]| {
            let Ok(Object::Dictionary(dictionary)) =
                crate::io::pdf::object::Parser::new(text, 0).object()
            else {
                panic!("bad dictionary");
            };
            dictionary
        };
        // Two rows of three RGB pixels: PNG predictor rows (Sub, Up,
        // Average, Paeth over two rows each) and TIFF predictor rows.
        let stride = 9;
        let cases: Vec<(&[u8], Vec<u8>)> =
            vec![
            (
                b"<< /Filter /FlateDecode /DecodeParms << /Predictor 15 /Colors 3 /Columns 3 >> >>",
                [1u8, 3, 9, 27, 2, 4, 6, 8, 10, 12, 4, 5, 250, 1, 7, 7, 7, 0, 0, 9]
                    .iter()
                    .chain(&[3u8, 1, 2, 3, 4, 5, 6, 7, 8, 9])
                    .copied()
                    .collect(),
            ),
            (
                b"<< /Filter /FlateDecode /DecodeParms << /Predictor 2 /Colors 3 /Columns 3 >> >>",
                vec![10, 20, 30, 1, 2, 3, 250, 250, 250, 5, 5, 5, 0, 0, 0, 9, 8, 7],
            ),
        ];
        for (text, rows) in cases {
            let dictionary = parse(text);
            let mut zlib = vec![0x78, 0x9c];
            crate::io::deflate::deflate(&rows, &mut zlib);
            let whole = decode(&dictionary, &zlib).unwrap();
            let mut samples = Samples::new(&dictionary, Cow::Borrowed(&zlib), stride).unwrap();
            assert!(matches!(samples, Samples::Flate(_)));
            let mut streamed = Vec::new();
            while let Some(row) = samples.next_row(stride).unwrap() {
                streamed.extend_from_slice(row);
            }
            assert_eq!(streamed, whole);
        }
        assert_eq!(
            run_length(&[2, b'a', b'b', b'c', 254, b'x', 128]).unwrap(),
            b"abcxxx"
        );
    }
}
