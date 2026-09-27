//! A PDF document: its cross-reference data (tables and streams, every
//! revision, or rebuilt by scanning when broken), objects resolved on
//! demand (from object streams too), and its pages in order.

use std::cell::RefCell;
use std::collections::HashMap;

use crate::io::pdf::PdfError;
use crate::io::pdf::filter;
use crate::io::pdf::object::{Dictionary, Object, Parser};

fn fail(message: impl Into<String>) -> PdfError {
    PdfError(message.into())
}

/// Where an object lives.
#[derive(Clone, Copy, Debug)]
enum Entry {
    Offset(usize),
    InStream { stream: u32, index: usize },
}

/// A page and the resources it inherits.
pub struct Page {
    pub dictionary: Dictionary,
    pub resources: Option<Dictionary>,
}

pub struct Document<'a> {
    bytes: &'a [u8],
    entries: HashMap<u32, Entry>,
    trailer: Dictionary,
    /// Decoded object streams, by object number.
    object_streams: RefCell<HashMap<u32, Vec<(u32, Object)>>>,
}

impl<'a> Document<'a> {
    pub fn open(bytes: &'a [u8]) -> Result<Document<'a>, PdfError> {
        if !bytes.starts_with(b"%PDF-") && find(bytes, b"%PDF-", 0).is_none_or(|at| at > 1024) {
            return Err(fail("not a PDF"));
        }
        let mut document = Document {
            bytes,
            entries: HashMap::new(),
            trailer: Dictionary::default(),
            object_streams: RefCell::new(HashMap::new()),
        };
        let read = document.read_cross_references();
        if read.is_err() || document.trailer.get(b"Root").is_none() || document.catalog().is_err() {
            document.rebuild()?;
        }
        if document.trailer.get(b"Encrypt").is_some() {
            return Err(fail("encrypted PDFs are not supported"));
        }
        Ok(document)
    }

    /// Reads the cross-reference sections from the last back through
    /// every earlier revision (`/Prev`); newer entries win.
    fn read_cross_references(&mut self) -> Result<(), PdfError> {
        let tail_start = self.bytes.len().saturating_sub(4096);
        let at = rfind(self.bytes, b"startxref", tail_start)
            .ok_or_else(|| fail("PDF has no startxref"))?;
        let mut parser = Parser::new(self.bytes, at + 9);
        let mut offset = parser
            .object()?
            .as_integer()
            .ok_or_else(|| fail("bad startxref"))? as usize;
        let mut seen = Vec::new();
        let mut first_trailer = None;
        while !seen.contains(&offset) && seen.len() < 64 {
            seen.push(offset);
            let trailer = self.read_section(offset)?;
            // A hybrid file points at an extra stream from its table.
            if let Some(stream_at) = trailer.get(b"XRefStm").and_then(Object::as_integer) {
                let _ = self.read_section(stream_at as usize);
            }
            let previous = trailer.get(b"Prev").and_then(Object::as_integer);
            if first_trailer.is_none() {
                first_trailer = Some(trailer);
            }
            match previous {
                Some(previous) => offset = previous as usize,
                None => break,
            }
        }
        self.trailer = first_trailer.ok_or_else(|| fail("PDF has no trailer"))?;
        Ok(())
    }

    /// One section: a table and its trailer, or a cross-reference stream.
    fn read_section(&mut self, offset: usize) -> Result<Dictionary, PdfError> {
        let mut parser = Parser::new(self.bytes, offset);
        if parser.keyword(b"xref") {
            loop {
                parser.skip();
                if parser.keyword(b"trailer") {
                    let trailer = parser.object()?;
                    return trailer
                        .as_dictionary()
                        .cloned()
                        .ok_or_else(|| fail("bad PDF trailer"));
                }
                let start = parser
                    .object()?
                    .as_integer()
                    .ok_or_else(|| fail("bad xref section"))?;
                let count = parser
                    .object()?
                    .as_integer()
                    .ok_or_else(|| fail("bad xref section"))?;
                for index in 0..count.max(0) {
                    let place = parser
                        .object()?
                        .as_integer()
                        .ok_or_else(|| fail("bad xref entry"))?;
                    let _generation = parser.object()?;
                    parser.skip();
                    let kind = self.bytes.get(parser.at).copied();
                    parser.at += 1;
                    let number = (start + index) as u32;
                    if kind == Some(b'n') && place > 0 {
                        self.entries
                            .entry(number)
                            .or_insert(Entry::Offset(place as usize));
                    }
                }
            }
        }
        // A cross-reference stream.
        let (_, _, object) = Parser::new(self.bytes, offset)
            .indirect(&mut |length| length.as_integer().map(|n| n as usize))?;
        let Object::Stream(dictionary, range) = object else {
            return Err(fail("PDF cross-reference is neither a table nor a stream"));
        };
        let data = filter::decode(&dictionary, &self.bytes[range])?;
        let widths: Vec<usize> = dictionary
            .get(b"W")
            .and_then(Object::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Object::as_integer)
                    .map(|w| w as usize)
                    .collect()
            })
            .unwrap_or_default();
        if widths.len() != 3 {
            return Err(fail("bad cross-reference stream widths"));
        }
        let size = dictionary
            .get(b"Size")
            .and_then(Object::as_integer)
            .unwrap_or(0);
        let index: Vec<i64> = dictionary
            .get(b"Index")
            .and_then(Object::as_array)
            .map(|items| items.iter().filter_map(Object::as_integer).collect())
            .unwrap_or_else(|| vec![0, size]);
        let row = widths.iter().sum::<usize>();
        let field = |bytes: &[u8]| {
            bytes
                .iter()
                .fold(0usize, |value, byte| value << 8 | usize::from(*byte))
        };
        let mut rows = data.chunks_exact(row.max(1));
        for pair in index.chunks_exact(2) {
            for number in pair[0]..pair[0] + pair[1] {
                let Some(entry) = rows.next() else { break };
                let kind = if widths[0] == 0 {
                    1
                } else {
                    field(&entry[..widths[0]])
                };
                let second = field(&entry[widths[0]..widths[0] + widths[1]]);
                let third = field(&entry[widths[0] + widths[1]..]);
                let number = number as u32;
                match kind {
                    1 => {
                        self.entries.entry(number).or_insert(Entry::Offset(second));
                    }
                    2 => {
                        self.entries.entry(number).or_insert(Entry::InStream {
                            stream: second as u32,
                            index: third,
                        });
                    }
                    _ => {}
                }
            }
        }
        Ok(dictionary)
    }

    /// Rebuilds the cross references by scanning for `n g obj`, and the
    /// trailer from the last one (or the catalog) found.
    fn rebuild(&mut self) -> Result<(), PdfError> {
        self.entries.clear();
        let mut at = 0;
        let mut trailer = None;
        while let Some(found) = find(self.bytes, b" obj", at) {
            at = found + 4;
            // Back up over `number generation` before ` obj`.
            let mut start = found;
            let mut spaces = 0;
            while start > 0 {
                let byte = self.bytes[start - 1];
                if byte.is_ascii_digit() {
                    start -= 1;
                } else if byte == b' ' && spaces == 0 {
                    spaces += 1;
                    start -= 1;
                } else {
                    break;
                }
            }
            let mut parser = Parser::new(self.bytes, start);
            if let Ok((number, _, object)) = parser.indirect(&mut |_| None) {
                self.entries.insert(number, Entry::Offset(start));
                if let Some(dictionary) = object.as_dictionary() {
                    if dictionary.get(b"Root").is_some() {
                        trailer = Some(dictionary.clone());
                    }
                    if dictionary.get(b"Type").and_then(Object::as_name) == Some(b"ObjStm") {
                        self.index_object_stream(number);
                    }
                }
                at = at.max(parser.at);
            }
        }
        let mut search = 0;
        while let Some(found) = find(self.bytes, b"trailer", search) {
            search = found + 7;
            if let Ok(Object::Dictionary(dictionary)) = Parser::new(self.bytes, found + 7).object()
            {
                if dictionary.get(b"Root").is_some() {
                    trailer = Some(dictionary);
                }
            }
        }
        if trailer.is_none() {
            let catalog = self.entries.keys().copied().find(|number| {
                self.get(*number)
                    .ok()
                    .and_then(|object| {
                        object.as_dictionary().and_then(|d| {
                            d.get(b"Type").and_then(Object::as_name).map(<[u8]>::to_vec)
                        })
                    })
                    .is_some_and(|kind| kind == b"Catalog")
            });
            if let Some(number) = catalog {
                trailer = Some(Dictionary(vec![(
                    b"Root".to_vec(),
                    Object::Reference(number, 0),
                )]));
            }
        }
        self.trailer = trailer.ok_or_else(|| fail("PDF has no catalog"))?;
        Ok(())
    }

    /// Records the objects of an object stream found by the scan.
    fn index_object_stream(&mut self, stream: u32) {
        if let Ok(objects) = self.object_stream(stream) {
            for (index, (number, _)) in objects.iter().enumerate() {
                self.entries
                    .entry(*number)
                    .or_insert(Entry::InStream { stream, index });
            }
        }
    }

    fn object_stream(&self, stream: u32) -> Result<Vec<(u32, Object)>, PdfError> {
        if let Some(objects) = self.object_streams.borrow().get(&stream) {
            return Ok(objects.clone());
        }
        let Object::Stream(dictionary, range) = self.get(stream)? else {
            return Err(fail("object stream is not a stream"));
        };
        let data = filter::decode(&dictionary, &self.bytes[range])?;
        let count = dictionary
            .get(b"N")
            .and_then(Object::as_integer)
            .unwrap_or(0)
            .max(0) as usize;
        let first = dictionary
            .get(b"First")
            .and_then(Object::as_integer)
            .unwrap_or(0)
            .max(0) as usize;
        let mut header = Parser::new(&data, 0);
        let mut places = Vec::with_capacity(count);
        for _ in 0..count {
            let number = header
                .object()?
                .as_integer()
                .ok_or_else(|| fail("bad object stream"))? as u32;
            let offset = header
                .object()?
                .as_integer()
                .ok_or_else(|| fail("bad object stream"))? as usize;
            places.push((number, offset));
        }
        let mut objects = Vec::with_capacity(count);
        for (number, offset) in places {
            let object = Parser::new(&data, first + offset).object()?;
            objects.push((number, object));
        }
        self.object_streams
            .borrow_mut()
            .insert(stream, objects.clone());
        Ok(objects)
    }

    /// An object by number. A stream's range is into the file's bytes.
    pub fn get(&self, number: u32) -> Result<Object, PdfError> {
        match self.entries.get(&number) {
            Some(Entry::Offset(offset)) => {
                let bytes = self.bytes;
                let mut length = |value: &Object| -> Option<usize> {
                    match value {
                        Object::Integer(length) => Some(*length as usize),
                        Object::Reference(number, _) => self
                            .get(*number)
                            .ok()?
                            .as_integer()
                            .map(|length| length as usize),
                        _ => None,
                    }
                };
                let (_, _, object) = Parser::new(bytes, *offset).indirect(&mut length)?;
                Ok(object)
            }
            Some(Entry::InStream { stream, index }) => {
                let objects = self.object_stream(*stream)?;
                objects
                    .iter()
                    .find(|(found, _)| *found == number)
                    .or_else(|| objects.get(*index))
                    .map(|(_, object)| object.clone())
                    .ok_or_else(|| fail("object missing from its object stream"))
            }
            None => Ok(Object::Null),
        }
    }

    /// Follows a reference (a few levels at most).
    pub fn resolve(&self, object: &Object) -> Result<Object, PdfError> {
        let mut current = object.clone();
        for _ in 0..16 {
            match current {
                Object::Reference(number, _) => current = self.get(number)?,
                other => return Ok(other),
            }
        }
        Err(fail("PDF references loop"))
    }

    /// A stream object's data with its filters applied (a DCT stream is
    /// left as its JPEG bytes).
    pub fn stream_data(&self, object: &Object) -> Result<(Dictionary, Vec<u8>), PdfError> {
        match self.resolve(object)? {
            Object::Stream(dictionary, range) => {
                let data = filter::decode(&dictionary, &self.bytes[range])?;
                Ok((dictionary, data))
            }
            _ => Err(fail("expected a PDF stream")),
        }
    }

    fn catalog(&self) -> Result<Dictionary, PdfError> {
        let root = self
            .trailer
            .get(b"Root")
            .ok_or_else(|| fail("PDF has no catalog"))?;
        self.resolve(root)?
            .as_dictionary()
            .cloned()
            .ok_or_else(|| fail("PDF catalog is not a dictionary"))
    }

    /// The pages in order, each with the resources it inherits.
    pub fn pages(&self) -> Result<Vec<Page>, PdfError> {
        let catalog = self.catalog()?;
        let root = catalog
            .get(b"Pages")
            .ok_or_else(|| fail("PDF has no pages"))?;
        let mut pages = Vec::new();
        let mut visited = Vec::new();
        self.walk(root, None, &mut pages, &mut visited, 0)?;
        Ok(pages)
    }

    fn walk(
        &self,
        node: &Object,
        inherited: Option<&Dictionary>,
        pages: &mut Vec<Page>,
        visited: &mut Vec<u32>,
        depth: usize,
    ) -> Result<(), PdfError> {
        if depth > 64 {
            return Err(fail("PDF page tree too deep"));
        }
        if let Object::Reference(number, _) = node {
            if visited.contains(number) {
                return Ok(());
            }
            visited.push(*number);
        }
        let dictionary = match self.resolve(node)? {
            Object::Dictionary(dictionary) => dictionary,
            _ => return Ok(()),
        };
        let resources = match dictionary.get(b"Resources") {
            Some(resources) => self.resolve(resources)?.as_dictionary().cloned(),
            None => inherited.cloned(),
        };
        match dictionary.get(b"Kids").map(|kids| self.resolve(kids)) {
            Some(Ok(Object::Array(kids))) => {
                for kid in &kids {
                    self.walk(kid, resources.as_ref(), pages, visited, depth + 1)?;
                }
            }
            _ => pages.push(Page {
                dictionary,
                resources,
            }),
        }
        Ok(())
    }

    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }
}

pub(crate) fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from >= haystack.len() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|at| at + from)
}

fn rfind(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    haystack[from..]
        .windows(needle.len())
        .rposition(|window| window == needle)
        .map(|at| at + from)
}
