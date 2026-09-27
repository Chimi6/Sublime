# TGA

Truevision TGA: an 18-byte header, an optional ID field and color map,
then pixels raw or run-length encoded, bottom-up unless the descriptor
says top-down, and in TGA 2.0 a footer naming an extension area.
Sublime reads every common layout into the image hub and writes TGA
from it (`src/io/tga.rs`).

## Status

- `tga` (also `.icb`, `.vda`, `.vst`) to and from every image format:
  shipped. Reading drops the ID field and any extension area
  (thumbnail, author, dates); writing loses nothing.

Oracles (`tests/tga_suite.rs`, fixtures in `tests/fixtures/tga`): all
247 files read to the pixels they mean: Pillow's gray, gray and alpha,
palette, RGB, and RGBA files, raw and RLE, in both orientations, and
files built from the specification for the 15 and 16-bit, alpha-less
32-bit, right-to-left, row-crossing RLE, and 16 and 32-bit color map
cases. The writer produces Pillow's RLE bytes; all 6 corrupt files are
refused.

## What the reader does

| Part | Read |
|---|---|
| Image types | 1 and 9 (color-mapped, 8-bit indices), 2 and 10 (truecolor), 3 and 11 (gray), raw and RLE |
| Depths | gray 8 and 16 (gray and alpha); truecolor 15, 16, 24, 32; color map entries 15, 16, 24, 32 |
| Alpha | 32-bit pixels carry alpha only when the descriptor gives alpha bits; 16-bit pixels likewise, bit 15 set being opaque |
| RLE | packets may run across rows; one that runs past the image is refused |
| Orientation | top-down files stream a row at a time; bottom-up files hold the file (not the pixels) and are decoded last row first, RLE from row starts found in one pass; right-to-left rows are flipped |

## What the writer does

Top-down (descriptor bit 5), run-length encoded (types 10 and 11) with
packets that end at each row: a run for two or more equal pixels, raw
otherwise, 128 at most, as Pillow writes. Gray is 8-bit, gray and alpha
16-bit, RGB 24-bit, RGBA 32-bit with 8 alpha bits. The TGA 2.0 footer
closes the file.

## Known deviations

- Pillow reads a 32-bit file without alpha bits as RGBA with its
  fourth bytes as alpha; ours reads it as RGB, as the descriptor says.
- Pillow reads 16-bit pixels with bit 15 inverted whatever the
  descriptor; ours follows the descriptor (no alpha bits: no alpha).
- Color maps indexed by 16-bit values, and the extension area's
  gamma and attributes type, are not read.
