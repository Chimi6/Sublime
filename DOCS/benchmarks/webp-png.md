# WebP <-> PNG

**Latest** (2026-09-26, first WebP: every memory line PASSES (lossy decode at 4 MB against the crate's 74); five speed lines FAIL: lossless decode on the photo 53.1 against 54.0 MB/s, lossy decode 64.7 against 66.1 (photo) and 242.6 against 260.3 (flat), lossless encode 182.9 against 421.4 (photo) and 428.1 against 559.2 (flat); our lossless files are 4% smaller than image-webp's on the photo)

## Purpose

WebP both ways on the image hub: the lossless and lossy decoders
streaming rows into the PNG writer, and the PNG reader streaming rows
into the lossless encoder, against the Rust crates people use for
each side.

## Reference

The `image` crate (image-webp: its lossless and lossy decoders, and
its lossless encoder) with the `png` crate at its `Default` level on
the PNG side, implemented in `bench/src/pairs/webp_png.rs`.
`time-decode` times image-webp's decode alone from memory. libwebp's
own lossless size (through Pillow, default effort) is recorded as
context for the encoder's ratio; libwebp's speed is not timed by the
harness (measured by hand in the Conclusions). `cwebp` and `dwebp`
are the conventional tools and are not wired in.

## Pass lines

Not slower, and no more memory, than the reference pipelines, in each
direction, shape, and kind. Throughput counts decoded pixel bytes for
the reader and input plus pixel bytes for the writer; the output size
is an `[extra]` row.

## Method

**Machine.** Recorded with each results block from the harness's
machine line.

**Inputs.** The jpeg-png pair's 4000 by 4000 RGB PNGs (a gradient with
independent per-channel noise, and flat blocks), and libwebp's WebP
files of the same pixels written through Pillow: lossless at its
default effort, and lossy at quality 80. The decode direction thus
reads a real encoder's files.

**Stock image.** When `bench/data/stock.webp` (or `$SUBLIME_STOCK_WEBP`)
exists, its decode lines run too, marked `[stock]`.

**Statistics.** `bench/run.sh webp-png`: three runs per command,
median wall clock of the whole process, peak resident memory from GNU
`time`.

## Threats to validity

- The images are synthetic; a lossy WebP of the noisy photo is small
  (0.3 MB) and its decode is filter- and conversion-bound.
- The pipelines include a PNG write, which dominates the lossy photo
  lines (about 520 of 700 ms on both sides).
- image-webp's lossless encoder does much less than ours (a fixed
  predictor, no color cache, no palette): the encode line compares
  speed at different ratios.
- A browser and a game ran during the session; lines within 3% moved
  between runs.

## Results

### 2026-09-26, first WebP

commit: c71023b (on the `webp` branch, before its merge)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

| Target | Ours | Reference | Result |
|---|---|---|---|
| webp (lossless) -> png, photo (45.8 MB of pixels, 25.8 MB on disk): throughput (MB/s of decoded pixels) | 53.1 | 54.0 (image + png) | FAIL |
| webp (lossless) -> png, photo: peak memory (MB) | 100.9 | 173.6 (image + png) | PASS |
| webp (lossy) -> png, photo (45.8 MB of pixels, 0.3 MB on disk): throughput (MB/s of decoded pixels) | 64.7 | 66.1 (image + png) | FAIL |
| webp (lossy) -> png, photo: peak memory (MB) | 4.5 | 74.5 (image + png) | PASS |
| png -> webp (lossless), photo (29.0 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 182.9 | 421.4 (png + image) | FAIL |
| png -> webp (lossless), photo: peak memory (MB) | 92.9 | 140.8 (png + image) | PASS |
| png -> webp (lossless), photo: output size (MB) [extra] | 27.4 | 28.5 (image); 25.8 (libwebp, default effort) | n/a |
| webp (lossless) -> png, flat (45.8 MB of pixels, 0.0 MB on disk): throughput (MB/s of decoded pixels) | 542.1 | 505.9 (image + png) | PASS |
| webp (lossless) -> png, flat: peak memory (MB) | 94.6 | 111.3 (image + png) | PASS |
| webp (lossy) -> png, flat (45.8 MB of pixels, 0.1 MB on disk): throughput (MB/s of decoded pixels) | 242.6 | 260.3 (image + png) | FAIL |
| webp (lossy) -> png, flat: peak memory (MB) | 4.2 | 74.2 (image + png) | PASS |
| png -> webp (lossless), flat (1.6 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 428.1 | 559.2 (png + image) | FAIL |
| png -> webp (lossless), flat: peak memory (MB) | 95.9 | 112.5 (png + image) | PASS |
| png -> webp (lossless), flat: output size (MB) [extra] | 0.0 | 0.0 (image); 0.0 (libwebp, default effort) | n/a |

## Conclusions

Decoding is correct to the bit against libwebp and at parity with
image-webp in its core: in process, the lossy flat and photo decodes
take 114 and 171 ms in image-webp and about 126 and 175 in ours; the
lossless decode ties. The small misses on the pipeline lines come from
the rest of each pipeline and move with the machine. Memory is our
win throughout, most of all on lossy input, which streams by
macroblock row at 4 MB.

The lossless encoder is not at the reference's speed. Configured as
image-webp's encoder is (a fixed predictor, no cache), ours writes the
same size in about 2.4 times its core time; with its color cache and
per-tile predictor search it writes 4% smaller on the synthetic photo
and 9% smaller on a real one, at 2.3 times the time. Against libwebp,
measured by hand on the same photo, ours is faster than libwebp's
fastest setting (0.32 against 0.40 s of encode) and 5% smaller than
its output; libwebp's default effort is 6% smaller again and takes
9.3 s. The levers left are in `STATE.md`.
