# ICO <-> PNG

**Latest** (2026-09-27, 0.23.1 release: 1 line FAILS, the 256-pixel icon (0.22.0 decision))

## Purpose

Icons both ways on the image hub: the largest entry of an icon to PNG,
and an icon of the standard sizes made from a PNG, against the Rust
crate people use for each.

## Reference

The `image` crate: its ICO decoder (the largest entry) with the `png`
crate at its `Default` level, and for icons the same seven standard
sizes ours writes (16 to 256 that fit), each its `thumbnail` of the
source encoded as a PNG entry by its multi-image ICO encoder,
implemented in `bench/src/pairs/ico_png.rs`.

## Pass lines

Not slower, and no more memory, than the reference pipelines. Times are
whole-process wall clock in milliseconds; icon sizes are an `[extra]`
row.

## Method

**Machine.** Recorded with each results block from the harness's
machine line.

**Inputs.** A 256 by 256 picture (the jpeg-png photo scaled with
Lanczos by Pillow) as PNG and as Pillow's icon of it (seven PNG
entries), and the jpeg-png pair's 4000 by 4000 photo and flat PNGs.

**Statistics.** `bench/run.sh ico-png`: three runs per command, median
wall clock of the whole process, peak resident memory from GNU `time`.

## Threats to validity

- The 256-pixel lines run in 5 to 30 ms, where process start and machine
  load are a large share; they changed sign between runs.
- The two sides scale differently (area averaging against the image
  crate's thumbnail filter) and compress differently (our default
  deflate against its fast level), so the icons differ in bytes.

## Results

### 2026-09-27, 0.23.1 release

| Target | Ours | Reference | Result |
|---|---|---|---|
| ico -> png, 256 icon (0.1 MB): wall time (ms) | 10.6 | 19.9 (image + png) | PASS |
| ico -> png, 256 icon: peak memory (MB) | 4.7 | 6.6 (image + png) | PASS |
| png -> ico, icon256 (0.0 MB in): wall time (ms) | 12.1 | 6.2 (png + image) | FAIL |
| png -> ico, icon256: peak memory (MB) | 4.9 | 6.4 (png + image) | PASS |
| png -> ico, icon256: output size (KB) [extra] | 56.8 | 101.3 (image) | n/a |
| png -> ico, jphoto (29.0 MB in): wall time (ms) | 95.4 | 135.8 (png + image) | PASS |
| png -> ico, jphoto: peak memory (MB) | 5.8 | 51.7 (png + image) | PASS |
| png -> ico, jphoto: output size (KB) [extra] | 56.4 | 103.0 (image) | n/a |
| png -> ico, jflat (1.6 MB in): wall time (ms) | 41.1 | 101.0 (png + image) | PASS |
| png -> ico, jflat: peak memory (MB) | 5.9 | 52.2 (png + image) | PASS |
| png -> ico, jflat: output size (KB) [extra] | 108.0 | 153.3 (image) | n/a |

commit: 121f26b (main at the release, before the version bump)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

### 2026-09-27, 0.22.0 release

commit: 89d3694 (main at the release, before the version bump)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

| Target | Ours | Reference | Result |
|---|---|---|---|
| ico -> png, 256 icon (0.1 MB): wall time (ms) | 10.1 | 12.2 (image + png) | PASS |
| ico -> png, 256 icon: peak memory (MB) | 4.8 | 6.8 (image + png) | PASS |
| png -> ico, icon256 (0.0 MB in): wall time (ms) | 24.1 | 20.4 (png + image) | FAIL |
| png -> ico, icon256: peak memory (MB) | 5.1 | 6.6 (png + image) | PASS |
| png -> ico, icon256: output size (KB) [extra] | 56.8 | 101.3 (image) | n/a |
| png -> ico, jphoto (29.0 MB in): wall time (ms) | 94.7 | 132.1 (png + image) | PASS |
| png -> ico, jphoto: peak memory (MB) | 5.7 | 51.5 (png + image) | PASS |
| png -> ico, jphoto: output size (KB) [extra] | 56.4 | 103.0 (image) | n/a |
| png -> ico, jflat (1.6 MB in): wall time (ms) | 41.9 | 88.7 (png + image) | PASS |
| png -> ico, jflat: peak memory (MB) | 5.6 | 52.0 (png + image) | PASS |
| png -> ico, jflat: output size (KB) [extra] | 108.0 | 153.3 (image) | n/a |

### 2026-09-27, first ICO

commit: a0283a5 (on the `ico` branch, before its merge)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

| Target | Ours | Reference | Result |
|---|---|---|---|
| ico -> png, 256 icon (0.1 MB): wall time (ms) | 27.1 | 16.8 (image + png) | FAIL |
| ico -> png, 256 icon: peak memory (MB) | 4.6 | 6.3 (image + png) | PASS |
| png -> ico, icon256 (0.0 MB in): wall time (ms) | 35.4 | 18.7 (png + image) | FAIL |
| png -> ico, icon256: peak memory (MB) | 4.9 | 6.4 (png + image) | PASS |
| png -> ico, icon256: output size (KB) [extra] | 56.8 | 101.3 (image) | n/a |
| png -> ico, jphoto (29.0 MB in): wall time (ms) | 93.5 | 130.2 (png + image) | PASS |
| png -> ico, jphoto: peak memory (MB) | 5.4 | 51.2 (png + image) | PASS |
| png -> ico, jphoto: output size (KB) [extra] | 56.4 | 103.0 (image) | n/a |
| png -> ico, jflat (1.6 MB in): wall time (ms) | 44.8 | 87.4 (png + image) | PASS |
| png -> ico, jflat: peak memory (MB) | 5.8 | 52.0 (png + image) | PASS |
| png -> ico, jflat: output size (KB) [extra] | 108.0 | 153.3 (image) | n/a |

## Conclusions

A large source streams into the one downscale to the largest entry (256
fitted), and the six smaller sizes are scaled from that small image at
the end: one pass over the source, where scaling each size from the
source was seven (300 to 125 ms), and a downscale whose loops take the
pixel width as a constant (125 to 94). Memory is the entries, never the
source. For a 256-pixel source the time is the seven PNG encodes at our
default level: the icon is 44% smaller than the crate's, which uses its
fast level; the owner's rule (smaller where it trades) keeps it.
