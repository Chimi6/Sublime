# HTML -> text

**Latest** (2026-09-24, references wired on an Apple M1 Max: html -> text 148.8 MB/s of input on the markup-dense shape and 271.3 on prose, at 29.5 and 14.2 MB peak, faster and far leaner than html2text and orders ahead of pandoc; every line PASSES)

## Purpose

HTML read into the Markdown event stream and written as plain text: the
cheapest path out of a page, and the one a search index or a summary
wants.

## Reference

Two references, in two columns. `html2text`, a pure-Rust HTML-to-text renderer
on html5ever (no browser engine), is the tight peer and the pass gate — it does
essentially this job (wrapped text, tables, links) and compiles into the bench
binary via `bench/src/pairs/html_text.rs`. `pandoc`, a general-purpose document
converter, runs as an external process for real-world context; it does far more
than we do (full document model, Haskell runtime), so it is a loose reference,
not the gate. The HTML *reader* is additionally peer-checked against `htmd` in
`html-markdown.md`.

## Pass lines

Throughput >= 50 MB/s of input and peak memory <= 64 MB on the benchmark
inputs: the goals the document paths share (`pages-json.md`). No peer
converts HTML to text in Rust without a browser engine; the reader's own
peer reference is in `html-markdown.md`.

## Method

As `html-markdown.md`: the same inputs (26.6 and 12.1 MB), the same
statistics, `bench/run.sh html-text`, and per shape the command
`sublime -q convert big.html big.txt --to text`.

## Threats to validity

As `html-markdown.md`: our own writer's HTML is cleaner than a real page.

## Results

### 2026-09-24, references wired: html2text and pandoc

commit: 64b08e7 (the reference wiring's working tree, before its commit)
machine: Darwin 24.5.0 arm64, 10 cpus, Apple M1 Max

| Target | Ours | html2text | pandoc | Result |
|---|---|---|---|---|
| html -> text, markup-dense (26.6 MB): throughput (MB/s of input) | 148.8 | 15.8 | 0.7 | PASS |
| html -> text, markup-dense: peak memory (MB) | 29.5 | 982.2 | 7605.7 | PASS |
| html -> text, prose (12.1 MB): throughput (MB/s of input) | 271.3 | 33.0 | 0.9 | PASS |
| html -> text, prose: peak memory (MB) | 14.2 | 175.5 | 3001.5 | PASS |

The pass gate is html2text (the tight peer); pandoc is context. Both are far
heavier than our streaming renderer — pandoc reaches gigabytes of RSS.

### 2026-09-24, first release of the HTML reader

commit: 34a9e90 (the reader's working tree, before its commit)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

| Target | Ours | Reference | Result |
|---|---|---|---|
| html -> text, markup-dense (26.6 MB): throughput (MB/s of input) | 193.8 | goal: 50 | PASS |
| html -> text, markup-dense: peak memory (MB) | 29.6 | goal: <= 64.0 | PASS |
| html -> text, prose (12.1 MB): throughput (MB/s of input) | 344.3 | goal: 50 | PASS |
| html -> text, prose: peak memory (MB) | 14.9 | goal: <= 64.0 | PASS |

## Conclusions

Four to seven times the goal on throughput and a quarter to a half of
the memory line. The text writer costs almost nothing over the reader.
