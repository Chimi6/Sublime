# Word -> text

**Latest** (2026-09-24, first release of the Word reader: docx -> text 150.5 MB/s of uncompressed input on the dense shape and 163.6 on prose, at 47.1 and 25.9 MB peak; every line PASSES, three times the docx-rs reference at a seventh of its memory)

## Purpose

A Word document read into the document model and projected to plain
text. It is the cheapest path out of Word, so it measures the reader
itself: ZIP, XML, styles, and the model build, with the lightest writer
behind them. The Markdown and HTML pairs add their writers on top.

## Pass lines

- Throughput >= 50 MB/s of uncompressed input and peak memory <= 64 MB on
  the benchmark inputs: the goals every Pages pair shares
  (`pages-json.md`), applied to Word input with the standard's
  compressed-input measure (`README.md`), since the reader's work is
  proportional to the XML it parses, not to the file's bytes.
- Not slower than `docx-rs` (a Rust Word reader) parsing the same file
  and writing the paragraph texts, and no more memory. The reference does
  less (no styles resolved, no model), so it bounds the cost of a Rust
  reader from below.

## Method

**Machine.** Recorded with each results block from the harness's
machine line.

**Inputs.** No large real Word document can be committed, so the harness
takes the Pages pair's generated documents (`pages-json.md`: `styled` is
`text-styles.pages` x 5,000, `prose` is `paragraphs.pages` x 2,000) and
writes them to Word with our own writer once (`bench/pairs/docx-text.sh`,
`docx_inputs`). The Word files are 0.5 and 0.2 MB on disk and 14.9 and
7.5 MB uncompressed: 55,000 and 30,000 paragraphs.

**Statistics.** `bench/run.sh docx-text`: three runs per command, median
wall clock of the whole process, peak resident memory from GNU `time`,
throughput over the uncompressed bytes of the package (`unzip -l`) with
the per-file-byte rate as an extra row. Rows and units follow
`README.md`.

**Commands.** Per shape:

- ours: `sublime -q convert styled.docx styled.txt --to text`
- reference: `sublime-bench docx-text crates styled.docx out.txt`
  (`docx-rs` 0.4, `read_docx` then the text of every run in every body
  paragraph, one paragraph per line)

## Threats to validity

- The inputs are our own writer's output: well-formed, no whitespace
  between elements, one style sheet. Word's own files carry more
  properties per run and larger style sheets; the reader walks them the
  same way, but the numbers here are an upper bound on real files.
- Repeated text compresses unusually well, so the per-file-byte extra row
  reads low; the uncompressed rows are the ones to compare.
- `docx-rs` builds a full tree of the document, which is where its memory
  goes; it is a reference for a reader in Rust, not for a text extractor.

## Results

### 2026-09-24, first release of the Word reader

commit: 58d9829 (the reader's working tree, before its commit)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

| Target | Ours | Reference | Result |
|---|---|---|---|
| docx -> text, styled (14.9 MB uncompressed): throughput (MB/s of uncompressed input) | 150.5 | goal: 50 | PASS |
| docx -> text, styled (.5 MB file): throughput (MB/s of file bytes) [extra] | 5.0 | recorded | n/a |
| docx -> text, styled: peak memory (MB) | 47.1 | goal: <= 64.0 | PASS |
| docx -> text, styled: throughput (MB/s of uncompressed input) [extra] | 150.5 | 51.8 (docx-rs, paragraph text only) | PASS |
| docx -> text, styled: peak memory (MB) [extra] | 47.1 | 353.8 (docx-rs) | PASS |
| docx -> text, prose (7.5 MB uncompressed): throughput (MB/s of uncompressed input) | 163.6 | goal: 50 | PASS |
| docx -> text, prose (.2 MB file): throughput (MB/s of file bytes) [extra] | 4.7 | recorded | n/a |
| docx -> text, prose: peak memory (MB) | 25.9 | goal: <= 64.0 | PASS |
| docx -> text, prose: throughput (MB/s of uncompressed input) [extra] | 163.6 | 75.8 (docx-rs, paragraph text only) | PASS |
| docx -> text, prose: peak memory (MB) [extra] | 25.9 | 132.1 (docx-rs) | PASS |

## Conclusions

The reader passes both goals on both shapes with room, and beats the
reference reader on time and memory while doing more. The dense shape
costs more per byte than prose (a run per few words, each with
properties to intern) and holds about 47 MB at peak: the uncompressed
XML (15 MB), the model, and the arena. If a larger goal ever needs it,
the levers are the XML reader's per-tag attribute vector (an allocation
per start tag) and interning identical run properties instead of pushing
one per run.
