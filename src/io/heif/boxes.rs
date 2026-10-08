//! The HEIF container (ISO/IEC 23008-12 over the ISO base media file
//! format, ISO/IEC 14496-12): the `meta` box's items, their locations,
//! references, and properties.

use super::HeifError;

/// A box: its four-character type and its payload.
pub struct BoxRef<'a> {
    pub kind: [u8; 4],
    pub payload: &'a [u8],
}

/// The boxes laid end to end in `data`.
pub fn boxes(data: &[u8]) -> Result<Vec<BoxRef<'_>>, HeifError> {
    let mut found = Vec::new();
    let mut at = 0usize;
    while at + 8 <= data.len() {
        let size32 = u32::from_be_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]);
        let kind = [data[at + 4], data[at + 5], data[at + 6], data[at + 7]];
        let (header, size) = match size32 {
            0 => (8, data.len() - at),
            1 => {
                let large = data
                    .get(at + 8..at + 16)
                    .ok_or_else(|| HeifError::new("a box cut short"))?;
                let large = u64::from_be_bytes(large.try_into().unwrap_or([0; 8]));
                (
                    16,
                    usize::try_from(large).map_err(|_| HeifError::new("a box too large"))?,
                )
            }
            size => (8, size as usize),
        };
        if size < header || at + size > data.len() {
            return Err(HeifError::new("a box running past its container"));
        }
        found.push(BoxRef {
            kind,
            payload: &data[at + header..at + size],
        });
        at += size;
    }
    Ok(found)
}

