# PNG -> HEIC

**Latest** (2026-10-09, `heic-write` branch, first HEIC writer: every line
PASSES against libheif's `heif-enc` (x265): files 1.4% smaller at equal
luma PSNR and 0.3% at equal SSIM and RGB PSNR on the Kodak photographs,
faster on every input, at a fifth to a quarter of its memory; 12 to 13%
smaller than Apple's hardware encoder (`sips`, context), which is far
faster)

## Purpose

HEIC write: an HEVC intra encoder and HEIF writer written here, from the
PNG reader's rows. The work is the encoder's search: modes, transform
trees, and levels chosen by rate and distortion, on every thread by CTB
rows.

## Reference

libheif's `heif-enc` (libheif 1.20.2 with x265 4.1, its defaults: the slow
preset tuned for SSIM, quality 50), the conventional tool. macOS `sips` is
recorded as context: it encodes on Apple's hardware HEVC block, so its
speed is not a software encoder's.

## Pass lines

Not slower, and no more memory, than `heif-enc` on each input; files no
larger than its at equal quality (BD-rate at or below 0) by each measure.

## Method

**Machine.** Recorded with each results block from the harness's machine
line.

**Speed and memory.** The jpeg-png pair's 4000 by 4000 RGB photo and flat
images, and when `/System/Library/Desktop Pictures/Sonoma.heic` exists its
6016 by 6016 picture decoded to PNG (marked `[stock]`), each at quality 50.
Three runs per command, median wall clock of the whole process (PNG decode
included), peak resident memory from GNU `time`.

**Compression.** The 24 Kodak photographs (768 by 512, fetched once into
`bench/data/kodak`), each encoder at six qualities (30 to 80; `sips` at its
own percent scale, 35 to 93, which spans the same sizes). Every file is
decoded by our decoder (bit-exact with libheif's planes) and scored against
its source: PSNR of luma (BT.601 weights), SSIM of luma (Gaussian window
11, sigma 1.5), and PSNR of RGB. Each image's curve of size against score
is compared with the reference's by BD-rate (the mean log-size difference
over the score range both cover, piecewise linear), and the 24 averaged.
The Kodak throughput row is the sum of one run per image at quality 50.

**Statistics.** `bench/run.sh png-heic`; `bench heic-png quality` and
`bench heic-png bd` do the scoring.

## Threats to validity

- The large inputs: the photo and flat images are synthetic and the stock
  picture was itself a HEIC; compression is measured only on Kodak, the
  usual photographic set, which is small and old (film scans).
- Small pictures (the Kodak photographs among them) are coded with 16-pixel
  CTBs, large ones with 32 (`DOCS/formats/heic.md`), so the compression
  rows are the small pictures' settings; on the large inputs 32-pixel CTBs
  code 9 to 24% smaller than 16-pixel ones would.
- Our encoder works on every core; `heif-enc` uses about two and a half
  on Kodak. On a machine busy with other work ours loses more of its
  speed: back to back on a quiet machine the 24 Kodak photographs took 3.2
  seconds against 3.9 to 4.3, and in a run during a burst of background
  work 6.9 against 5.5. The rows below were recorded with the machine's
  load average falling from 42 to 9.
- x265 has hand-written NEON for its transforms, SATD, and quantization;
  ours is plain Rust and retires 2.1 times its instructions on Kodak.
- BD-rate on six points per curve, piecewise linear rather than the cubic
  fit, over 24 small images: differences of a tenth of a percent are
  within what the method resolves.
- Peak resident memory counts what the allocator keeps of freed blocks.

## Results

### 2026-10-09, first writer

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
