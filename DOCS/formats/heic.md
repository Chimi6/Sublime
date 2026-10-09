# HEIC

Apple's photo format since iOS 11: an HEVC (H.265) still image in the
HEIF container (ISO/IEC 23008-12, over the ISO base media file format).
Sublime reads it with an HEVC decoder and a HEIF parser written here
(`src/io/hevc`, `src/io/heif`), no libde265 or libheif. This is the
living map of what the reader handles, tied to the tests that prove it.

HEVC is covered by patent pools (Access Advance, Via LA). Decoding it
in software may need a licence where those patents hold.

## Status

- `heic -> png`, `heic -> jpeg`, `heic -> webp`, `heic -> bmp`, and every
  other image format, and `heic -> pdf`: shipped, conditional (the
  primary image, cropped, turned, and mirrored as the file says, with
  its alpha; 10-bit samples go to PNG, TIFF, and Netpbm at 16 bits and
  elsewhere at 8; the ICC profile and Exif are carried into PNG and JPEG,
  the Exif's orientation set to 1 since the pixels come out upright, and
  dropped with a warning elsewhere).
- JPEG output takes the picture's own YCbCr when it is JFIF's (8-bit
  4:2:0, BT.601, full range, as phones write it, not turned): no
  conversion to RGB and back, no second chroma subsampling.
- Extensions `heic`, `heif`, `hif`; a file branded `heic` is known by
  its first bytes too. Pictures in documents set into a PDF read HEIC
  by its brand.
- Writing HEIC is next (v1).

Oracles (`tests/heic_suite.rs`, fixtures in `tests/fixtures/heic`, all
of them synthetic images written by macOS `sips` or by libheif 1.20
with x265 4.1): every picture decodes to the same samples as libheif
(checksums of each plane), including Apple's grids, odd sizes with a
clean aperture, 10-bit, 4:4:4, and alpha; quarter turns and both
mirrors move the pixels as libheif does; the profile and Exif reach
the PNG; a grid streamed by bands equals its whole canvas; 10-bit goes
on at 16 bits; a JPEG sink gets the picture's own planes; cut files are
refused. Outside the suite, checked against libheif by hand: planes
bit-exact with libheif on 14 files from both encoders, 9 of Apple's 6016
by 6016 wallpapers, the 5 benchmark inputs, and 13 x265 variants
(small CTUs, transform skip, lossless blocks, no sign hiding, no SAO or
deblocking, constrained intra, deep transform trees); 18,000 damaged
files refused without a panic.

Three multi-slice x265 streams with filtering across slices off are not
bit-exact with libheif, and libheif is the one that is wrong: libde265
looks up a chroma CTB's slice at chroma coordinates read as luma
(`sao.cc`, `get_SliceHeader(xC, yC)`), so on a CTB's border it skips
offsets the standard applies. Apple's decoder agrees with ours there
(mean error 0.5 against libde265's 1.1 at those samples).

## What the reader does

| Part | Read |
|---|---|
| Boxes | `ftyp`, `meta` with `hdlr`, `pitm`, `iinf`/`infe` (versions 2 and 3), `iloc` (versions 0 to 2, `idat` and file offsets, several extents), `iref` (`dimg`, `auxl`, `cdsc`, `thmb`), `iprp`/`ipco`/`ipma`, `idat` |
| Items | `hvc1` (one HEVC picture), `grid` (tiles assembled onto the canvas, cut at its edge), `Exif`; the alpha item (`auxl` with the HEVC or MPEG alpha URN) |
| Properties | `hvcC` (parameter sets), `ispe`, `clap`, `irot`, `imir`, `colr` (`nclx` and ICC), `pixi`, `auxC`; transforms applied in their association order |
| HEVC | Main, Main 10, Main Still Picture, and the format range extensions' intra tools: 4:0:0, 4:2:0, 4:2:2, 4:4:4, 8 to 16 bits (tested at 8 and 10; not extended precision), CABAC with every intra syntax element, slices and dependent slices, tiles, wavefront substreams, PCM, transquant bypass, transform skip with rotation and its own contexts, implicit residual DPCM, persistent Rice adaptation, bypass alignment, cross-component prediction, scaling lists, chroma QP offsets, deblocking, sample adaptive offset |
| Colour | YCbCr to RGB by the `nclx` matrix, else the stream's VUI, else BT.601 full range: BT.709, BT.601, BT.2020, SMPTE 240M, FCC, identity (GBR), YCgCo; full and video range |

Chroma is brought up to the luma's size by interpolating between
chroma samples, each centred among the luma samples it covers (weights
3/4 and 1/4 across and down). libheif and Apple's ImageIO repeat each
chroma sample instead. On a photograph and a synthetic pattern encoded
by libheif at qualities 50 to 95, interpolating comes closer to the
original than either: 0.1 to 1.4 dB of PSNR over libheif's default (the
least on the hard-edged pattern), and 1.0 to 3.3 dB over `sips`. libheif's own bilinear option scores 40 dB where its
default scores 50, so it is not used as a reference.

