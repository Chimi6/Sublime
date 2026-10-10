# WebP

Google's web image format: a RIFF container around either a lossless
bitstream (VP8L: prefix-coded ARGB pixels with LZ77 and four
transforms) or a lossy one (a VP8 key frame, with an optional alpha
plane). Sublime reads both into the image hub (`src/io/webp`) and
writes lossless WebP from it. This is the living map of what the
reader and writer handle, tied to the tests that prove it.

## Status

- `webp -> png`, `webp -> bmp`: shipped, conditional (the pixels as
  libwebp decodes them; an animation keeps its first frame; ICC, Exif,
  and XMP chunks are dropped and reported by name).
- `webp -> jpeg`: shipped, lossy (the JPEG writer's terms).
- `png -> webp`, `jpeg -> webp`: shipped, conditional (a lossless WebP
  of the 8-bit pixels; metadata dropped). `bmp -> webp`: lossless.
- Lossy WebP output is not written; JPEG covers lossy output for now.

Oracles (`tests/webp_suite.rs`, fixtures in `tests/fixtures/webp`
written by Pillow's libwebp 1.6 with the pixels it decodes beside each
as `.pix`): all 224 images decode bit for bit to libwebp's pixels, 147
lossless (every transform, color-cache size, and the alpha files) and
77 lossy (qualities 20 to 100, 7 sizes from 1 by 1, alpha compressed
and raw, all three alpha filters); all 6 corrupt files are refused;
every image survives our writer and reader unchanged, and Pillow reads
our files to the same pixels.

## What the reader does

| Part | Read |
|---|---|
| RIFF, `VP8 `, `VP8L` | the simple forms |
| `VP8X` | the extended form: `ALPH` joined to a lossy image; `ICCP`, `EXIF`, `XMP ` skipped and reported |
| `ANIM`, `ANMF` | the first frame placed on the canvas (transparent elsewhere); the rest reported as dropped |
| VP8L | prefix codes (simple and normal), meta prefix codes by tile, the color cache, LZ77 with the 120 plane codes, and the four transforms (predictor with all 14 modes, cross-color, subtract-green, color indexing with bundled indices), all in libwebp's arithmetic |
| VP8 | key frames: segmentation, both loop filters with sharpness and deltas, token partitions, the 16x16, chroma, and ten 4x4 intra modes with libwebp's frame-edge values, the inverse DCT and Walsh-Hadamard transforms |
| `ALPH` | raw or VP8L-compressed alpha, with the horizontal, vertical, and gradient unfilters |

Lossy macroblock rows stream through a window of two rows: each is
reconstructed, loop-filtered, and the row above it goes out through
libwebp's fancy chroma upsampler and fixed-point color conversion, so
the pixels are never held (4 MB peak on a 46 MB image). A lossless
image is decoded whole as ARGB, since its back-references reach
anywhere above.

## What the writer does

Lossless only:

- While the rows seen have at most 256 colors they are held as a byte
  per pixel; such an image becomes a sorted palette with indices
  packed two, four, or eight to a pixel when there are few colors. The
  257th color turns what is held into ARGB words.
- Otherwise: subtract-green, then a predictor chosen per 16x16 tile
  from top, the average of left and top, select, and the gradient
  clamp by the smallest sum of residual magnitudes on every other row
  of the tile; residuals are computed in place.
- Symbols: runs copying the pixel to the left or the pixel above, a
  1024-entry color cache, literals; one set of five prefix codes,
  length-limited to 15 bits.
- `--effort` (the shared scale): `fast` writes the gradient predictor
  everywhere with no search; `balanced` (the default) scores four
  predictors on every other row by residual magnitude; `max` chooses
  each tile's predictor by an entropy estimate (the residuals priced by
  what the tiles above chose) on every row. Until the effort scale,
  `--quality` stood for this (50 and under fast, 90 and up max); it now
  does nothing for WebP and says so.
- Two passes: the first chooses and counts every symbol and records
  its choices in two bits per pixel (a cache hit, the start of a
  copy) with a list of copy lengths; the second replays them to write.

## Known deviations

- Not written: lossy WebP, animation, metadata.
- The writer uses no meta prefix codes, cross-color transform, or
  general LZ77; libwebp at its default effort writes files about 6%
  smaller on a real photograph (and takes about ten times longer).
- VP8 inter frames do not occur in WebP and are refused.
