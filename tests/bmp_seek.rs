//! A bottom-up BMP read from a seekable input (backwards, a block of rows
//! at a time) gives exactly the rows a stream read gives, across block
//! boundaries and a partial last block, and a file cut short fails both
//! ways.

use std::io::Cursor;

use sublime::image::ColorType;
use sublime::io::bmp::{read_bmp_rows, read_bmp_rows_seekable};
use sublime::io::png::RowSink;

#[derive(Default)]
struct Rows {
    start: Option<(u32, u32, ColorType)>,
    pixels: Vec<u8>,
}

impl RowSink for Rows {
    fn start(&mut self, width: u32, height: u32, color: ColorType) -> std::io::Result<()> {
        self.start = Some((width, height, color));
        Ok(())
    }

    fn row(&mut self, pixels: &[u8]) -> std::io::Result<()> {
        self.pixels.extend_from_slice(pixels);
        Ok(())
    }
}

/// A bottom-up 24-bit BMP whose pixel at (x, y) counted from the top is
/// a function of both, so a row out of order or shifted shows.
fn bottom_up(width: u32, height: u32) -> Vec<u8> {
    let row_bytes = (width as usize * 3).div_ceil(4) * 4;
    let pixel_bytes = row_bytes * height as usize;
    let mut file = Vec::new();
    file.extend_from_slice(b"BM");
    file.extend_from_slice(&((54 + pixel_bytes) as u32).to_le_bytes());
    file.extend_from_slice(&[0; 4]);
    file.extend_from_slice(&54u32.to_le_bytes());
    file.extend_from_slice(&40u32.to_le_bytes());
    file.extend_from_slice(&(width as i32).to_le_bytes());
    file.extend_from_slice(&(height as i32).to_le_bytes());
    file.extend_from_slice(&1u16.to_le_bytes());
    file.extend_from_slice(&24u16.to_le_bytes());
    file.extend_from_slice(&[0; 24]);
    for stored in 0..height {
        let y = height - 1 - stored;
        let mut row = Vec::with_capacity(row_bytes);
        for x in 0..width {
            row.extend_from_slice(&[(x * 7 + y) as u8, (y * 3) as u8, (x ^ y) as u8]);
        }
        row.resize(row_bytes, 0);
        file.extend_from_slice(&row);
    }
    file
}

fn both_ways(file: &[u8]) -> (Result<Rows, String>, Result<Rows, String>) {
    let mut streamed = Rows::default();
    let stream = read_bmp_rows(&mut Cursor::new(file), &mut streamed)
        .map(|()| streamed)
        .map_err(|error| format!("{error:?}"));
    let mut sought = Rows::default();
    let seek = read_bmp_rows_seekable(&mut Cursor::new(file.to_vec()), &mut sought)
        .map(|()| sought)
        .map_err(|error| format!("{error:?}"));
    (stream, seek)
}

#[test]
fn a_bottom_up_file_reads_the_same_from_the_end() {
    // 701 by 520 at 24 bits is 2104 bytes a row (1 of padding) and 1.1 MB
    // in all: one full 1 MB block and a partial one. The small image is
    // one partial block; the fixture is ImageMagick's.
    let fixture = std::fs::read(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/dpi/r150.bmp"),
    )
    .expect("fixture");
    for file in [bottom_up(701, 520), bottom_up(5, 3), fixture] {
        let (stream, seek) = both_ways(&file);
        let (stream, seek) = (stream.expect("stream"), seek.expect("seek"));
        assert_eq!(stream.start, seek.start);
        assert!(stream.pixels == seek.pixels, "rows differ");
    }
}

#[test]
fn a_file_cut_short_fails_both_ways() {
    let mut file = bottom_up(701, 520);
    file.truncate(file.len() - 100);
    let (stream, seek) = both_ways(&file);
    assert!(stream.is_err());
    assert!(seek.is_err());
}
