# Numbers -> Excel

**Latest** (2026-10-06, 0.27.1: 144 to 146 MB/s of input plus uncompressed output on 200,000-row documents at 127 to 147 MB, against numbers-parser with openpyxl's 2.3 to 2.4 at 4.2 GB; on a real 65,000-row document 139 MB/s at 56 MB, against numbers-parser's 4.2 at 365 MB and LibreOffice's 1.4 at 1.4 GB; every line PASSES)

## Purpose

A Numbers document into an Excel workbook, a worksheet per table
(`numbers-to-xlsx`, `DOCS/formats/numbers.md`), with numbers, dates, and
booleans as cells of their type. One direction: Sublime does not write
Numbers.

## Reference

- **numbers-parser** (Python, MIT) with **openpyxl** in write-only
  (streaming) mode, a worksheet per table
  (`bench/pairs/numbers-parser-xlsx.py`): the usual Python route, since
  numbers-parser writes no workbook.
- **LibreOffice** (libetonyek), `--convert-to xlsx`, on the real document
  only: it reads only the last 256-row tile of a table numbers-parser
  wrote.

No Rust crate reads Numbers.

## Pass lines

Not slower, and no more memory, than either reference on the same input.
Excel output is compressed, so throughput is MB/s of the input plus the
output's uncompressed bytes (`README.md`); every tool writes the same
cells, so ours' output measures each.

## Method

**Machine.** Recorded with each results block from the harness's machine
line.

**Inputs.** The `numbers-csv` pair's (`numbers-csv.md`): `real`, a
65,553-row table Numbers saved (numbers-parser's `issue-50.numbers`, 4.1
MB); `dense`, one table of 200,000 records in eight mixed columns (14.3
MB); `sheets`, the same records across twenty sheets of three tables
(15.1 MB).

**Statistics.** `bench/run.sh numbers-xlsx 200000`: three runs per command,
median wall clock of the whole process, peak resident memory from GNU
`time`.

**Commands.** `sublime -q convert in.numbers out.xlsx`; `python
bench/pairs/numbers-parser-xlsx.py in.numbers out.xlsx`; `soffice
--headless --convert-to xlsx in.numbers`.

## Threats to validity

The generated inputs are numbers-parser's own output, simpler than what
Numbers writes; the real document is one table of one column. Python's
and LibreOffice's start-up are inside their times. openpyxl writes typed
cells from numbers-parser's values, ours types cells by their text under
the writer's read-back rule (`DOCS/formats/xlsx.md`); both workbooks hold
the same sheets, rows, and values (60 sheets and 200,040 rows on `sheets`,
checked by reading both back with openpyxl).

## Results

### 2026-10-06, 0.27.1

With typed dates and booleans: the output is smaller than 0.27.0's (86.2
against 102.3 MB uncompressed on `dense`) in the same wall time, so the
rate per byte reads lower.

| Target | Ours | numbers-parser + openpyxl | LibreOffice | Result |
|---|---|---|---|---|
| numbers -> xlsx, real (4.1 MB in + 6.4 MB out): throughput (MB/s of input plus uncompressed output) | 139.1 | 4.2 | 1.4 | PASS |
| numbers -> xlsx, real: peak memory (MB) | 56.1 | 364.9 | 1418.4 | PASS |
| numbers -> xlsx, dense (14.3 MB in + 86.2 MB out): throughput (MB/s of input plus uncompressed output) | 146.1 | 2.3 | n/a | PASS |
| numbers -> xlsx, dense: peak memory (MB) | 146.6 | 4264.4 | n/a | PASS |
| numbers -> xlsx, sheets (15.1 MB in + 83.2 MB out): throughput (MB/s of input plus uncompressed output) | 144.3 | 2.4 | n/a | PASS |
| numbers -> xlsx, sheets: peak memory (MB) | 126.8 | 4198.2 | n/a | PASS |

commit: 88908dd (v0.27.1)
machine: Darwin 24.5.0 arm64, 10 cpus, Apple M1 Max

### 2026-10-06, 0.27.0 (as rows of the numbers-csv pair)

Dates and booleans were text cells then.

| Target | Ours | numbers-parser + openpyxl | LibreOffice | Result |
|---|---|---|---|---|
| numbers -> xlsx, real (4.1 MB in + 6.4 MB out): throughput (MB/s of input plus uncompressed output) | 109.2 | 4.0 | 1.4 | PASS |
| numbers -> xlsx, real: peak memory (MB) | 54.2 | 395.3 | 1422.2 | PASS |
| numbers -> xlsx, dense (14.3 MB in + 102.3 MB out): throughput (MB/s of input plus uncompressed output) | 164.7 | 2.8 | n/a | PASS |
| numbers -> xlsx, dense: peak memory (MB) | 146.2 | 4256.4 | n/a | PASS |
| numbers -> xlsx, sheets (15.1 MB in + 99.3 MB out): throughput (MB/s of input plus uncompressed output) | 169.4 | 2.7 | n/a | PASS |
| numbers -> xlsx, sheets: peak memory (MB) | 130.3 | 4321.3 | n/a | PASS |

commit: 49fc22e
machine: Darwin 24.5.0 arm64, 10 cpus, Apple M1 Max

## Conclusions

Every line passes by more than an order of magnitude: 33 to 63 times
numbers-parser with openpyxl and 99 times LibreOffice, at a sixth to a
twenty-ninth of numbers-parser's memory and a twenty-fifth of
LibreOffice's. Memory is the reader's, the same as `numbers -> csv`: the
workbook writer streams each sheet. The levers left are the reader's
(`numbers-csv.md`, Conclusions).
