# TGA <-> PNG

**Latest** (2026-09-27, 0.24.0 release: every line PASSES)

## Purpose

TGA both ways on the image hub: the reader streaming rows into the PNG
writer, and the PNG reader streaming rows into the TGA writer, against
the Rust crates people use for each side.

## Reference

The `image` crate's TGA codec (run-length encoded both ways) with the
`png` crate at its `Default` level on the PNG side, implemented in
`bench/src/pairs/tga_png.rs`.

## Pass lines

Not slower, and no more memory, than the reference pipelines, in each
direction and shape. Throughput counts decoded pixel bytes for the
reader and input plus pixel bytes for the writer.

## Method

**Machine.** Recorded with each results block from the harness's
machine line.

**Inputs.** The jpeg-png pair's 4000 by 4000 RGB PNGs and the stock
RGBA photograph (`bench/data/stock.png`, `[stock]`), as TGA through
Pillow's defaults: run-length encoded and bottom-up, the layout most
TGA files in the wild have.

**Statistics.** `bench/run.sh tga-png`: three runs per command, median
wall clock of the whole process, peak resident memory from GNU `time`;
outputs are removed before each run.

## Threats to validity

- The flat decode line is within 1% and can flip with load.

## Results

### 2026-09-27, 0.24.0 release

| Target | Ours | Reference | Result |
|---|---|---|---|
| tga -> png, photo (45.8 MB of pixels, 45.9 MB on disk): throughput (MB/s of decoded pixels) | 145.4 | 73.4 (image + png) | PASS |
| tga -> png, photo: peak memory (MB) | 50.8 | 96.3 (image + png) | PASS |
| tga -> png, photo: output size (MB) [extra] | 25.0 | 27.1 (png) | n/a |
| png -> tga, photo (29.0 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 519.8 | 432.9 (png + image) | PASS |
| png -> tga, photo: peak memory (MB) | 4.7 | 97.3 (png + image) | PASS |
| tga -> png, flat (45.8 MB of pixels, 1.0 MB on disk): throughput (MB/s of decoded pixels) | 468.6 | 437.4 (image + png) | PASS |
| tga -> png, flat: peak memory (MB) | 5.4 | 51.8 (image + png) | PASS |
| tga -> png, flat: output size (MB) [extra] | 0.1 | 0.3 (png) | n/a |
| png -> tga, flat (1.6 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 631.9 | 443.5 (png + image) | PASS |
| png -> tga, flat: peak memory (MB) | 5.0 | 97.4 (png + image) | PASS |
| tga -> png, stock (418.4 MB of pixels, 151.1 MB on disk): throughput (MB/s of decoded pixels) [stock] | 156.9 | 98.8 (image + png) | PASS |
| tga -> png, stock: peak memory (MB) [stock] | 156.2 | 574.2 (image + png) | PASS |
| tga -> png, stock: output size (MB) [extra] | 28.9 | 44.2 (png) | n/a |
| png -> tga, stock (24.2 MB in + 418.4 MB of pixels): throughput (MB/s of input plus pixels) [stock] | 400.7 | 326.7 (png + image) | PASS |
| png -> tga, stock: peak memory (MB) [stock] | 5.1 | 842.7 (png + image) | PASS |

commit: e8088c4 (main at the release, before the version bump)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

### 2026-09-27, 0.23.1 release

| Target | Ours | Reference | Result |
|---|---|---|---|
| tga -> png, photo (45.8 MB of pixels, 45.9 MB on disk): throughput (MB/s of decoded pixels) | 142.7 | 73.3 (image + png) | PASS |
| tga -> png, photo: peak memory (MB) | 51.0 | 95.9 (image + png) | PASS |
| tga -> png, photo: output size (MB) [extra] | 25.0 | 27.1 (png) | n/a |
| png -> tga, photo (29.0 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 495.6 | 428.0 (png + image) | PASS |
| png -> tga, photo: peak memory (MB) | 4.8 | 97.0 (png + image) | PASS |
| tga -> png, flat (45.8 MB of pixels, 1.0 MB on disk): throughput (MB/s of decoded pixels) | 457.2 | 450.8 (image + png) | PASS |
| tga -> png, flat: peak memory (MB) | 5.2 | 51.7 (image + png) | PASS |
| tga -> png, flat: output size (MB) [extra] | 0.1 | 0.3 (png) | n/a |
| png -> tga, flat (1.6 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 638.2 | 439.5 (png + image) | PASS |
| png -> tga, flat: peak memory (MB) | 4.7 | 97.2 (png + image) | PASS |
| tga -> png, stock (418.4 MB of pixels, 151.1 MB on disk): throughput (MB/s of decoded pixels) [stock] | 158.5 | 99.0 (image + png) | PASS |
| tga -> png, stock: peak memory (MB) [stock] | 155.9 | 573.7 (image + png) | PASS |
| tga -> png, stock: output size (MB) [extra] | 28.9 | 44.2 (png) | n/a |
| png -> tga, stock (24.2 MB in + 418.4 MB of pixels): throughput (MB/s of input plus pixels) [stock] | 402.5 | 327.4 (png + image) | PASS |
| png -> tga, stock: peak memory (MB) [stock] | 4.7 | 842.5 (png + image) | PASS |

commit: 121f26b (main at the release, before the version bump)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

### 2026-09-27, 0.22.0 release

commit: 89d3694 (main at the release, before the version bump)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

| Target | Ours | Reference | Result |
|---|---|---|---|
| tga -> png, photo (45.8 MB of pixels, 45.9 MB on disk): throughput (MB/s of decoded pixels) | 143.0 | 70.2 (image + png) | PASS |
| tga -> png, photo: peak memory (MB) | 50.8 | 96.1 (image + png) | PASS |
| tga -> png, photo: output size (MB) [extra] | 25.0 | 27.1 (png) | n/a |
| png -> tga, photo (29.0 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 525.1 | 421.1 (png + image) | PASS |
| png -> tga, photo: peak memory (MB) | 4.8 | 97.2 (png + image) | PASS |
| tga -> png, flat (45.8 MB of pixels, 1.0 MB on disk): throughput (MB/s of decoded pixels) | 458.8 | 454.0 (image + png) | PASS |
| tga -> png, flat: peak memory (MB) | 5.4 | 51.8 (image + png) | PASS |
| tga -> png, flat: output size (MB) [extra] | 0.1 | 0.3 (png) | n/a |
| png -> tga, flat (1.6 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 584.3 | 414.6 (png + image) | PASS |
| png -> tga, flat: peak memory (MB) | 4.3 | 97.3 (png + image) | PASS |
| tga -> png, stock (418.4 MB of pixels, 151.1 MB on disk): throughput (MB/s of decoded pixels) [stock] | 153.2 | 99.1 (image + png) | PASS |
| tga -> png, stock: peak memory (MB) [stock] | 155.9 | 574.2 (image + png) | PASS |
| tga -> png, stock: output size (MB) [extra] | 28.9 | 44.2 (png) | n/a |
| png -> tga, stock (24.2 MB in + 418.4 MB of pixels): throughput (MB/s of input plus pixels) [stock] | 402.3 | 336.0 (png + image) | PASS |
| png -> tga, stock: peak memory (MB) [stock] | 5.0 | 842.2 (png + image) | PASS |

### 2026-09-27, first TGA

commit: fbeb82c (on the `tga` branch, before its merge)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

| Target | Ours | Reference | Result |
|---|---|---|---|
| tga -> png, photo (45.8 MB of pixels, 45.9 MB on disk): throughput (MB/s of decoded pixels) | 145.0 | 73.0 (image + png) | PASS |
| tga -> png, photo: peak memory (MB) | 51.0 | 95.6 (image + png) | PASS |
| tga -> png, photo: output size (MB) [extra] | 25.0 | 27.1 (png) | n/a |
| png -> tga, photo (29.0 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 517.7 | 428.8 (png + image) | PASS |
| png -> tga, photo: peak memory (MB) | 4.7 | 96.8 (png + image) | PASS |
| tga -> png, flat (45.8 MB of pixels, 1.0 MB on disk): throughput (MB/s of decoded pixels) | 468.4 | 466.1 (image + png) | PASS |
| tga -> png, flat: peak memory (MB) | 5.3 | 51.7 (image + png) | PASS |
| tga -> png, flat: output size (MB) [extra] | 0.1 | 0.3 (png) | n/a |
| png -> tga, flat (1.6 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 607.1 | 425.8 (png + image) | PASS |
| png -> tga, flat: peak memory (MB) | 4.6 | 97.3 (png + image) | PASS |
| tga -> png, stock (418.4 MB of pixels, 151.1 MB on disk): throughput (MB/s of decoded pixels) [stock] | 159.6 | 99.0 (image + png) | PASS |
| tga -> png, stock: peak memory (MB) [stock] | 155.7 | 573.6 (image + png) | PASS |
| tga -> png, stock: output size (MB) [extra] | 28.9 | 44.2 (png) | n/a |
| png -> tga, stock (24.2 MB in + 418.4 MB of pixels): throughput (MB/s of input plus pixels) [stock] | 394.8 | 333.1 (png + image) | PASS |
| png -> tga, stock: peak memory (MB) [stock] | 4.4 | 842.3 (png + image) | PASS |

## Conclusions

A bottom-up TGA's first row is the image's last, so something is held.
The reader holds the file rather than the pixels: raw rows are read
from their fixed offsets last first, and RLE data gets one pass over
its packet headers that records where each row starts, then the rows
are decoded last first. A flat image's RLE is a fiftieth of its
pixels (5 MB peak where holding pixels took 50). Packets are copied a
span at a time and runs repeated with the pixel width as a constant.
The writer is top-down, so it streams at a row. The photo decode's
lead is the PNG write, since deflate stopped searching on noise.
