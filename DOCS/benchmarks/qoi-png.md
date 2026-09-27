# QOI <-> PNG

**Latest** (2026-09-27, first QOI: every line PASSES; decode 65.3 against 64.7 MB/s on the photo (the PNG write is most of it, and ours is 7% smaller), 1241 against 617 on the flat image, 158.8 against 102.6 on the stock photo; encode 423.5 against 397.0, 1230.5 against 1191.2, and 583.8 against 582.0 (a tie); 4 to 5 MB of memory everywhere against 52 to 524)

## Purpose

QOI both ways on the image hub: the QOI reader streaming rows into the
PNG writer, and the PNG reader streaming rows into the QOI writer,
against the Rust crates people use for each side.

## Reference

The `qoi` crate (the one the `image` crate uses) with the `png` crate
at its `Default` level on the PNG side, implemented in
`bench/src/pairs/qoi_png.rs`. The qoi crate's default encoder is not
the reference algorithm (it writes an index where a run of one would
go); ours writes the bytes `qoi.h` writes.

## Pass lines

Not slower, and no more memory, than the reference pipelines, in each
direction and shape. Throughput counts decoded pixel bytes for the
reader and input plus pixel bytes for the writer; the PNG output size
is an `[extra]` row.

## Method

**Machine.** Recorded with each results block from the harness's
machine line.

**Inputs.** The jpeg-png pair's 4000 by 4000 RGB PNGs (a gradient with
independent per-channel noise, and flat blocks), and Pillow's QOI
files of the same pixels.

**Stock image.** When `bench/data/stock.png` (or `$SUBLIME_STOCK_PNG`)
exists, its lines run too, marked `[stock]`: an 11220 by 9775 RGBA
photograph.

**Statistics.** `bench/run.sh qoi-png`: three runs per command, median
wall clock of the whole process, peak resident memory from GNU `time`.

## Threats to validity

- The photo is synthetic noise; its QOI is nearly as large as its
  pixels, and its PNG write dominates the decode line on both sides.
- Two margins are thin (the photo decode and the stock encode, within
  1%) and can flip with machine load.

## Results

### 2026-09-27, first QOI

commit: 0b53cae (on the `qoi` branch, before its merge)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

| Target | Ours | Reference | Result |
|---|---|---|---|
| qoi -> png, photo (45.8 MB of pixels, 45.1 MB on disk): throughput (MB/s of decoded pixels) | 65.3 | 64.7 (qoi + png) | PASS |
| qoi -> png, photo: peak memory (MB) | 4.2 | 123.0 (qoi + png) | PASS |
| qoi -> png, photo: output size (MB) [extra] | 25.3 | 27.1 (png) | n/a |
| png -> qoi, photo (29.0 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 423.5 | 397.0 (png + qoi) | PASS |
| png -> qoi, photo: peak memory (MB) | 4.4 | 95.8 (png + qoi) | PASS |
| qoi -> png, flat (45.8 MB of pixels, 0.7 MB on disk): throughput (MB/s of decoded pixels) | 1241.1 | 616.8 (qoi + png) | PASS |
| qoi -> png, flat: peak memory (MB) | 4.5 | 51.9 (qoi + png) | PASS |
| qoi -> png, flat: output size (MB) [extra] | 0.1 | 0.3 (png) | n/a |
| png -> qoi, flat (1.6 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 1230.5 | 1191.2 (png + qoi) | PASS |
| png -> qoi, flat: peak memory (MB) | 4.3 | 51.6 (png + qoi) | PASS |
| qoi -> png, stock (418.4 MB of pixels, 55.8 MB on disk): throughput (MB/s of decoded pixels) [stock] | 158.8 | 102.6 (qoi + png) | PASS |
| qoi -> png, stock: peak memory (MB) [stock] | 4.7 | 523.6 (qoi + png) | PASS |
| qoi -> png, stock: output size (MB) [extra] | 28.8 | 44.2 (png) | n/a |
| png -> qoi, stock (24.2 MB in + 418.4 MB of pixels): throughput (MB/s of input plus pixels) [stock] | 583.8 | 582.0 (png + qoi) | PASS |
| png -> qoi, stock: peak memory (MB) [stock] | 4.8 | 479.7 (png + qoi) | PASS |

## Conclusions

QOI is simple enough that the two sides do the same work; the margins
came from loop shape. The decoder's chunk loop is generic over the
channel count, so a pixel's copy into the row is a fixed-size store
(a runtime length made it a call: 158 to 109 ms on the photo, 556 to
404 on the stock image); the read position lives in locals, and the
buffer is topped up only when a chunk could straddle its end. The
encoder keeps its state in locals for each row, writes into a scratch
slice sized once, hashes in one multiply (the qoi crate's form), and
closes a final run once per row. Memory is the streaming win: a row,
not the image.
