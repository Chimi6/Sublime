# Word -> Markdown

**Latest** (2026-09-24, pandoc reference wired on an Apple M1 Max: docx -> markdown 99.7 MB/s of uncompressed input on the styled shape and 98.0 on prose, at 21.2 and 31.9 MB peak, far faster and leaner than pandoc; every line PASSES)

## Purpose

A Word document read into the document model and projected into the
Markdown event stream, which the Markdown writer renders. The reader is the same
as the text pair's (`docx-text.md`, which also carries the peer
reference); this pair adds the Markdown writer's cost on top.

## Reference

`pandoc` converts docx to Markdown and is the reference, run as an external
process and timed on the same uncompressed input bytes as ours
(`bench/pairs/docx-markdown.sh`). No Rust crate does Word to Markdown end to end;
pandoc does full-fidelity conversion with a Haskell runtime, so it is a loose
upper bound — we are much faster — but a real tool rather than a goal. The docx
*reader* is additionally peer-checked against `docx-rs` in `docx-text.md`.

## Pass lines

Throughput >= 50 MB/s of uncompressed input and peak memory <= 64 MB on
the benchmark inputs: the goals every Pages pair shares
(`pages-json.md`), applied to Word input with the standard's
compressed-input measure (`README.md`). No peer implementation of
Word -> Markdown in Rust is measured; the reader's own reference is in the
text pair.

## Method

**Machine.** Recorded with each results block from the harness's
machine line.

**Inputs.** The Pages pair's generated documents written to Word by our
own writer (`docx-text.md`, Method): 0.5 and 0.2 MB on disk, 14.9 and
7.5 MB uncompressed, 55,000 and 30,000 paragraphs.

**Statistics.** `bench/run.sh docx-markdown`: three runs per command, median
wall clock of the whole process, peak resident memory from GNU `time`,
throughput over the uncompressed bytes of the package with the
per-file-byte rate as an extra row. Rows and units follow `README.md`.

**Commands.** Per shape:

- ours: `sublime -q convert styled.docx styled.md --to markdown`

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
| docx -> markdown, styled (6.0 MB uncompressed): throughput (MB/s of uncompressed input) | 99.7 | 1.3 (pandoc) | PASS |
| docx -> markdown, styled (0.2 MB file): throughput (MB/s of file bytes) [extra] | 3.4 | recorded | n/a |
| docx -> markdown, styled: peak memory (MB) | 21.2 | 821.5 (pandoc) | PASS |
| docx -> markdown, prose (7.5 MB uncompressed): throughput (MB/s of uncompressed input) | 98.0 | 1.3 (pandoc) | PASS |
| docx -> markdown, prose (0.2 MB file): throughput (MB/s of file bytes) [extra] | 2.9 | recorded | n/a |
| docx -> markdown, prose: peak memory (MB) | 31.9 | 1331.7 (pandoc) | PASS |

### 2026-09-24, first release of the Word reader

commit: 58d9829 (the reader's working tree, before its commit)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

| Target | Ours | Reference | Result |
|---|---|---|---|
| docx -> markdown, styled (14.9 MB uncompressed): throughput (MB/s of uncompressed input) | 139.4 | goal: 50 | PASS |
| docx -> markdown, styled (.5 MB file): throughput (MB/s of file bytes) [extra] | 4.6 | recorded | n/a |
| docx -> markdown, styled: peak memory (MB) | 47.2 | goal: <= 64.0 | PASS |
| docx -> markdown, prose (7.5 MB uncompressed): throughput (MB/s of uncompressed input) | 127.6 | goal: 50 | PASS |
| docx -> markdown, prose (.2 MB file): throughput (MB/s of file bytes) [extra] | 3.7 | recorded | n/a |
| docx -> markdown, prose: peak memory (MB) | 25.9 | goal: <= 64.0 | PASS |

## Conclusions

Both shapes pass both goals with room. The Markdown writer costs more on prose than the text writer does (128 against 164 MB/s): long paragraphs are scanned for characters to escape, which plain text never needs. Memory is the reader's; the writer streams.
