# Markdown -> Word

**Latest** (2026-09-25, 0.17.0 Markdown speedups: markdown -> docx 206.7 MB/s of input plus uncompressed output on the markup-dense shape and 288.8 on prose, at 92 and 35 MB peak; all lines PASS against pulldown-cmark with docx-rs)

## Purpose

Markdown parsed into events, built into the document model by the events
bridge (`src/document/from_events.rs`), and written as a Word package.
It is the first path into Word from a text format, and the bridge it
measures will carry HTML and plain text into Word as well.

## Reference

`pulldown-cmark` (the Rust Markdown parser, with tables, footnotes,
strikethrough, and task lists enabled) feeding `docx-rs` (a Rust Word writer)
with paragraphs and runs. The reference does less than we do (no lists, tables,
links, or footnotes), so it bounds from below what a conventional Rust pipeline
to Word costs. Implemented in `bench/src/pairs/markdown_docx.rs`.

## Pass lines

Not slower, and no more memory, than a reference pipeline of
`pulldown-cmark` (the Rust Markdown parser, with tables, footnotes,
strikethrough, and task lists enabled) feeding `docx-rs` (a Rust Word
writer) with paragraphs and runs: headings by size, bold and italic, code
as text, list items as paragraphs, and nothing else. The reference does
less than we do (no styles, lists, tables, links, or footnotes), so it
bounds from below what a Rust pipeline to Word costs. Word is a
compressed package, so throughput counts the input plus the output's
uncompressed bytes (`README.md`).

## Method

**Machine.** Recorded with each results block from the harness's
machine line.

**Inputs.** The `markdown-html` pair's generator (`markdown-html.md`,
Method), capped at 20,000 units since the output is measured
uncompressed: `markup-dense` (headings, lists, code, quotes, tables,
links, footnotes; 14.6 MB) and `prose` (long paragraphs with the odd
emphasis and link; 11.3 MB).

**Statistics.** `bench/run.sh markdown-docx`: three runs per command,
median wall clock of the whole process, peak resident memory from GNU
`time`, throughput over the input file plus the output package's
uncompressed bytes (`unzip -l`), with the rate per input byte as an extra
row. Rows and units follow `README.md`.

**Commands.** Per shape:

- ours: `sublime -q convert big.md big.docx --to docx`
- reference: `sublime-bench markdown-docx crates big.md out.docx`
  (`bench/src/pairs/markdown_docx.rs`)

## Threats to validity

- The reference writes far less structure, so its output is smaller and
  its throughput over "bytes handled" is understated relative to ours;
  the per-input-byte extra row is the like-for-like rate, and we lead on
  it too (20.9 against 8.3, 77.7 against 27.0).
- The dense shape's Word XML is seven times its Markdown (every list item
  and table cell is a paragraph with properties), which is why MB/s of
  input reads low on it; the bytes-handled row is the honest one.
- Peak memory on the dense shape is 360 MB: the model holds every run of
  a 14.6 MB document (about a million runs with their properties) before
  the writer streams the body. The pass line is the reference, which
  holds four times more; a 64 MB goal like the Pages pairs' would fail
  here, and the lever is a streaming bridge that writes each top-level
  block as it closes.

## Results

### 2026-09-25, 0.17.0 Markdown speedups

commit: 0b33385 (the merge of the rows-to-document bridge, before the version bump)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

| Target | Ours | Reference | Result |
|---|---|---|---|
| markdown -> docx, markup-dense (14.7 MB in + 90.7 MB out): throughput (MB/s of input plus uncompressed output) | 206.7 | 45.7 (pulldown-cmark + docx-rs, paragraphs and runs only) | PASS |
| markdown -> docx, markup-dense: throughput (MB/s of input) [extra] | 28.8 | 8.4 (reference) | n/a |
| markdown -> docx, markup-dense: peak memory (MB) | 91.9 | 1580.9 (reference) | PASS |
| markdown -> docx, prose (11.3 MB in + 20.2 MB out): throughput (MB/s of input plus uncompressed output) | 288.8 | 90.1 (pulldown-cmark + docx-rs, paragraphs and runs only) | PASS |
| markdown -> docx, prose: throughput (MB/s of input) [extra] | 103.6 | 26.9 (reference) | n/a |
| markdown -> docx, prose: peak memory (MB) | 35.0 | 444.9 (reference) | PASS |

Recorded at the 0.17.0 release because the Markdown writer (text
copied in runs, digits scanned only where a list could open), the block
parser (one cell vector per table), and inline rendering (a plain-text
fast path) changed for the rows-to-document bridge. The 4.7 MB prose
inputs run in about fifteen milliseconds, so their throughput medians
swing between runs; the dense rows are the stable ones.

### 2026-09-24, the streaming bridge

commit: 34a9e90 (the working tree, before its commit)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

| Target | Ours | Reference | Result |
|---|---|---|---|
| markdown -> docx, markup-dense (14.6 MB in + 90.7 MB out): throughput (MB/s of input plus uncompressed output) | 204.8 | 46.0 (pulldown-cmark + docx-rs, paragraphs and runs only) | PASS |
| markdown -> docx, markup-dense: throughput (MB/s of input) [extra] | 28.4 | 8.4 (reference) | n/a |
| markdown -> docx, markup-dense: peak memory (MB) | 91.5 | 1579.8 (reference) | PASS |
| markdown -> docx, prose (11.3 MB in + 20.2 MB out): throughput (MB/s of input plus uncompressed output) | 291.4 | 90.7 (pulldown-cmark + docx-rs, paragraphs and runs only) | PASS |
| markdown -> docx, prose: throughput (MB/s of input) [extra] | 104.6 | 27.1 (reference) | n/a |
| markdown -> docx, prose: peak memory (MB) | 34.8 | 445.1 (reference) | PASS |

The bridge now hands each top-level block to the Word writer as it
closes; links are HYPERLINK fields rather than relationships; and
`numbering.xml` streams into the package. Dense peak memory 360 -> 92 MB
(the Markdown parser's block tree is what remains above the HTML path's
58 MB), prose 64 -> 35; throughput up a fifth. The output is 12 MB
smaller uncompressed without the relationship ids.

### 2026-09-24, first release of the bridge

commit: e25c786 (the bridge's working tree, before its commit)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

| Target | Ours | Reference | Result |
|---|---|---|---|
| markdown -> docx, markup-dense (14.6 MB in + 102.8 MB out): throughput (MB/s of input plus uncompressed output) | 167.7 | 45.6 (pulldown-cmark + docx-rs, paragraphs and runs only) | PASS |
| markdown -> docx, markup-dense: throughput (MB/s of input) [extra] | 20.9 | 8.3 (reference) | n/a |
| markdown -> docx, markup-dense: peak memory (MB) | 360.1 | 1579.3 (reference) | PASS |
| markdown -> docx, prose (11.3 MB in + 22.6 MB out): throughput (MB/s of input plus uncompressed output) | 233.1 | 90.5 (pulldown-cmark + docx-rs, paragraphs and runs only) | PASS |
| markdown -> docx, prose: throughput (MB/s of input) [extra] | 77.7 | 27.0 (reference) | n/a |
| markdown -> docx, prose: peak memory (MB) | 63.7 | 443.9 (reference) | PASS |

## Conclusions

Both shapes pass both lines with room. Prose runs at the Word writer's
pace; the dense shape spends its time in the writer's paragraph and list
machinery. Memory is now the input, the parser's block tree, the text
arena, and the link table; the whole document is never held.
