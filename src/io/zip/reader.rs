//! Reads a ZIP archive held in memory: the central directory first, then
//! any entry on demand. Handles stored and deflated entries and the ZIP64
//! extensions for sizes and offsets.

use std::fmt;

use super::crc32::crc32;
use crate::io::deflate::{InflateError, inflate};

const EOCD_SIGNATURE: u32 = 0x0605_4B50;
const EOCD64_LOCATOR_SIGNATURE: u32 = 0x0706_4B50;
const EOCD64_SIGNATURE: u32 = 0x0606_4B50;
const CENTRAL_SIGNATURE: u32 = 0x0201_4B50;
const LOCAL_SIGNATURE: u32 = 0x0403_4B50;
const METHOD_STORED: u16 = 0;
const METHOD_DEFLATE: u16 = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZipError {
    /// No end-of-central-directory record: not a ZIP file.
    NotZip,
    /// A record extends past the end of the data.
    Truncated {
        what: &'static str,
    },
    /// A signature that is not what the structure requires.
    BadSignature {
        what: &'static str,
        offset: usize,
    },
    /// A compression method other than stored or deflate.
    UnsupportedMethod {
        name: String,
        method: u16,
    },
    /// Encrypted entry.
    Encrypted {
        name: String,
    },
    /// Checksum or size of the inflated data did not match the directory.
    BadData {
        name: String,
    },
    Inflate {
        name: String,
        error: InflateError,
    },
}

impl fmt::Display for ZipError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ZipError::NotZip => write!(formatter, "not a ZIP archive"),
            ZipError::Truncated { what } => write!(formatter, "ZIP {what} is truncated"),
            ZipError::BadSignature { what, offset } => {
                write!(formatter, "ZIP {what} at byte {offset} has a bad signature")
            }
            ZipError::UnsupportedMethod { name, method } => {
                write!(
                    formatter,
                    "ZIP entry {name:?} uses unsupported compression method {method}"
                )
            }
            ZipError::Encrypted { name } => write!(formatter, "ZIP entry {name:?} is encrypted"),
            ZipError::BadData { name } => {
                write!(formatter, "ZIP entry {name:?} failed its checksum")
            }
            ZipError::Inflate { name, error } => write!(formatter, "ZIP entry {name:?}: {error}"),
        }
    }
}

impl std::error::Error for ZipError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub method: u16,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    pub crc32: u32,
    pub flags: u16,
    local_header_offset: u64,
}

impl Entry {
    pub fn is_directory(&self) -> bool {
        self.name.ends_with('/')
    }
}

pub struct ZipArchive<'a> {
    bytes: &'a [u8],
    entries: Vec<Entry>,
}

