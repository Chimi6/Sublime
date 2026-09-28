# PNG <-> PDF

**Latest** (2026-09-27, 0.24.0 release: every line PASSES)

## Purpose

Images to PDF and back on the image hub: the PNG reader streaming rows
into the PDF writer (a page at the image's own size, Flate with PNG
predictors, alpha as a soft mask), and the PDF reader streaming the
page's largest image into the PNG writer, against the Rust crates
people use for each side.

## Reference

PNG to PDF: the `png` crate decoding and printpdf 0.8 building the
page, set lossless (Flate, no resize, no JPEG re-encode, no gray
detection); its defaults re-encode color images as JPEG and shrink any
image over 2 MB, which is not the same job. PDF to PNG: lopdf loading
the file and finding the page's image XObjects, the largest decoded
with its Flate and predictor and its soft mask joined as alpha, then
the `png` crate at its `Default` level. Implemented in
`bench/src/pairs/pdf.rs`.

## Pass lines

Not slower, and no more memory, than the reference pipelines, in each
direction and shape. Throughput counts decoded pixel bytes for the
reader and input plus pixel bytes for the writer.

## Method

**Machine.** Recorded with each results block from the harness's
machine line.

**Inputs.** The jpeg-png pair's 4000 by 4000 RGB PNGs and the stock
RGBA photograph (`bench/data/stock.png`, `[stock]`); the PDF inputs
are ImageMagick's (`-compress Zip`, one image per page, alpha as a
soft mask).

**Statistics.** `bench/run.sh png-pdf`: three runs per command, median
wall clock of the whole process, peak resident memory from GNU `time`;
outputs are removed before each run.

## Threats to validity

- The PDF inputs hold one image per page, as image-to-PDF tools write
  them. Scanned and mixed PDFs (JPEG, CCITT, JBIG2, several images a
  page) are proven by the fixtures, not timed here.
- The reader holds the whole file (it needs the cross-reference table
  at the end); peak memory is about the file's size plus rows.

## Results

### 2026-09-27, 0.24.0 release