/// Reads big-endian fields.
pub struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Reader<'a> {
        Reader { data, at: 0 }
    }

    pub fn bytes(&mut self, count: usize) -> Result<&'a [u8], HeifError> {
        let slice = self
            .data
            .get(self.at..self.at + count)
            .ok_or_else(|| HeifError::new("a box field past the box's end"))?;
        self.at += count;
        Ok(slice)
    }

    pub fn u8(&mut self) -> Result<u8, HeifError> {
        Ok(self.bytes(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16, HeifError> {
        let bytes = self.bytes(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    pub fn u32(&mut self) -> Result<u32, HeifError> {
        let bytes = self.bytes(4)?;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// An unsigned number of 0, 4, or 8 bytes (`iloc`'s sizes).
    pub fn sized(&mut self, size: u8) -> Result<u64, HeifError> {
        Ok(match size {
            0 => 0,
            4 => u64::from(self.u32()?),
            8 => {
                let bytes = self.bytes(8)?;
                u64::from_be_bytes(bytes.try_into().unwrap_or([0; 8]))
            }
            _ => return Err(HeifError::new("an item location field of an odd size")),
        })
    }

    /// A full box's version and flags.
    pub fn full_box(&mut self) -> Result<(u8, u32), HeifError> {
        let value = self.u32()?;
        Ok(((value >> 24) as u8, value & 0x00FF_FFFF))
    }

    /// A NUL-terminated string.
    pub fn string(&mut self) -> Result<&'a str, HeifError> {
        let rest = &self.data[self.at.min(self.data.len())..];
        let end = rest
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(rest.len());
        self.at += (end + 1).min(rest.len());
        Ok(std::str::from_utf8(&rest[..end]).unwrap_or(""))
    }

    pub fn rest(&self) -> &'a [u8] {
        &self.data[self.at.min(self.data.len())..]
    }
}

/// Where an item's data is: its construction method, base offset, and
/// extents of offset and length.
pub type Location = (u8, u64, Vec<(u64, u64)>);

/// One item: its type and where its bytes lie.
#[derive(Debug, Clone, Default)]
pub struct Item {
    pub id: u32,
    pub kind: [u8; 4],
    /// `mime` items' content type.
    pub content_type: String,
    pub location: Option<Location>,
    /// Indices into the property container, in association order.
    pub properties: Vec<usize>,
}

/// An item property this reader uses.
#[derive(Debug, Clone)]
pub enum Property {
    /// The decoder configuration: its parameter set NAL units, and the
    /// size of the length before each NAL unit in the item's data.
    HevcConfig {
        nals: Vec<Vec<u8>>,
        length_size: usize,
    },
    Size {
        width: u32,
        height: u32,
    },
    /// Rotation anticlockwise, in quarter turns.
    Rotation(u8),
    /// Mirroring top to bottom when 0, left to right when 1 (as libheif
    /// reads the axis).
    Mirror(u8),
    /// The clean aperture: width, height, and the centre's offsets, each
    /// as a fraction.
    CleanAperture([(i64, i64); 4]),
    Colour(Colour),
    BitDepths(Vec<u8>),
    Auxiliary(String),
    Other,
}

/// A colour property: the coefficients of nclx, or an ICC profile.
#[derive(Debug, Clone, PartialEq)]
pub enum Colour {
    Nclx {
        primaries: u16,
        transfer: u16,
        matrix: u16,
        full_range: bool,
    },
    Icc(Vec<u8>),
}

/// The `meta` box read: items, references, properties, and `idat`.
pub struct Meta<'a> {
    pub primary: u32,
    pub items: Vec<Item>,
    /// (type, from item, to items) of `iref`.
    pub references: Vec<([u8; 4], u32, Vec<u32>)>,
    pub properties: Vec<Property>,
    pub item_data: &'a [u8],
}

impl<'a> Meta<'a> {
    pub fn item(&self, id: u32) -> Option<&Item> {
        self.items.iter().find(|item| item.id == id)
    }

    /// The items `from` refers to by references of `kind`, in order.
    pub fn referenced(&self, kind: &[u8; 4], from: u32) -> Vec<u32> {
        self.references
            .iter()
            .filter(|(reference, source, _)| reference == kind && *source == from)
            .flat_map(|(_, _, targets)| targets.iter().copied())
            .collect()
    }

    /// The items referring to `to` by references of `kind`.
    pub fn referring(&self, kind: &[u8; 4], to: u32) -> Vec<u32> {
        self.references
            .iter()
            .filter(|(reference, _, targets)| reference == kind && targets.contains(&to))
            .map(|(_, source, _)| *source)
            .collect()
    }

    pub fn properties_of(&self, item: &Item) -> impl Iterator<Item = &Property> {
        item.properties
            .iter()
            .filter_map(|&index| self.properties.get(index))
    }

    /// An item's bytes, from the file, `idat`, or other items.
    /// An item's bytes: borrowed when they lie in one extent, joined
    /// when in several.
    pub fn data(
        &self,
        file: &'a [u8],
        item: &Item,
    ) -> Result<std::borrow::Cow<'a, [u8]>, HeifError> {
        let (method, base, extents) = item
            .location
            .as_ref()
            .ok_or_else(|| HeifError::new("an item without a location"))?;
        let mut bytes = Vec::new();
        for (index, &(offset, length)) in extents.iter().enumerate() {
            let source: &[u8] = match method {
                0 => file,
                1 => self.item_data,
                _ => return Err(HeifError::new("an item built from other items")),
            };
            let start = usize::try_from(base + offset)
                .map_err(|_| HeifError::new("an item offset past the file"))?;
            let end = if length == 0 {
                source.len()
            } else {
                start
                    .checked_add(length as usize)
                    .ok_or_else(|| HeifError::new("an item past the file"))?
            };
            let part = source
                .get(start..end)
                .ok_or_else(|| HeifError::new("an item past the file's end"))?;
            if extents.len() == 1 && index == 0 {
                return Ok(std::borrow::Cow::Borrowed(part));
            }
            bytes.extend_from_slice(part);
        }
        Ok(std::borrow::Cow::Owned(bytes))
    }
}

/// Reads the `meta` box of a HEIF file.
pub fn read_meta(file: &[u8]) -> Result<Meta<'_>, HeifError> {
    let top = boxes(file)?;
    let brand_ok = top.iter().any(|found| {
        &found.kind == b"ftyp" && {
            let brands: Vec<&[u8]> = found.payload.chunks(4).collect();
            brands.iter().enumerate().any(|(index, brand)| {
                index != 1
                    && matches!(
                        *brand,
                        b"heic"
                            | b"heix"
                            | b"heim"
                            | b"heis"
                            | b"mif1"
                            | b"msf1"
                            | b"hevc"
                            | b"hevx"
                            | b"mif2"
                            | b"MiHE"
                    )
            })
        }
    });
    if !brand_ok {
        return Err(HeifError::new("not a HEIF file (no heic or mif1 brand)"));
    }
    let meta = top
        .iter()
        .find(|found| &found.kind == b"meta")
        .ok_or_else(|| HeifError::new("a HEIF file without its meta box"))?;
    let inner = boxes(&meta.payload[4.min(meta.payload.len())..])?;
    let mut result = Meta {
        primary: 0,
        items: Vec::new(),
        references: Vec::new(),
        properties: Vec::new(),
        item_data: &[],
    };
    for found in &inner {
        let mut reader = Reader::new(found.payload);
        match &found.kind {
            b"pitm" => {
                let (version, _) = reader.full_box()?;
                result.primary = if version == 0 {
                    u32::from(reader.u16()?)
                } else {
                    reader.u32()?
                };
            }
            b"iinf" => {
                let (version, _) = reader.full_box()?;
                if version == 0 {
                    reader.u16()?;
                } else {
                    reader.u32()?;
                }
                for entry in boxes(reader.rest())? {
                    if &entry.kind != b"infe" {
                        continue;
                    }
                    let mut entry_reader = Reader::new(entry.payload);
                    let (version, _) = entry_reader.full_box()?;
                    if version < 2 {
                        continue;
                    }
                    let id = if version == 2 {
                        u32::from(entry_reader.u16()?)
                    } else {
                        entry_reader.u32()?
                    };
                    entry_reader.u16()?;
                    let kind: [u8; 4] = entry_reader.bytes(4)?.try_into().unwrap_or(*b"    ");
                    entry_reader.string()?;
                    let content_type = if &kind == b"mime" {
                        entry_reader.string()?.to_string()
                    } else {
                        String::new()
                    };
                    let item = item_entry(&mut result.items, id);
                    item.kind = kind;
                    item.content_type = content_type;
                }
            }
            b"iloc" => {
                let (version, _) = reader.full_box()?;
                let sizes = reader.u16()?;
                let offset_size = (sizes >> 12) as u8;
                let length_size = ((sizes >> 8) & 15) as u8;
                let base_size = ((sizes >> 4) & 15) as u8;
                let index_size = if version >= 1 { (sizes & 15) as u8 } else { 0 };
                let count = if version < 2 {
                    u32::from(reader.u16()?)
                } else {
                    reader.u32()?
                };
                for _ in 0..count {
                    let id = if version < 2 {
                        u32::from(reader.u16()?)
                    } else {
                        reader.u32()?
                    };
                    let method = if version >= 1 {
                        (reader.u16()? & 15) as u8
                    } else {
                        0
                    };
                    reader.u16()?;
                    let base = reader.sized(base_size)?;
                    let extent_count = reader.u16()?;
                    let mut extents = Vec::with_capacity(usize::from(extent_count));
                    for _ in 0..extent_count {
                        if index_size > 0 {
                            reader.sized(index_size)?;
                        }
                        let offset = reader.sized(offset_size)?;
                        let length = reader.sized(length_size)?;
                        extents.push((offset, length));
                    }
                    item_entry(&mut result.items, id).location = Some((method, base, extents));
                }
            }
            b"iref" => {
                let (version, _) = reader.full_box()?;
                for reference in boxes(reader.rest())? {
                    let mut entry = Reader::new(reference.payload);
                    let read_id = |entry: &mut Reader<'_>| -> Result<u32, HeifError> {
                        if version == 0 {
                            Ok(u32::from(entry.u16()?))
                        } else {
                            entry.u32()
                        }
                    };
                    let from = read_id(&mut entry)?;
                    let count = entry.u16()?;
                    let mut targets = Vec::with_capacity(usize::from(count));
                    for _ in 0..count {
                        targets.push(read_id(&mut entry)?);
                    }
                    result.references.push((reference.kind, from, targets));
                }
            }
            b"iprp" => {
                for part in boxes(found.payload)? {
                    match &part.kind {
                        b"ipco" => {
                            for property in boxes(part.payload)? {
                                result.properties.push(read_property(&property)?);
                            }
                        }
                        b"ipma" => {
                            let mut map = Reader::new(part.payload);
                            let (version, flags) = map.full_box()?;
                            let count = map.u32()?;
                            for _ in 0..count {
                                let id = if version < 1 {
                                    u32::from(map.u16()?)
                                } else {
                                    map.u32()?
                                };
                                let associations = map.u8()?;
                                let mut indices = Vec::with_capacity(usize::from(associations));
                                for _ in 0..associations {
                                    let index = if flags & 1 == 1 {
                                        usize::from(map.u16()? & 0x7FFF)
                                    } else {
                                        usize::from(map.u8()? & 0x7F)
                                    };
                                    if index > 0 {
                                        indices.push(index - 1);
                                    }
                                }
                                item_entry(&mut result.items, id).properties.extend(indices);
                            }
                        }
                        _ => {}
                    }
                }
            }
            b"idat" => result.item_data = found.payload,
            _ => {}
        }
    }
    Ok(result)
}

