# ODS <-> CSV

**Latest** (2026-10-17, branch `ods`: every line PASSES)

## Purpose

The OpenDocument spreadsheet reader and writer. Spreadsheet to CSV is
the streaming inflate, the XML pull reader, and the number-format
renderer; CSV to spreadsheet is the row reader and the streaming ZIP
writer.

## Reference

Reading: `calamine` 0.36.1 (the Rust spreadsheet reader) loading the
first sheet as a range, written out through the `csv` crate, and
LibreOffice's CSV export (`soffice --convert-to csv`, cells as shown,
the first sheet). Writing: LibreOffice converting the CSV to ODS.
`spreadsheet-ods`, the Rust ODS writer, cannot be built: every release
needs `zip` 6, which is yanked. Implemented in `bench/pairs/ods-csv.sh`
and `bench/src/pairs/ods_csv.rs`.

## Pass lines

Not slower, and no more memory, than every reference, in each direction.
Throughput follows the standard's rule for compressed formats:
uncompressed bytes parsed for the reader, input plus uncompressed output
for the writer.

## Method

**Machine.** Recorded with each results block from the harness's machine
line.

**Inputs.** Generated: the csv-json generator's 1,000,000 rows (56.8 MB
of CSV) saved as ODS by LibreOffice: 764 MB of XML. Real: `veh9901.ods`,
the Department for Transport's vehicle licensing table (Open Government
Licence), 15 MB holding 710 MB of XML in several sheets, read for its
first sheet. The real file is one of the ODS check's held-out set, and
the reader's speed and memory work was done on it; the generated file
was not looked at until the second run below.

**Statistics.** `bench/run.sh ods-csv`: three runs per command, median
wall clock of the whole process, peak resident memory from GNU `time`.

**Commands.**

- ours: `sublime -q convert in.ods --sheet 1 out.csv` and `sublime -q convert medium.csv out.ods`
- calamine: `sublime-bench ods-csv crates-ods-csv in.ods out.csv`
- LibreOffice: `soffice --headless --convert-to 'csv:...,1' in.ods` and `soffice --headless --convert-to ods medium.csv`

## Threats to validity

- calamine and LibreOffice give the cells their own way (calamine raw
  values, LibreOffice as shown); ours shows them as LibreOffice does, so
  it does more per cell than calamine.
- calamine's rate on the generated file moves between runs (283.5,
  218.7, 273.3 MB/s); ours led it in both runs of the final code (269.6
  against 218.7, 325.7 against 273.3). Memory does not move (177 to 205 MB
  against 920 to 940).
- LibreOffice's times include its start (about a second).

## Results

2026-10-17, `ods` branch, final code:

| Target | Ours | calamine | LibreOffice | Result |
|---|---|---|---|---|
| ods -> csv, generated (764.0 MB uncompressed): throughput (MB/s of uncompressed input) | 325.7 | 273.3 | 84.3 | PASS |
| ods -> csv, generated: peak memory (MB) | 204.8 | 920.8 | 841.1 | PASS |
| ods -> csv, real (710.1 MB uncompressed): throughput (MB/s of uncompressed input) | 947.4 | 311.3 | 40.4 | PASS |
| ods -> csv, real: peak memory (MB) | 40.1 | 898.3 | 399.7 | PASS |
| csv -> ods, generated (56.8 MB in + 583.7 MB out): throughput (MB/s of input plus uncompressed output) | 532.7 | n/a | 50.8 | PASS |
| csv -> ods, generated: peak memory (MB) | 7.4 | n/a | 764.9 | PASS |

machine: Darwin 24.5.0 arm64, 10 cpus, Apple M1 Max

The run before, after the reader became one streaming pass:

| Target | Ours | calamine | LibreOffice | Result |
|---|---|---|---|---|
| ods -> csv, generated (764.0 MB uncompressed): throughput (MB/s of uncompressed input) | 269.6 | 218.7 | 81.8 | PASS |
| ods -> csv, generated: peak memory (MB) | 177.2 | 940.3 | 810.2 | PASS |
| ods -> csv, real (710.1 MB uncompressed): throughput (MB/s of uncompressed input) | 949.3 | 319.6 | 30.8 | PASS |
| ods -> csv, real: peak memory (MB) | 40.3 | 836.8 | 407.2 | PASS |
| csv -> ods, generated (56.8 MB in + 583.7 MB out): throughput (MB/s of input plus uncompressed output) | 519.9 | n/a | 47.7 | PASS |
| csv -> ods, generated: peak memory (MB) | 6.6 | n/a | 771.5 | PASS |

machine: Darwin 24.5.0 arm64, 10 cpus, Apple M1 Max

The first run, with the reader holding `content.xml` whole and parsing
each sheet twice, failed two lines: generated throughput 164.3 against
calamine's 283.5 MB/s, and real memory 729.4 against LibreOffice's 354.0
MB. Reading `content.xml` four megabytes at a time in one pass, each
sheet gathered as its cells' text, fixed both.
