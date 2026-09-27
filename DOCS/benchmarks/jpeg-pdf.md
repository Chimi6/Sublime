# JPEG <-> PDF

**Latest** (2026-09-27, pdf-bench branch: every line PASSES)

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
one to ten milliseconds, where a single run times process start and the
file system more than the copy, and three single runs swung by several
times from run to run. So each throughput line times 100 runs back to
back (the output removed before each, both sides alike), three times,
and takes the median; memory is the peak resident size of single runs
from GNU `time`.

## Threats to validity

- Throughput at this scale still includes process start and file
  creation on both sides; it ranks the whole command, not the copy.

## Results

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
command. The harness now times 100 runs back to back.
