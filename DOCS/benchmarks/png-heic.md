# PNG -> HEIC

**Latest** (2026-10-10, `heic-write` branch, `--effort` and held-out
photographs: against libheif's `heif-enc` (x265), files smaller at equal
quality on every measure on 12 photographs of 4032 pixels, 32 held-out
photographs, and Kodak's PSNR and SSIM, but not Kodak's SSIMULACRA2
(+1.0%, the one failing line); `--effort fast` at x265's speed on the
12 photographs, `balanced` (the default) 17% slower and `max` 1.9 times
slower; a fifth to a quarter of its memory; 12 to 17% smaller than
Apple's hardware encoder (`sips`, context), which is far faster)

## Purpose

HEIC write: an HEVC intra encoder and HEIF writer written here, from the
PNG reader's rows. The work is the encoder's search: modes, transform
trees, and levels chosen by rate and distortion, on every thread by CTB
rows; `--effort` sets how much of that search runs.

## Reference

libheif's `heif-enc` (libheif 1.20.2 with x265 4.1, its defaults: the slow
preset tuned for SSIM, quality 50), the conventional tool; its veryslow
preset, measured once on every set below, lands within 0.2% of the
default. macOS `sips` is recorded as context: it encodes on Apple's
hardware HEVC block, so its speed is not a software encoder's.

## Pass lines

At the default effort, files no larger than `heif-enc`'s at equal quality
(BD-rate at or below 0) by each measure on each set, and no more memory.
Speed: `--effort fast` not slower than `heif-enc` on the photographs, and
the default effort not slower on the other inputs; balanced and max on the
photographs are recorded as context (they buy size with time).

## Method

**Machine.** Recorded with each results block from the harness's machine
line.

**Speed and memory.** The jpeg-png pair's 4000 by 4000 RGB photo and flat
images, and when `/System/Library/Desktop Pictures/Sonoma.heic` exists its
6016 by 6016 picture decoded to PNG (marked `[stock]`), each at quality 50.
Three runs per command, median wall clock of the whole process (PNG decode
included), peak resident memory from GNU `time`. These inputs flatter the
encoder (synthetic noise, flat color, a smooth wallpaper): the photograph
rows below are the realistic case.

**Photographs.** Two sets, fetched once into `bench/data`: the 24 Kodak
photographs (768 by 512) and 12 of Wikimedia Commons' featured pictures
(landscapes, people, two paintings, architecture, a plant, a bird, a
cityscape, boats), checked by hash and scaled by `sips` to 4032 pixels
across, a phone's size. Each encoder codes each photograph at six
qualities (30 to 80; `sips` at its own percent scale, 35 to 93, which
spans the same sizes). Every file is decoded by our decoder (bit-exact
with libheif's planes) and scored against its source: PSNR of luma
(BT.601 weights), SSIM of luma (Gaussian window 11, sigma 1.5), PSNR of
RGB, and SSIMULACRA2 (the `ssimulacra2` tool, when installed). Each
photograph's curve of size against score is compared with the
reference's by BD-rate (the mean log-size difference over the score
range both cover, piecewise linear), and the set's photographs averaged.
The throughput rows are the sum of one run per photograph at quality 50;
the 12 photographs are timed at each effort.

**Statistics.** `bench/run.sh png-heic`; `bench heic-png quality` and
`bench heic-png bd` do the scoring.

## Held-out evaluation

The effort levels were tuned on half of CLIC 2020's professional
validation set (21 photographs, about 2048 by 1365) and then measured once
on 32 photographs never used for tuning (the other 20 and the 12 above),
with the same scoring. Against `heif-enc`:

| Set | Effort | PSNR of luma | SSIM | PSNR of RGB | SSIMULACRA2 |
|---|---|---|---|---|---|
| CLIC, all 41 | max | -3.9% | -3.0% | -4.0% | -3.3% |
| 32 held out | balanced | -2.2% | -1.1% | -2.5% | -1.3% |
| 32 held out | fast | -1.9% | -0.7% | -2.4% | -1.2% |
| 5 pages of text | max | -9.6% | -4.6% | -9.6% | -11.8% |
| 3 wallpapers, 2 icons | max | -0.9% | -0.3% | -2.0% | -5.7% |
| 6 photographs in gray | max | -0.8% | +0.1% | -0.8% | +0.6% |

Against max on the held-out photographs, balanced is 0.5 to 0.9% larger
and fast 0.8 to 1.6%. Every file of these runs decodes in `sips` (558)
and libheif (93 checked). Two of five graphics lose: sharp-edged icons,
by up to 10% at equal RGB PSNR.

## Threats to validity

- Kodak's SSIMULACRA2 line fails (+1.0%): on small, grainy film scans
  x265's tuning for perceived quality wins where our search, which
  minimizes squared error, does not; it is not the 16-pixel CTBs (32-pixel
  ones lose there too, +0.8%). The larger and held-out photographs do not
  show it.
- The SSIM margins are thin on the 12 photographs (-0.4%) though wider on
  the 32 held out (-1.1%).
- `sips` scales the 12 photographs; another macOS might scale them
  differently. Their sources are pinned by hash.
- Our encoder works on every core, `heif-enc` on about two and a half on
  Kodak and seven on the photographs; on a machine busy with other work
  ours loses more of its speed.
- x265 has hand-written NEON for its transforms, SATD, and quantization;
  ours is plain Rust.
