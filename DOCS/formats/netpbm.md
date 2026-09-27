# Netpbm

The Netpbm family: PBM (black and white), PGM (gray), and PPM (color),
each in a plain ASCII form (P1, P2, P3) and a raw binary one (P4, P5,
P6), and PAM (P7), which holds gray or color with or without alpha.
Sublime reads every form into the image hub and writes each kind from
it (`src/io/netpbm.rs`), streaming rows both ways.

## Status

- `pbm`, `pgm`, `ppm` (also `.pnm`), `pam` to and from `png`, `bmp`,
  `jpeg`, `webp`, `qoi`, and each other: shipped.
- Reading loses nothing at 8 bits; wider samples (maxval over 255) are
  scaled to 8 bits and reported. Writing PAM loses nothing; PPM
  flattens alpha onto white; PGM also takes luma; PBM thresholds luma
  at half, black below.

Oracles (`tests/netpbm_suite.rs`, fixtures in `tests/fixtures/netpbm`):
all 140 files (every form, maxvals 1, 15, 255, 1000, and 65535, ASCII
with comments and uneven spacing, every PAM depth and BLACKANDWHITE)
read to the 8-bit pixels they mean; the PPM, PGM, and PBM writers
produce Pillow's bytes; PAM round-trips every image; all 8 corrupt
files are refused.

## What the reader does

| Part | Read |
|---|---|
| Header | the magic, then width, height, and maxval separated by whitespace and `#` comments; PAM's `WIDTH`, `HEIGHT`, `DEPTH`, `MAXVAL`, `TUPLTYPE`, `ENDHDR` lines |
| Raw raster | one or two bytes a sample (big-endian above 255), a row at a time from the input |
| Plain raster | numbers separated by whitespace and comments; PBM digits need no separators; a sample above maxval is an error |
| PBM | 1 is black, 0 white (PAM's BLACKANDWHITE is the other way, 1 white, as its maxval says) |
| Scaling | `(v * 255 + maxval / 2) / maxval`, Netpbm's own rounding |

## What the writers do

| Kind | Writes |
|---|---|
| PAM | P7 at maxval 255 with the tuple type of the image (GRAYSCALE, GRAYSCALE_ALPHA, RGB, RGB_ALPHA) |
| PPM | P6; gray repeated into RGB, alpha flattened onto white |
| PGM | P5; color as luma (libjpeg's and Pillow's weights), alpha flattened onto white |
| PBM | P4; luma below 128 is black, no dithering |

Headers are Pillow's layout (`P6\nW H\n255\n`), so the files match
its byte for byte.

## Known deviations

- Only the first image of a multi-image Netpbm stream is read.
- PBM output thresholds; Pillow's default `convert("1")` dithers.
