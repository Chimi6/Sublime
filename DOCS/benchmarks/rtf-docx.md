# RTF <-> Word

**Latest** (2026-10-06, 0.26.0 with RTF: docx -> rtf at 91.8 MB/s on the styled shape and 130.6 on prose, rtf -> docx at 184.7 and 251.4 MB/s of input plus uncompressed output, every line PASSES against pandoc at 25 to 60 times pandoc's speed and a twentieth of its memory)

## Purpose

RTF read into the document model and written as Word, and Word read into
the model and written as RTF (`DOCS/formats/rtf.md`). A pair is
unordered: both directions share the inputs, and each direction has its own
rows.

## Reference

`pandoc` reads and writes both RTF and Word and is the reference, run as an
external process on the same files (`bench/pairs/rtf-docx.sh`). No Rust
crate converts between them. Pandoc's RTF reader and writer keep less than
ours (no list tables, merged cells, or section layout), so it is a loose
upper bound, but a real tool rather than a goal.

## Pass lines

Not slower, and no more memory, than `pandoc` on the same input. Word is a
compressed package, so `docx -> rtf` is timed on the uncompressed bytes the
reader parses, and `rtf -> docx` on the input plus the output's
uncompressed bytes, the compressor's work (`README.md`); the rate per input
byte is an extra row.

## Method

**Machine.** Recorded with each results block from the harness's
machine line.

**Inputs.** The Pages pair's generated documents (`pages-json.md`, Method)
written to Word by our Word writer and to RTF by our RTF writer: 6.0 and
7.5 MB of uncompressed Word, 4.6 and 7.7 MB of RTF.

**Statistics.** `bench/run.sh rtf-docx`: three runs per command, median
wall clock of the whole process, peak resident memory from GNU `time`.
Rows and units follow `README.md`.

**Commands.** Per shape:

- ours: `sublime -q convert styled.docx out-styled.rtf --to rtf` and
  `sublime -q convert styled.rtf out-styled-from-rtf.docx --to docx`
- pandoc: `pandoc -f docx -t rtf -s styled.docx -o ...` and
  `pandoc -f rtf -t docx styled.rtf -o ...`

## Threats to validity

The inputs are our own writers' output: real RTF from Word carries far
more control words per character (every run restates its fonts, the
associated fonts, and its language), and real Word files larger style
sheets, so both directions will run somewhat slower on them. The RTF input
is the RTF our writer makes, which pandoc reads in full.

## Results

### 2026-10-06, 0.26.0 with RTF

| Target | Ours | Reference | Result |
|---|---|---|---|
| docx -> rtf, styled (6.0 MB uncompressed): throughput (MB/s of uncompressed input) | 91.8 | 1.6 (pandoc) | PASS |
| docx -> rtf, styled: peak memory (MB) | 24.7 | 566.8 (pandoc) | PASS |
| rtf -> docx, styled (4.6 MB in + 9.7 MB out): throughput (MB/s of input plus uncompressed output) | 184.7 | 4.6 (pandoc) | PASS |
| rtf -> docx, styled: throughput (MB/s of input) [extra] | 59.6 | 1.9 (pandoc) | n/a |
| rtf -> docx, styled: peak memory (MB) | 21.7 | 382.9 (pandoc) | PASS |
| docx -> rtf, prose (7.5 MB uncompressed): throughput (MB/s of uncompressed input) | 130.6 | 1.7 (pandoc) | PASS |
| docx -> rtf, prose: peak memory (MB) | 30.8 | 775.8 (pandoc) | PASS |
| rtf -> docx, prose (7.7 MB in + 11.9 MB out): throughput (MB/s of input plus uncompressed output) | 251.4 | 5.1 (pandoc) | PASS |
| rtf -> docx, prose: throughput (MB/s of input) [extra] | 98.6 | 2.6 (pandoc) | n/a |
| rtf -> docx, prose: peak memory (MB) | 27.2 | 435.6 (pandoc) | PASS |

commit: 2de5881 (the `rtf` branch)
machine: Darwin 24.5.0 arm64, 10 cpus, Apple M1 Max

## Conclusions

Both directions pass with a wide margin and hold under 31 MB on these
shapes, inside the 64 MB the Pages and Word pairs keep to. The writer is
the slower side: `docx -> rtf` escapes every character outside ASCII and
restates each run's formatting, which is what RTF readers expect. Re-run on
the Linux bench machine for the record beside the other pairs.
