# JPEG <-> PNG

**Latest** (2026-09-27, 0.24.0 release: every line PASSES)

## Purpose

The first lossy image pair: the JPEG reader (markers, Huffman, IDCT,
upsampling, color conversion) streaming rows into the PNG writer, and
the PNG reader streaming rows into the JPEG writer (color conversion,
downsampling, FDCT, quantization, Huffman). Both directions are
measured against the Rust crates people use for each side, and every
JPEG written is scored against the source pixels, since a size at
"the same quality number" says nothing when the subsampling differs.

## Reference

Decode: the `image` crate (zune-jpeg, a decoder with AVX2 and NEON
paths under `unsafe`) writing PNG through the `png` crate at its
`Default` level, the same level the png-bmp pair holds it to. Encode:
the `jpeg-encoder` crate (a mozjpeg port; 4:2:0 below quality 90 like
ours; its optional AVX2 path is off, as it is by default) as the pass
gate, and the `image` crate's own baseline encoder (4:2:2, standard
tables) as context. Both encode from the `png` crate's decode.
Implemented in `bench/src/pairs/jpeg_png.rs`; `time-decode` and
`time-encode` time the crates alone from memory for phase splits.
ImageMagick and libjpeg-turbo's `djpeg`/`cjpeg` are the conventional
external tools and are not wired in.

## Pass lines

Not slower, and no more memory, than the reference pipelines, in each
direction and shape. Throughput follows the standard's rule for
compressed formats: decoded pixel bytes for the reader, input plus
pixel bytes for the writer. Output size and PSNR at quality 85 are
`[extra]` rows: the PSNR is measured by the harness against the
source PNG's pixels for every encoder.

## Method

**Machine.** Recorded with each results block from the harness's
machine line.

**Inputs.** Two 4000 by 4000 RGB images written by the `png` crate:
`photo`, three gradients with independent noise of plus or minus
eight per channel; `flat`, 64-pixel blocks of five colors with hard
edges. The JPEG inputs are the same pixels written at quality 85
(4:2:0) by the `jpeg-encoder` crate, so the decode direction reads a
real encoder's file: 1.6 MB and 0.3 MB.

**Stock image.** When `bench/data/stock.jpg` (or `$SUBLIME_STOCK_JPG`)
exists, the same lines run on it, marked `[stock]` (see the README);
the PNG for the encode direction is our decode of it.

**Statistics.** `bench/run.sh jpeg-png`: three runs per command,
median wall clock of the whole process, peak resident memory from GNU
`time`. Rows and units follow `README.md`.

**Commands.**

- ours: `sublime -q convert photo.jpg out.png` and `sublime -q convert photo.png out.jpg`
- reference: `sublime-bench jpeg-png crates-jpeg-png photo.jpg out.png`, `crates-png-jpeg-encoder photo.png out.jpg` (jpeg-encoder), `crates-png-jpeg photo.png out.jpg` (image)

## Threats to validity

- The images are synthetic. The photo shape has noise around gradients
  and no photographic structure; the flat shape is the worst case for
  4:2:0 chroma, which both 4:2:0 encoders score the same on and the
  image crate's 4:2:2 scores 24 dB higher on at a larger file.
- The decode line measures a whole pipeline: our JPEG decode alone is
  at parity with zune-jpeg in process (35 ms on the flat image against
  33, 70 against 63 on the photo); what moves the line is the PNG
  writer behind it, our adaptive filters and deflate against the png
  crate's fixed Sub filter and miniz.
- The encoders differ in what they hold: ours streams sixteen rows,
  the crates take the whole image. The memory rows say what that
  costs, not what the crates could do.
- The machine's background load moved lines by up to 10% in the
  session; a line near parity is a coin toss between runs.

## Results

### 2026-09-27, 0.24.0 release

