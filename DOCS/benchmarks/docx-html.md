# Word -> HTML

**Latest** (2026-09-24, pandoc reference wired on an Apple M1 Max: docx -> html 102.6 MB/s of uncompressed input on the styled shape and 113.9 on prose, at 21.6 and 32.0 MB peak, far faster and leaner than pandoc; every line PASSES)

## Purpose

A Word document read into the document model and projected into the
Markdown event stream, which the HTML writer renders. The reader is the same
as the text pair's (`docx-text.md`, which also carries the peer
reference); this pair adds the HTML writer's cost on top.

## Reference

`pandoc` converts docx to HTML and is the reference, run as an external process
and timed on the same uncompressed input bytes as ours (`bench/pairs/docx-html.sh`).
No Rust crate does Word to HTML end to end; pandoc does full-fidelity conversion
with a Haskell runtime, so it is a loose upper bound — we are much faster — but a
real tool rather than a goal. The docx *reader* is additionally peer-checked
against `docx-rs` in `docx-text.md`.

## Pass lines

Throughput >= 50 MB/s of uncompressed input and peak memory <= 64 MB on
the benchmark inputs: the goals every Pages pair shares
(`pages-json.md`), applied to Word input with the standard's
compressed-input measure (`README.md`). No peer implementation of
Word -> HTML in Rust is measured; the reader's own reference is in the
text pair.

## Method

**Machine.** Recorded with each results block from the harness's
machine line.

**Inputs.** The Pages pair's generated documents written to Word by our
own writer (`docx-text.md`, Method): 0.5 and 0.2 MB on disk, 14.9 and
7.5 MB uncompressed, 55,000 and 30,000 paragraphs.

**Statistics.** `bench/run.sh docx-html`: three runs per command, median
wall clock of the whole process, peak resident memory from GNU `time`,
throughput over the uncompressed bytes of the package with the
per-file-byte rate as an extra row. Rows and units follow `README.md`.

**Commands.** Per shape:

- ours: `sublime -q convert styled.docx styled.html --to html`

## Threats to validity

As in `docx-text.md`: the inputs are our own writer's output, so real
Word files with larger style sheets and more properties per run will
read somewhat slower; repeated text makes the per-file-byte row read low.

## Results

### 2026-09-24, pandoc reference wired

commit: 64b08e7 (the reference wiring's working tree, before its commit)
machine: Darwin 24.5.0 arm64, 10 cpus, Apple M1 Max

| Target | Ours | Reference | Result |
|---|---|---|---|
| docx -> html, styled (6.0 MB uncompressed): throughput (MB/s of uncompressed input) | 102.6 | 1.5 (pandoc) | PASS |
| docx -> html, styled (0.2 MB file): throughput (MB/s of file bytes) [extra] | 3.5 | recorded | n/a |
| docx -> html, styled: peak memory (MB) | 21.6 | 549.0 (pandoc) | PASS |
| docx -> html, prose (7.5 MB uncompressed): throughput (MB/s of uncompressed input) | 113.9 | 1.4 (pandoc) | PASS |
| docx -> html, prose (0.2 MB file): throughput (MB/s of file bytes) [extra] | 3.3 | recorded | n/a |
| docx -> html, prose: peak memory (MB) | 32.0 | 776.1 (pandoc) | PASS |

### 2026-09-24, first release of the Word reader

commit: 58d9829 (the reader's working tree, before its commit)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

| Target | Ours | Reference | Result |
|---|---|---|---|
| docx -> html, styled (14.9 MB uncompressed): throughput (MB/s of uncompressed input) | 147.4 | goal: 50 | PASS |
| docx -> html, styled (.5 MB file): throughput (MB/s of file bytes) [extra] | 4.9 | recorded | n/a |
| docx -> html, styled: peak memory (MB) | 47.1 | goal: <= 64.0 | PASS |
| docx -> html, prose (7.5 MB uncompressed): throughput (MB/s of uncompressed input) | 162.3 | goal: 50 | PASS |
| docx -> html, prose (.2 MB file): throughput (MB/s of file bytes) [extra] | 4.7 | recorded | n/a |
| docx -> html, prose: peak memory (MB) | 26.0 | goal: <= 64.0 | PASS |

## Conclusions

Both shapes pass both goals with room, within a few percent of the text pair: the HTML writer's escaping is a bulk scan and costs almost nothing over the reader. Memory is the reader's; the writer streams.
