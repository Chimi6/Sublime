# PNG <-> BMP

**Latest** (2026-09-26, with the JPEG release's deflate and filter changes: every line PASSES, the stock rows included; photo png -> bmp 528.1 against 443.4 MB/s, bmp -> png 167.8 against 146.3; flat 1040.4 against 742.3 and 1915.8 against 615.0; stock 537.2 against 521.8 and 294.4 against 177.0; memory 4 to 65 MB against 66 to 887)

## Purpose

The first image pair: the PNG reader (chunks, the streaming inflater,
unfiltering) into the pixel hub and the BMP writer out of it, and the
BMP reader with the PNG writer (adaptive filters, our deflate). Both
directions are measured against the Rust crates people use for each
side.

## Reference

`png` (the Rust PNG codec, fdeflate for decoding, flate2 and
miniz_oxide at its `Default` compression for encoding, its default
fixed Sub filter) with the `image` crate's BMP codec, implemented in
`bench/src/pairs/png_bmp.rs`: `crates-png-bmp` decodes with `png` and
encodes BMP with `image`; `crates-bmp-png` decodes BMP with `image`
and encodes with `png` at `Default`. The crate's `Fast` level
(fdeflate, a larger file) is an `[extra]` row for context, not a gate.
`time-decode` times the crate's decode alone from memory for the phase
split below. ImageMagick's `magick` is the conventional external tool
and is not wired in.

## Pass lines

Not slower, and no more memory, than the reference pipelines, in each
direction and shape. Throughput follows the standard's rule for
compressed formats: decoded pixel bytes for the reader, input plus
pixel bytes for the writer; the on-disk rate and the output size are
`[extra]` rows.

## Method

**Machine.** Recorded with each results block from the harness's
machine line.

**Inputs.** Two 4000 by 4000 RGBA images (61.0 MB of pixels) written
by the `png` crate: `photo`, three gradients with independent noise of
plus or minus eight per channel (32.8 MB on disk; the first generator
gave every channel one noise value with the sign flipped for green,
which LZ77 sees through the Sub filter and not through Average, and
was replaced); `flat`, 64-pixel blocks of five colors with hard edges
(1.7 MB on disk). The BMP inputs are our conversions of the two.

**Stock image.** When `bench/data/stock.png` (or `$SUBLIME_STOCK_PNG`)
exists, the same lines run on it, marked `[stock]`. The file is a
photograph from the web and is not committed, so the rows are
reproducible only where the owner put the image; they are recorded
because a real photograph, saved by a real encoder with its Paeth
filters and profile chunks, is the case the generated shapes cannot
stand in for. The image behind the block below: 11220 by 9775 RGBA,
418.4 MB of pixels in a 24.2 MB file, Paeth on 7557 of 9775 rows and
Up on the rest, with pHYs, iCCP, and cHRM chunks.

**Statistics.** `bench/run.sh png-bmp`: three runs per command,
median wall clock of the whole process, peak resident memory from GNU
`time`. Rows and units follow `README.md`.

**Commands.**

- ours: `sublime -q convert photo.png out.bmp` and `sublime -q convert photo.bmp out.png`
- reference: `sublime-bench png-bmp crates-png-bmp photo.png out.bmp` and `crates-bmp-png photo.bmp out.png`

## Threats to validity

- The images are synthetic. Noise around gradients is a fair stand-in
  for a photograph's incompressible detail but has no photographic
  structure (edges, textures, repeated regions), so the encode ratio
  says little about real photos.
- The crate's default filter is a fixed Sub with no trials; ours
  tries all five per row, which costs about 100 ms of the encode and
  can pick a different filter. On the photo both give the same size.
- `png` decodes with fdeflate and checks with SIMD CRC-32 and Adler-32
  through `unsafe` intrinsics; the main crate forbids `unsafe`, so its
  inflater, checksums, and unfilters are what auto-vectorization gives
  the baseline x86-64 target. That is the gap the photo decode line
  measures.
- The `[stock]` rows come from one image nobody else can fetch from
  the repository; they say how the pair behaves on a real photograph,
  not what a reader of this document can re-run.
- The machine ran a game client's helper process at about half a core
  during the session; runs moved by up to 10%, and the recorded block
  is one run of three-run medians.

## Results

### 2026-09-26, with the JPEG release's deflate and filter changes

commit: c9683c4 (on the `jpeg` branch, before its merge; deflate's payoff threshold at eight, filter trials on every fourth row, stb's Paeth)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

| Target | Ours | Reference | Result |
|---|---|---|---|
| png -> bmp, photo (61.0 MB of pixels): throughput (MB/s of decoded pixels) | 528.1 | 443.4 (png + image) | PASS |
| png -> bmp, photo (32.8 MB on disk): throughput (MB/s of file bytes) [extra] | 283.8 | 238.3 (png + image) | n/a |
| png -> bmp, photo: peak memory (MB) | 5.5 | 66.2 (png + image) | PASS |
| bmp -> png, photo (61.0 MB in + 61.0 MB of pixels): throughput (MB/s of input plus pixels) | 167.8 | 146.3 (image + png) | PASS |
| bmp -> png, photo: peak memory (MB) | 64.7 | 160.1 (image + png) | PASS |
| bmp -> png, photo: output size (MB) [extra] | 33.6 | 32.2 (png) | n/a |
| bmp -> png, photo: the png crate's fast level, throughput and size [extra] | - | 524.3 MB/s, 32.8 MB (png fast) | n/a |
| png -> bmp, flat (61.0 MB of pixels): throughput (MB/s of decoded pixels) | 1040.4 | 742.3 (png + image) | PASS |
| png -> bmp, flat (1.7 MB on disk): throughput (MB/s of file bytes) [extra] | 28.9 | 20.6 (png + image) | n/a |
| png -> bmp, flat: peak memory (MB) | 5.2 | 66.4 (png + image) | PASS |
| bmp -> png, flat (61.0 MB in + 61.0 MB of pixels): throughput (MB/s of input plus pixels) | 1915.8 | 615.0 (image + png) | PASS |
| bmp -> png, flat: peak memory (MB) | 64.7 | 127.8 (image + png) | PASS |
| bmp -> png, flat: output size (MB) [extra] | 0.3 | 0.4 (png) | n/a |
| bmp -> png, flat: the png crate's fast level, throughput and size [extra] | - | 766.5 MB/s, 1.7 MB (png fast) | n/a |
| png -> bmp, stock (418.4 MB of pixels, 24.2 MB on disk): throughput (MB/s of decoded pixels) [stock] | 537.2 | 521.8 (png + image) | PASS |
| png -> bmp, stock: peak memory (MB) [stock] | 5.4 | 423.7 (png + image) | PASS |
| bmp -> png, stock (418.4 MB in + 418.4 MB of pixels): throughput (MB/s of input plus pixels) [stock] | 294.4 | 177.0 (image + png) | PASS |
| bmp -> png, stock: peak memory (MB) [stock] | 4.0 | 886.8 (image + png) | PASS |
| bmp -> png, stock: output size (MB) [stock] | 29.8 | 44.2 (png) | n/a |

### 2026-09-26, bmp -> png streams rows into the PNG writer

commit: 5b31fc0 (on the `bmp-rows-stream` branch, before its merge)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

| Target | Ours | Reference | Result |
|---|---|---|---|
| png -> bmp, photo (61.0 MB of pixels): throughput (MB/s of decoded pixels) | 423.0 | 375.3 (png + image) | PASS |
| png -> bmp, photo (32.8 MB on disk): throughput (MB/s of file bytes) [extra] | 227.3 | 201.7 (png + image) | n/a |
| png -> bmp, photo: peak memory (MB) | 5.9 | 66.4 (png + image) | PASS |
| bmp -> png, photo (61.0 MB in + 61.0 MB of pixels): throughput (MB/s of input plus pixels) | 153.6 | 145.6 (image + png) | PASS |
| bmp -> png, photo: peak memory (MB) | 64.8 | 159.7 (image + png) | PASS |
| bmp -> png, photo: output size (MB) [extra] | 33.6 | 32.2 (png) | n/a |
| bmp -> png, photo: the png crate's fast level, throughput and size [extra] | - | 513.6 MB/s, 32.8 MB (png fast) | n/a |
| png -> bmp, flat (61.0 MB of pixels): throughput (MB/s of decoded pixels) | 976.1 | 707.6 (png + image) | PASS |
| png -> bmp, flat (1.7 MB on disk): throughput (MB/s of file bytes) [extra] | 27.1 | 19.7 (png + image) | n/a |
| png -> bmp, flat: peak memory (MB) | 5.3 | 66.8 (png + image) | PASS |
| bmp -> png, flat (61.0 MB in + 61.0 MB of pixels): throughput (MB/s of input plus pixels) | 1170.5 | 595.8 (image + png) | PASS |
| bmp -> png, flat: peak memory (MB) | 64.4 | 128.0 (image + png) | PASS |
| bmp -> png, flat: output size (MB) [extra] | 0.3 | 0.4 (png) | n/a |
| bmp -> png, flat: the png crate's fast level, throughput and size [extra] | - | 766.5 MB/s, 1.7 MB (png fast) | n/a |
| png -> bmp, stock (418.4 MB of pixels, 24.2 MB on disk): throughput (MB/s of decoded pixels) [stock] | 515.9 | 514.9 (png + image) | PASS |
| png -> bmp, stock: peak memory (MB) [stock] | 5.6 | 424.3 (png + image) | PASS |
| bmp -> png, stock (418.4 MB in + 418.4 MB of pixels): throughput (MB/s of input plus pixels) [stock] | 242.1 | 177.0 (image + png) | PASS |
| bmp -> png, stock: peak memory (MB) [stock] | 3.6 | 886.8 (image + png) | PASS |
| bmp -> png, stock: output size (MB) [stock] | 29.0 | 44.2 (png) | n/a |

### 2026-09-26, with a stock photograph beside the generated shapes

commit: e15be6b (on the `bench-stock-image` branch, before its merge)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

| Target | Ours | Reference | Result |
|---|---|---|---|
| png -> bmp, photo (61.0 MB of pixels): throughput (MB/s of decoded pixels) | 443.7 | 441.4 (png + image) | PASS |
| png -> bmp, photo (32.8 MB on disk): throughput (MB/s of file bytes) [extra] | 238.4 | 237.2 (png + image) | n/a |
| png -> bmp, photo: peak memory (MB) | 5.7 | 66.3 (png + image) | PASS |
| bmp -> png, photo (61.0 MB in + 61.0 MB of pixels): throughput (MB/s of input plus pixels) | 151.3 | 143.7 (image + png) | PASS |
| bmp -> png, photo: peak memory (MB) | 124.7 | 159.6 (image + png) | PASS |
| bmp -> png, photo: output size (MB) [extra] | 33.6 | 32.2 (png) | n/a |
| bmp -> png, photo: the png crate's fast level, throughput and size [extra] | - | 512.7 MB/s, 32.8 MB (png fast) | n/a |
| png -> bmp, flat (61.0 MB of pixels): throughput (MB/s of decoded pixels) | 963.3 | 724.4 (png + image) | PASS |
| png -> bmp, flat (1.7 MB on disk): throughput (MB/s of file bytes) [extra] | 26.8 | 20.1 (png + image) | n/a |
| png -> bmp, flat: peak memory (MB) | 5.4 | 66.6 (png + image) | PASS |
| bmp -> png, flat (61.0 MB in + 61.0 MB of pixels): throughput (MB/s of input plus pixels) | 983.2 | 606.9 (image + png) | PASS |
| bmp -> png, flat: peak memory (MB) | 124.9 | 127.8 (image + png) | PASS |
| bmp -> png, flat: output size (MB) [extra] | 0.3 | 0.4 (png) | n/a |
| bmp -> png, flat: the png crate's fast level, throughput and size [extra] | - | 762.9 MB/s, 1.7 MB (png fast) | n/a |
| png -> bmp, stock (418.4 MB of pixels, 24.2 MB on disk): throughput (MB/s of decoded pixels) [stock] | 458.0 | 412.7 (png + image) | PASS |
| png -> bmp, stock: peak memory (MB) [stock] | 5.5 | 424.0 (png + image) | PASS |
| bmp -> png, stock (418.4 MB in + 418.4 MB of pixels): throughput (MB/s of input plus pixels) [stock] | 232.8 | 176.3 (image + png) | PASS |
| bmp -> png, stock: peak memory (MB) [stock] | 839.5 | 886.6 (image + png) | PASS |
| bmp -> png, stock: output size (MB) [stock] | 29.0 | 44.2 (png) | n/a |

### 2026-09-26, png -> bmp streams rows into a top-down BMP

commit: 00ee909 (on the `inflate-speculative-literals` branch, before its merge)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

| Target | Ours | Reference | Result |
|---|---|---|---|
| png -> bmp, photo (61.0 MB of pixels): throughput (MB/s of decoded pixels) | 438.2 | 412.0 (png + image) | PASS |
| png -> bmp, photo (32.8 MB on disk): throughput (MB/s of file bytes) [extra] | 235.5 | 221.4 (png + image) | n/a |
| png -> bmp, photo: peak memory (MB) | 5.4 | 66.1 (png + image) | PASS |
| bmp -> png, photo (61.0 MB in + 61.0 MB of pixels): throughput (MB/s of input plus pixels) | 149.9 | 142.8 (image + png) | PASS |
| bmp -> png, photo: peak memory (MB) | 124.4 | 159.6 (image + png) | PASS |
| bmp -> png, photo: output size (MB) [extra] | 33.6 | 32.2 (png) | n/a |
| bmp -> png, photo: the png crate's fast level, throughput and size [extra] | - | 509.2 MB/s, 32.8 MB (png fast) | n/a |
| png -> bmp, flat (61.0 MB of pixels): throughput (MB/s of decoded pixels) | 1011.0 | 713.8 (png + image) | PASS |
| png -> bmp, flat (1.7 MB on disk): throughput (MB/s of file bytes) [extra] | 28.1 | 19.8 (png + image) | n/a |
| png -> bmp, flat: peak memory (MB) | 5.1 | 66.3 (png + image) | PASS |
| bmp -> png, flat (61.0 MB in + 61.0 MB of pixels): throughput (MB/s of input plus pixels) | 999.9 | 615.4 (image + png) | PASS |
| bmp -> png, flat: peak memory (MB) | 124.6 | 127.7 (image + png) | PASS |
| bmp -> png, flat: output size (MB) [extra] | 0.3 | 0.4 (png) | n/a |
| bmp -> png, flat: the png crate's fast level, throughput and size [extra] | - | 765.3 MB/s, 1.7 MB (png fast) | n/a |

### 2026-09-26, first release of the image category

commit: 14cd1d4 (on the `png` branch, before its merge)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

| Target | Ours | Reference | Result |
|---|---|---|---|
| png -> bmp, photo (61.0 MB of pixels): throughput (MB/s of decoded pixels) | 358.2 | 432.5 (png + image) | FAIL |
| png -> bmp, photo (32.8 MB on disk): throughput (MB/s of file bytes) [extra] | 192.5 | 232.4 (png + image) | n/a |
| png -> bmp, photo: peak memory (MB) | 64.9 | 66.4 (png + image) | PASS |
| bmp -> png, photo (61.0 MB in + 61.0 MB of pixels): throughput (MB/s of input plus pixels) | 150.9 | 145.5 (image + png) | PASS |
| bmp -> png, photo: peak memory (MB) | 125.0 | 159.8 (image + png) | PASS |
| bmp -> png, photo: output size (MB) [extra] | 33.6 | 32.2 (png) | n/a |
| bmp -> png, photo: the png crate's fast level, throughput and size [extra] | - | 508.8 MB/s, 32.8 MB (png fast) | n/a |
| png -> bmp, flat (61.0 MB of pixels): throughput (MB/s of decoded pixels) | 719.6 | 583.5 (png + image) | PASS |
| png -> bmp, flat (1.7 MB on disk): throughput (MB/s of file bytes) [extra] | 20.0 | 16.2 (png + image) | n/a |
| png -> bmp, flat: peak memory (MB) | 65.3 | 66.4 (png + image) | PASS |
| bmp -> png, flat (61.0 MB in + 61.0 MB of pixels): throughput (MB/s of input plus pixels) | 934.9 | 627.6 (image + png) | PASS |
| bmp -> png, flat: peak memory (MB) | 125.0 | 127.8 (image + png) | PASS |
| bmp -> png, flat: output size (MB) [extra] | 0.3 | 0.4 (png) | n/a |
| bmp -> png, flat: the png crate's fast level, throughput and size [extra] | - | 762.4 MB/s, 1.7 MB (png fast) | n/a |

## Conclusions

Every pass line passes, the stock rows included since the deflate matcher's payoff threshold moved to eight bytes and the filter trials to every fourth row (flat encode 1915.8 against 615.0 MB/s), and the stock photograph (a real encoder's Paeth rows and profile chunks, 418 MB of pixels) agrees: decode 458.0 against 412.7 MB/s in 5.5 MB against 424.0, encode 232.8 against 176.3 MB/s writing 29.0 MB against 44.2, pixels identical when the crates read our file back. The photo decode went from 358 to 438 MB/s
against the crates' 412, and its memory from 65 MB to 5, when the
conversion stopped holding an image: rows go from the unfilter into a
top-down BMP as they complete, which drops the 61 MB buffer, its
16,000 first-touch page faults, and the separate swizzle pass that the
three-crate reference pipeline cannot avoid. Direct medians of seven
runs put the two at 101 and 107 ms; the block's margin moves with the
machine's load, so the difference to trust is the memory line and the
flat decode, both a third or more ahead.

Two spikes that measured nothing are recorded in `STATE.md` and the
patterns file: the x86-64-v2 baseline and unchecked access in the
inflater. fdeflate is safe Rust; its loop shape (three lookups from one
unchanged buffer per refill) is now ours too, and the checksums run as
interleaved streams and lane sums, but phase timers showed the gap
was the pipeline, not the loop.

The encodes stream too: the BMP reader hands rows to the PNG writer, so a top-down BMP (ours) is never held (3.6 MB on the 418 MB stock image against the crates' 886.8) and a bottom-up one is held once as file bytes (64.8 MB on the photo, where an image copy made it 125). They pass with the same speed margins as before: the photo at 150
against 143 MB/s with a 4% larger output on this incompressible image
(the matcher's payoff-adaptive chain budget, `STATE.md` Tech Debt),
the flat image smaller than the crate's output at 1.6 times its speed.
