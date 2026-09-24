# Pages -> plain text

**Latest** (2026-09-24: pages -> text runs at 15.3 MB/s of input on the dense shape and 18.9 MB/s on prose, at 142.5 and 64.7 MB peak; both lines FAIL against our own package round trip (about 31 and 35 MB/s, 45 and 27 MB). Blockers and the 0.6.1 plan are in `STATE.md`)

## Purpose

A Pages document as plain text through the Markdown event projection and the text writer: the "just give me the text" ask, and the least work any document path can do. If this path cannot pass, no document path can.

## Pass lines

No peer implementation exists. The reference is our own package round
trip, `pages -> pages-json`, on the same input and in the same session: it
decodes every object and writes several times more bytes than any document
path, so a document path that costs more than it is doing avoidable work.
The memory line is the same reference's peak.

| Target | Pass line |
|---|---|
| `pages -> text` throughput (MB/s of the package) | >= `pages -> pages-json` on the same input |
| Peak resident memory | <= `pages -> pages-json` on the same input |

## Method

**Machine.** Recorded with each results block from the harness's machine
line.

**Inputs.** No large real Pages document can be committed, and Pages
cannot be scripted from this machine, so the harness scales fixtures:
`bench/target/release/sublime-bench pages-json gen <seed> <units> <out>`
reads a fixture, repeats its body text `units` times with a paragraph
break between copies, repeats every character-indexed attribute table
(paragraph and character styles, list styles and levels, list starts,
sections, layouts, fields) shifted to match, and writes the result back as
a real package through the lossless JSON form. Two shapes:

- `styled`: `text-styles.pages` x 5,000 (2.7 MB package, 7.9 MB of
  decompressed streams, 55,000 paragraphs, 170,000 formatted runs). Short
  paragraphs with a character style change every few words: the
  markup-dense shape.
- `prose`: `paragraphs.pages` x 2,000 (1.6 MB package, 7.0 MB decompressed,
  30,000 paragraphs, 34,000 runs, with line and page breaks). Long
  paragraphs, few runs: what most documents look like.

The stylesheet, theme, and every other stream stay exactly as Pages wrote
them, so the fixed costs (570 objects of presets) are real and the text is
what grows.

**Statistics.** `bench/run.sh <pair>`: three runs per command, median wall
clock, peak resident memory from GNU `time`, throughput in MB/s over the
package's bytes on disk (the compressed input a user hands us).

**Commands.** `bench/run.sh pages-text` runs, per shape:

- ours: `sublime -q convert styled.pages styled.text --to text`
- reference: `sublime -q convert styled.pages styled.json --to pages-json`

The pair's harness module (`bench/src/pairs/pages_text.rs`) exposes `ours`
for in-process runs; inputs come from the `pages-json` pair's generator.

## Threats to validity

- Repetition compresses unusually well, so Snappy and Deflate work is
  lighter per byte than on a real document; the package is 2.7 MB for
  7.9 MB of streams, where the resume in hand is 328 KB for 75 KB. Ratios
  between our paths are trustworthy; absolute MB/s of the package is
  flattering.
- The scaled document has 5,000 identical copies of the seed's styles and
  sections; style resolution repeats the same forty objects. A document
  with 5,000 distinct styles would stress the stylesheet reader instead.
- Single machine, and it was busy during these runs: the startup row's
  spawn floor moved by a millisecond between pairs. Medians of three cover
  the throughput rows; the startup row is not to be read from these blocks.
- No peer implementation exists for the modern Pages format, so nothing
  here says how we compare to another tool; the pass lines are internal
  floors and our own package layer.
- The reference does its work without a document model; a path that builds
  one (paragraphs, runs, resolved styles) pays allocation the reference
  never does, so the line measures the model's cost as much as the
  writer's. That is intended: the model is what must earn its keep.

## Results

### 2026-09-24, standard rows

commit: c60d037
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

Same inputs and commit as the block below, re-run with the standard row names and units (`DOCS/benchmarks/README.md`); the numbers are within run noise of it.

| Target | Ours | Reference | Result |
|---|---|---|---|
| pages -> text, styled (2.5 MB): throughput (MB/s of input) | 15.3 | 32.4 our pages -> pages-json on the same input; line: not slower | FAIL |
| pages -> text, styled: peak memory (MB) | 142.5 | 44.6 our pages -> pages-json on the same input; line: not more | FAIL |
| pages -> text, prose (1.5 MB): throughput (MB/s of input) | 18.9 | 36.9 our pages -> pages-json on the same input; line: not slower | FAIL |
| pages -> text, prose: peak memory (MB) | 64.7 | 27.7 our pages -> pages-json on the same input; line: not more | FAIL |

### 2026-09-24

commit: 73fd5f5
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

| Target | Ours | Reference | Result |
|---|---|---|---|
| pages -> text throughput, styled (MB/s of the package) | 15.2 | 33.4 (pages-json) | FAIL |
| Peak RSS pages -> text, styled (MB) | 142.1 | 44.3 (pages-json) | FAIL |
| pages -> text throughput, prose (MB/s of the package) | 16.4 | 36.9 (pages-json) | FAIL |
| Peak RSS pages -> text, prose (MB) | 64.7 | 27.5 (pages-json) | FAIL |
| Binary size, gnu (bytes) | 1399664 | <= 1400000 (size-budget, what CI checks) | PASS |
| Binary size, musl static (bytes, the release asset) | 1499776 | recorded | n/a |
| Startup above spawn floor (ms, 1 KB file) | 0.204 (spawn 0.503, floor 0.299) | < 1 | PASS |

Both lines fail on both shapes, at the same ratios as the Markdown and
HTML pairs: the text writer is a few milliseconds; the model and the
package decode are the cost.

## Conclusions

- The floor of the document paths is set by reading, not writing. The
  levers are the reader's (`pages-docx.md`). Until they land, the text path
  is the clearest measure of the model's overhead: 3.4 MB of text out of a
  2.7 MB package in 175 ms and 142 MB.
