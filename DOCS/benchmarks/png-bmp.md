# PNG <-> BMP

**Latest** (2026-09-26, `png -> bmp` streams rows into a top-down BMP: png -> bmp 438.2 MB/s of decoded pixels on the photo at 5.4 MB peak, 1011.0 on the flat image; bmp -> png 149.9 and 999.9 MB/s of input plus pixels at 125 MB peak; every line PASSES against the png and image crates)

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
- The machine ran a game client's helper process at about half a core
  during the session; runs moved by up to 10%, and the recorded block
  is one run of three-run medians.

## Results

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

Every pass line passes. The photo decode went from 358 to 438 MB/s
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

The encodes pass with the same margins as before: the photo at 150
against 143 MB/s with a 4% larger output on this incompressible image
(the matcher's payoff-adaptive chain budget, `STATE.md` Tech Debt),
the flat image smaller than the crate's output at 1.6 times its speed.
