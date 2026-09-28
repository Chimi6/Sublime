# PNG

Portable Network Graphics: chunks around a zlib stream of filtered
rows. Sublime reads every kind of PNG into the image hub
(`src/io/png/reader.rs`) and writes 8-bit images from it
(`src/io/png/writer.rs`), since 0.19.0. The hub (`src/image`) is an
8-bit pixel buffer in gray, gray+alpha, RGB, or RGBA; every raster
format reads into it and writes from it, so PNG reaches BMP and BMP
reaches PNG. This is the living map of what the reader and writer
handle, tied to the tests that prove it.

## Status

- `png -> bmp`: shipped, conditional (16-bit samples become 8-bit, gray
  becomes RGB in the BMP, and metadata is dropped). Streams: rows go to
  the BMP writer as the unfilter produces them, top-down, and no image
  is held (5 MB peak on a 61 MB image).
- `bmp -> png`: shipped, lossless. Streams too: `PngRows` is a
  `RowSink` that filters each row against the one before and deflates
  as it goes, so the BMP reader hands rows over and nothing is held.

Oracles (`tests/png_suite.rs`, the PngSuite images in
`tests/fixtures/png` with the pixels Pillow decodes beside each as
`.pix`): all 116 valid images decode to Pillow's pixels (two cases
where Pillow departs from the specification, 16-bit gray+alpha and
sub-8-bit gray transparency keys, are corrected in the fixture
generator and noted in the test); all 14 corrupt images (bad
signatures, chunk CRCs, lengths, IHDR fields, missing chunks) are
refused; every valid image survives our writer and reader; and every
image survives PNG to BMP to PNG through the converters
(`tests/png_bmp.rs`). The reader is fed in pieces by the converter, so
memory is the image plus a few hundred kilobytes whatever the file
size, and `read_png_rows` hands each row to a `RowSink` instead of
holding an image at all (an interlaced image is decoded whole first,
since its rows arrive out of order).

## What the reader does

| Part | Read |
|---|---|
| signature, IHDR | width, height, bit depth, color type, interlace; refused when the combination is not one the specification allows or the image is empty or larger than 2^31 pixels |
| PLTE, tRNS | the palette; palette alpha, or a transparency key for gray and RGB images |
| IDAT (any number) | the zlib stream, inflated as the chunks arrive; each row unfiltered the moment it is complete, straight into the image for 8-bit gray, gray+alpha, RGB, and RGBA, through an expansion row otherwise |
| IEND | the end; a file cut short before it is refused |
| chunk CRCs | checked on every chunk |
| ancillary chunks | skipped and reported as warnings by name (gamma, color profile, text, timing) |
| unknown critical chunks | refused |

The inflater decodes up to three table entries (nine literals) from
one refill, the shape of fdeflate's loop; the chunk CRC runs as four
interleaved streams joined by zlib's combine, and the Adler-32 as
lane sums over 64-byte blocks, all in safe scalar code.

Samples: 1, 2, and 4 bits scale to 8 (`0..3` to `0..255`); 16-bit
samples keep their high byte, reported as a loss; palette indexes
expand through the palette, with alpha from tRNS when present, so a
palette image becomes RGB or RGBA. A transparency key makes gray
gray+alpha and RGB RGBA. Adam7 interlaced images are decoded pass by
pass into place. The filters (None, Sub, Up, Average, Paeth) reverse
per row; Sub runs a pixel's bytes in one word, two pixels at a time up
to four bytes per pixel.

## What the writer does

`PngRows` takes rows one at a time (`write_png` feeds it an image's
rows); it writes one IHDR (8-bit, the hub's color type, no interlace), IDAT chunks of
about 256 KiB of deflated rows each (our deflate at its default level,
one zlib stream across them, Adler-32 at the end), IEND. Each row
takes the filter whose residuals have the smallest sum of magnitudes
(the standard heuristic). The trial runs on every fourth row and its
winner is kept for the rows between (image statistics change slowly
down the rows; the trials cost four extra passes a row and the choice
moved sizes by at most 1.4% on the benchmark inputs); a trial tries
Sub first, stops any filter whose running sum passes the best, and
stops trying once a filter's residuals average under a sixteenth. No
text, gamma, or color profile chunks are written.

## Known deviations

- 16-bit images lose their low byte; the hub is 8-bit.
- Metadata (gamma, sRGB, ICC profiles, text, physical size, time) is
  not carried in either direction.
- APNG frames are not read; the first image is what the still PNG
  holds.
- On noise-like images the filter heuristic can choose Average where a
  fixed Sub compresses smaller; the measured case is in
  `DOCS/benchmarks/png-bmp.md`.
