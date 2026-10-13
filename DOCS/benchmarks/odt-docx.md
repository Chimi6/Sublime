# ODT <-> Word

**Latest** (2026-10-13, the `odt` branch: docx -> odt at 161.4 MB/s of input plus output on the styled shape and 168.6 on prose, odt -> docx at 192.7 and 243.4; every line PASSES against pandoc at 77 to 122 times its speed and a fifteenth to a thirty-first of its memory)

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
