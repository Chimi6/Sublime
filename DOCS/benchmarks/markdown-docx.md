# Markdown -> Word

**Latest** (2026-09-24, first release of the bridge: markdown -> docx 167.7 MB/s of input plus uncompressed output on the markup-dense shape and 233.1 on prose, at 360.1 and 63.7 MB peak; every line PASSES against pulldown-cmark feeding docx-rs, at 3.7 and 2.6 times its throughput and a quarter of its memory)

## Purpose

Markdown parsed into events, built into the document model by the events
bridge (`src/document/from_events.rs`), and written as a Word package.
It is the first path into Word from a text format, and the bridge it
measures will carry HTML and plain text into Word as well.

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
pace; the dense shape spends its time in the writer's paragraph and
list machinery (a numbering instance per list, a character style per
formatting triple) and holds a large model. If memory on very large
dense inputs ever matters, the bridge can hand each top-level block to
the writer as it closes instead of building the whole document first;
the writer already streams its body, so the change is in the bridge.
