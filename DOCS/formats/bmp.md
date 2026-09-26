# BMP

Windows bitmaps: a file header, an info header, optional masks and
palette, and rows of pixels, bottom-up by default. Sublime reads BMP
into the image hub and writes it from the hub (`src/io/bmp/mod.rs`),
since 0.19.0. See `png.md` for the hub and the pairs.

## Status

- `bmp -> png`: shipped, lossless.
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

## What the writer does

Opaque images become 24-bit BGR with a 40-byte header; images with
alpha become 32-bit BGRA with a V4 header carrying the four masks and
an sRGB color space, so the alpha survives in readers that honor it.
Gray and gray+alpha expand to RGB and RGBA. Rows are written
bottom-up in one-megabyte pieces.