- BD-rate on six points per curve, piecewise linear rather than the cubic
  fit: differences of a tenth of a percent are within what the method
  resolves.
- Peak resident memory counts what the allocator keeps of freed blocks.

## Results

### 2026-10-10, `--effort`, photographs of 4032 pixels, SSIMULACRA2

| Target | Ours | Reference (heif-enc) | Context (sips) | Result |
|---|---|---|---|---|
| png -> heic, jphoto (45.8 MB of pixels, quality 50): throughput (MB/s of pixels) | 38.8 | 14.8 | 125.2 | PASS |
| png -> heic, jphoto: peak memory (MB) | 71.8 | 316.4 | 203.8 | PASS |
| png -> heic, jflat (45.8 MB of pixels, quality 50): throughput (MB/s of pixels) | 179.4 | 64.7 | 174.6 | PASS |
| png -> heic, jflat: peak memory (MB) | 71.4 | 300.8 | 173.3 | PASS |
| png -> heic, hstock (103.5 MB of pixels, quality 50): throughput (MB/s of pixels) [stock] | 76.9 | 30.6 | 190.4 | PASS |
| png -> heic, hstock: peak memory (MB) [stock] | 138.6 | 631.5 | 389.8 | PASS |
| png -> heic, Kodak (24 photographs): BD-rate at equal PSNR of luma (negative: ours smaller) | -1.4% | 0% (itself) | ours -12.2% | PASS |
| png -> heic, Kodak (24 photographs): BD-rate at equal SSIM (negative: ours smaller) | -0.3% | 0% (itself) | ours -13.3% | PASS |
| png -> heic, Kodak (24 photographs): BD-rate at equal PSNR of RGB (negative: ours smaller) | -0.3% | 0% (itself) | ours -12.0% | PASS |
| png -> heic, Kodak (24 photographs): BD-rate at equal SSIMULACRA2 (negative: ours smaller) | +1.0% | 0% (itself) | ours -13.2% | FAIL |
| png -> heic, Kodak (24 photographs of 0.4 megapixels, quality 50): throughput (MB/s of pixels) | 9.0 | 6.8 | 11.3 | PASS |
| png -> heic, 12 photographs of 4032 pixels: BD-rate at equal PSNR of luma (negative: ours smaller) | -1.8% | 0% (itself) | ours -11.8% | PASS |
| png -> heic, 12 photographs of 4032 pixels: BD-rate at equal SSIM (negative: ours smaller) | -0.4% | 0% (itself) | ours -13.6% | PASS |
| png -> heic, 12 photographs of 4032 pixels: BD-rate at equal PSNR of RGB (negative: ours smaller) | -2.0% | 0% (itself) | ours -14.7% | PASS |
| png -> heic, 12 photographs of 4032 pixels: BD-rate at equal SSIMULACRA2 (negative: ours smaller) | -0.7% | 0% (itself) | ours -16.9% | PASS |
| png -> heic, 12 photographs of 4032 pixels (quality 50, --effort fast): throughput (MB/s of pixels) | 20.2 | 20.1 | n/a | PASS |
| png -> heic, 12 photographs of 4032 pixels (quality 50, --effort balanced): throughput (MB/s of pixels) | 17.2 | 20.1 | n/a | context |
| png -> heic, 12 photographs of 4032 pixels (quality 50, --effort max): throughput (MB/s of pixels) | 10.7 | 20.1 | n/a | context |

commit: 1b0225a (plus the `heic-write` working tree)
machine: Darwin 24.5.0 arm64, 10 cpus, Apple M1 Max

### 2026-10-09, first writer (synthetic and Kodak only; its speed lines
flattered: on real photographs it was half x265's speed)

| Target | Ours | Reference (heif-enc) | Context (sips) | Result |
|---|---|---|---|---|
| png -> heic, jphoto (45.8 MB of pixels, quality 50): throughput (MB/s of pixels) | 8.2 | 6.6 | 123.0 | PASS |
| png -> heic, jphoto: peak memory (MB) | 71.8 | 314.2 | 203.9 | PASS |
| png -> heic, jflat (45.8 MB of pixels, quality 50): throughput (MB/s of pixels) | 31.3 | 24.5 | 173.4 | PASS |
| png -> heic, jflat: peak memory (MB) | 75.1 | 300.4 | 173.7 | PASS |
| png -> heic, hstock (103.5 MB of pixels, quality 50): throughput (MB/s of pixels) [stock] | 17.7 | 11.4 | 191.8 | PASS |
| png -> heic, hstock: peak memory (MB) [stock] | 141.2 | 633.9 | 389.4 | PASS |
| png -> heic, Kodak (24 photographs): BD-rate at equal PSNR of luma (negative: ours smaller) | -1.4% | 0% (itself) | ours -12.2% | PASS |
| png -> heic, Kodak (24 photographs): BD-rate at equal SSIM (negative: ours smaller) | -0.3% | 0% (itself) | ours -13.3% | PASS |
| png -> heic, Kodak (24 photographs): BD-rate at equal PSNR of RGB (negative: ours smaller) | -0.3% | 0% (itself) | ours -12.0% | PASS |
| png -> heic, Kodak (24 photographs of 0.4 megapixels, quality 50): throughput (MB/s of pixels) | 4.4 | 3.4 | 9.0 | PASS |

commit: fea6ffc (plus the `heic-write` working tree)
machine: Darwin 24.5.0 arm64, 10 cpus, Apple M1 Max
