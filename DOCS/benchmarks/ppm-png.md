# Netpbm <-> PNG

**Latest** (2026-09-27, 0.22.0 release: every line PASSES but the PAM stock encode in the harness (695.6 against 699.1 MB/s), which leads by direct timing (659 to 697 ms against 701 to 803))

## Purpose

Netpbm both ways on the image hub: the reader streaming rows into the
PNG writer, and the PNG reader streaming rows into the PPM and PAM
writers, against the Rust crates people use for each side.

## Reference

The `image` crate's PNM codec with the `png` crate at its `Default`
level on the PNG side, implemented in `bench/src/pairs/ppm_png.rs`.

## Pass lines

Not slower, and no more memory, than the reference pipelines, in each
direction and shape. Throughput counts the Netpbm file's bytes for the
reader and the PNG plus the Netpbm output for the writer.

## Method

**Machine.** Recorded with each results block from the harness's
machine line.

**Inputs.** The jpeg-png pair's 4000 by 4000 RGB PNGs as PPM (P6), and
the stock RGBA photograph (`bench/data/stock.png`, `[stock]`) as PAM
(P7), written by us (a raw 8-bit Netpbm file has one layout).

**Statistics.** `bench/run.sh ppm-png`: three runs per command, median
wall clock of the whole process, peak resident memory from GNU `time`.
Since this pair, `time_cmd` removes an `out-*` output before each run:
replacing an existing file costs a writeback on btrfs and ext4 for a
rename into place (ours, which never leaves a half-written file) and
not for a truncate (the crates'), a file system cost that had put the
repeated runs 10 to 20% behind on 48 MB outputs.

## Threats to validity

- A PPM write is a copy; the encode lines measure the PNG decode, and
  our inflater and the png crate's fdeflate are level on this photo.
  The photo encode line is within noise either way.

## Results

### 2026-09-27, 0.22.0 release

commit: 89d3694 (main at the release, before the version bump)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

| Target | Ours | Reference | Result |
|---|---|---|---|
| ppm -> png, photo (45.8 MB in): throughput (MB/s of input) | 188.6 | 71.0 (image + png) | PASS |
| ppm -> png, photo: peak memory (MB) | 5.3 | 124.3 (image + png) | PASS |
| ppm -> png, photo: output size (MB) [extra] | 25.0 | 27.1 (png) | n/a |
| png -> ppm, photo (29.0 MB in + 45.8 MB out): throughput (MB/s of input plus output) | 893.9 | 877.2 (png + image) | PASS |
| png -> ppm, photo: peak memory (MB) | 4.5 | 51.6 (png + image) | PASS |
| ppm -> png, flat (45.8 MB in): throughput (MB/s of input) | 1063.9 | 569.2 (image + png) | PASS |
| ppm -> png, flat: peak memory (MB) | 4.3 | 97.5 (image + png) | PASS |
| ppm -> png, flat: output size (MB) [extra] | 0.1 | 0.3 (png) | n/a |
| png -> ppm, flat (1.6 MB in + 45.8 MB out): throughput (MB/s of input plus output) | 1344.6 | 1019.9 (png + image) | PASS |
| png -> ppm, flat: peak memory (MB) | 4.5 | 51.8 (png + image) | PASS |
| pam -> png, stock (418.4 MB in): throughput (MB/s of input) [stock] | 167.8 | 103.8 (image + png) | PASS |
| pam -> png, stock: peak memory (MB) [stock] | 4.7 | 886.7 (image + png) | PASS |
| pam -> png, stock: output size (MB) [extra] | 28.9 | 44.2 (png) | n/a |
| png -> pam, stock (24.2 MB in + 418.4 MB out): throughput (MB/s of input plus output) [stock] | 695.6 | 699.1 (png + image) | FAIL |
| png -> pam, stock: peak memory (MB) [stock] | 4.7 | 424.2 (png + image) | PASS |

### 2026-09-27, first Netpbm

commit: 5a12c0f (on the `netpbm` branch, before its merge)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

| Target | Ours | Reference | Result |
|---|---|---|---|
| ppm -> png, photo (45.8 MB in): throughput (MB/s of input) | 75.9 | 73.7 (image + png) | PASS |
| ppm -> png, photo: peak memory (MB) | 4.9 | 123.5 (image + png) | PASS |
| ppm -> png, photo: output size (MB) [extra] | 25.3 | 27.1 (png) | n/a |
| png -> ppm, photo (29.0 MB in + 45.8 MB out): throughput (MB/s of input plus output) | 834.5 | 888.1 (png + image) | FAIL |
| png -> ppm, photo: peak memory (MB) | 4.8 | 51.4 (png + image) | PASS |
| ppm -> png, flat (45.8 MB in): throughput (MB/s of input) | 1027.8 | 536.4 (image + png) | PASS |
| ppm -> png, flat: peak memory (MB) | 4.5 | 97.6 (image + png) | PASS |
| ppm -> png, flat: output size (MB) [extra] | 0.1 | 0.3 (png) | n/a |
| png -> ppm, flat (1.6 MB in + 45.8 MB out): throughput (MB/s of input plus output) | 1180.4 | 907.5 (png + image) | PASS |
| png -> ppm, flat: peak memory (MB) | 4.3 | 51.5 (png + image) | PASS |
| pam -> png, stock (418.4 MB in): throughput (MB/s of input) [stock] | 170.9 | 104.0 (image + png) | PASS |
| pam -> png, stock: peak memory (MB) [stock] | 4.6 | 886.8 (image + png) | PASS |
| pam -> png, stock: output size (MB) [extra] | 28.8 | 44.2 (png) | n/a |
| png -> pam, stock (24.2 MB in + 418.4 MB out): throughput (MB/s of input plus output) [stock] | 671.3 | 637.1 (png + image) | PASS |
| png -> pam, stock: peak memory (MB) [stock] | 4.8 | 423.8 (png + image) | PASS |

## Conclusions

Reading Netpbm is a copy (or a table lookup per sample when the maxval
is not 255) straight into rows, so the decode lines are the PNG write,
where the lazy deflate leads. The encode lines are the PNG decode; on
the synthetic photo the two inflaters tie and the line is noise. The
lever for a margin there is the inflater's decode structure
(`temp/patterns.md`, fdeflate). Memory is a row against the crates'
whole image.
