# Pages -> HTML

## Purpose

A Pages document as HTML through the Markdown event projection and the HTML writer. It is the browser view of a document, and the last step before PDF.

## Pass lines

No peer implementation exists. The reference is our own package round
trip, `pages -> pages-json`, on the same input and in the same session: it
decodes every object and writes several times more bytes than any document
path, so a document path that costs more than it is doing avoidable work.
The memory line is the same reference's peak.

| Target | Pass line |
|---|---|
| `pages -> html` throughput (MB/s of the package) | >= `pages -> pages-json` on the same input |
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

**Commands.** `bench/run.sh pages-html` runs, per shape:

- ours: `sublime -q convert styled.pages styled.html --to html`
- reference: `sublime -q convert styled.pages styled.json --to pages-json`

The pair's harness module (`bench/src/pairs/pages_html.rs`) exposes `ours`
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
| pages -> html throughput, styled (MB/s of the package) | 15.2 | 33.1 (pages-json) | FAIL |
| Peak RSS pages -> html, styled (MB) | 142.5 | 44.3 (pages-json) | FAIL |
| pages -> html throughput, prose (MB/s of the package) | 19.3 | 38.0 (pages-json) | FAIL |
| Peak RSS pages -> html, prose (MB) | 64.4 | 27.7 (pages-json) | FAIL |
| Binary size, gnu (bytes) | 1399664 | <= 1400000 (size-budget, what CI checks) | PASS |
| Binary size, musl static (bytes, the release asset) | 1499776 | recorded | n/a |
| Startup above spawn floor (ms, 1 KB file) | -0.472 (spawn 0.497, floor 0.969) | < 1 | PASS |

The same picture as `pages-markdown.md`: both lines fail, throughput at
half the reference and memory at 3.2x and 2.3x, with the HTML writer a
small part of the cost.

## Conclusions

- Shares the Markdown pair's diagnosis and levers (`pages-docx.md`): the
  document model and the package decode, not the writer.
