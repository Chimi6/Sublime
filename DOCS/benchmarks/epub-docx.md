# EPUB <-> Word

**Latest** (2026-10-21, 1.0.0 release run: docx -> epub at 62.4 MB/s of input plus output on the styled shape and 87.6 on prose, epub -> docx at 62.3 and 84.1; every line PASSES against pandoc at 45 to 420 times its speed and a tenth to a twenty-fourth of its memory)

## Purpose

An EPUB read into the document model and written as Word, and Word read
into the model and written as an EPUB (`DOCS/formats/epub.md`). A
pair is unordered: both directions share the inputs, and each direction
has its own rows.

## Reference

`pandoc` reads and writes both EPUB and Word and is the reference, run as
an external process on the same files (`bench/pairs/epub-docx.sh`). No
Rust crate converts between them. Calibre converts both ways too, but as
a desktop suite started per file it is not a fair speed line.

## Pass lines

Not slower, and no more memory, than `pandoc` on the same input. Both
formats are compressed packages, so each direction is timed on the
uncompressed bytes the reader parses plus the uncompressed bytes the writer
compresses (`README.md`); the rate per input byte is an extra row.

## Method

**Machine.** Recorded with each results block from the harness's
machine line.

**Inputs.** The Pages pair's generated documents (`pages-json.md`, Method)
written to Word by our Word writer and to EPUB by our EPUB writer.

**Statistics.** `bench/run.sh epub-docx`: three runs per command, median
wall clock of the whole process, peak resident memory from GNU `time`.
Rows and units follow `README.md`.

**Commands.** Per shape:

- ours: `sublime -q convert styled.docx out-styled.epub --to epub` and
  `sublime -q convert styled.epub out-styled-from-epub.docx --to docx`
- pandoc: `pandoc -f docx -t epub3 styled.docx -o ...` and
  `pandoc -f epub -t docx styled.epub -o ...`

## Threats to validity

The inputs are our own writers' output: a book from them is a few long
chapters with one style sheet, where real books split into many files
and real Word files carry larger style sheets.

## Results

### 2026-10-21, 1.0.0 release run (`v1-wrapup` branch)

| Target | Ours | Reference | Result |
|---|---|---|---|
| docx -> epub, styled (6.0 MB in + 6.1 MB out, uncompressed): throughput (MB/s of input plus output) | 62.4 | 1.4 (pandoc) | PASS |
| docx -> epub, styled: throughput (MB/s of uncompressed input) [extra] | 30.8 | 0.9 (pandoc) | n/a |
| docx -> epub, styled: peak memory (MB) | 33.5 | 568.6 (pandoc) | PASS |
| epub -> docx, styled (6.1 MB in + 3.6 MB out, uncompressed): throughput (MB/s of input plus output) | 62.3 | 0.5 (pandoc) | PASS |
| epub -> docx, styled: throughput (MB/s of uncompressed input) [extra] | 39.2 | 0.2 (pandoc) | n/a |
| epub -> docx, styled: peak memory (MB) | 38.5 | 400.1 (pandoc) | PASS |
| docx -> epub, prose (7.5 MB in + 7.8 MB out, uncompressed): throughput (MB/s of input plus output) | 87.6 | 1.4 (pandoc) | PASS |
| docx -> epub, prose: throughput (MB/s of uncompressed input) [extra] | 43.1 | 0.9 (pandoc) | n/a |
| docx -> epub, prose: peak memory (MB) | 33.4 | 812.1 (pandoc) | PASS |
| epub -> docx, prose (7.8 MB in + 5.8 MB out, uncompressed): throughput (MB/s of input plus output) | 84.1 | 0.2 (pandoc) | PASS |
| epub -> docx, prose: throughput (MB/s of uncompressed input) [extra] | 48.4 | 0.1 (pandoc) | n/a |
| epub -> docx, prose: peak memory (MB) | 42.9 | 459.6 (pandoc) | PASS |

commit: ec68950 (on the `v1-wrapup` branch, before the 1.0.0 release)
machine: Darwin 24.5.0 arm64, 10 cpus, Apple M1 Max

Every line passes. The machine was in use during this run (load average
7 to 12 on 10 cores: a browser, a VPN client, a remote-desktop host), so
the wall-clock rows are noisier than earlier blocks'. On the same machine
and inputs as the block below, every throughput row is level or a few
percent higher and memory moves by at most 2.5 MB either way.

### 2026-10-14, the `epub` branch

| Target | Ours | Reference | Result |
|---|---|---|---|
| docx -> epub, styled (6.0 MB in + 6.1 MB out, uncompressed): throughput (MB/s of input plus output) | 60.6 | 1.3 (pandoc) | PASS |
| docx -> epub, styled: throughput (MB/s of uncompressed input) [extra] | 29.9 | 0.9 (pandoc) | n/a |
| docx -> epub, styled: peak memory (MB) | 31.5 | 569.0 (pandoc) | PASS |
| epub -> docx, styled (6.1 MB in + 3.6 MB out, uncompressed): throughput (MB/s of input plus output) | 62.0 | 0.5 (pandoc) | PASS |
| epub -> docx, styled: throughput (MB/s of uncompressed input) [extra] | 39.0 | 0.2 (pandoc) | n/a |
| epub -> docx, styled: peak memory (MB) | 39.3 | 399.3 (pandoc) | PASS |
| docx -> epub, prose (7.5 MB in + 7.8 MB out, uncompressed): throughput (MB/s of input plus output) | 83.6 | 1.4 (pandoc) | PASS |
| docx -> epub, prose: throughput (MB/s of uncompressed input) [extra] | 41.1 | 0.9 (pandoc) | n/a |
| docx -> epub, prose: peak memory (MB) | 35.9 | 811.7 (pandoc) | PASS |
| epub -> docx, prose (7.8 MB in + 5.8 MB out, uncompressed): throughput (MB/s of input plus output) | 82.6 | 0.2 (pandoc) | PASS |
| epub -> docx, prose: throughput (MB/s of uncompressed input) [extra] | 47.5 | 0.1 (pandoc) | n/a |
| epub -> docx, prose: peak memory (MB) | 43.0 | 459.8 (pandoc) | PASS |

commit: 78dfbdf plus the uncommitted `epub` branch
machine: Darwin 24.5.0 arm64, 10 cpus, Apple M1 Max

## Conclusions

Both directions pass with a wide margin and hold under 44 MB, inside the
64 MB the Pages and Word pairs keep to. Writing a book is slower than
writing ODT (60 to 84 MB/s against 161 to 169); it has not been profiled
yet, and is the lever if this pair needs one. Pandoc is slowest
reading a book (0.2 MB/s), which ours reads at 39 to 48 MB/s of input.
Re-run on the Linux bench machine for the record beside the other pairs.
