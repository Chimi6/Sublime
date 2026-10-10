# HEIC

Apple's photo format since iOS 11: an HEVC (H.265) still image in the
HEIF container (ISO/IEC 23008-12, over the ISO base media file format).
Sublime reads and writes it with an HEVC decoder and encoder and a HEIF
parser and writer written here (`src/io/hevc`, `src/io/heif`), no
libde265, x265, or libheif. This is the living map of what the reader
and the writer handle, tied to the tests that prove it.

HEVC is covered by patent pools (Access Advance, Via LA). Decoding or
encoding it in software may need a licence where those patents hold.

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
- Every image format `-> heic`: shipped, lossy (an HEVC picture at the
  quality given, 50 by default as `heif-enc`; 4:2:0 chroma; 16-bit
  samples at 10 bits; alpha, the ICC profile, and Exif carried). A
  JPEG source hands over its own YCbCr, so its chroma is not resampled.

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

## What the writer does

The rows become YCbCr 4:2:0 as libheif makes them by default: BT.601 at
full range, chroma from each 2x2 block's average, signalled by an `nclx`
box and in the stream's VUI. An odd width or height is coded one longer
(4:2:0 crops to even sizes) and a `clap` takes the extra column or row
off. The file is libheif's layout: `ftyp` (`heic`, `mif1`, `miaf`), then
`meta` (`hdlr`, `pitm`, `iloc`, `iinf`, `iref`, and `iprp` with `hvcC`,
`colr`, `ispe`, `pixi`, and `clap`), then `mdat`. Alpha is an auxiliary
picture (`auxC` with the HEVC alpha URN, `auxl` to the primary), coded
4:2:0 with flat chroma so every Main decoder takes it; the ICC profile a
second `colr`; Exif an item described by `cdsc`.

The picture is one IDR of Main (or Main 10 for 16-bit sources), one
slice of wavefront rows: each row starts from the row above's contexts
after its second CTB and runs two CTBs behind it, and every thread
takes whichever row can go on next, the earliest first. Each row owns
its band of the reconstruction and hears the row above's last lines as
it goes, so a picture's memory is its source and a few rows.

CTBs are 32 pixels, or 16 for a small picture: one whose 32-pixel CTBs,
shared among eight workers, number fewer than the wavefront's diagonal
(W/32 + 2H/32 CTBs long) would leave the workers waiting on it. On
large photographs 32-pixel CTBs code 9 to 24% smaller than 16-pixel
ones; on the Kodak photographs (768 by 512, under the line) 16-pixel
CTBs cost 0.1 to 1.1% and code in about two thirds of the time. The rule reads
the picture alone, so a file is the same on every machine.

The decisions, by rate and distortion (lambda 0.57 * 2^((QP - 12) / 3),
as HM's):

- CUs from the CTB's size down to 8, an 8x8 CU also as four 4x4
  prediction blocks; a CU that codes no levels is not tried smaller.
- Luma modes: all 35 by SATD and their bits, then the best 8 (4x4 and
  8x8 blocks) or 6 (16x16 and 32x32), with the most probable modes,
  each coded in full with its transform tree (whole or split once)
  searched; a split is given up as soon as it costs more than the block
  whole or the best mode so far.
- Chroma: all five modes coded along the chosen tree.
- Levels by rate-distortion optimized quantization (HM's: each level
  among nearest, one less, and zero by its bits at the current context
  states, groups dropped and the last position moved where cheaper, the
  whole block dropped), signs hidden in the same pass (where a group's
  parity is wrong, the level whose change costs least in error and bits
  moves); transform skip on 4x4 blocks, tried for the chosen mode.
- Every choice priced by the syntax that will be written, through a
  CABAC estimator over the same context states as the encoder.
- Adaptive quantization: a QP for each CTB from its detail (busy areas
  coarser, flat ones finer; strength 0.95 with 32-pixel CTBs, 0.7 with
  16, each measured on Kodak against x265), sent as `cu_qp_delta`.
- The deblocking filter on, its offsets at -1 (measured kinder to
  detail: 0.2% at equal SSIM and RGB PSNR); no SAO (it gains x265 0.2%
  on the photographs measured, and costs a filter pass in the encoder).

Measured against the alternatives and not kept: quantization groups of
16 within 32-pixel CTBs, AQ by variance alone, SSIM-weighted lambda,
plain quantization while comparing modes (12% faster, 1.3% larger at
equal SSIM), fewer candidates or a cheap first pass over them (10 to 20%
faster, 0.1 to 0.4% larger), and leaving 8x8 CUs' transforms unsplit
(13% faster, 0.3% larger).

The encoder's reconstruction is made by the decoder's own prediction and
inverse transforms, so it is what every decoder produces: the tests
decode each test picture with the decoder here and require it to equal
the encoder's reconstruction sample for sample, at 8 and 10 bits, odd
sizes, QPs 12 to 40, both CTB sizes, and QP changes per CTB and per
quarter CTB; and a picture coded over and over by encoders racing for
the cores comes out the same each time.

Oracles for the writer: `src/io/hevc/encode` (CABAC bins decode back,
transforms round trip, the reconstruction equals the decoder's output)
and `tests/heic_write.rs` (pictures of 1x1 to 200x136, odd sizes,
alpha, gray, 16 bits at 10, the profile and Exif, smaller files at lower
qualities). By hand: libheif and macOS `sips` and Quick Look read odd
sizes, alpha, gray, 10-bit, and a 12000-pixel-wide picture to the same
samples as our reader.

## Measured

Reading: `DOCS/benchmarks/heic-png.md` has the pair: every line passes against
libheif's `heif-dec`, PNG output 8 to 34 times faster and JPEG output 2.8
to 10 times faster at 15 to 57% of its memory. Apple's `sips`, on the
hardware decoder, is behind on every line but the 36-megapixel stock
grid to JPEG (430 against 529 MB/s, 66 against 42 MB).

Writing: `DOCS/benchmarks/png-heic.md` has the pair: every line passes
against libheif's `heif-enc` (x265, slow preset): on the 24 Kodak
photographs files 1.4% smaller at equal luma PSNR and 0.3% at equal SSIM
and RGB PSNR; 1.2 to 1.6 times its speed on 4000 and 6016 pixel pictures
at a fifth to a quarter of its memory, and 1.3 times on Kodak. Apple's
`sips`, on the hardware encoder, is 2 to 15 times faster and 12 to 13%
larger.

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
- Written: 4:2:0 only (no 4:4:4 or 4:0:0 output, so a gray image is
  4:2:0 with flat chroma), no lossless mode, no SAO, and one picture,
  not a grid, up to 16384 pixels a side (macOS reads 12000 wide; older
  hardware decoders may want a grid).
