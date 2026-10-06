# Pages -> Excel

**Latest** (2026-10-06, 0.27.1: 14.0 and 17.2 MB/s of input against the 50 MB/s goal, and 2,231 and 736 MB peak against 64 MB; every line FAILS, on table-heavy documents the Pages pairs had not measured; the cause is the package decode, below)

## Purpose

A Pages document's tables into a workbook, a worksheet per table
(`pages -> markdown -> xlsx`, the second hop `markdown-to-xlsx`). One
direction: a workbook into Pages is `xlsx -> markdown -> pages` and is not
timed here.

## Reference

No other tool converts Pages to Excel: LibreOffice opens Pages as a text
document and writes no workbook from it, and Pages exports none. The
reference column holds the goals every Pages pair shares (`pages-json.md`):
50 MB/s of input and 64 MB peak.

## Pass lines

At least 50 MB/s of input and at most 64 MB peak. Excel output is
compressed, so the input plus the output's uncompressed bytes is an extra
row (`README.md`).

## Method

**Machine.** Recorded with each results block from the harness's machine
line.

**Inputs** (`bench/pairs/pages-xlsx.sh`), Markdown from
`bench/pairs/pages-xlsx-gen.py` written to Pages by our own writer (no
large Pages document with tables can be committed); cells mix text,
decimals, ISO dates, and `TRUE` and `FALSE`:

- `tables`: 500 tables of 200 rows (100,000 rows), a heading over each,
  57.6 MB;
- `report`: four paragraphs of prose, then a heading and a 25-row table,
  1,000 times (25,000 rows), 21.6 MB.

The Pages writer gives each table its own entries and writes no ZIP64, so
a package holds about 2,000 tables; the shapes stay under it.

**Statistics.** `bench/run.sh pages-xlsx 100000`: three runs, median wall
clock of the whole process, peak resident memory from GNU `time`.

**Commands.** `sublime -q convert in.pages out.xlsx`.

## Threats to validity

The inputs are our writer's packages. Its `Index/Metadata.iwa` (the
component and identifier map) is far larger than the one Pages writes: 16.7
MB here, 3.2 million decoded fields of the 31 million below. A Pages-made
document of the same tables will read lighter; the per-table cost remains.

## Results

### 2026-10-06, 0.27.1

| Target | Ours | Reference | Result |
|---|---|---|---|
| pages -> xlsx, tables (57.6 MB): throughput (MB/s of input) | 14.0 | goal: 50 | FAIL |
| pages -> xlsx, tables: peak memory (MB) | 2230.7 | goal: <= 64.0 | FAIL |
| pages -> xlsx, tables (57.6 MB in + 27.8 MB out): throughput (MB/s of input plus uncompressed output) [extra] | 20.8 | n/a | n/a |
| pages -> xlsx, report (21.6 MB): throughput (MB/s of input) | 17.2 | goal: 50 | FAIL |
| pages -> xlsx, report: peak memory (MB) | 735.9 | goal: <= 64.0 | FAIL |
| pages -> xlsx, report (21.6 MB in + 7.6 MB out): throughput (MB/s of input plus uncompressed output) [extra] | 23.2 | n/a | n/a |

commit: 38a872d (main before 0.27.1)
machine: Darwin 24.5.0 arm64, 10 cpus, Apple M1 Max

## Conclusions

The cost is the Pages reader, not the workbook: `pages -> markdown` alone
peaks at 2,247 MB in 3.9 s on `tables`, and `markdown -> xlsx` at 62 MB in
0.2 s. Of the reader's peak, 1.7 GB is the package decode: 31 million
protobuf fields in the generic tree (957 MB of entries) for 600,000 cells,
about fifty a cell, before the document model is built. The text-only
Pages pairs never showed it (40 to 57 MB). The levers are the ones the
Numbers reader took (`numbers-csv.md`): defer the tables' tile rows and
data lists as bytes and read cells from them, skip the objects no reader
uses, decode stream by stream; and for this route, write each table to the
workbook as it is read rather than building the whole document first.
