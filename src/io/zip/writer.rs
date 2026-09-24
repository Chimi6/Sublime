//! Writes a ZIP archive. Entries are stored or deflated as the caller
//! chooses; iWork packages store their own streams, Office packages
//! deflate theirs.

use std::io::{self, Write};

use super::crc32::crc32;
use crate::io::deflate::deflate;

const LOCAL_SIGNATURE: u32 = 0x0403_4B50;
const CENTRAL_SIGNATURE: u32 = 0x0201_4B50;
const EOCD_SIGNATURE: u32 = 0x0605_4B50;
const METHOD_STORED: u16 = 0;
const METHOD_DEFLATE: u16 = 8;

struct Written {
    name: Vec<u8>,
    method: u16,
    crc: u32,
    size: u32,
    compressed_size: u32,
    offset: u32,
}

pub struct ZipWriter<W: Write> {
    sink: W,
    position: u32,
    entries: Vec<Written>,
}

impl<W: Write> ZipWriter<W> {
    pub fn new(sink: W) -> ZipWriter<W> {
        ZipWriter {
            sink,
            position: 0,
            entries: Vec::new(),
        }
    }

    /// Adds one stored entry.
    pub fn add(&mut self, name: &str, data: &[u8]) -> io::Result<()> {
        self.add_entry(name, data, METHOD_STORED, data)
    }

    /// Adds one entry, deflated when that is smaller and stored otherwise.
    pub fn add_deflated(&mut self, name: &str, data: &[u8]) -> io::Result<()> {
        let mut compressed = Vec::with_capacity(data.len() / 2);
        deflate(data, &mut compressed);
        if compressed.len() < data.len() {
            self.add_entry(name, data, METHOD_DEFLATE, &compressed)
        } else {
            self.add_entry(name, data, METHOD_STORED, data)
        }
    }

    fn add_entry(
        &mut self,
        name: &str,
        data: &[u8],
        method: u16,
        payload: &[u8],
    ) -> io::Result<()> {
        let crc = crc32(data);
        let size =
            u32::try_from(data.len()).map_err(|_| io::Error::other("ZIP entry over 4 GiB"))?;
        let compressed_size =
            u32::try_from(payload.len()).map_err(|_| io::Error::other("ZIP entry over 4 GiB"))?;
        let name_bytes = name.as_bytes();
        let mut header = Vec::with_capacity(30 + name_bytes.len());
        header.extend_from_slice(&LOCAL_SIGNATURE.to_le_bytes());
        header.extend_from_slice(&[20, 0, 0, 0]); // version 2.0, no flags
        header.extend_from_slice(&method.to_le_bytes());
        header.extend_from_slice(&[0, 0, 0x21, 0]); // time, date
        header.extend_from_slice(&crc.to_le_bytes());
        header.extend_from_slice(&compressed_size.to_le_bytes());
        header.extend_from_slice(&size.to_le_bytes());
        header.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        header.extend_from_slice(&0u16.to_le_bytes());
        header.extend_from_slice(name_bytes);
        self.sink.write_all(&header)?;
        self.sink.write_all(payload)?;
        self.entries.push(Written {
            name: name_bytes.to_vec(),
            method,
            crc,
            size,
            compressed_size,
            offset: self.position,
        });
        self.position = self
            .position
            .checked_add(header.len() as u32)
            .and_then(|position| position.checked_add(compressed_size))
            .ok_or_else(|| io::Error::other("ZIP archive over 4 GiB"))?;
        Ok(())
    }

    /// Writes the central directory and flushes.
    pub fn finish(mut self) -> io::Result<W> {
        let directory_offset = self.position;
        let mut directory = Vec::new();
        for entry in &self.entries {
            directory.extend_from_slice(&CENTRAL_SIGNATURE.to_le_bytes());
            directory.extend_from_slice(&[20, 3, 20, 0, 0, 0]); // made by unix 2.0, needed 2.0, flags
            directory.extend_from_slice(&entry.method.to_le_bytes());
            directory.extend_from_slice(&[0, 0, 0x21, 0]); // time, date
            directory.extend_from_slice(&entry.crc.to_le_bytes());
            directory.extend_from_slice(&entry.compressed_size.to_le_bytes());
            directory.extend_from_slice(&entry.size.to_le_bytes());
            directory.extend_from_slice(&(entry.name.len() as u16).to_le_bytes());
            directory.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]); // extra, comment, disk, internal attrs
            directory.extend_from_slice(&0x81A4_0000u32.to_le_bytes()); // external attrs: regular file 0644
            directory.extend_from_slice(&entry.offset.to_le_bytes());
            directory.extend_from_slice(&entry.name);
        }
        let count = u16::try_from(self.entries.len())
            .map_err(|_| io::Error::other("more than 65535 ZIP entries"))?;
        let mut end = Vec::with_capacity(22);
        end.extend_from_slice(&EOCD_SIGNATURE.to_le_bytes());
        end.extend_from_slice(&[0, 0, 0, 0]);
        end.extend_from_slice(&count.to_le_bytes());
        end.extend_from_slice(&count.to_le_bytes());
        end.extend_from_slice(&(directory.len() as u32).to_le_bytes());
        end.extend_from_slice(&directory_offset.to_le_bytes());
        end.extend_from_slice(&0u16.to_le_bytes());
        self.sink.write_all(&directory)?;
        self.sink.write_all(&end)?;
        self.sink.flush()?;
        Ok(self.sink)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::zip::ZipArchive;

    #[test]
    fn written_archives_read_back() {
        let mut writer = ZipWriter::new(Vec::new());
        writer.add("a/b.txt", b"hello").unwrap();
        writer.add("c.bin", &[0, 1, 2, 255]).unwrap();
        let bytes = writer.finish().unwrap();
        let archive = ZipArchive::parse(&bytes).unwrap();
        assert_eq!(archive.entries().len(), 2);
        let mut out = Vec::new();
        archive
            .read(archive.find("c.bin").unwrap(), &mut out)
            .unwrap();
        assert_eq!(out, [0, 1, 2, 255]);
    }

    #[test]
    fn deflated_entries_read_back() {
        let text = b"a compressible entry, repeated many times over. ".repeat(500);
        let mut writer = ZipWriter::new(Vec::new());
        writer.add_deflated("text.txt", &text).unwrap();
        writer.add_deflated("tiny.bin", &[1, 2, 3]).unwrap();
        let bytes = writer.finish().unwrap();
        assert!(bytes.len() < text.len() / 4);
        let archive = ZipArchive::parse(&bytes).unwrap();
        assert_eq!(archive.find("text.txt").unwrap().method, METHOD_DEFLATE);
        assert_eq!(archive.find("tiny.bin").unwrap().method, METHOD_STORED);
        let mut out = Vec::new();
        archive
            .read(archive.find("text.txt").unwrap(), &mut out)
            .unwrap();
        assert_eq!(out, text);
    }
}
