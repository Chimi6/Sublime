# Markdown -> plain text

## Purpose

The first lossy document path and the first renderer added on top of the
existing Markdown event stream. It exists so the event stream is proven as
the shared front end for every Markdown output, and because "just give me
the text" is a daily ask.

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
