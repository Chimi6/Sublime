# XLSX <-> CSV

**Latest** (2026-09-25, first release of Excel workbooks: xlsx -> csv 133.6 MB/s of uncompressed input (16.2 of file bytes) at 415 MB peak; csv -> xlsx 284.3 MB/s of input plus uncompressed output at 4.7 MB peak; every line PASSES against calamine and rust_xlsxwriter with the csv crate)

## Purpose

The first format to use the ZIP reader, the inflater, the XML pull
reader, and the row writers together. Workbook to CSV is the reader
and the row writer; CSV to workbook is the row reader and the streaming
ZIP writer. Both are measured here against the Rust crates people use
for each side.

## Reference

Reading: `calamine` (the Rust spreadsheet reader) loading the first
sheet as a range, written out through the `csv` crate with the same
cell rendering (numbers as text, dates as `YYYY-MM-DD`). Writing: the
`csv` crate's reader into `rust_xlsxwriter` (the Rust workbook writer,
a port of XlsxWriter), with the same rule for which cells become
numbers. Implemented in `bench/src/pairs/xlsx_csv.rs`. LibreOffice's
`soffice --convert-to csv` is the conventional external tool and is not
wired in.

## Pass lines

Not slower, and no more memory, than the reference pipelines, in each
direction. Throughput follows the standard's rule for compressed
formats: uncompressed bytes parsed for the reader, input plus
uncompressed output for the writer.

## Method

**Machine.** Recorded with each results block from the harness's
machine line.

**Inputs.** A workbook written by `rust_xlsxwriter` with 1,000,000
rows of the csv-json record shape (an integer, three strings, a float,
a date-formatted serial, a note): 39.5 MB on disk, 326.4 MB inflated.
The CSV is the csv-json generator's file at 1,000,000 rows (56.8 MB);
our workbook of it inflates to 387.7 MB. The cap is the workbook: ten
million rows would be a 400 MB file that inflates past 3 GB.

**Statistics.** `bench/run.sh xlsx-csv`: three runs per command,
median wall clock of the whole process, peak resident memory from GNU
`time`. Rows and units follow `README.md`; the on-disk rate is an
`[extra]` row.

**Commands.**

- ours: `sublime -q convert big.xlsx out.csv` and `sublime -q convert medium.csv out.xlsx`
- reference: `sublime-bench xlsx-csv crates-xlsx-csv big.xlsx out.csv` and `crates-csv-xlsx medium.csv out.xlsx`

## Threats to validity

- The workbook is machine-written and regular: no shared-string reuse
  beyond the note column, no merged cells, one date column. A workbook
  from Excel with rich text and many styles spends more time in the
  shared strings and styles parts, which both sides read the same way.
- `calamine` loads the sheet as a range of typed cells and
  `rust_xlsxwriter` builds the whole workbook in memory before saving;
  their memory rows say what those designs cost, not that the crates
  could not stream.
- Ours inflates the sheet part whole before parsing, so its read memory
  scales with the sheet; that is the row to watch.

## Results

### 2026-09-25, first release of Excel workbooks

commit: 18e676f
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

| Target | Ours | Reference | Result |
|---|---|---|---|
| xlsx -> csv, 1000000 rows (326.4 MB uncompressed): throughput (MB/s of uncompressed input) | 133.6 | 112.9 (calamine + csv) | PASS |
| xlsx -> csv, 1000000 rows (39.5 MB on disk): throughput (MB/s of file bytes) [extra] | 16.2 | 13.7 (calamine + csv) | n/a |
| xlsx -> csv, 1000000 rows: peak memory (MB) | 414.7 | 592.6 (calamine + csv) | PASS |
| csv -> xlsx, 1000000 rows (56.8 MB in + 387.7 MB out): throughput (MB/s of input plus uncompressed output) | 284.3 | 101.1 (csv + rust_xlsxwriter) | PASS |
| csv -> xlsx, 1000000 rows: peak memory (MB) | 4.7 | 1300.9 (csv + rust_xlsxwriter) | PASS |

## Conclusions

The writer streams and is the clear win: 284 MB/s of bytes handled in
under 5 MB against a reference that holds the workbook. The reader is
ahead of calamine on time and memory but holds the inflated sheet
(326 MB of the 415 MB peak); a windowed inflate feeding the XML reader
is the lever, recorded in `STATE.md`, and would make the read memory
constant the way the CSV and JSON readers are. The numbers do not
justify claims about workbooks heavy in shared strings, styles, or
merged cells.
