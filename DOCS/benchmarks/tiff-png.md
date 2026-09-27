# TIFF <-> PNG

**Latest** (2026-09-27, 0.22.0 release: every line PASSES)

## Purpose

TIFF both ways on the image hub: the reader streaming the first page's
rows into the PNG writer, and the PNG reader streaming rows into the
TIFF writer, against the Rust crates people use for each side.

## Reference

The `image` crate's TIFF decoder (the `tiff` crate, its memory limit
lifted, which refuses the stock photograph otherwise) with the `png`
crate at its `Default` level; for TIFF output the `tiff` crate with
deflate at its balanced level and the horizontal predictor, as ours
writes. Implemented in `bench/src/pairs/tiff_png.rs`.

## Pass lines

Not slower, and no more memory, than the reference pipelines, in each
direction and shape. Throughput counts decoded pixel bytes for the
reader and input plus pixel bytes for the writer.

## Method

**Machine.** Recorded with each results block from the harness's
machine line.

**Inputs.** The jpeg-png pair's 4000 by 4000 RGB PNGs and the stock
RGBA photograph (`bench/data/stock.png`, `[stock]`), as TIFF written by
ImageMagick with LZW and the horizontal predictor (the most common
compressed TIFF).

**Statistics.** `bench/run.sh tiff-png`: three runs per command, median
wall clock of the whole process, peak resident memory from GNU `time`;
outputs are removed before each run.

## Threats to validity

- Real TIFFs vary more than these inputs (tiles, planar, 16-bit, CMYK);
  those layouts are proven by the oracle, not timed here.

## Results

### 2026-09-27, 0.22.0 release

commit: 89d3694 (main at the release, before the version bump)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

| Target | Ours | Reference | Result |
|---|---|---|---|
| tiff -> png, photo (45.8 MB of pixels, 32.8 MB on disk): throughput (MB/s of decoded pixels) | 93.3 | 53.8 (image + png) | PASS |
| tiff -> png, photo: peak memory (MB) | 38.3 | 129.6 (image + png) | PASS |
| tiff -> png, photo: output size (MB) [extra] | 25.0 | 27.1 (png) | n/a |
| png -> tiff, photo (29.0 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 272.3 | 113.3 (png + image) | PASS |
| png -> tiff, photo: peak memory (MB) | 32.0 | 52.8 (png + image) | PASS |
| tiff -> png, flat (45.8 MB of pixels, 0.7 MB on disk): throughput (MB/s of decoded pixels) | 379.3 | 302.7 (image + png) | PASS |
| tiff -> png, flat: peak memory (MB) | 6.3 | 97.3 (image + png) | PASS |
| tiff -> png, flat: output size (MB) [extra] | 0.1 | 0.3 (png) | n/a |
| png -> tiff, flat (1.6 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 809.5 | 670.6 (png + image) | PASS |
| png -> tiff, flat: peak memory (MB) | 5.3 | 53.5 (png + image) | PASS |
| tiff -> png, stock (418.4 MB of pixels, 44.7 MB on disk): throughput (MB/s of decoded pixels) [stock] | 117.7 | 89.8 (image + png) | PASS |
| tiff -> png, stock: peak memory (MB) [stock] | 50.0 | 886.5 (image + png) | PASS |
| tiff -> png, stock: output size (MB) [extra] | 28.9 | 44.2 (png) | n/a |
| png -> tiff, stock (24.2 MB in + 418.4 MB of pixels): throughput (MB/s of input plus pixels) [stock] | 123.8 | 104.9 (png + image) | PASS |
| png -> tiff, stock: peak memory (MB) [stock] | 59.1 | 425.9 (png + image) | PASS |

### 2026-09-27, first TIFF

commit: 2bb48ac (on the `tiff` branch, before its merge)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

| Target | Ours | Reference | Result |
|---|---|---|---|
| tiff -> png, photo (45.8 MB of pixels, 32.8 MB on disk): throughput (MB/s of decoded pixels) | 93.4 | 55.8 (image + png) | PASS |
| tiff -> png, photo: peak memory (MB) | 38.4 | 129.3 (image + png) | PASS |
| tiff -> png, photo: output size (MB) [extra] | 25.0 | 27.1 (png) | n/a |
| png -> tiff, photo (29.0 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 277.6 | 117.0 (png + image) | PASS |
| png -> tiff, photo: peak memory (MB) | 32.3 | 53.0 (png + image) | PASS |
| tiff -> png, flat (45.8 MB of pixels, 0.7 MB on disk): throughput (MB/s of decoded pixels) | 393.9 | 307.0 (image + png) | PASS |
| tiff -> png, flat: peak memory (MB) | 6.1 | 97.3 (image + png) | PASS |
| tiff -> png, flat: output size (MB) [extra] | 0.1 | 0.3 (png) | n/a |
| png -> tiff, flat (1.6 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 796.6 | 650.0 (png + image) | PASS |
| png -> tiff, flat: peak memory (MB) | 5.4 | 53.4 (png + image) | PASS |
| tiff -> png, stock (418.4 MB of pixels, 44.7 MB on disk): throughput (MB/s of decoded pixels) [stock] | 119.6 | 89.7 (image + png) | PASS |
| tiff -> png, stock: peak memory (MB) [stock] | 49.7 | 886.4 (image + png) | PASS |
| tiff -> png, stock: output size (MB) [extra] | 28.9 | 44.2 (png) | n/a |
| png -> tiff, stock (24.2 MB in + 418.4 MB of pixels): throughput (MB/s of input plus pixels) [stock] | 126.8 | 104.7 (png + image) | PASS |
| png -> tiff, stock: peak memory (MB) [stock] | 58.9 | 425.8 (png + image) | PASS |

## Conclusions

A TIFF's strips can lie anywhere in the file, so the reader holds the
file (38 MB for the photo's) and decodes a band of rows at a time, a
strip or a row of tiles, straight into the sink; the crates hold the
decoded image (129 to 886 MB). The writer holds the compressed strips
until the last row, because their offsets go in the directory at the
front. Encode's margin is the lazy deflate; the horizontal predictor
runs forward from the source row (in place and backward it did not
vectorize and was a quarter of the flat encode).
