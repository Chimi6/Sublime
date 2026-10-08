# HEIC -> PNG and JPEG

**Latest** (2026-10-08, `heic-read` branch, wavefront rows in parallel: every
line PASSES against libheif; ahead of Apple's hardware decoder (`sips`,
context) on every line but the 36-megapixel stock image to JPEG (437 against
524 MB/s) and memory on JPEG output)

## Purpose

HEIC read, the way photos leave phones: an HEVC decoder and HEIF parser
written here, into the PNG writer (lossless, 16-bit for 10-bit sources) and
the JPEG writer (the picture's own YCbCr when it is JFIF's). The work is the
decoder's: CABAC, prediction, transforms, and the two loop filters.

## Reference

libheif's `heif-dec` (libheif 1.20, libde265 1.0), the conventional tool and
what most others (ImageMagick, GIMP, Pillow's plugin) call underneath.
macOS `sips` is recorded as context: it decodes on Apple's hardware HEVC
block, so its numbers are not a software decoder's.

## Pass lines

Not slower, and no more memory, than `heif-dec`, for each output and input.

## Method

**Machine.** Recorded with each results block from the harness's machine
line.

**Inputs.** The jpeg-png pair's 4000 by 4000 RGB photo and flat images,
encoded by libheif with x265 4.1 (one picture with wavefront substreams:
the photo at qualities 50 and 90 and, from a 16-bit PNG of it, at 10 bits;
the flat image at 75) and by `sips` (Apple's encoder: 8 by 8 tiles of 512).

**Stock image.** When `/System/Library/Desktop Pictures/Sonoma.heic` exists
(6016 by 6016, 6 by 6 tiles of 1024, in a file of several images), its rows
run too, marked `[stock]`.

**Statistics.** `bench/run.sh heic-png`: three runs per command, median wall
clock of the whole process, peak resident memory from GNU `time`.

## Threats to validity

- The images are synthetic; the noisy photo makes CABAC-heavy streams.
- `heif-dec` writes 16-bit PNGs for 10-bit inputs, as we do; its PNG writer
  (libpng at its default level) is most of its PNG time.
- libheif decodes a grid's tiles on several threads and a single picture on
  one; we decode a single picture's wavefront rows on every thread too, and
  JPEG bands of a megapixel image on every thread.
- Ours and libheif's outputs differ in chroma upsampling (ours interpolates,
  libheif repeats); decoded planes are bit-identical (`tests/heic_suite.rs`).

## Results

### 2026-10-08, wavefront rows, filters, and JPEG bands in parallel

| Target | Ours | Reference (heif-dec) | Context (sips) | Result |
|---|---|---|---|---|
| heic -> png, hphoto-x265-q50 (45.8 MB of pixels, 2.1 MB on disk): throughput (MB/s of decoded pixels) | 108.8 | 10.7 | 47.9 | PASS |
| heic -> png, hphoto-x265-q50: peak memory (MB) | 49.8 | 120.3 | 149.8 | PASS |
| heic -> jpeg, hphoto-x265-q50 (45.8 MB of pixels, 2.1 MB on disk): throughput (MB/s of decoded pixels) | 523.1 | 53.9 | 290.5 | PASS |
| heic -> jpeg, hphoto-x265-q50: peak memory (MB) | 50.8 | 74.6 | 32.3 | PASS |
| heic -> png, hphoto-x265-q90 (45.8 MB of pixels, 11.9 MB on disk): throughput (MB/s of decoded pixels) | 66.0 | 8.7 | 41.4 | PASS |
| heic -> png, hphoto-x265-q90: peak memory (MB) | 80.9 | 150.0 | 189.2 | PASS |
| heic -> jpeg, hphoto-x265-q90 (45.8 MB of pixels, 11.9 MB on disk): throughput (MB/s of decoded pixels) | 236.6 | 32.8 | 207.2 | PASS |
| heic -> jpeg, hphoto-x265-q90: peak memory (MB) | 83.1 | 103.8 | 76.4 | PASS |
| heic -> png, hphoto-x265-10bit (45.8 MB of pixels, 8.7 MB on disk): throughput (MB/s of decoded pixels) | 57.7 | 1.9 | 16.3 | PASS |
| heic -> png, hphoto-x265-10bit: peak memory (MB) | 106.0 | 239.6 | 183.1 | PASS |
| heic -> jpeg, hphoto-x265-10bit (45.8 MB of pixels, 8.7 MB on disk): throughput (MB/s of decoded pixels) | 225.9 | 32.7 | 191.4 | PASS |
| heic -> jpeg, hphoto-x265-10bit: peak memory (MB) | 113.6 | 167.1 | 60.3 | PASS |
| heic -> png, hflat-x265-q75 (45.8 MB of pixels, 0.1 MB on disk): throughput (MB/s of decoded pixels) | 661.4 | 95.6 | 82.5 | PASS |
| heic -> png, hflat-x265-q75: peak memory (MB) | 45.4 | 110.5 | 142.5 | PASS |
| heic -> jpeg, hflat-x265-q75 (45.8 MB of pixels, 0.1 MB on disk): throughput (MB/s of decoded pixels) | 1258.9 | 275.8 | 387.6 | PASS |
| heic -> jpeg, hflat-x265-q75: peak memory (MB) | 47.7 | 68.4 | 18.8 | PASS |
| heic -> png, hphoto-apple (45.8 MB of pixels, 2.3 MB on disk): throughput (MB/s of decoded pixels) | 100.5 | 10.7 | 39.8 | PASS |
| heic -> png, hphoto-apple: peak memory (MB) | 42.6 | 93.6 | 148.3 | PASS |
| heic -> jpeg, hphoto-apple (45.8 MB of pixels, 2.3 MB on disk): throughput (MB/s of decoded pixels) | 526.2 | 175.5 | 291.6 | PASS |
| heic -> jpeg, hphoto-apple: peak memory (MB) | 43.1 | 47.5 | 27.6 | PASS |
| heic -> png, stock (103.5 MB of pixels, 19.4 MB on disk): throughput (MB/s of decoded pixels) [stock] | 101.1 | 3.6 | 45.4 | PASS |
| heic -> png, stock: peak memory (MB) [stock] | 126.0 | 227.2 | 316.0 | PASS |
| heic -> jpeg, stock (103.5 MB of pixels, 19.4 MB on disk): throughput (MB/s of decoded pixels) [stock] | 436.7 | 75.6 | 523.5 | PASS |
| heic -> jpeg, stock: peak memory (MB) [stock] | 119.3 | 131.0 | 38.8 | PASS |

commit: the `heic-read` branch, this change
machine: Darwin 24.5.0 arm64, 10 cpus, Apple M1 Max

### 2026-10-08, decoder speed and memory pass (single-threaded pictures)

| Target | Ours | Reference (heif-dec) | Context (sips) | Result |
|---|---|---|---|---|
| heic -> png, hphoto-x265-q50 (45.8 MB of pixels, 2.1 MB on disk): throughput (MB/s of decoded pixels) | 58.6 | 10.7 | 47.5 | PASS |
| heic -> png, hphoto-x265-q50: peak memory (MB) | 47.8 | 120.9 | 152.7 | PASS |
| heic -> jpeg, hphoto-x265-q50 (45.8 MB of pixels, 2.1 MB on disk): throughput (MB/s of decoded pixels) | 94.3 | 53.8 | 285.3 | PASS |
| heic -> jpeg, hphoto-x265-q50: peak memory (MB) | 41.4 | 74.6 | 31.9 | PASS |
| heic -> png, hphoto-x265-q90 (45.8 MB of pixels, 11.9 MB on disk): throughput (MB/s of decoded pixels) | 31.6 | 8.6 | 41.1 | PASS |
| heic -> png, hphoto-x265-q90: peak memory (MB) | 77.5 | 149.9 | 189.3 | PASS |
| heic -> jpeg, hphoto-x265-q90 (45.8 MB of pixels, 11.9 MB on disk): throughput (MB/s of decoded pixels) | 46.4 | 32.7 | 202.5 | PASS |
| heic -> jpeg, hphoto-x265-q90: peak memory (MB) | 73.5 | 104.1 | 76.3 | PASS |
| heic -> png, hphoto-x265-10bit (45.8 MB of pixels, 8.7 MB on disk): throughput (MB/s of decoded pixels) | 29.0 | 1.9 | 16.2 | PASS |
| heic -> png, hphoto-x265-10bit: peak memory (MB) | 99.4 | 239.6 | 183.2 | PASS |
| heic -> jpeg, hphoto-x265-10bit (45.8 MB of pixels, 8.7 MB on disk): throughput (MB/s of decoded pixels) | 44.0 | 32.6 | 191.6 | PASS |
| heic -> jpeg, hphoto-x265-10bit: peak memory (MB) | 93.5 | 171.1 | 60.5 | PASS |
| heic -> png, hflat-x265-q75 (45.8 MB of pixels, 0.1 MB on disk): throughput (MB/s of decoded pixels) | 391.6 | 94.3 | 82.6 | PASS |
| heic -> png, hflat-x265-q75: peak memory (MB) | 41.1 | 114.2 | 142.2 | PASS |
| heic -> jpeg, hflat-x265-q75 (45.8 MB of pixels, 0.1 MB on disk): throughput (MB/s of decoded pixels) | 408.4 | 274.3 | 397.8 | PASS |
| heic -> jpeg, hflat-x265-q75: peak memory (MB) | 37.7 | 68.3 | 19.0 | PASS |
| heic -> png, hphoto-apple (45.8 MB of pixels, 2.3 MB on disk): throughput (MB/s of decoded pixels) | 99.3 | 10.7 | 39.3 | PASS |
| heic -> png, hphoto-apple: peak memory (MB) | 42.3 | 92.1 | 147.8 | PASS |
| heic -> jpeg, hphoto-apple (45.8 MB of pixels, 2.3 MB on disk): throughput (MB/s of decoded pixels) | 415.5 | 173.2 | 292.2 | PASS |
| heic -> jpeg, hphoto-apple: peak memory (MB) | 33.6 | 48.5 | 27.3 | PASS |
| heic -> png, stock (103.5 MB of pixels, 19.4 MB on disk): throughput (MB/s of decoded pixels) [stock] | 97.5 | 3.6 | 44.9 | PASS |
| heic -> png, stock: peak memory (MB) [stock] | 116.6 | 236.6 | 315.0 | PASS |
| heic -> jpeg, stock (103.5 MB of pixels, 19.4 MB on disk): throughput (MB/s of decoded pixels) [stock] | 370.1 | 75.3 | 511.2 | PASS |
| heic -> jpeg, stock: peak memory (MB) [stock] | 110.1 | 135.2 | 40.1 | PASS |

commit: the `heic-read` branch, this change (d576263 plus the working tree)
machine: Darwin 24.5.0 arm64, 10 cpus, Apple M1 Max

## Conclusions

Against libheif every line passes: PNG output 3.7 to 27 times faster (its
PNG writer and, for 10-bit, its 16-bit path are slow), JPEG output 1.3 to 5
times faster, at 36 to 81% of its memory. The decoder alone, in
process and best of seven, beats libde265 on every input (the 4000-pixel
photo at quality 50: 422 ms against 771; the flat image: 61 against 104).

How it got there, each step bit-exact on 41 reference files:

- Colour conversion as whole-row integer loops the compiler vectorizes, the
  matrix fused with the byte interleave; the first rewrite lost precision
  (1.6% of samples off float rounding) and went back to 16 fraction bits.
- Intra prediction's references gathered a 4x4 unit at a time (one
  availability check a unit, not a sample) into fixed arrays, and blocks
  written as row slices: the flat decode 139 to 97 ms.
