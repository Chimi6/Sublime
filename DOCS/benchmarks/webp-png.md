# WebP <-> PNG

**Latest** (2026-09-27, 0.23.1 release: 2 lines FAIL, the lossless photo encode (0.21.0 decision) and the lossless flat decode (ahead when timed alternately; STATE Decisions))

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

### 2026-09-27, 0.23.1 release

| Target | Ours | Reference | Result |
|---|---|---|---|
| webp (lossless) -> png, photo (45.8 MB of pixels, 25.8 MB on disk): throughput (MB/s of decoded pixels) | 93.4 | 54.6 (image + png) | PASS |
| webp (lossless) -> png, photo: peak memory (MB) | 101.7 | 174.0 (image + png) | PASS |
| webp (lossy) -> png, photo (45.8 MB of pixels, 0.3 MB on disk): throughput (MB/s of decoded pixels) | 79.7 | 66.6 (image + png) | PASS |
| webp (lossy) -> png, photo: peak memory (MB) | 5.2 | 75.1 (image + png) | PASS |
| png -> webp (lossless), photo (29.0 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 249.6 | 446.4 (png + image) | FAIL |
| png -> webp (lossless), photo: peak memory (MB) | 95.3 | 141.0 (png + image) | PASS |
| png -> webp (lossless), photo: output size (MB) [extra] | 27.4 | 28.5 (image); 25.8 (libwebp, default effort) | n/a |
| webp (lossless) -> png, flat (45.8 MB of pixels, 0.0 MB on disk): throughput (MB/s of decoded pixels) | 539.5 | 541.7 (image + png) | FAIL |
| webp (lossless) -> png, flat: peak memory (MB) | 95.3 | 112.2 (image + png) | PASS |
| webp (lossy) -> png, flat (45.8 MB of pixels, 0.1 MB on disk): throughput (MB/s of decoded pixels) | 278.1 | 264.8 (image + png) | PASS |
| webp (lossy) -> png, flat: peak memory (MB) | 4.8 | 74.9 (image + png) | PASS |
| png -> webp (lossless), flat (1.6 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 744.7 | 626.4 (png + image) | PASS |
| png -> webp (lossless), flat: peak memory (MB) | 51.8 | 112.8 (png + image) | PASS |
| png -> webp (lossless), flat: output size (MB) [extra] | 0.0 | 0.0 (image); 0.0 (libwebp, default effort) | n/a |

commit: 121f26b (main at the release, before the version bump)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

### 2026-09-27, 0.22.0 release

commit: 89d3694 (main at the release, before the version bump)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

| Target | Ours | Reference | Result |
|---|---|---|---|
| webp (lossless) -> png, photo (45.8 MB of pixels, 25.8 MB on disk): throughput (MB/s of decoded pixels) | 92.0 | 53.1 (image + png) | PASS |
| webp (lossless) -> png, photo: peak memory (MB) | 101.9 | 173.9 (image + png) | PASS |
| webp (lossy) -> png, photo (45.8 MB of pixels, 0.3 MB on disk): throughput (MB/s of decoded pixels) | 78.2 | 66.2 (image + png) | PASS |
| webp (lossy) -> png, photo: peak memory (MB) | 4.9 | 75.1 (image + png) | PASS |
| png -> webp (lossless), photo (29.0 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 255.8 | 438.8 (png + image) | FAIL |
| png -> webp (lossless), photo: peak memory (MB) | 94.9 | 140.4 (png + image) | PASS |
| png -> webp (lossless), photo: output size (MB) [extra] | 27.4 | 28.5 (image); 25.8 (libwebp, default effort) | n/a |
| webp (lossless) -> png, flat (45.8 MB of pixels, 0.0 MB on disk): throughput (MB/s of decoded pixels) | 573.5 | 549.8 (image + png) | PASS |
| webp (lossless) -> png, flat: peak memory (MB) | 95.5 | 112.0 (image + png) | PASS |
| webp (lossy) -> png, flat (45.8 MB of pixels, 0.1 MB on disk): throughput (MB/s of decoded pixels) | 276.2 | 268.4 (image + png) | PASS |
| webp (lossy) -> png, flat: peak memory (MB) | 4.7 | 74.4 (image + png) | PASS |
| png -> webp (lossless), flat (1.6 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 765.7 | 601.5 (png + image) | PASS |
| png -> webp (lossless), flat: peak memory (MB) | 52.2 | 112.4 (png + image) | PASS |
| png -> webp (lossless), flat: output size (MB) [extra] | 0.0 | 0.0 (image); 0.0 (libwebp, default effort) | n/a |

### 2026-09-27, deflate stops searching on noise

commit: 39d8d97 (on the `deflate-noise` branch, before its merge)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

| Target | Ours | Reference | Result |
|---|---|---|---|
| webp (lossless) -> png, photo (45.8 MB of pixels, 25.8 MB on disk): throughput (MB/s of decoded pixels) | 93.3 | 54.4 (image + png) | PASS |
| webp (lossless) -> png, photo: peak memory (MB) | 102.2 | 174.3 (image + png) | PASS |
| webp (lossy) -> png, photo (45.8 MB of pixels, 0.3 MB on disk): throughput (MB/s of decoded pixels) | 79.4 | 66.6 (image + png) | PASS |
| webp (lossy) -> png, photo: peak memory (MB) | 4.7 | 75.0 (image + png) | PASS |
| png -> webp (lossless), photo (29.0 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 258.0 | 439.9 (png + image) | FAIL |
| png -> webp (lossless), photo: peak memory (MB) | 95.2 | 140.9 (png + image) | PASS |
| png -> webp (lossless), photo: output size (MB) [extra] | 27.4 | 28.5 (image); 25.8 (libwebp, default effort) | n/a |
| webp (lossless) -> png, flat (45.8 MB of pixels, 0.0 MB on disk): throughput (MB/s of decoded pixels) | 579.7 | 502.2 (image + png) | PASS |
| webp (lossless) -> png, flat: peak memory (MB) | 95.4 | 112.2 (image + png) | PASS |
| webp (lossy) -> png, flat (45.8 MB of pixels, 0.1 MB on disk): throughput (MB/s of decoded pixels) | 276.1 | 263.1 (image + png) | PASS |
| webp (lossy) -> png, flat: peak memory (MB) | 4.6 | 74.5 (image + png) | PASS |
| png -> webp (lossless), flat (1.6 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 764.7 | 590.1 (png + image) | PASS |
| png -> webp (lossless), flat: peak memory (MB) | 52.3 | 112.6 (png + image) | PASS |
| png -> webp (lossless), flat: output size (MB) [extra] | 0.0 | 0.0 (image); 0.0 (libwebp, default effort) | n/a |

On noise the deflate searches one position in eight and writes the rest as literals; the lossless photo decode, whose time is the PNG write of a noise photograph, goes from 53.1 to 93.3 MB/s.

### 2026-09-27, lazy deflate, effort levels

commit: afaff87 (on the `webp-spikes` branch, before its merge)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

| Target | Ours | Reference | Result |
|---|---|---|---|
| webp (lossless) -> png, photo (45.8 MB of pixels, 25.8 MB on disk): throughput (MB/s of decoded pixels) | 53.1 | 54.4 (image + png) | FAIL |
| webp (lossless) -> png, photo: peak memory (MB) | 101.9 | 173.8 (image + png) | PASS |
| webp (lossy) -> png, photo (45.8 MB of pixels, 0.3 MB on disk): throughput (MB/s of decoded pixels) | 80.7 | 66.5 (image + png) | PASS |
| webp (lossy) -> png, photo: peak memory (MB) | 4.7 | 74.7 (image + png) | PASS |
| png -> webp (lossless), photo (29.0 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 249.3 | 428.4 (png + image) | FAIL |
| png -> webp (lossless), photo: peak memory (MB) | 95.9 | 140.7 (png + image) | PASS |
| png -> webp (lossless), photo: output size (MB) [extra] | 27.4 | 28.5 (image); 25.8 (libwebp, default effort) | n/a |
| webp (lossless) -> png, flat (45.8 MB of pixels, 0.0 MB on disk): throughput (MB/s of decoded pixels) | 583.3 | 533.5 (image + png) | PASS |
| webp (lossless) -> png, flat: peak memory (MB) | 95.4 | 111.5 (image + png) | PASS |
| webp (lossy) -> png, flat (45.8 MB of pixels, 0.1 MB on disk): throughput (MB/s of decoded pixels) | 269.2 | 266.9 (image + png) | PASS |
| webp (lossy) -> png, flat: peak memory (MB) | 4.8 | 74.0 (image + png) | PASS |
| png -> webp (lossless), flat (1.6 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 671.6 | 604.0 (png + image) | PASS |
| png -> webp (lossless), flat: peak memory (MB) | 52.6 | 112.5 (png + image) | PASS |
| png -> webp (lossless), flat: output size (MB) [extra] | 0.0 | 0.0 (image); 0.0 (libwebp, default effort) | n/a |

### 2026-09-27, encoder margins

commit: d1dc7cc (on the `webp-speed` branch, before its merge)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

| Target | Ours | Reference | Result |
|---|---|---|---|
| webp (lossless) -> png, photo (45.8 MB of pixels, 25.8 MB on disk): throughput (MB/s of decoded pixels) | 52.5 | 53.2 (image + png) | FAIL |
| webp (lossless) -> png, photo: peak memory (MB) | 100.7 | 173.6 (image + png) | PASS |
| webp (lossy) -> png, photo (45.8 MB of pixels, 0.3 MB on disk): throughput (MB/s of decoded pixels) | 65.3 | 65.7 (image + png) | FAIL |
| webp (lossy) -> png, photo: peak memory (MB) | 4.6 | 74.7 (image + png) | PASS |
| png -> webp (lossless), photo (29.0 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 247.5 | 395.7 (png + image) | FAIL |
| png -> webp (lossless), photo: peak memory (MB) | 95.4 | 140.2 (png + image) | PASS |
| png -> webp (lossless), photo: output size (MB) [extra] | 27.4 | 28.5 (image); 25.8 (libwebp, default effort) | n/a |
| webp (lossless) -> png, flat (45.8 MB of pixels, 0.0 MB on disk): throughput (MB/s of decoded pixels) | 496.3 | 526.4 (image + png) | FAIL |
| webp (lossless) -> png, flat: peak memory (MB) | 94.3 | 111.6 (image + png) | PASS |
| webp (lossy) -> png, flat (45.8 MB of pixels, 0.1 MB on disk): throughput (MB/s of decoded pixels) | 251.2 | 260.1 (image + png) | FAIL |
| webp (lossy) -> png, flat: peak memory (MB) | 4.1 | 74.2 (image + png) | PASS |
| png -> webp (lossless), flat (1.6 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 659.0 | 596.7 (png + image) | PASS |
| png -> webp (lossless), flat: peak memory (MB) | 52.1 | 112.8 (png + image) | PASS |
| png -> webp (lossless), flat: output size (MB) [extra] | 0.0 | 0.0 (image); 0.0 (libwebp, default effort) | n/a |

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
image-webp in its core. The decode lines are whole pipelines whose
larger part is the PNG write; they land within 1 to 6% of the
reference and the lossless flat line passed in one run and failed in
the next. The png crate's writer (per-row adaptive filters at its
balanced level) writes photo PNGs 5 to 10% smaller than ours at about
the same speed: that is the PNG writer's lever, not WebP's. Memory is
our win throughout, most of all on lossy input, which streams by
macroblock row at 4 MB.

The encoder margins (2026-09-27) came from the patterns file's own
lessons, applied to the encoder: predictors as byte loops over whole
rows (the compiler vectorizes them), the first pass's decisions
recorded as two bits per pixel so the writing pass replays them
instead of searching and hashing again, that first pass as one
function with its state in locals (a closure's captured position was
reloaded per pixel), the palette built as rows arrive with a byte per
pixel, and four predictor candidates where six chose nearly the same
tiles. The photo encode went from 183 to 248 MB/s at identical
output; the flat one from 428 to 659 and passes, at half the memory.

The photo encode still misses, and by design: image-webp writes a
fixed predictor with no color cache. Configured the same way ours is
now at parity on a real photo (test1, 0.15 s both) and 1.5 times its
time on the synthetic one. The cache and the predictor search are what
make our files smaller: without the cache a real photo grows 8%
(test1) to 37% (a 150-megapixel PNG), and against image-webp ours are
9% and 28% smaller on those two. Against libwebp, ours is faster than
its fastest setting and smaller than its output; libwebp's default
effort is 6% smaller again at thirty times the time.

The second round (2026-09-27) found the decode lines' time in the PNG
write, and the PNG write's in deflate: our default level wrote 8% more
than zlib's level 6 on the same filtered bytes, and three times more
on a flat image. The default level is now zlib's lazy evaluation with
two changes found here: chains hashed on four bytes, and a price check
that takes a short match only when the last block's codes make it
cheaper than its literals (on a filtered photograph most are not:
literals alone beat zlib's level 6 there). With it the PNGs are smaller
than the png crate's on every photo line and the writes faster; the
lossy decode lines pass. The lossless photo decode is 2% short and its
PNG 7% smaller than the reference's.

`--quality` now sets lossless WebP effort, as cwebp reads it: 50 and
under writes the gradient predictor without a search (0.22 s on the
photo, and on a 150-megapixel graphic smaller than the default), 90 and
up chooses predictors by an entropy estimate on every row (1 to 1.5%
smaller than the default, 40% slower). No level wins the photo encode
line while the color cache is on, and the cache is what makes real
photos 8 to 37% smaller; the line stays a known miss by design.
