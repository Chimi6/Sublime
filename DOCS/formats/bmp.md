# BMP

Windows bitmaps: a file header, an info header, optional masks and
palette, and rows of pixels, bottom-up by default. Sublime reads BMP
into the image hub and writes it from the hub (`src/io/bmp/mod.rs`),
since 0.19.0. See `png.md` for the hub and the pairs.

## Status

- `bmp -> png`: shipped, lossless. Streams: rows go to the PNG writer
  as they are decoded. A top-down file (ours, and any with a negative
  height) is never held (4 MB peak on a 418 MB image); a bottom-up
  file stores its last row first, so its pixel data is read once and
  handed over from the end (66 MB peak on a 61 MB image, where an
  image copy made it 125).
- `png -> bmp`: shipped, conditional (gray becomes RGB, 16-bit becomes
  8-bit, metadata dropped).

Oracles: `tests/png_bmp.rs` sends every PngSuite image through PNG to
BMP to PNG and compares pixels; an extensionless BMP is detected by
its `BM` magic on the command line.

## What the reader does

| Field | Read |
|---|---|
| headers | BITMAPINFOHEADER (40 bytes) and the V4 and V5 headers; a negative height means top-down |
| bits per pixel | 1, 4, and 8 through the palette; 16 and 32 through channel masks (the header's, the bit-fields masks, or the defaults: 5-5-5 for 16, 8-8-8 for 32); 24 as BGR |
| alpha | a 32-bit image with an alpha mask (V4 or later, or four bit-fields masks) reads as RGBA; otherwise opaque |
| compression | none and bit fields; RLE and JPEG are refused as unsupported |

Rows are padded to four bytes; a file shorter than its rows is refused.
`read_bmp_rows` reads the headers and palette (at most 1178 bytes) from
a stream, starts its `RowSink`, and then either streams rows or holds
the pixel data once, as above; `read_bmp` reads a whole buffer into an
image through the same row decoder.

## What the writer does

Opaque images become 24-bit BGR with a 40-byte header; images with
alpha become 32-bit BGRA with a V4 header carrying the four masks and
an sRGB color space, so the alpha survives in readers that honor it.
Gray and gray+alpha expand to RGB and RGBA. Rows are written in
one-megabyte pieces: bottom-up from a whole image (`write_bmp`), or
top-down (a negative height in the header, the form Windows has read
since 95 and every common decoder takes) when rows arrive one at a
time from a decoder (`BmpRows`, the `png -> bmp` path), so nothing is
held.
