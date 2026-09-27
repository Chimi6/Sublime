# HTML -> Word

**Latest** (2026-09-27, the lazy deflate and the faster bit writer: 332.4 MB/s on the markup-dense shape (from 217.8) and 423.7 on prose (from 288.9); every line PASSES)

## Purpose

HTML read into the Markdown event stream, built into the document model
by the events bridge one block at a time, and streamed out as a Word
package. It is the path from a web page or an exported document into
Word, and the heaviest thing the bridge does.

## Reference

`pandoc` converts HTML to docx and is the reference, run as an external process
and timed on the same workload (input plus uncompressed output) as ours
(`bench/pairs/html-docx.sh`). No Rust crate does HTML to Word end to end; pandoc
does full-fidelity conversion with a Haskell runtime, so it is a loose upper
bound — we are much faster — but a real tool rather than a goal. Both stages are
additionally peer-checked in sibling pairs: the HTML *reader* against `htmd` in
`html-markdown.md`, and the Word *writer* against `docx-rs` in `markdown-docx.md`.

## Pass lines

Not slower, and no more memory, than `pandoc` converting the same HTML to Word,
timed on input plus the output's uncompressed bytes (`README.md`). Pandoc does
full-fidelity conversion, so it is a loose upper bound.

## Method

**Machine.** Recorded with each results block from the harness's
machine line.

**Inputs.** As `html-markdown.md`: the `markdown-html` generator's two
shapes written to HTML by our own writer, 20,000 units, 26.6 and 12.1
MB. The dense shape holds 100,000 links, 60,000 lists, and 1,000
footnotes.

**Statistics.** `bench/run.sh html-docx`: three runs per command, median
wall clock of the whole process, peak resident memory from GNU `time`,
throughput over the input file plus the output package's uncompressed
bytes (`unzip -l`), with the rate per input byte as an extra row.

**Commands.** Per shape: `sublime -q convert big.html big.docx --to docx`.

## Threats to validity

- The dense shape is harder than most pages on links and lists and
  easier on markup depth; memory scales with the input and the link
  table, throughput with the output's size.
- Word's XML for lists and tables is large (90 MB from 26.6 MB of HTML),
  which is why the per-input-byte row reads low; the bytes-handled row is
  the honest one.

## Results

### 2026-09-27, the lazy deflate and the faster bit writer

commit: a5c702a (on the `webp-spikes` branch, before its merge)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

| Target | Ours | Reference | Result |
|---|---|---|---|
| html -> docx, markup-dense (26.6 MB in + 90.7 MB out): throughput (MB/s of input plus uncompressed output) | 327.7 | 1.6 (pandoc) | PASS |
| html -> docx, markup-dense: throughput (MB/s of input) [extra] | 74.4 | recorded | n/a |
| html -> docx, markup-dense: peak memory (MB) | 57.9 | 11670.6 (pandoc) | PASS |
| html -> docx, prose (12.1 MB in + 20.2 MB out): throughput (MB/s of input plus uncompressed output) | 427.0 | 3.3 (pandoc) | PASS |
| html -> docx, prose: throughput (MB/s of input) [extra] | 160.0 | recorded | n/a |
| html -> docx, prose: peak memory (MB) | 30.4 | 1433.1 (pandoc) | PASS |

### 2026-09-24, pandoc reference wired

commit: 64b08e7 (the reference wiring's working tree, before its commit)
machine: Darwin 24.5.0 arm64, 10 cpus, Apple M1 Max

At 20,000 units pandoc's html -> docx needed about 15 GB of RSS and its memory
measurement was unstable, so this block uses 4,000 units, where pandoc stays a
few GB. Throughput counts input plus uncompressed output (`README.md`).

| Target | Ours | Reference | Result |
|---|---|---|---|
| html -> docx, markup-dense (5.3 MB in + 18.1 MB out): throughput (MB/s of input plus uncompressed output) | 153.0 | 2.2 (pandoc) | PASS |
| html -> docx, markup-dense: throughput (MB/s of input) [extra] | 34.6 | recorded | n/a |
| html -> docx, markup-dense: peak memory (MB) | 24.2 | 2782.8 (pandoc) | PASS |
| html -> docx, prose (2.4 MB in + 4.0 MB out): throughput (MB/s of input plus uncompressed output) | 150.1 | 2.5 (pandoc) | PASS |
| html -> docx, prose: throughput (MB/s of input) [extra] | 56.2 | recorded | n/a |
| html -> docx, prose: peak memory (MB) | 12.8 | 327.9 (pandoc) | PASS |

### 2026-09-24, first release of the HTML reader, with the streaming bridge

commit: 34a9e90 (the reader's working tree, before its commit)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

| Target | Ours | Reference | Result |
|---|---|---|---|
| html -> docx, markup-dense (26.6 MB in + 90.7 MB out): throughput (MB/s of input plus uncompressed output) | 217.8 | goal: 50 | PASS |
| html -> docx, markup-dense: throughput (MB/s of input) [extra] | 49.4 | recorded | n/a |
| html -> docx, markup-dense: peak memory (MB) | 57.8 | goal: <= 64.0 | PASS |
| html -> docx, prose (12.1 MB in + 20.2 MB out): throughput (MB/s of input plus uncompressed output) | 288.9 | goal: 50 | PASS |
| html -> docx, prose: throughput (MB/s of input) [extra] | 108.2 | recorded | n/a |
| html -> docx, prose: peak memory (MB) | 30.9 | goal: <= 64.0 | PASS |

Before the streaming bridge, the same run held 365 MB on the dense
shape and 65.7 on prose; with it, 156 and 51; with links as HYPERLINK
fields instead of relationships, and `numbering.xml` compressed in
parts, 58 and 31.

## Conclusions

Both shapes pass both lines. What remains at peak is the input held
whole (the reader borrows it), the text arena's copy of it, and the
table of unique link targets: about twice the input on the dense shape.
A reader that streamed its input would halve that; nothing needs it.
