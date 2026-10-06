# Numbers -> CSV and Excel

**Latest** (2026-10-06, 0.27.0 with Excel rows: numbers -> xlsx at 165 to 169 MB/s of input plus uncompressed output on 200,000-row documents at 130 to 146 MB, against numbers-parser with openpyxl's 2.7 to 2.8 at 4.3 GB, and 109 on the real document against LibreOffice's 1.4 at 1.4 GB; every line PASSES. Earlier, the `workbooks` branch: 37 MB/s of input on 200,000-row documents at 125 to 145 MB, against numbers-parser's 0.5 MB/s at 4.3 to 4.5 GB; on a real 65,000-row document 68 MB/s at 54 MB, against numbers-parser's 2.4 at 353 MB and LibreOffice's 0.9 at 1.4 GB; every line PASSES)

## Purpose

Numbers documents read into rows (`DOCS/formats/numbers.md`): a CSV per
table, and an Excel workbook with a worksheet per table. One direction:
Sublime does not write Numbers.

## Reference

Two tools read the modern Numbers format:

- **numbers-parser** (Python, MIT), the library most tools read Numbers
  through, with Python's `csv` module writing a CSV per table
  (`bench/pairs/numbers-parser-csv.py`), and with openpyxl in write-only
  (streaming) mode writing a worksheet per table
  (`bench/pairs/numbers-parser-xlsx.py`).
- **LibreOffice** (libetonyek), the other converter, exporting every table
  as CSV. It reads only the last 256-row tile of a table numbers-parser
  wrote, so it is timed on the real document alone.

No Rust crate reads Numbers.

## Pass lines

Not slower, and no more memory, than either reference on the same input.
A Numbers document is a ZIP of stored entries, so its bytes on disk are the
bytes the readers parse; throughput into CSV is MB/s of the file. Excel
output is compressed, so throughput into Excel is MB/s of the input plus
the output's uncompressed bytes (`README.md`); ours' output measures every
tool's, since each writes the same cells.

## Method

**Machine.** Recorded with each results block from the harness's machine
line.

**Inputs** (`bench/pairs/numbers-csv.sh`):

- `real`: `issue-50.numbers` from numbers-parser's test data, a
  65,553-row table Numbers itself saved (4.1 MB), fetched at a pinned
  commit;
- `dense`: one table of 200,000 records in eight mixed columns (text,
  integers, decimals, dates, booleans), written by numbers-parser
  (`bench/pairs/numbers-gen.py`), 14.3 MB;
- `sheets`: the same records across twenty sheets of three tables each,
  15.1 MB.

**Statistics.** `bench/run.sh numbers-csv 200000`: three runs per command,
median wall clock of the whole process, peak resident memory from GNU
`time`. Rows and units follow `README.md`.

**Commands.** Per input:

- ours: `sublime -q convert in.numbers out.csv` (a folder of a CSV per
  table when there are several);
- numbers-parser: `python bench/pairs/numbers-parser-csv.py in.numbers dir/`;
- LibreOffice: `soffice --headless --convert-to 'csv:...:-1' in.numbers`
  (every sheet).
- Excel: `sublime -q convert in.numbers out.xlsx`; `python
  bench/pairs/numbers-parser-xlsx.py in.numbers out.xlsx`; `soffice
  --headless --convert-to xlsx in.numbers`.

## Threats to validity

The generated inputs are numbers-parser's own output, which is simpler
than what Numbers writes (one cell style, no formulas); the real document
is one table of one column. Python's start-up and LibreOffice's (about
two seconds) are inside their times, which matters most on the 4 MB file.

## Results

### 2026-10-06, 0.27.0, with the Excel rows

| Target | Ours | numbers-parser | LibreOffice | Result |
|---|---|---|---|---|
| numbers -> csv, real (4.1 MB): throughput (MB/s of input) | 64.1 | 2.2 | 0.9 | PASS |
| numbers -> csv, real: peak memory (MB) | 53.2 | 371.1 | 1421.0 | PASS |
| numbers -> csv, dense (14.3 MB): throughput (MB/s of input) | 36.5 | 0.5 | n/a | PASS |
| numbers -> csv, dense: peak memory (MB) | 141.2 | 4252.7 | n/a | PASS |
| numbers -> csv, sheets (15.1 MB): throughput (MB/s of input) | 36.1 | 0.5 | n/a | PASS |
| numbers -> csv, sheets: peak memory (MB) | 125.3 | 4196.3 | n/a | PASS |
| numbers -> xlsx, real (4.1 MB in + 6.4 MB out): throughput (MB/s of input plus uncompressed output) | 109.2 | 4.0 | 1.4 | PASS |
| numbers -> xlsx, real: peak memory (MB) | 54.2 | 395.3 | 1422.2 | PASS |
| numbers -> xlsx, dense (14.3 MB in + 102.3 MB out): throughput (MB/s of input plus uncompressed output) | 164.7 | 2.8 | n/a | PASS |
| numbers -> xlsx, dense: peak memory (MB) | 146.2 | 4256.4 | n/a | PASS |
| numbers -> xlsx, sheets (15.1 MB in + 99.3 MB out): throughput (MB/s of input plus uncompressed output) | 169.4 | 2.7 | n/a | PASS |
| numbers -> xlsx, sheets: peak memory (MB) | 130.3 | 4321.3 | n/a | PASS |

commit: 49fc22e (main after 0.27.0)
machine: Darwin 24.5.0 arm64, 10 cpus, Apple M1 Max

Ours and numbers-parser with openpyxl write the same worksheets and rows
(60 and 200,040 on `sheets`, checked by reading both back). One difference
in kind: ours writes dates and booleans as text cells (`2024-08-08`,
`FALSE`), where openpyxl writes a date and a boolean; numbers are numbers
in both. Typed dates and booleans are a writer change for later, and cost
a style table and a few bytes a cell.

### 2026-10-06, the `workbooks` branch

| Target | Ours | numbers-parser | LibreOffice | Result |
|---|---|---|---|---|
| numbers -> csv, real (4.1 MB): throughput (MB/s of input) | 68.0 | 2.4 | 0.9 | PASS |
| numbers -> csv, real: peak memory (MB) | 54.1 | 352.8 | 1417.2 | PASS |
| numbers -> csv, dense (14.3 MB): throughput (MB/s of input) | 36.7 | 0.5 | n/a | PASS |
| numbers -> csv, dense: peak memory (MB) | 145.4 | 4329.8 | n/a | PASS |
| numbers -> csv, sheets (15.1 MB): throughput (MB/s of input) | 37.1 | 0.5 | n/a | PASS |
| numbers -> csv, sheets: peak memory (MB) | 124.6 | 4465.3 | n/a | PASS |

commit: 16c328c plus the streaming reader (the `workbooks` branch)
machine: Darwin 24.5.0 arm64, 10 cpus, Apple M1 Max

## Conclusions

Into Excel, every line passes by the same margins as into CSV: 27 to 61
times numbers-parser with openpyxl, 78 times LibreOffice, at the same
memory as the CSV path (the workbook writer streams each sheet). Into CSV,
every line passes by more than an order of magnitude: 28 to 75 times
numbers-parser's speed at a seventh to a thirtieth of its memory, and 75
times LibreOffice's speed at a twenty-sixth of its memory. The first run
of this pair measured 310 MB on `dense`; reading a table a tile (256 rows)
at a time, decoding each stream and freeing it before the next, and
keeping tile rows as bytes brought it to 145 MB. What remains is the
decoded string table and the tiles' cell buffers, held for the whole
document; reading tiles from the stream bytes in place is the next lever.
Re-run on the Linux bench machine for the record beside the other pairs.
