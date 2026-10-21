# Numbers -> CSV

**Latest** (2026-10-21, 1.0.0 release run: 30 to 31 MB/s of input on 200,000-row documents at 105 to 128 MB, against numbers-parser's 0.5 MB/s at 4.5 GB; on a real 65,000-row document 63 MB/s at 41 MB, against numbers-parser's 2.3 at 379 MB and LibreOffice's 0.9 at 1.4 GB; every line PASSES. Numbers -> Excel is its own pair, `numbers-xlsx.md`. Earlier, 0.27.0: 36 MB/s of input on 200,000-row documents at 125 to 141 MB, against numbers-parser's 0.5 MB/s at 4.2 GB; on a real 65,000-row document 64 MB/s at 53 MB, against numbers-parser's 2.2 at 371 MB and LibreOffice's 0.9 at 1.4 GB; every line PASSES)

## Purpose

Numbers documents read into rows (`DOCS/formats/numbers.md`), a CSV per
table. One direction: Sublime does not write Numbers. Numbers -> Excel is
the `numbers-xlsx` pair (`numbers-xlsx.md`), on the same inputs.

## Reference

Two tools read the modern Numbers format:

- **numbers-parser** (Python, MIT), the library most tools read Numbers
  through, with Python's `csv` module writing a CSV per table
  (`bench/pairs/numbers-parser-csv.py`).
- **LibreOffice** (libetonyek), the other converter, exporting every table
  as CSV. It reads only the last 256-row tile of a table numbers-parser
  wrote, so it is timed on the real document alone.

No Rust crate reads Numbers.

## Pass lines

Not slower, and no more memory, than either reference on the same input.
A Numbers document is a ZIP of stored entries, so its bytes on disk are the
bytes the readers parse; throughput is MB/s of the file.

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

## Threats to validity

The generated inputs are numbers-parser's own output, which is simpler
than what Numbers writes (one cell style, no formulas); the real document
is one table of one column. Python's start-up and LibreOffice's (about
two seconds) are inside their times, which matters most on the 4 MB file.

## Results

### 2026-10-21, `v1-wrapup` branch, the 1.0.0 release run

| Target | Ours | numbers-parser | LibreOffice | Result |
|---|---|---|---|---|
| numbers -> csv, real (4.1 MB): throughput (MB/s of input) | 63.0 | 2.3 | 0.9 | PASS |
| numbers -> csv, real: peak memory (MB) | 41.4 | 379.2 | 1408.7 | PASS |
| numbers -> csv, dense (14.3 MB): throughput (MB/s of input) | 29.9 | 0.5 | n/a | PASS |
| numbers -> csv, dense: peak memory (MB) | 127.9 | 4552.5 | n/a | PASS |
| numbers -> csv, sheets (15.1 MB): throughput (MB/s of input) | 30.8 | 0.5 | n/a | PASS |
| numbers -> csv, sheets: peak memory (MB) | 105.1 | 4465.2 | n/a | PASS |

commit: ec68950
machine: Darwin 24.5.0 arm64, 10 cpus, Apple M1 Max

Every line passes. The machine was in use during this run (load average
7 to 12 on 10 cores: a browser, a VPN client, a remote-desktop host), so
the wall-clock rows are noisier than earlier blocks'. Against 0.27.0 on
the same machine, throughput is lower on the 200,000-row documents
(dense 36.5 -> 29.9 MB/s, sheets 36.1 -> 30.8) and about level on the
real document (64.1 -> 63.0), and peak memory is lower on every row
(real 53.2 -> 41.4 MB, dense 141.2 -> 127.9, sheets 125.3 -> 105.1).
The commit, ec68950, holds the registry as a const table (converter
metadata as data, the image pairs built only when an image format is an
end), which lowered every conversion's starting memory by about 0.3 MB;
the rest of the drop came with the releases since 0.27.0.

### 2026-10-06, 0.27.0

| Target | Ours | numbers-parser | LibreOffice | Result |
|---|---|---|---|---|
| numbers -> csv, real (4.1 MB): throughput (MB/s of input) | 64.1 | 2.2 | 0.9 | PASS |
| numbers -> csv, real: peak memory (MB) | 53.2 | 371.1 | 1421.0 | PASS |
| numbers -> csv, dense (14.3 MB): throughput (MB/s of input) | 36.5 | 0.5 | n/a | PASS |
| numbers -> csv, dense: peak memory (MB) | 141.2 | 4252.7 | n/a | PASS |
| numbers -> csv, sheets (15.1 MB): throughput (MB/s of input) | 36.1 | 0.5 | n/a | PASS |
| numbers -> csv, sheets: peak memory (MB) | 125.3 | 4196.3 | n/a | PASS |

commit: 49fc22e (main after 0.27.0)
machine: Darwin 24.5.0 arm64, 10 cpus, Apple M1 Max

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

Every line passes by more than an order of magnitude: 28 to 75 times
numbers-parser's speed at a seventh to a thirtieth of its memory, and 75
times LibreOffice's speed at a twenty-sixth of its memory. The first run
of this pair measured 310 MB on `dense`; reading a table a tile (256 rows)
at a time, decoding each stream and freeing it before the next, and
keeping tile rows as bytes brought it to 145 MB. What remains is the
decoded string table and the tiles' cell buffers, held for the whole
document; reading tiles from the stream bytes in place is the next lever.
Re-run on the Linux bench machine for the record beside the other pairs.

On 2026-10-21, under load, the margins were 27 to 62 times
numbers-parser's speed at a ninth to a forty-second of its memory, and
70 times LibreOffice's speed at a thirty-fourth of its memory; the real
document's 27 times is below the 28 stated above.