A file is read an item at a time: its `meta` box first, then each
picture's or tile's bytes as it decodes, so a 19 MB wallpaper is never
held whole (a stream on standard input is read whole first).

Grid tiles decode on as many threads as the machine has (one in
WebAssembly), each worker reusing its decoding buffers and placing its
tile itself. A grid whose rows come out top to bottom streams: each row
of tiles is converted and handed on as soon as it is in, but for its
last two rows, which wait for the row below (the chroma interpolation
reads across the seam); a band's buffer holds the four rows above it on
top, and goes back to a pool. Tiles start no further ahead than the band
below the one being handed on, and a tile a thread past it, so a slow
sink holds a few bands, not the canvas.

A single picture with wavefront substreams (x265's default, and every
picture a phone writes in tiles) streams too: each CTB row decodes on a
thread into a band buffer from a small pool, deblocks inside itself
there, and a thread of its own runs the deblocking between bands and
SAO a band at a time while the calling thread converts the band before
and hands its rows on. The picture's 4x4 records live with each row's
band, and availability is computed, not tabled. Otherwise the canvas is
assembled, 8-bit samples as bytes, and a turned image is read down its
columns a strip of 64 at a time.

Conversion is exact to 0.5 of a step: at 8 bits 0.23% of samples sit one
off float rounding, all on exact halves; at 16 bits 3 samples in half a
million.

## Measured

`DOCS/benchmarks/heic-png.md` has the pair: every line passes against
libheif's `heif-dec`, PNG output 8 to 34 times faster and JPEG output 2.8
to 10 times faster at 15 to 57% of its memory. Apple's `sips`, on the
hardware decoder, is behind on every line but the 36-megapixel stock
grid to JPEG (430 against 529 MB/s, 66 against 42 MB).

## Known deviations

- Samples deeper than 8 bits reach 8-bit formats (JPEG, WebP, BMP) at 8.
- `imir` follows libheif: axis 0 flips top to bottom, 1 left to right.
  Apple's ImageIO ignores `imir` (its own files never use it).
- Cross-component prediction is implemented from the standard but no
  encoder at hand writes it (x265, Apple, and libde265 do not), so it is
  untested.
- Only the primary image is read; others (bursts, depth maps, other
  pictures in a collection) are reported as dropped. XMP is dropped.
- WebP, TIFF, and the other formats drop the profile and Exif with a
  warning; their writers do not carry metadata yet.
- A grid's tiles are each decoded whole on one thread: a 1024-pixel
  tile is a megabyte and a half held per thread, and a grid's last
  tiles leave threads idle. Tiles decoded by CTB rows as they finish,
  from one pool of rows across the grid, are the lever.
- A picture without wavefront substreams decodes on one thread and is
  held whole until its loop filters have run.
- Not read: AVIF (`av01`), JPEG items, overlays (`iovl`), image
  sequences, and derived `iden` items; the stream's inter frames (a
  HEIF holds intra pictures).
