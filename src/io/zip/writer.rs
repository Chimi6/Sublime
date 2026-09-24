//! Writes a ZIP archive with every entry stored (no compression), which is
//! what iWork packages use for their own streams.

use std::io::{self, Write};

use super::crc32::crc32;

const LOCAL_SIGNATURE: u32 = 0x0403_4B50;
const CENTRAL_SIGNATURE: u32 = 0x0201_4B50;
const EOCD_SIGNATURE: u32 = 0x0605_4B50;

struct Written {
    name: Vec<u8>,
    crc: u32,
    size: u32,
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
        let crc = crc32(data);
        let size =
            u32::try_from(data.len()).map_err(|_| io::Error::other("ZIP entry over 4 GiB"))?;
        let name_bytes = name.as_bytes();
        let mut header = Vec::with_capacity(30 + name_bytes.len());
        header.extend_from_slice(&LOCAL_SIGNATURE.to_le_bytes());
        header.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0x21, 0, 0]); // version 2.0, no flags, stored, time, date
        header.extend_from_slice(&crc.to_le_bytes());
        header.extend_from_slice(&size.to_le_bytes());
        header.extend_from_slice(&size.to_le_bytes());
        header.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        header.extend_from_slice(&0u16.to_le_bytes());
        header.extend_from_slice(name_bytes);
        self.sink.write_all(&header)?;
        self.sink.write_all(data)?;
        self.entries.push(Written {
            name: name_bytes.to_vec(),
            crc,
            size,
            offset: self.position,
        });
        self.position = self
            .position
            .checked_add(header.len() as u32)
            .and_then(|position| position.checked_add(size))
            .ok_or_else(|| io::Error::other("ZIP archive over 4 GiB"))?;
        Ok(())
    }

    /// Writes the central directory and flushes.
    pub fn finish(mut self) -> io::Result<W> {
        let directory_offset = self.position;
        let mut directory = Vec::new();
        for entry in &self.entries {
            directory.extend_from_slice(&CENTRAL_SIGNATURE.to_le_bytes());
            directory.extend_from_slice(&[20, 3, 20, 0, 0, 0, 0, 0, 0, 0x21, 0, 0]); // made by unix 2.0, needed 2.0, flags, stored, time, date
            directory.extend_from_slice(&entry.crc.to_le_bytes());
            directory.extend_from_slice(&entry.size.to_le_bytes());
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
}