fn item_entry(items: &mut Vec<Item>, id: u32) -> &mut Item {
    if let Some(index) = items.iter().position(|item| item.id == id) {
        return &mut items[index];
    }
    items.push(Item {
        id,
        ..Item::default()
    });
    items.last_mut().expect("just pushed")
}

fn read_property(property: &BoxRef<'_>) -> Result<Property, HeifError> {
    let mut reader = Reader::new(property.payload);
    Ok(match &property.kind {
        b"hvcC" => {
            let header = reader.bytes(22)?;
            let length_size = usize::from(header[21] & 3) + 1;
            let arrays = reader.u8()?;
            let mut nals = Vec::new();
            for _ in 0..arrays {
                reader.u8()?;
                let count = reader.u16()?;
                for _ in 0..count {
                    let length = usize::from(reader.u16()?);
                    nals.push(reader.bytes(length)?.to_vec());
                }
            }
            Property::HevcConfig { nals, length_size }
        }
        b"ispe" => {
            reader.full_box()?;
            Property::Size {
                width: reader.u32()?,
                height: reader.u32()?,
            }
        }
        b"irot" => Property::Rotation(reader.u8()? & 3),
        b"imir" => Property::Mirror(reader.u8()? & 1),
        b"clap" => {
            let mut fractions = [(0i64, 1i64); 4];
            for (index, fraction) in fractions.iter_mut().enumerate() {
                let numerator = reader.u32()?;
                let denominator = i64::from(reader.u32()?).max(1);
                // Width and height are unsigned; the offsets signed.
                let numerator = if index < 2 {
                    i64::from(numerator)
                } else {
                    i64::from(numerator as i32)
                };
                *fraction = (numerator, denominator);
            }
            Property::CleanAperture(fractions)
        }
        b"colr" => {
            let kind = reader.bytes(4)?;
            match kind {
                b"nclx" => Property::Colour(Colour::Nclx {
                    primaries: reader.u16()?,
                    transfer: reader.u16()?,
                    matrix: reader.u16()?,
                    full_range: reader.u8()? & 0x80 != 0,
                }),
                b"prof" | b"rICC" => Property::Colour(Colour::Icc(reader.rest().to_vec())),
                _ => Property::Other,
            }
        }
        b"pixi" => {
            reader.full_box()?;
            let channels = reader.u8()?;
            Property::BitDepths(reader.bytes(usize::from(channels))?.to_vec())
        }
        b"auxC" => {
            reader.full_box()?;
            Property::Auxiliary(reader.string()?.to_string())
        }
        _ => Property::Other,
    })
}
