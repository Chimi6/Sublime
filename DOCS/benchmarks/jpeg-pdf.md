# JPEG <-> PDF

**Latest** (2026-09-27, 0.23.0 release: every line PASSES)

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

**Statistics.** `bench/run.sh jpeg-pdf`: every command here finishes in
one to ten milliseconds, where single runs time the machine's noise and
back-to-back blocks catch its slow spells unevenly (three single runs,
then blocks of 100, both flipped lines between runs). So each
throughput line alternates ours and the reference for 101 rounds, the
outputs removed before each run, timed by bash's microsecond clock, and
compares the two medians; memory is the peak resident size of single
runs from GNU `time`.

## Threats to validity

- Throughput at this scale still includes process start and file
  creation on both sides; it ranks the whole command, not the copy.

## Results

### 2026-09-27, 0.23.0 release

| Target | Ours | Reference | Result |
|---|---|---|---|
| jpeg -> pdf, photo (1.6 MB): throughput (MB/s of JPEG, median of 101 alternated runs) | 344.6 | 246.4 (lopdf) | PASS |
| jpeg -> pdf, photo: peak memory (MB) | 5.0 | 9.2 (lopdf) | PASS |
| pdf -> jpeg, photo (1.6 MB): throughput (MB/s of JPEG, median of 101 alternated runs) | 499.7 | 309.9 (lopdf) | PASS |
| pdf -> jpeg, photo: peak memory (MB) | 5.4 | 8.5 (lopdf) | PASS |
| jpeg -> pdf, flat (0.3 MB): throughput (MB/s of JPEG, median of 101 alternated runs) | 197.5 | 168.1 (lopdf) | PASS |
| jpeg -> pdf, flat: peak memory (MB) | 4.0 | 6.6 (lopdf) | PASS |
| pdf -> jpeg, flat (0.3 MB): throughput (MB/s of JPEG, median of 101 alternated runs) | 128.6 | 67.9 (lopdf) | PASS |
| pdf -> jpeg, flat: peak memory (MB) | 4.2 | 5.8 (lopdf) | PASS |
| jpeg -> pdf, stock (3.2 MB): throughput (MB/s of JPEG, median of 101 alternated runs) [stock] | 572.2 | 400.5 (lopdf) | PASS |
| jpeg -> pdf, stock: peak memory (MB) [stock] | 6.9 | 12.2 (lopdf) | PASS |
| pdf -> jpeg, stock (3.2 MB): throughput (MB/s of JPEG, median of 101 alternated runs) [stock] | 471.8 | 297.6 (lopdf) | PASS |
| pdf -> jpeg, stock: peak memory (MB) [stock] | 7.2 | 11.1 (lopdf) | PASS |

commit: cb93dff (main at the release, before the version bump)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

### 2026-09-27, pdf-bench branch (single runs)

Interleaved, 200 rounds, stock (3.2 MB JPEG), milliseconds:

| Command | Ours median | Ours min | lopdf median | lopdf min |
|---|---|---|---|---|
| jpeg -> pdf | 7.381 | 2.504 | 10.413 | 3.450 |
| pdf -> jpeg | 9.023 | 3.134 | 11.849 | 4.224 |

Flat (0.3 MB), jpeg -> pdf, 300 rounds: ours 0.990 ms median, lopdf
1.359 ms.

With three single runs a command, the harness's throughput lines
flipped between runs (the stock jpeg -> pdf line read 698.6 against
159.3 MB/s, then 175.9 against 207.2), and at the release commit three
lines read FAIL; the interleaved timing above had ours ahead on every
command. Blocks of 100 runs back to back flipped the stock pdf -> jpeg
line too (344.5 against lopdf's 552.4, 378.3 against 206.9 the run
before); the harness now alternates the two commands.