| Target | Ours | Reference | Result |
|---|---|---|---|
| pdf -> png, photo (45.8 MB of pixels, 40.7 MB on disk): throughput (MB/s of decoded pixels) | 123.8 | 62.8 (lopdf + png) | PASS |
| pdf -> png, photo: peak memory (MB) | 45.8 | 92.4 (lopdf + png) | PASS |
| pdf -> png, photo: output size (MB) [extra] | 25.0 | 27.1 (png) | n/a |
| png -> pdf, photo (29.0 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 258.6 | 81.6 (png + printpdf) | PASS |
| png -> pdf, photo: peak memory (MB) | 5.6 | 227.4 (png + printpdf) | PASS |
| png -> pdf, photo: output size (MB) [extra] | 25.0 | 37.9 (printpdf) | n/a |
| pdf -> png, flat (45.8 MB of pixels, 0.4 MB on disk): throughput (MB/s of decoded pixels) | 1350.5 | 687.4 (lopdf + png) | PASS |
| pdf -> png, flat: peak memory (MB) | 5.1 | 54.8 (lopdf + png) | PASS |
| pdf -> png, flat: output size (MB) [extra] | 0.1 | 0.3 (png) | n/a |
| png -> pdf, flat (1.6 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 971.7 | 260.6 (png + printpdf) | PASS |
| png -> pdf, flat: peak memory (MB) | 5.3 | 190.1 (png + printpdf) | PASS |
| png -> pdf, flat: output size (MB) [extra] | 0.1 | 0.2 (printpdf) | n/a |
| pdf -> png, stock (418.4 MB of pixels, 93.7 MB on disk): throughput (MB/s of decoded pixels) [stock] | 130.7 | 88.9 (lopdf + png) | PASS |
| pdf -> png, stock: peak memory (MB) [stock] | 99.2 | 890.3 (lopdf + png) | PASS |
| pdf -> png, stock: output size (MB) [extra] | 28.9 | 44.2 (png) | n/a |
| png -> pdf, stock (24.2 MB in + 418.4 MB of pixels): throughput (MB/s of input plus pixels) [stock] | 161.3 | 111.7 (png + printpdf) | PASS |
| png -> pdf, stock: peak memory (MB) [stock] | 7.1 | 1679.7 (png + printpdf) | PASS |
| png -> pdf, stock: output size (MB) [extra] | 26.3 | 88.7 (printpdf) | n/a |

commit: e8088c4 (main at the release, before the version bump)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

### 2026-09-27, 0.23.1 release

| Target | Ours | Reference | Result |
|---|---|---|---|
| pdf -> png, photo (45.8 MB of pixels, 40.7 MB on disk): throughput (MB/s of decoded pixels) | 116.1 | 62.7 (lopdf + png) | PASS |
| pdf -> png, photo: peak memory (MB) | 45.8 | 92.1 (lopdf + png) | PASS |
| pdf -> png, photo: output size (MB) [extra] | 25.0 | 27.1 (png) | n/a |
| png -> pdf, photo (29.0 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 253.0 | 82.3 (png + printpdf) | PASS |
| png -> pdf, photo: peak memory (MB) | 5.6 | 227.4 (png + printpdf) | PASS |
| png -> pdf, photo: output size (MB) [extra] | 25.0 | 37.9 (printpdf) | n/a |
| pdf -> png, flat (45.8 MB of pixels, 0.4 MB on disk): throughput (MB/s of decoded pixels) | 1312.6 | 659.1 (lopdf + png) | PASS |
| pdf -> png, flat: peak memory (MB) | 5.1 | 54.2 (lopdf + png) | PASS |
| pdf -> png, flat: output size (MB) [extra] | 0.1 | 0.3 (png) | n/a |
| png -> pdf, flat (1.6 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 940.8 | 261.3 (png + printpdf) | PASS |
| png -> pdf, flat: peak memory (MB) | 5.1 | 189.6 (png + printpdf) | PASS |
| png -> pdf, flat: output size (MB) [extra] | 0.1 | 0.2 (printpdf) | n/a |
| pdf -> png, stock (418.4 MB of pixels, 93.7 MB on disk): throughput (MB/s of decoded pixels) [stock] | 130.9 | 89.2 (lopdf + png) | PASS |
| pdf -> png, stock: peak memory (MB) [stock] | 99.3 | 890.6 (lopdf + png) | PASS |
| pdf -> png, stock: output size (MB) [extra] | 28.9 | 44.2 (png) | n/a |
| png -> pdf, stock (24.2 MB in + 418.4 MB of pixels): throughput (MB/s of input plus pixels) [stock] | 160.4 | 111.4 (png + printpdf) | PASS |
| png -> pdf, stock: peak memory (MB) [stock] | 6.8 | 1679.4 (png + printpdf) | PASS |
| png -> pdf, stock: output size (MB) [extra] | 26.3 | 88.7 (printpdf) | n/a |

commit: 121f26b (main at the release, before the version bump)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

### 2026-09-27, 0.23.0 release

| Target | Ours | Reference | Result |
|---|---|---|---|
| pdf -> png, photo (45.8 MB of pixels, 40.7 MB on disk): throughput (MB/s of decoded pixels) | 112.2 | 62.7 (lopdf + png) | PASS |
| pdf -> png, photo: peak memory (MB) | 45.8 | 91.9 (lopdf + png) | PASS |
| pdf -> png, photo: output size (MB) [extra] | 25.0 | 27.1 (png) | n/a |
| png -> pdf, photo (29.0 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 242.4 | 81.9 (png + printpdf) | PASS |
| png -> pdf, photo: peak memory (MB) | 5.4 | 227.1 (png + printpdf) | PASS |
| png -> pdf, photo: output size (MB) [extra] | 25.0 | 37.9 (printpdf) | n/a |
| pdf -> png, flat (45.8 MB of pixels, 0.4 MB on disk): throughput (MB/s of decoded pixels) | 1305.2 | 687.9 (lopdf + png) | PASS |
| pdf -> png, flat: peak memory (MB) | 5.3 | 54.6 (lopdf + png) | PASS |
| pdf -> png, flat: output size (MB) [extra] | 0.1 | 0.3 (png) | n/a |
| png -> pdf, flat (1.6 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 887.9 | 257.7 (png + printpdf) | PASS |
| png -> pdf, flat: peak memory (MB) | 5.3 | 189.6 (png + printpdf) | PASS |
| png -> pdf, flat: output size (MB) [extra] | 0.1 | 0.2 (printpdf) | n/a |
| pdf -> png, stock (418.4 MB of pixels, 93.7 MB on disk): throughput (MB/s of decoded pixels) [stock] | 131.6 | 88.5 (lopdf + png) | PASS |
| pdf -> png, stock: peak memory (MB) [stock] | 98.8 | 890.3 (lopdf + png) | PASS |
| pdf -> png, stock: output size (MB) [extra] | 28.9 | 44.2 (png) | n/a |
| png -> pdf, stock (24.2 MB in + 418.4 MB of pixels): throughput (MB/s of input plus pixels) [stock] | 161.9 | 110.8 (png + printpdf) | PASS |
| png -> pdf, stock: peak memory (MB) [stock] | 6.9 | 1679.1 (png + printpdf) | PASS |
| png -> pdf, stock: output size (MB) [extra] | 26.3 | 88.7 (printpdf) | n/a |

commit: cb93dff (main at the release, before the version bump)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

### 2026-09-27, pdf-bench branch

The PDF reader streams a lone Flate image's rows (inflate, predictor,
color) into the writer; before, it held the stream, the samples and the
pixels, and lost the memory lines (176.7 MB photo, 1341.6 MB stock) and
the flat throughput line (470.8 MB/s against 664.7).

| Target | Ours | Reference | Result |
|---|---|---|---|
| pdf -> png, photo (45.8 MB of pixels, 40.7 MB on disk): throughput (MB/s of decoded pixels) | 113.0 | 60.1 (lopdf + png) | PASS |
| pdf -> png, photo: peak memory (MB) | 45.8 | 92.3 (lopdf + png) | PASS |
| pdf -> png, photo: output size (MB) [extra] | 25.0 | 27.1 (png) | n/a |
| png -> pdf, photo (29.0 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 243.1 | 80.7 (png + printpdf) | PASS |
| png -> pdf, photo: peak memory (MB) | 5.9 | 226.8 (png + printpdf) | PASS |
| png -> pdf, photo: output size (MB) [extra] | 25.0 | 37.9 (printpdf) | n/a |
| pdf -> png, flat (45.8 MB of pixels, 0.4 MB on disk): throughput (MB/s of decoded pixels) | 1342.6 | 662.3 (lopdf + png) | PASS |
| pdf -> png, flat: peak memory (MB) | 5.1 | 54.2 (lopdf + png) | PASS |
| pdf -> png, flat: output size (MB) [extra] | 0.1 | 0.3 (png) | n/a |
| png -> pdf, flat (1.6 MB in + 45.8 MB of pixels): throughput (MB/s of input plus pixels) | 876.9 | 251.7 (png + printpdf) | PASS |
| png -> pdf, flat: peak memory (MB) | 5.3 | 189.5 (png + printpdf) | PASS |
| png -> pdf, flat: output size (MB) [extra] | 0.1 | 0.2 (printpdf) | n/a |
| pdf -> png, stock (418.4 MB of pixels, 93.7 MB on disk): throughput (MB/s of decoded pixels) [stock] | 130.8 | 88.4 (lopdf + png) | PASS |
| pdf -> png, stock: peak memory (MB) [stock] | 99.2 | 890.6 (lopdf + png) | PASS |
| pdf -> png, stock: output size (MB) [extra] | 28.9 | 44.2 (png) | n/a |
| png -> pdf, stock (24.2 MB in + 418.4 MB of pixels): throughput (MB/s of input plus pixels) [stock] | 161.5 | 112.8 (png + printpdf) | PASS |
| png -> pdf, stock: peak memory (MB) [stock] | 7.5 | 1679.2 (png + printpdf) | PASS |
| png -> pdf, stock: output size (MB) [extra] | 26.3 | 88.7 (printpdf) | n/a |

commit: b853f53
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K
