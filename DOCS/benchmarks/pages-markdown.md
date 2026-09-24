# Pages -> Markdown

## Purpose

A Pages document projected into the Markdown event stream and written as Markdown: headings, lists, formatting, links, images, tables, footnotes. It is the path into every Markdown workflow, and the projection also feeds HTML and text, so its cost is shared.

## Pass lines

No peer implementation exists. The reference is our own package round
trip, `pages -> pages-json`, on the same input and in the same session: it
decodes every object and writes several times more bytes than any document
path, so a document path that costs more than it is doing avoidable work.
The memory line is the same reference's peak.

| Target | Pass line |
|---|---|
| `pages -> markdown` throughput (MB/s of the package) | >= `pages -> pages-json` on the same input |
| Peak resident memory | <= `pages -> pages-json` on the same input |
| Binary size | within `size-budget` (gnu, what CI checks); the musl release asset is recorded |
| Startup above spawn floor | < 1 ms (binary-wide) |

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

**Commands.** `bench/run.sh pages-markdown` runs, per shape:

- ours: `sublime -q convert styled.pages styled.markdown --to markdown`
- reference: `sublime -q convert styled.pages styled.json --to pages-json`

The pair's harness module (`bench/src/pairs/pages_markdown.rs`) exposes `ours`
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

### 2026-09-24

commit: 73fd5f5
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

| Target | Ours | Reference | Result |
|---|---|---|---|
| pages -> markdown throughput, styled (MB/s of the package) | 15.6 | 32.6 (pages-json) | FAIL |
| Peak RSS pages -> markdown, styled (MB) | 142.4 | 44.6 (pages-json) | FAIL |
| pages -> markdown throughput, prose (MB/s of the package) | 17.7 | 38.3 (pages-json) | FAIL |
| Peak RSS pages -> markdown, prose (MB) | 64.7 | 27.8 (pages-json) | FAIL |
| Binary size, gnu (bytes) | 1399664 | <= 1400000 (size-budget, what CI checks) | PASS |
| Binary size, musl static (bytes, the release asset) | 1499776 | recorded | n/a |
| Startup above spawn floor (ms, 1 KB file) | 0.137 (spawn 0.483, floor 0.346) | < 1 | PASS |

Both lines fail on both shapes: throughput is half the reference, memory
3.2x on the dense shape and 2.3x on prose. The writer itself is cheap
(22 ms of the 170 ms on `styled` in-process); the cost is the document
model the reader builds and the package decode, which every document path
shares (see `pages-docx.md` for the split).

## Conclusions

- The Markdown projection and writer are not the problem; the model and
  the package decode are. The three levers in `pages-docx.md` (a slimmer
  run, decode on lookup) carry this pair with them; the streaming Word
  body does not apply here.
- Once the model is fixed, this pair should pass with room: the reference
  writes 35 MB of JSON where this path writes 3.4 MB of Markdown.
