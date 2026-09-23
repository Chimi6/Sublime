//! In-memory pipe for chaining converters across threads.

use std::io::{self, Read, Write};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

const CHANNEL_DEPTH: usize = 4;

pub struct PipeWriter {
    sender: SyncSender<Vec<u8>>,
}

pub struct PipeReader {
    receiver: Receiver<Vec<u8>>,
    current: Vec<u8>,
    offset: usize,
}

pub fn pipe() -> (PipeWriter, PipeReader) {
    let (sender, receiver) = sync_channel(CHANNEL_DEPTH);
    let writer = PipeWriter { sender };
    let reader = PipeReader {
        receiver,
        current: Vec::new(),
        offset: 0,
    };
    (writer, reader)
}

impl Write for PipeWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let chunk = buffer.to_vec();
        match self.sender.send(chunk) {
            Ok(()) => Ok(buffer.len()),
            Err(_) => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "pipe reader closed",
            )),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Read for PipeReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.offset == self.current.len() {
            match self.receiver.recv() {
                Ok(chunk) => {
                    self.current = chunk;
                    self.offset = 0;
                }
                Err(_) => return Ok(0),
            }
        }
        let available = &self.current[self.offset..];
        let count = available.len().min(buffer.len());
        buffer[..count].copy_from_slice(&available[..count]);
        self.offset += count;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn data_flows_from_writer_to_reader_and_eof_after_drop() {
        let (mut writer, mut reader) = pipe();
        let producer = std::thread::spawn(move || {
            writer.write_all(b"hello ").unwrap();
            writer.write_all(b"world").unwrap();
        });
        let mut text = String::new();
        reader.read_to_string(&mut text).unwrap();
        producer.join().unwrap();
        assert_eq!(text, "hello world");
    }

    #[test]
    fn writing_after_reader_dropped_is_broken_pipe() {
        let (mut writer, reader) = pipe();
        drop(reader);
        let error = writer.write_all(b"x").unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn partial_reads_across_chunks() {
        let (mut writer, mut reader) = pipe();
        writer.write_all(b"abcdef").unwrap();
        drop(writer);
        let mut small = [0u8; 4];
        let first = reader.read(&mut small).unwrap();
        assert_eq!(first, 4);
        let second = reader.read(&mut small).unwrap();
        assert_eq!(second, 2);
        let third = reader.read(&mut small).unwrap();
        assert_eq!(third, 0);
    }
}