- The inverse transform in its row-vectorized form with the even and odd
  halves of each output pair from one pass (half the multiplies). A full
  even-odd butterfly measured slower (620 to 651 ms) and was reverted.
- Residual coding over a list of the significant positions, and signs and
  level suffixes as runs of bypass bins: the quality-50 decode 570 to 465.
- Planes held as bytes at 8 bits; SAO a CTB row at a time from a window
  instead of a copy of the picture; the crop moved in place; item data and
  NAL payloads borrowed from the file.
- Grids streamed by bands of tiles when their rows come out top to bottom,
  each worker decoding into its own reused buffers and placing its tile:
  the allocator had kept freed tile and band buffers resident (204 MB of
  large blocks allocated for a 54 MB peak on the stock image).
- JPEG output takes the picture's own 4:2:0 YCbCr when it is BT.601 at full
  range: no conversion to RGB and back, and no second chroma subsampling.

The second round parallelized what was left single-threaded, each step
bit-exact on the same 41 files:

- Wavefront rows decoded in parallel (x265 writes them by default, and so
  does Apple inside each tile): the decoder became row-oriented, each CTB
  row owning its band of the picture and hearing the bottom line, units,
  and records of the row above as messages, with the contexts after its
  second CTB; the crate's no-`unsafe` rule holds. The quality-50 photo's
  decode 429 to 103 ms.
- The deblocking filter by bands (vertical edges by CTB rows, horizontal
  edges from bands four rows up, so no edge spans two) and SAO by bands
  from boundary lines copied first: 103 to 65 ms.
- JPEG output of a megapixel and up with a restart interval each band
  (0.02% larger), bands encoded on every thread: decoded pixels identical
  (libjpeg-turbo and ours), quality-50 photo to JPEG 0.136 to 0.087 s.
- Dequantization as levels are read, significance contexts from tables
  built once, and CABAC in the branch-free form (packed state, one
  transition table, masks for the LPS case).

`sips` decodes on Apple's hardware. It still leads on the 36-megapixel
stock image to JPEG (437 against 524 MB/s): its 36 tiles of 1024 decode
in 4.5 waves on 10 cores, two of them efficiency cores, 1.42 s of work
taking 0.2 s where an even share would take 0.16; and it holds less
memory on JPEG output (a streaming hardware path). The levers are a single
pool of wavefront rows across a grid's tiles, and a decoder that hands on
rows as its loop filters finish (`DOCS/STATE.md`).
