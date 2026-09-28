# HTML -> Markdown

**Latest** (2026-09-25, 0.17.0 Markdown speedups: html -> markdown 167.4 MB/s of input on the markup-dense shape and 234.9 on prose, at 29.6 and 15.0 MB peak; every line PASSES against htmd, whose throughput rounds to zero at one decimal)

## Purpose

HTML read into the Markdown event stream by the tag-soup reader
(`src/io/html/reader.rs`) and written as Markdown. It measures the
reader itself with the lightest structured writer behind it; the text
and Word pairs share the reader.

## Reference

`htmd`, an HTML-to-Markdown converter in Rust built on html5ever (a full HTML5
parser building a DOM). It is the closest peer available: the same job done the
conventional way, so the comparison is real rather than a goal. Implemented in
`bench/src/pairs/html_markdown.rs`.

## Pass lines

Not slower, and no more memory, than `htmd`, an HTML to Markdown
converter in Rust built on html5ever (a full HTML5 parser building a
DOM). It is the closest peer: the same job, done the conventional way.

## Method

**Machine.** Recorded with each results block from the harness's
machine line.

**Inputs.** The `markdown-html` generator's two shapes (`markdown-html.md`,
Method) written to HTML by our own writer, capped at 20,000 units:
`markup-dense` (26.6 MB of HTML: headings, lists, code, quotes, tables,
links, footnotes) and `prose` (12.1 MB: long paragraphs with the odd
emphasis and link). The cap is the reference's: on 100,000 units it held
2.5 GB and had not finished after twenty minutes.

**Statistics.** `bench/run.sh html-markdown`: three runs per command,
median wall clock of the whole process, peak resident memory from GNU
`time`, throughput in MB/s over the input file's bytes. Rows and units
follow `README.md`.

**Commands.** Per shape:

- ours: `sublime -q convert big.html big.md --to markdown`
- reference: `sublime-bench html-markdown crates big.html out.md`
  (`htmd::convert`)

## Threats to validity

- The inputs are our own writer's HTML: well formed, no CSS, no scripts,
  no deeply nested layout. A real page has more markup per word of
  content, and both readers would spend more time in tags; the reader's
  tag path is a byte scan and a name match, so the ratio should hold.
- `htmd` builds a DOM and walks it; its numbers say what a parser-first
  design costs, not that it is slow at what it does.

## Results

### 2026-09-25, 0.17.0 Markdown speedups

commit: 0b33385 (the merge of the rows-to-document bridge, before the version bump)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

| Target | Ours | Reference | Result |
|---|---|---|---|
| html -> markdown, markup-dense (26.6 MB): throughput (MB/s of input) | 167.4 | 0.1 (htmd) | PASS |
| html -> markdown, markup-dense: peak memory (MB) | 29.6 | 550.8 (htmd) | PASS |
| html -> markdown, prose (12.1 MB): throughput (MB/s of input) | 234.9 | 0.4 (htmd) | PASS |
| html -> markdown, prose: peak memory (MB) | 15.0 | 139.4 (htmd) | PASS |

Recorded at the 0.17.0 release because the Markdown writer (text
copied in runs, digits scanned only where a list could open), the block
parser (one cell vector per table), and inline rendering (a plain-text
fast path) changed for the rows-to-document bridge. The 4.7 MB prose
inputs run in about fifteen milliseconds, so their throughput medians
swing between runs; the dense rows are the stable ones.

### 2026-09-24, first release of the HTML reader

commit: 34a9e90 (the reader's working tree, before its commit)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

| Target | Ours | Reference | Result |
|---|---|---|---|
| html -> markdown, markup-dense (26.6 MB): throughput (MB/s of input) | 166.3 | 0 (htmd) | PASS |
| html -> markdown, markup-dense: peak memory (MB) | 29.7 | 550.2 (htmd) | PASS |
| html -> markdown, prose (12.1 MB): throughput (MB/s of input) | 227.7 | .4 (htmd) | PASS |
| html -> markdown, prose: peak memory (MB) | 15.3 | 138.0 (htmd) | PASS |

The reference's throughput rounds to zero at one decimal: it took
minutes per run on the dense shape and about half a minute on prose.

## Conclusions

The reader runs at the Markdown parser's pace and holds little beyond
the input and the writer's buffers: it never builds a tree, borrows text
from the source in the longest runs whose whitespace is already single
spaces, and decodes only the references it meets. Nothing here is close
to a limit; the next cost to look at, if one ever matters, is the
per-tag attribute vector.