| Target | Ours | Reference | Result |
|---|---|---|---|
| jpeg -> png, photo (45.8 MB of pixels, 1.6 MB on disk): throughput (MB/s of decoded pixels) | 103.1 | 41.3 (image + png) | PASS |
| jpeg -> png, photo: peak memory (MB) | 6.8 | 70.3 (image + png) | PASS |
| png -> jpeg, photo (29.0 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 518.1 | 453.0 (png + jpeg-encoder); 262.2 (png + image) | PASS |
| png -> jpeg, photo: peak memory (MB) | 4.9 | 51.5 (png + jpeg-encoder); 51.4 (png + image) | PASS |
| png -> jpeg, photo: output size (MB) at quality 85 [extra] | 1.5 | 1.6 (jpeg-encoder); 2.0 (image) | n/a |
| png -> jpeg, photo: PSNR against the source (dB) at quality 85 [extra] | 36.19 | 35.90 (jpeg-encoder); 36.47 (image) | n/a |
| jpeg -> png, flat (45.8 MB of pixels, 0.3 MB on disk): throughput (MB/s of decoded pixels) | 745.9 | 643.6 (image + png) | PASS |
| jpeg -> png, flat: peak memory (MB) | 5.4 | 52.7 (image + png) | PASS |
| png -> jpeg, flat (1.6 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 578.8 | 452.0 (png + jpeg-encoder); 225.3 (png + image) | PASS |
| png -> jpeg, flat: peak memory (MB) | 5.2 | 51.3 (png + jpeg-encoder); 51.5 (png + image) | PASS |
| png -> jpeg, flat: output size (MB) at quality 85 [extra] | 0.3 | 0.3 (jpeg-encoder); 0.5 (image) | n/a |
| png -> jpeg, flat: PSNR against the source (dB) at quality 85 [extra] | 32.39 | 32.39 (jpeg-encoder); 56.88 (image) | n/a |

commit: e8088c4 (main at the release, before the version bump)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

### 2026-09-27, 0.23.1 release

| Target | Ours | Reference | Result |
|---|---|---|---|
| jpeg -> png, photo (45.8 MB of pixels, 1.6 MB on disk): throughput (MB/s of decoded pixels) | 103.6 | 41.5 (image + png) | PASS |
| jpeg -> png, photo: peak memory (MB) | 6.8 | 70.1 (image + png) | PASS |
| png -> jpeg, photo (29.0 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 503.2 | 431.9 (png + jpeg-encoder); 261.4 (png + image) | PASS |
| png -> jpeg, photo: peak memory (MB) | 4.9 | 51.2 (png + jpeg-encoder); 51.6 (png + image) | PASS |
| png -> jpeg, photo: output size (MB) at quality 85 [extra] | 1.5 | 1.6 (jpeg-encoder); 2.0 (image) | n/a |
| png -> jpeg, photo: PSNR against the source (dB) at quality 85 [extra] | 36.19 | 35.90 (jpeg-encoder); 36.47 (image) | n/a |
| jpeg -> png, flat (45.8 MB of pixels, 0.3 MB on disk): throughput (MB/s of decoded pixels) | 759.7 | 668.9 (image + png) | PASS |
| jpeg -> png, flat: peak memory (MB) | 4.9 | 52.6 (image + png) | PASS |
| png -> jpeg, flat (1.6 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 615.0 | 470.8 (png + jpeg-encoder); 222.6 (png + image) | PASS |
| png -> jpeg, flat: peak memory (MB) | 4.6 | 51.1 (png + jpeg-encoder); 51.1 (png + image) | PASS |
| png -> jpeg, flat: output size (MB) at quality 85 [extra] | 0.3 | 0.3 (jpeg-encoder); 0.5 (image) | n/a |
| png -> jpeg, flat: PSNR against the source (dB) at quality 85 [extra] | 32.39 | 32.39 (jpeg-encoder); 56.88 (image) | n/a |

commit: 121f26b (main at the release, before the version bump)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

### 2026-09-27, 0.22.0 release

commit: 89d3694 (main at the release, before the version bump)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

| Target | Ours | Reference | Result |
|---|---|---|---|
| jpeg -> png, photo (45.8 MB of pixels, 1.6 MB on disk): throughput (MB/s of decoded pixels) | 102.3 | 41.7 (image + png) | PASS |
| jpeg -> png, photo: peak memory (MB) | 6.2 | 70.3 (image + png) | PASS |
| png -> jpeg, photo (29.0 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 505.5 | 445.1 (png + jpeg-encoder); 262.9 (png + image) | PASS |
| png -> jpeg, photo: peak memory (MB) | 4.5 | 51.1 (png + jpeg-encoder); 51.6 (png + image) | PASS |
| png -> jpeg, photo: output size (MB) at quality 85 [extra] | 1.5 | 1.6 (jpeg-encoder); 2.0 (image) | n/a |
| png -> jpeg, photo: PSNR against the source (dB) at quality 85 [extra] | 36.19 | 35.90 (jpeg-encoder); 36.47 (image) | n/a |
| jpeg -> png, flat (45.8 MB of pixels, 0.3 MB on disk): throughput (MB/s of decoded pixels) | 710.3 | 637.0 (image + png) | PASS |
| jpeg -> png, flat: peak memory (MB) | 5.3 | 52.6 (image + png) | PASS |
| png -> jpeg, flat (1.6 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 608.4 | 450.5 (png + jpeg-encoder); 223.0 (png + image) | PASS |
| png -> jpeg, flat: peak memory (MB) | 4.7 | 51.0 (png + jpeg-encoder); 51.2 (png + image) | PASS |
| png -> jpeg, flat: output size (MB) at quality 85 [extra] | 0.3 | 0.3 (jpeg-encoder); 0.5 (image) | n/a |
| png -> jpeg, flat: PSNR against the source (dB) at quality 85 [extra] | 32.39 | 32.39 (jpeg-encoder); 56.88 (image) | n/a |

### 2026-09-27, the lazy deflate

commit: a5c702a (on the `webp-spikes` branch, before its merge)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

| Target | Ours | Reference | Result |
|---|---|---|---|
| jpeg -> png, photo (45.8 MB of pixels, 1.6 MB on disk): throughput (MB/s of decoded pixels) | 102.8 | 41.6 (image + png) | PASS |
| jpeg -> png, photo: peak memory (MB) | 6.1 | 69.8 (image + png) | PASS |
| png -> jpeg, photo (29.0 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 497.2 | 452.6 (png + jpeg-encoder); 259.8 (png + image) | PASS |
| png -> jpeg, photo: peak memory (MB) | 5.1 | 51.2 (png + jpeg-encoder); 51.3 (png + image) | PASS |
| png -> jpeg, photo: output size (MB) at quality 85 [extra] | 1.5 | 1.6 (jpeg-encoder); 2.0 (image) | n/a |
| png -> jpeg, photo: PSNR against the source (dB) at quality 85 [extra] | 36.19 | 35.90 (jpeg-encoder); 36.47 (image) | n/a |
| jpeg -> png, flat (45.8 MB of pixels, 0.3 MB on disk): throughput (MB/s of decoded pixels) | 780.3 | 646.6 (image + png) | PASS |
| jpeg -> png, flat: peak memory (MB) | 5.0 | 52.3 (image + png) | PASS |
| png -> jpeg, flat (1.6 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 578.2 | 460.1 (png + jpeg-encoder); 223.7 (png + image) | PASS |
| png -> jpeg, flat: peak memory (MB) | 5.5 | 51.6 (png + jpeg-encoder); 51.1 (png + image) | PASS |
| png -> jpeg, flat: output size (MB) at quality 85 [extra] | 0.3 | 0.3 (jpeg-encoder); 0.5 (image) | n/a |
| png -> jpeg, flat: PSNR against the source (dB) at quality 85 [extra] | 32.39 | 32.39 (jpeg-encoder); 56.88 (image) | n/a |

### 2026-09-26, chroma from summed RGB

commit: 8c81b1e (on the `jpeg-margins` branch, before its merge; a browser and a game took about half a core during the run)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

| Target | Ours | Reference | Result |
|---|---|---|---|
| jpeg -> png, photo (45.8 MB of pixels, 1.6 MB on disk): throughput (MB/s of decoded pixels) | 87.4 | 41.2 (image + png) | PASS |
| jpeg -> png, photo: peak memory (MB) | 5.8 | 69.8 (image + png) | PASS |
| png -> jpeg, photo (29.0 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 490.7 | 445.7 (png + jpeg-encoder); 261.8 (png + image) | PASS |
| png -> jpeg, photo: peak memory (MB) | 4.9 | 51.0 (png + jpeg-encoder); 51.4 (png + image) | PASS |
| png -> jpeg, photo: output size (MB) at quality 85 [extra] | 1.5 | 1.6 (jpeg-encoder); 2.0 (image) | n/a |
| png -> jpeg, photo: PSNR against the source (dB) at quality 85 [extra] | 36.19 | 35.90 (jpeg-encoder); 36.47 (image) | n/a |
| jpeg -> png, flat (45.8 MB of pixels, 0.3 MB on disk): throughput (MB/s of decoded pixels) | 705.3 | 640.4 (image + png) | PASS |
| jpeg -> png, flat: peak memory (MB) | 4.3 | 51.8 (image + png) | PASS |
| png -> jpeg, flat (1.6 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 565.3 | 451.3 (png + jpeg-encoder); 224.6 (png + image) | PASS |
| png -> jpeg, flat: peak memory (MB) | 4.8 | 51.4 (png + jpeg-encoder); 50.8 (png + image) | PASS |
| png -> jpeg, flat: output size (MB) at quality 85 [extra] | 0.3 | 0.3 (jpeg-encoder); 0.5 (image) | n/a |
| png -> jpeg, flat: PSNR against the source (dB) at quality 85 [extra] | 32.39 | 32.39 (jpeg-encoder); 56.88 (image) | n/a |

### 2026-09-26, first release of JPEG

commit: c9683c4 (on the `jpeg` branch, before its merge)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

| Target | Ours | Reference | Result |
|---|---|---|---|
| jpeg -> png, photo (45.8 MB of pixels, 1.6 MB on disk): throughput (MB/s of decoded pixels) | 87.2 | 41.5 (image + png) | PASS |
| jpeg -> png, photo: peak memory (MB) | 5.6 | 70.4 (image + png) | PASS |
| png -> jpeg, photo (29.0 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 440.4 | 444.5 (png + jpeg-encoder); 254.8 (png + image) | FAIL |
| png -> jpeg, photo: peak memory (MB) | 4.6 | 51.1 (png + jpeg-encoder); 51.4 (png + image) | PASS |
| png -> jpeg, photo: output size (MB) at quality 85 [extra] | 1.5 | 1.6 (jpeg-encoder); 2.0 (image) | n/a |
| png -> jpeg, photo: PSNR against the source (dB) at quality 85 [extra] | 36.19 | 35.90 (jpeg-encoder); 36.47 (image) | n/a |
| jpeg -> png, flat (45.8 MB of pixels, 0.3 MB on disk): throughput (MB/s of decoded pixels) | 670.9 | 652.7 (image + png) | PASS |
| jpeg -> png, flat: peak memory (MB) | 4.1 | 52.1 (image + png) | PASS |
| png -> jpeg, flat (1.6 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 537.4 | 472.3 (png + jpeg-encoder); 229.8 (png + image) | PASS |
| png -> jpeg, flat: peak memory (MB) | 4.6 | 51.1 (png + jpeg-encoder); 51.2 (png + image) | PASS |
| png -> jpeg, flat: output size (MB) at quality 85 [extra] | 0.3 | 0.3 (jpeg-encoder); 0.5 (image) | n/a |
| png -> jpeg, flat: PSNR against the source (dB) at quality 85 [extra] | 32.39 | 32.39 (jpeg-encoder); 56.88 (image) | n/a |

## Conclusions

**2026-09-26, margins.** The photo encode moved from a tie to a lead
when the 4:2:0 encoder started computing each chroma sample once from
a 2x2 block's summed RGB instead of converting every pixel and
averaging: a quarter of the chroma arithmetic, no downsampling pass,
and one rounding instead of two, so PSNR holds (36.19 dB) or rises.
Walking nonzero coefficients by bitmask gave another 1%. Two decode
spikes on color conversion (libjpeg's lookup tables, planar groups)
measured slower and were reverted; the flat decode's margin rests on
the PNG writer behind it. Earlier notes follow.

Seven of eight pass lines pass, and the eighth is a tie the harness
scores as a 1% miss: the photo encode at 440 against 445 MB/s in the
recorded block, 449 against 448 in the block before it, and 163
against 164 ms in seven direct runs each. The encoder is level with a
mozjpeg port on speed and ahead on what it writes: 1.5 MB at 36.19 dB
against 1.6 MB at 35.90 on the photo, and the `image` crate's 4:2:2
default costs a third more bytes for a quarter of a decibel. Both
4:2:0 encoders score the same 32.4 dB on the flat image, where 4:2:2
scores 56.9; choosing the subsampling from the chroma's own detail is
the lever recorded in `STATE.md`.

Decode wins by memory and by the writer behind it: our JPEG decode
alone is at parity with zune-jpeg in process (photo 70 ms against 63,
flat 35 against 33), and the rows go straight into the PNG writer,
whose deflate on decoded photographs runs twice as fast as the png
crate's at the same size since its chain budget follows the match
length. The flat decode line turned from a 16% miss into a pass when
the PNG writer's filter trials moved to every fourth row.

What was tried and reverted is in the patterns file: eight-lane
transforms, lane-form color conversion, one-pass filter scoring, and a
hand-written branch-free Paeth all measured slower than the scalar
forms.
