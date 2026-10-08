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
  its alpha; 10-bit samples become 8-bit; the ICC profile and Exif are
  carried into PNG and JPEG, the Exif's orientation set to 1 since the
  pixels come out upright, and dropped with a warning elsewhere).
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
the PNG; cut files are refused. Outside the suite: bit-exact planes
with libheif on 14 files from both encoders and on 9 of Apple's 6016
by 6016 wallpapers (36 tiles each), and 16,000 damaged files (cut,
bit flips, byte changes) refused without a panic since the last fix.

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

Grid tiles decode on as many threads as the machine has, each handed to
the canvas as it finishes. The canvas holds 8-bit samples as bytes.
Rows are converted one at a time. A turned image is converted whole,
then read down its columns.

## Measured

Apple's Sonoma wallpaper (6016 by 6016, 36 tiles, 20 MB) to JPEG on
an M-series Mac with 10 cores: 0.94 to 1.15 s and 200 MB, against
libheif's `heif-dec` at 1.34 s and 126 MB (4.5 s of CPU each). To PNG,
`heif-dec` takes 28 s. `sips` takes 0.17 s on the hardware decoder.

A 1600 by 1200 x265 picture at quality 90 (614 KB, one picture, no
tiles): 150 ms to decode and convert, against about 125 ms for libheif to
decode alone. The single-picture decoder is CABAC-bound: residual
coding and the arithmetic decoder are two thirds of the time.

## Known deviations

- Samples deeper than 8 bits are reduced to 8; the image hub is 8-bit.
- `imir` follows libheif: axis 0 flips top to bottom, 1 left to right.
  Apple's ImageIO ignores `imir` (its own files never use it).
- Cross-component prediction is implemented from the standard but no
  encoder at hand writes it (x265, Apple, and libde265 do not), so it is
  untested.
- Only the primary image is read; others (bursts, depth maps, other
  pictures in a collection) are reported as dropped. XMP is dropped.
- WebP, TIFF, and the other formats drop the profile and Exif with a
  warning; their writers do not carry metadata yet.
- One picture without tiles decodes on one thread; wavefront rows
  could run in parallel.
- A whole grid is held while it is converted; a grid whose rows are not
  turned could stream by tile rows.
- Not read: AVIF (`av01`), JPEG items, overlays (`iovl`), image
  sequences, and derived `iden` items; the stream's inter frames (a
  HEIF holds intra pictures).
