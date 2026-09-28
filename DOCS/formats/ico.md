# ICO and CUR

Windows icons and cursors: a directory of entries, each a PNG or a
headerless BMP (a DIB whose height is doubled, followed by a 1-bit AND
mask where 1 is transparent). A cursor is the same with a hotspot in
place of the planes and depth fields. Sublime reads either into the
image hub and writes icons from it (`src/io/ico.rs`).

## Status

- `ico` to and from every image format, `cur` from it: shipped.
  Reading takes the largest entry (the deepest of equal sizes) and
  reports the other sizes and a cursor's hotspot as dropped. Writing
  makes an icon of the standard sizes that fit (16, 24, 32, 48, 64,
  128, 256), the source fitted into each with its aspect kept, and the
  source itself as the largest when it is 256 pixels or less on a side;
  every entry is a PNG.

Oracles (`tests/ico_suite.rs`, fixtures in `tests/fixtures/ico`): all
46 files open to their largest, deepest entry's pixels (Pillow's PNG
entry icons of every size and color type; its BMP entry icons at 1, 8,
24, and 32 bits; built here: a 4-bit palette entry with an AND mask, a
32-bit entry whose alpha is all zero, a cursor, and mixed depths at one
size); written icons hold the planned sizes, the source unchanged when
it fits, and a constant color stays constant at every size; all 6
corrupt files are refused.

## What the reader does

| Part | Read |
|---|---|
| Directory | type 1 (icon) or 2 (cursor); every entry must lie inside the file |
| PNG entries | through the PNG reader |
| BMP entries | 1, 4, 8 (palette), 16, 24, and 32 bits, bottom-up; the AND mask for transparency below 32 bits, and at 32 bits when every alpha byte is zero (as Windows does) |

## What the writer does

The largest entry is built as the rows stream (a downscale when the
source is larger than 256), so a source is never held; the smaller
sizes are scaled from it by area averaging, color weighted by alpha
(`src/image/resize.rs`). Entries are PNG at our default deflate level,
smallest first.

## Known deviations

- Pillow writes BMP-entry AND masks without row padding and misreads
  them back; its decode of its own 1-bit BMP icons shows stray
  transparent pixels. Ours reads the file as written (opaque).
- Pillow scales with Lanczos; ours averages areas, so a downscaled
  entry differs from Pillow's by a few levels at edges.
- ICNS (Apple's icon container) is not read or written yet.
