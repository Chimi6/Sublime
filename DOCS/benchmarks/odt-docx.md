# ODT <-> Word

**Latest** (2026-10-21, the 1.0.0 release run: docx -> odt at 176.4 MB/s of input plus output on the styled shape and 171.4 on prose, odt -> docx at 181.1 and 231.5; every line PASSES against pandoc at 88 to 122 times its speed and a sixteenth to a thirtieth of its memory)

## Purpose

OpenDocument Text read into the document model and written as Word, and
Word read into the model and written as ODT (`DOCS/formats/odt.md`). A
pair is unordered: both directions share the inputs, and each direction
has its own rows.

## Reference

`pandoc` reads and writes both ODT and Word and is the reference, run as an
external process on the same files (`bench/pairs/odt-docx.sh`). No Rust
crate converts between them. Pandoc keeps less than ours (no page setup,
headers, merged cells, or comments in ODT), so it is a loose upper bound,
but a real tool rather than a goal. LibreOffice converts both ways with
more fidelity, but as a desktop suite started per file it is not a fair
speed line.

## Pass lines

Not slower, and no more memory, than `pandoc` on the same input. Both
formats are compressed packages, so each direction is timed on the
uncompressed bytes the reader parses plus the uncompressed bytes the writer
compresses (`README.md`); the rate per input byte is an extra row.

## Method

**Machine.** Recorded with each results block from the harness's
machine line.

**Inputs.** The Pages pair's generated documents (`pages-json.md`, Method)
written to Word by our Word writer and to ODT by our ODT writer.

**Statistics.** `bench/run.sh odt-docx`: three runs per command, median
wall clock of the whole process, peak resident memory from GNU `time`.
Rows and units follow `README.md`.

**Commands.** Per shape:

- ours: `sublime -q convert styled.docx out-styled.odt --to odt` and
  `sublime -q convert styled.odt out-styled-from-odt.docx --to docx`
- pandoc: `pandoc -f docx -t odt styled.docx -o ...` and
  `pandoc -f odt -t docx styled.odt -o ...`

## Threats to validity

The inputs are our own writers' output. LibreOffice's ODT names more
automatic styles and settings per paragraph, and real Word files carry
larger style sheets, so both directions will run somewhat slower on them.

## Results

### 2026-10-21, the `v1-wrapup` branch, the 1.0.0 release run

| Target | Ours | Reference | Result |
|---|---|---|---|
| docx -> odt, styled (6.0 MB in + 3.8 MB out, uncompressed): throughput (MB/s of input plus output) | 176.4 | 2.0 (pandoc) | PASS |
| docx -> odt, styled: throughput (MB/s of uncompressed input) [extra] | 108.1 | 1.2 (pandoc) | n/a |
| docx -> odt, styled: peak memory (MB) | 33.0 | 619.1 (pandoc) | PASS |
| odt -> docx, styled (3.8 MB in + 5.9 MB out, uncompressed): throughput (MB/s of input plus output) | 181.1 | 1.5 (pandoc) | PASS |
| odt -> docx, styled: throughput (MB/s of uncompressed input) [extra] | 71.0 | 0.6 (pandoc) | n/a |
| odt -> docx, styled: peak memory (MB) | 21.8 | 371.5 (pandoc) | PASS |
| docx -> odt, prose (7.5 MB in + 5.7 MB out, uncompressed): throughput (MB/s of input plus output) | 171.4 | 1.6 (pandoc) | PASS |
| docx -> odt, prose: throughput (MB/s of uncompressed input) [extra] | 97.5 | 0.9 (pandoc) | n/a |
| docx -> odt, prose: peak memory (MB) | 49.0 | 1503.4 (pandoc) | PASS |
| odt -> docx, prose (5.7 MB in + 7.2 MB out, uncompressed): throughput (MB/s of input plus output) | 231.5 | 1.9 (pandoc) | PASS |
| odt -> docx, prose: throughput (MB/s of uncompressed input) [extra] | 102.9 | 0.8 (pandoc) | n/a |
| odt -> docx, prose: peak memory (MB) | 34.3 | 561.1 (pandoc) | PASS |

commit: ec68950
machine: Darwin 24.5.0 arm64, 10 cpus, Apple M1 Max

The machine was in use during this run (load average 7 to 12 on 10
cores: a browser, a VPN client, a remote-desktop host), so the
wall-clock rows are noisier than the previous block's. Every line still
passes. The rates moved by up to 10% either way against 2026-10-13
(`docx -> odt` styled 161.4 to 176.4, `odt -> docx` styled 192.7 to
181.1), and peak memory by up to 3.1 MB either way, the most still `docx -> odt` on
prose at 49.0 MB.

### 2026-10-13, the `odt` branch

| Target | Ours | Reference | Result |
|---|---|---|---|
| docx -> odt, styled (6.0 MB in + 3.8 MB out, uncompressed): throughput (MB/s of input plus output) | 161.4 | 2.1 (pandoc) | PASS |
| docx -> odt, styled: throughput (MB/s of uncompressed input) [extra] | 98.8 | 1.3 (pandoc) | n/a |
| docx -> odt, styled: peak memory (MB) | 29.9 | 618.5 (pandoc) | PASS |
| odt -> docx, styled (3.8 MB in + 5.9 MB out, uncompressed): throughput (MB/s of input plus output) | 192.7 | 1.6 (pandoc) | PASS |
| odt -> docx, styled: throughput (MB/s of uncompressed input) [extra] | 75.5 | 0.7 (pandoc) | n/a |
| odt -> docx, styled: peak memory (MB) | 24.3 | 371.1 (pandoc) | PASS |
| docx -> odt, prose (7.5 MB in + 5.7 MB out, uncompressed): throughput (MB/s of input plus output) | 168.6 | 1.8 (pandoc) | PASS |
| docx -> odt, prose: throughput (MB/s of uncompressed input) [extra] | 95.8 | 1.0 (pandoc) | n/a |
| docx -> odt, prose: peak memory (MB) | 48.8 | 1503.8 (pandoc) | PASS |
| odt -> docx, prose (5.7 MB in + 7.2 MB out, uncompressed): throughput (MB/s of input plus output) | 243.4 | 2.0 (pandoc) | PASS |
| odt -> docx, prose: throughput (MB/s of uncompressed input) [extra] | 108.2 | 0.9 (pandoc) | n/a |
| odt -> docx, prose: peak memory (MB) | 34.3 | 560.0 (pandoc) | PASS |

commit: a26e4bc plus the uncommitted `odt` branch
machine: Darwin 24.5.0 arm64, 10 cpus, Apple M1 Max

## Conclusions

Both directions pass with a wide margin and hold under 49 MB, inside the
64 MB the Pages and Word pairs keep to; the most is `docx -> odt` on the
prose shape. The ODT writer builds the body before the styles part (which
names every automatic style the body used), so it holds the body until
the end; a streaming writer would need the styles first. Re-run on the
Linux bench machine for the record beside the other pairs.

2026-10-21, the 1.0.0 release run: `docx -> odt` on prose peaked at
49.0 MB, at the 49 MB mark rather than under it; every line still
passes, inside the 64 MB.
