# Markdown -> plain text

**Latest** (2026-10-21, 1.0.0 release run on the Apple M1 Max: markdown -> text 99.3 MB/s of input on the markup-dense shape (73.8 MB) and 272.9 on prose (56.6 MB), at 352.2 and 83.8 MB peak; all lines PASS against a pulldown-cmark text dump at 5.3 and 253.0)

## Purpose

The first lossy document path and the first renderer added on top of the
existing Markdown event stream. It exists so the event stream is proven as
the shared front end for every Markdown output, and because "just give me
the text" is a daily ask.

## Reference

A bare text dump of `pulldown-cmark` events (text and code concatenated, a
newline per block end). This is a floor, not a peer: no widely used
Markdown-to-text crate exists, so the reference does strictly less than our
renderer (no list markers, aligned tables, link URLs, or footnotes) and only
bounds the parse-and-render cost from below. Implemented in
`bench/src/pairs/markdown_text.rs`.

## Pass lines

| Target | Pass line |
|---|---|
| Markdown -> text throughput | >= a bare text dump of `pulldown-cmark` events (text and code concatenated, a newline per block end) |
| Peak resident memory | <= the same reference on the same input |

The reference does strictly less than our renderer: no list markers,
indentation, aligned tables, link URLs, or footnote numbering. It is a floor
for the parse-and-render cost, not a peer implementation.

## Method

**Machine, inputs, statistics, and reference options** are those of the
`markdown-html` pair: the markup-dense and prose documents at 100,000
units, three runs each, median wall clock, peak RSS from GNU `time`.

**Commands.** `bench/run.sh markdown-text` runs:

- ours: `sublime -q convert big.md out.txt` (and `prose.md`)
- reference: `sublime-bench markdown-text crates big.md out.txt`

## Threats to validity

- The reference is a floor, as said above; being faster than it means the
  extra rendering work is free relative to parsing, not that we beat a
  comparable renderer (there is no widely used Markdown-to-text crate).
- The reference's footnote handling is quadratic in the number of
  footnotes, which dominates it on the dense input at this size (see the
  `markdown-html` document). The prose input has no footnotes.

## Results

### 2026-10-21, 1.0.0 release run (`v1-wrapup` branch)

commit: ec68950 (on the `v1-wrapup` branch, before the 1.0.0 release)
machine: Darwin 24.5.0 arm64, 10 cpus, Apple M1 Max

| Target | Ours | Reference | Result |
|---|---|---|---|
| markdown -> text, markup-dense (73.8 MB): throughput (MB/s of input) | 99.3 | 5.3 | PASS |
| markdown -> text, prose (56.6 MB): throughput (MB/s of input) | 272.9 | 253.0 | PASS |
| markdown -> text, markup-dense: peak memory (MB) | 352.2 | 733.2 (reference) | PASS |
| markdown -> text, prose: peak memory (MB) | 83.8 | 316.6 (reference) | PASS |

Every line passes. The first attempt at this run read stale, smaller
inputs left in `bench/data` (2.2 MB dense, 1.7 MB prose: the scripts
generate only when the file is missing, and several pairs share the
names) and failed prose throughput on them (88.2 against 98.7); they
were deleted, regenerated at the documented sizes, and the pair rerun.
The machine was in use during the rerun, more heavily than during the
rest of this run (load average 16 to 26 on 10 cores: a browser, a VPN
client, a remote-desktop host), so the wall-clock rows are noisier than
earlier blocks'. The machine is the Apple M1 Max, not the Linux one of
the block below, and the prose input is the full 56.6 MB rather than
that block's 4.7 MB, so the rows do not compare with it; on prose we
are 8% ahead of the reference, against 2% below.

### 2026-09-25, 0.17.0 Markdown speedups

commit: 0b33385 (the merge of the rows-to-document bridge, before the version bump)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

| Target | Ours | Reference | Result |
|---|---|---|---|
| markdown -> text, markup-dense (73.8 MB): throughput (MB/s of input) | 118.9 | 4.1 | PASS |
| markdown -> text, prose (4.7 MB): throughput (MB/s of input) | 230.4 | 226.0 | PASS |
| markdown -> text, markup-dense: peak memory (MB) | 306.0 | 617.3 (reference) | PASS |
| markdown -> text, prose: peak memory (MB) | 10.0 | 15.9 (reference) | PASS |

Recorded at the 0.17.0 release because the Markdown writer (text
copied in runs, digits scanned only where a list could open), the block
parser (one cell vector per table), and inline rendering (a plain-text
fast path) changed for the rows-to-document bridge. The 4.7 MB prose
inputs run in about fifteen milliseconds, so their throughput medians
swing between runs; the dense rows are the stable ones.

2026-09-23, 0.3.0. Medians of three:

| Target | Ours | Reference | Result |
|---|---|---|---|
| Markdown -> text throughput, markup-dense, 86 MB (MB/s) | 108.9 | 4.1 | PASS |
| Markdown -> text throughput, prose, 59 MB (MB/s) | 333.6 | 325.5 | PASS |
| Peak RSS, markup-dense (MB) | 304.0 | 614.6 | PASS |
| Peak RSS, prose (MB) | 83.0 | 223.4 | PASS |

**Size cost.** The text renderer, the Markdown writer, the JSON event
reader and writer, and their three converters together added 86 KB to the
release binary (756 KB to 842 KB, glibc build); the size budget moved to
900,000.

## Conclusions

- On prose we render structured text (markers, aligned tables, link URLs,
  numbered footnotes) slightly faster than the reference dumps bare text,
  at 37% of its memory. The renderer's own work is not measurable next to
  parsing.
- The dense number is the reference's footnote cost, not a renderer
  comparison; the `markdown-html` document has the like-for-like figure
  for the parser.
