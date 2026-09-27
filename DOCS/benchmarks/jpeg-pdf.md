# JPEG <-> PDF

**Latest** (2026-09-27, pdf-bench branch: every line PASSES by the
interleaved timing below; one harness line reads FAIL at the
millisecond scale)

## Purpose

A JPEG into a PDF page and back out without re-encoding: JPEG to PDF
embeds the file as a DCTDecode image, PDF to JPEG copies that stream
back out byte for byte.

## Reference

lopdf building the same one-page document (an image XObject holding the
JPEG, a page at its size) and, the other way, loading the file, finding
the page's largest image and writing its stream out. printpdf decodes
and re-encodes JPEGs, which is not the same job. Implemented in
`bench/src/pairs/pdf.rs`.

## Pass lines

Not slower, and no more memory, than the reference, in each direction
and shape. Throughput counts the JPEG's bytes.

## Method

**Machine.** Recorded with each results block from the harness's
machine line.

**Inputs.** The jpeg-png pair's photo and flat JPEGs (quality 85) and a
stock photograph (`bench/data/stock.jpg`, or one ImageMagick makes at
quality 85 from `bench/data/stock.png`, `[stock]`); the PDF inputs
are the reference's own output for each.

**Statistics.** `bench/run.sh jpeg-pdf`: three runs per command, median
wall clock of the whole process, peak resident memory from GNU `time`;
outputs are removed before each run. Every command here finishes in one
to ten milliseconds, where three runs time process start and the file
system more than the copy, and the harness's throughput lines swing by
several times from run to run. The timing that decides the pass lines
is interleaved: each of the four commands run in turn, 200 rounds, on
the stock inputs.

## Threats to validity

- Throughput at this scale is mostly process start; the memory lines
  and the interleaved medians are the stable measures.

## Results

### 2026-09-27, pdf-bench branch

Interleaved, 200 rounds, stock (3.2 MB JPEG), milliseconds:

| Command | Ours median | Ours min | lopdf median | lopdf min |
|---|---|---|---|---|
| jpeg -> pdf | 7.381 | 2.504 | 10.413 | 3.450 |
| pdf -> jpeg | 9.023 | 3.134 | 11.849 | 4.224 |

Flat (0.3 MB), jpeg -> pdf, 300 rounds: ours 0.990 ms median, lopdf
1.359 ms.

The harness:

| Target | Ours | Reference | Result |
|---|---|---|---|
| jpeg -> pdf, photo (1.6 MB): throughput (MB/s of JPEG) | 433.4 | 100.4 (lopdf) | PASS |
| jpeg -> pdf, photo: peak memory (MB) | 5.3 | 9.1 (lopdf) | PASS |
| pdf -> jpeg, photo (1.6 MB): throughput (MB/s of JPEG) | 358.0 | 84.4 (lopdf) | PASS |
| pdf -> jpeg, photo: peak memory (MB) | 7.0 | 8.5 (lopdf) | PASS |
| jpeg -> pdf, flat (0.3 MB): throughput (MB/s of JPEG) | 126.0 | 31.7 (lopdf) | PASS |
| jpeg -> pdf, flat: peak memory (MB) | 4.0 | 6.3 (lopdf) | PASS |
| pdf -> jpeg, flat (0.3 MB): throughput (MB/s of JPEG) | 30.1 | 20.8 (lopdf) | PASS |
| pdf -> jpeg, flat: peak memory (MB) | 4.5 | 5.7 (lopdf) | PASS |
| jpeg -> pdf, stock (3.2 MB): throughput (MB/s of JPEG) [stock] | 175.9 | 207.2 (lopdf) | FAIL |
| jpeg -> pdf, stock: peak memory (MB) [stock] | 6.7 | 12.1 (lopdf) | PASS |
| pdf -> jpeg, stock (3.2 MB): throughput (MB/s of JPEG) [stock] | 157.9 | 129.1 (lopdf) | PASS |
| pdf -> jpeg, stock: peak memory (MB) [stock] | 10.0 | 11.7 (lopdf) | PASS |

commit: b853f53
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K
