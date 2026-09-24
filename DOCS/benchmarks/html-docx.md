# HTML -> Word

**Latest** (2026-09-24, first release of the HTML reader, with the streaming bridge: html -> docx 217.8 MB/s of input plus uncompressed output on the markup-dense shape and 288.9 on prose, at 57.8 and 30.9 MB peak; every line PASSES)

## Purpose

HTML read into the Markdown event stream, built into the document model
by the events bridge one block at a time, and streamed out as a Word
package. It is the path from a web page or an exported document into
Word, and the heaviest thing the bridge does.

## Reference

A stated goal today (`pages-json.md`), with both stages peer-checked in sibling
pairs: the HTML *reader* against `htmd` in `html-markdown.md`, and the Word
*writer* against `docx-rs` in `markdown-docx.md`. No Rust crate does HTML to
Word end to end, but `pandoc` does and should be wired in as an external
reference — a loose upper bound, since it does more, but a real number rather
than a goal. Until it is, the goal stands and this note keeps the gap visible.
Implemented in `bench/src/pairs/html_docx.rs`.

## Pass lines

Throughput >= 50 MB/s of input plus uncompressed output and peak memory
<= 64 MB on the benchmark inputs: the goals the document paths share
(`pages-json.md`), with the standard's compressed-output measure
(`README.md`). No peer converts HTML to Word in Rust; the bridge's own
peer reference is in `markdown-docx.md`.

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