impl<'a> ZipArchive<'a> {
    /// Parses the central directory of an archive held in `bytes`.
    pub fn parse(bytes: &'a [u8]) -> Result<ZipArchive<'a>, ZipError> {
        let eocd = find_eocd(bytes)?;
        let mut entry_count = u64::from(read_u16(bytes, eocd + 10)?);
        let mut directory_offset = u64::from(read_u32(bytes, eocd + 16)?);
        let needs_zip64 = entry_count == 0xFFFF || directory_offset == 0xFFFF_FFFF;
        if needs_zip64 && eocd >= 20 && read_u32(bytes, eocd - 20)? == EOCD64_LOCATOR_SIGNATURE {
            let eocd64 = read_u64(bytes, eocd - 20 + 8)? as usize;
            if read_u32(bytes, eocd64)? != EOCD64_SIGNATURE {
                return Err(ZipError::BadSignature {
                    what: "ZIP64 end of central directory",
                    offset: eocd64,
                });
            }
            entry_count = read_u64(bytes, eocd64 + 32)?;
            directory_offset = read_u64(bytes, eocd64 + 48)?;
        }
        let mut entries = Vec::with_capacity(entry_count.min(1 << 16) as usize);
        let mut offset = directory_offset as usize;
        for _ in 0..entry_count {
            if read_u32(bytes, offset)? != CENTRAL_SIGNATURE {
                return Err(ZipError::BadSignature {
                    what: "central directory entry",
                    offset,
                });
            }
            let flags = read_u16(bytes, offset + 8)?;
            let method = read_u16(bytes, offset + 10)?;
            let crc = read_u32(bytes, offset + 16)?;
            let mut compressed_size = u64::from(read_u32(bytes, offset + 20)?);
            let mut uncompressed_size = u64::from(read_u32(bytes, offset + 24)?);
            let name_length = usize::from(read_u16(bytes, offset + 28)?);
            let extra_length = usize::from(read_u16(bytes, offset + 30)?);
            let comment_length = usize::from(read_u16(bytes, offset + 32)?);
            let mut local_header_offset = u64::from(read_u32(bytes, offset + 42)?);
            let name_start = offset + 46;
            let name_bytes = slice(bytes, name_start, name_length, "entry name")?;
            let extra = slice(
                bytes,
                name_start + name_length,
                extra_length,
                "entry extra field",
            )?;
            zip64_extra(
                extra,
                &mut uncompressed_size,
                &mut compressed_size,
                &mut local_header_offset,
            )?;
            entries.push(Entry {
                name: String::from_utf8_lossy(name_bytes).into_owned(),
                method,
                compressed_size,
                uncompressed_size,
                crc32: crc,
                flags,
                local_header_offset,
            });
            offset = name_start + name_length + extra_length + comment_length;
        }
        Ok(ZipArchive { bytes, entries })
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn find(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.name == name)
    }

    /// Decompresses `entry` into `out` and verifies its checksum.
    pub fn read(&self, entry: &Entry, out: &mut Vec<u8>) -> Result<(), ZipError> {
        if entry.flags & 1 != 0 {
            return Err(ZipError::Encrypted {
                name: entry.name.clone(),
            });
        }
        let header = entry.local_header_offset as usize;
        if read_u32(self.bytes, header)? != LOCAL_SIGNATURE {
            return Err(ZipError::BadSignature {
                what: "local file header",
                offset: header,
            });
        }
        let name_length = usize::from(read_u16(self.bytes, header + 26)?);
        let extra_length = usize::from(read_u16(self.bytes, header + 28)?);
        let data_start = header + 30 + name_length + extra_length;
        let data = slice(
            self.bytes,
            data_start,
            entry.compressed_size as usize,
            "entry data",
        )?;
        let start = out.len();
        match entry.method {
            METHOD_STORED => out.extend_from_slice(data),
            METHOD_DEFLATE => {
                inflate(data, out, entry.uncompressed_size as usize).map_err(|error| {
                    ZipError::Inflate {
                        name: entry.name.clone(),
                        error,
                    }
                })?;
            }
            method => {
                return Err(ZipError::UnsupportedMethod {
                    name: entry.name.clone(),
                    method,
                });
            }
        }
        let produced = &out[start..];
        if produced.len() as u64 != entry.uncompressed_size || crc32(produced) != entry.crc32 {
            return Err(ZipError::BadData {
                name: entry.name.clone(),
            });
        }
        Ok(())
    }
}

/// The end-of-central-directory record sits in the last 64 KiB plus its
/// own size; this scans backwards for it.
fn find_eocd(bytes: &[u8]) -> Result<usize, ZipError> {
    if bytes.len() < 22 {
        return Err(ZipError::NotZip);
    }
    let lowest = bytes.len().saturating_sub(22 + 0xFFFF);
    let mut offset = bytes.len() - 22;
    loop {
        if read_u32(bytes, offset)? == EOCD_SIGNATURE {
            let comment_length = usize::from(read_u16(bytes, offset + 20)?);
            if offset + 22 + comment_length == bytes.len() {
                return Ok(offset);
            }
        }
        if offset == lowest {
            return Err(ZipError::NotZip);
        }
        offset -= 1;
    }
}

/// Applies the ZIP64 extra field (id 1) to whichever of the three values
/// the 32-bit record left saturated.
fn zip64_extra(
    extra: &[u8],
    uncompressed: &mut u64,
    compressed: &mut u64,
    offset: &mut u64,
) -> Result<(), ZipError> {
    let mut position = 0usize;
    while position + 4 <= extra.len() {
        let id = read_u16(extra, position)?;
        let length = usize::from(read_u16(extra, position + 2)?);
        let body = slice(extra, position + 4, length, "extra field")?;
        if id == 1 {
            let mut cursor = 0usize;
            for value in [&mut *uncompressed, &mut *compressed, &mut *offset] {
                if *value == 0xFFFF_FFFF && cursor + 8 <= body.len() {
                    *value = read_u64(body, cursor)?;
                    cursor += 8;
                }
            }
        }
        position += 4 + length;
    }
    Ok(())
}

fn slice<'a>(
    bytes: &'a [u8],
    start: usize,
    length: usize,
    what: &'static str,
) -> Result<&'a [u8], ZipError> {
    let end = start
        .checked_add(length)
        .ok_or(ZipError::Truncated { what })?;
    bytes.get(start..end).ok_or(ZipError::Truncated { what })
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, ZipError> {
    let slice = slice(bytes, offset, 2, "record")?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, ZipError> {
    let slice = slice(bytes, offset, 4, "record")?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, ZipError> {
    let slice = slice(bytes, offset, 8, "record")?;
    let mut array = [0u8; 8];
    array.copy_from_slice(slice);
    Ok(u64::from_le_bytes(array))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-entry archive built by hand: stored "hello" as "a.txt".
    fn stored_archive() -> Vec<u8> {
        let data = b"hello";
        let crc = crc32(data).to_le_bytes();
        let mut bytes = Vec::new();
        // local header
        bytes.extend_from_slice(&LOCAL_SIGNATURE.to_le_bytes());
        bytes.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]); // version, flags, method, time, date
        bytes.extend_from_slice(&crc);
        bytes.extend_from_slice(&5u32.to_le_bytes());
        bytes.extend_from_slice(&5u32.to_le_bytes());
        bytes.extend_from_slice(&5u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(b"a.txt");
        bytes.extend_from_slice(data);
        let directory = bytes.len();
        bytes.extend_from_slice(&CENTRAL_SIGNATURE.to_le_bytes());
        bytes.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]); // made by, needed, flags, method, time, date
        bytes.extend_from_slice(&crc);
        bytes.extend_from_slice(&5u32.to_le_bytes());
        bytes.extend_from_slice(&5u32.to_le_bytes());
        bytes.extend_from_slice(&5u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]); // disk, internal attrs, external attrs
        bytes.extend_from_slice(&0u32.to_le_bytes()); // local header offset
        bytes.extend_from_slice(b"a.txt");
        let directory_size = bytes.len() - directory;
        bytes.extend_from_slice(&EOCD_SIGNATURE.to_le_bytes());
        bytes.extend_from_slice(&[0, 0, 0, 0, 1, 0, 1, 0]);
        bytes.extend_from_slice(&(directory_size as u32).to_le_bytes());
        bytes.extend_from_slice(&(directory as u32).to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes
    }

    #[test]
    fn reads_a_stored_entry() {
        let bytes = stored_archive();
        let archive = ZipArchive::parse(&bytes).unwrap();
        let entry = archive.find("a.txt").unwrap();
        assert_eq!(entry.uncompressed_size, 5);
        let mut out = Vec::new();
        archive.read(entry, &mut out).unwrap();
        assert_eq!(out, b"hello");
    }

    #[test]
    fn corrupt_data_fails_the_checksum() {
        let mut bytes = stored_archive();
        let position = bytes
            .windows(5)
            .position(|window| window == b"hello")
            .unwrap();
        bytes[position] = b'j';
        let archive = ZipArchive::parse(&bytes).unwrap();
        let entry = archive.find("a.txt").unwrap();
        let mut out = Vec::new();
        assert!(matches!(
            archive.read(entry, &mut out),
            Err(ZipError::BadData { .. })
        ));
    }

    #[test]
    fn not_a_zip() {
        assert_eq!(ZipArchive::parse(b"hello").err(), Some(ZipError::NotZip));
        assert_eq!(ZipArchive::parse(&[0u8; 40]).err(), Some(ZipError::NotZip));
    }
}
