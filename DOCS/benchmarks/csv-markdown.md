# CSV <-> Markdown table

**Latest** (2026-10-21, 1.0.0 release run: csv -> markdown 161.6 MB/s of input at 2.4 MB peak, PASS against Miller; markdown -> csv 49.6 MB/s at 636 MB peak; markdown -> csv throughput FAILS against pulldown-cmark with the csv crate (79.4), its memory passes)

## Purpose

The bridge between the row formats and the document hub: rows become a
Markdown table (and from there HTML, Word, and every document format),
and a document's first table becomes rows. This pair measures the table
path of the Markdown writer and parser, which every spreadsheet-to-
document conversion runs through.

## Reference

CSV to Markdown: Miller (`mlr --icsv --omd cat`), the data tool that
writes Markdown tables from CSV and the real tool people use for this;
installed through Homebrew on both platforms. As context, an `[extra]`
row against the `csv` crate feeding a bespoke table printer in the
harness that escapes what GFM would misread (a printer that escaped only
pipes would turn every email into a link); it is not a tool anyone uses,
so it is not the pass line. Markdown to CSV: `pulldown-cmark`'s events
(with the same extensions our parser runs) into the `csv` crate, the
conventional Rust way. Implemented in `bench/src/pairs/csv_markdown.rs`.
pandoc reads CSV too (`pandoc -f csv -t gfm`) but takes 6 s on 100,000
rows, sixty times Miller, so it is not wired in.

## Pass lines

Not slower, and no more memory, than Miller (CSV to Markdown) and
pulldown-cmark with the csv crate (Markdown to CSV).

## Method

**Machine.** Recorded with each results block from the harness's
machine line.

**Inputs.** The csv-json generator's file capped at 1,000,000 rows
(56.8 MB: an integer, three strings, a number, a note with a quoted comma
every seventh row), and our own Markdown table of it (70.9 MB), so the
two directions are independent of each other's code path.

**Statistics.** `bench/run.sh csv-markdown`: three runs per command,
median wall clock of the whole process, peak resident memory from GNU
`time`, throughput in MB/s over the input file's bytes. Rows and units
follow `README.md`.

**Commands.**

- ours: `sublime -q convert medium.csv out.md` and `sublime -q convert medium-table.md out.csv`
- references: `mlr --icsv --omd cat medium.csv`, `sublime-bench csv-markdown crates-csv-markdown`, `crates-markdown-csv`

## Threats to validity

- Miller's Markdown output escapes nothing, so its job is a little
  smaller than ours; the printer row shows what a minimal correct
  writer of the same output costs (about 20 percent less than ours),
  the price of a general writer that dispatches three events per cell.
- The Markdown table is regular: one line per row, no inline formatting
  in cells; a table of prose cells runs the inline parser per cell,
  which the plain-text fast path skips here.
- Both Markdown parsers hold the whole table before its rows come out;
  the memory rows are that, not a limit of either.

## Results

### 2026-10-21, `v1-wrapup` branch, the 1.0.0 release run

commit: ec68950
machine: Darwin 24.5.0 arm64, 10 cpus, Apple M1 Max

| Target | Ours | Reference | Result |
|---|---|---|---|
| csv -> markdown, 1000000 rows (56.8 MB): throughput (MB/s of input) | 161.6 | 117.4 (Miller) | PASS |
| csv -> markdown, 1000000 rows: peak memory (MB) | 2.4 | 405.6 (Miller) | PASS |
| csv -> markdown, 1000000 rows: throughput against a bespoke printer in the harness (MB/s of input) [extra] | 161.6 | 193.0 (csv + table printer) | n/a |
| markdown -> csv, 1000000 rows (70.9 MB): throughput (MB/s of input) | 49.6 | 79.4 (pulldown-cmark + csv) | FAIL |
| markdown -> csv, 1000000 rows: peak memory (MB) | 635.9 | 822.7 (pulldown-cmark + csv) | PASS |

markdown -> csv throughput FAILS: 49.6 MB/s against pulldown-cmark's
79.4. It passed in the previous block, 120.8 against 107.1. Its peak
memory also rose from 215.1 MB to 635.9 MB, still under the reference's
822.7. The other lines pass. This run is on a different machine from the
previous block (an M1 Max, not the Linux i7-13700K), and the machine was
in use during it (load average 7 to 12 on 10 cores: a browser, a VPN
client, a remote-desktop host), so the wall-clock rows are noisier than
earlier blocks'. Miller was not installed at the first attempt, which
read markdown -> csv at 68.4 against 104.7, also a FAIL; Miller 6.22.0
was installed from Homebrew and the pair rerun, and this block is the
rerun. A paired A/B of our binary at ec68950 against the commit before
it (93a37d4), 9 alternated runs each, gave the same median on
markdown -> csv (1.01 s against 1.02 s) and on csv -> markdown (0.31 s
both): the registry change in ec68950 did not cause the failure. The drop
against the previous block accrued over the releases since, or is load;
this run does not separate the two.

### 2026-09-25, first release of the bridge

commit: b315c6c
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

| Target | Ours | Reference | Result |
|---|---|---|---|
| csv -> markdown, 1000000 rows (56.8 MB): throughput (MB/s of input) | 202.9 | 175.6 (Miller) | PASS |
| csv -> markdown, 1000000 rows: peak memory (MB) | 3.0 | 381.6 (Miller) | PASS |
| csv -> markdown, 1000000 rows: throughput against a bespoke printer in the harness (MB/s of input) [extra] | 202.9 | 248.0 (csv + table printer) | n/a |
| markdown -> csv, 1000000 rows (70.9 MB): throughput (MB/s of input) | 120.8 | 107.1 (pulldown-cmark + csv) | PASS |
| markdown -> csv, 1000000 rows: peak memory (MB) | 215.1 | 716.0 (pulldown-cmark + csv) | PASS |

The first runs failed both throughput rows: the writer pushed text a
byte at a time and scanned every cell for list-opening digits, the
parser allocated a vector per table row, and every cell ran the inline
node machinery. Runs of plain text, a scan only where digits can
matter, one cell vector per table, and a plain-text fast path are what
the block above measures; each is a general Markdown change, not a
table one.

## Conclusions

The bridge streams rows in and holds nothing on the way to Markdown;
the reverse holds the table because the block parser finishes a block
before its inline content is rendered. Streaming table rows out of the
block parser as they close is the lever for both memory and the last
20 percent against a bespoke printer, and would help every large table
in every document. The numbers do not justify claims about tables of
prose cells or of heavy inline formatting.
