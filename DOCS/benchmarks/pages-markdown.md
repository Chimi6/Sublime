# Pages -> Markdown

**Latest** (2026-09-24, phase 3: pages -> markdown 26.3 MB/s of input on the dense shape and 25.1 on prose, at 70.0 and 41.6 MB peak; styled throughput FAIL, styled memory FAIL, prose throughput FAIL, prose memory PASS; see Conclusions and `STATE.md`)

## Purpose

A Pages document projected into the Markdown event stream and written
as Markdown: headings, lists, formatting, links, images, tables, footnotes.
It is the path into every Markdown workflow, and the projection also feeds
the HTML and text pairs.

## Pass lines

The goals every Pages pair shares, defined and justified in
`pages-json.md`: throughput >= 50 MB/s of input and peak memory <= 64 MB on
the benchmark inputs. No peer implementation exists to measure against.

## Method

**Machine.** Recorded with each results block from the harness's
machine line.

**Inputs.** No large real Pages document can be committed and Pages
cannot be scripted from this machine, so the harness scales fixtures
(`bench/target/release/sublime-bench pages-json gen <seed> <units> <out>`):
a fixture's body text is repeated `units` times with a paragraph break
between copies, every character-indexed attribute table (paragraph and
character styles, lists and levels, list starts, sections, layouts) is
repeated and shifted to match, and the result is written back as a real
package through the lossless JSON form. The stylesheet and every other
stream stay exactly as Pages wrote them. Two shapes:

- `styled`: `text-styles.pages` x 5,000: 2.5 MB package, 7.5 MB of
  decompressed streams, 55,000 paragraphs, 170,000 formatted runs. Short
  paragraphs with a character style change every few words.
- `prose`: `paragraphs.pages` x 2,000: 1.5 MB package, 6.6 MB decompressed,
  30,000 paragraphs, 34,000 runs, with line and page breaks. What most
  documents look like.

**Statistics.** `bench/run.sh <pair>`: three runs per command, median wall
clock of the whole process, peak resident memory from GNU `time`,
throughput in MB/s over the input file's bytes on disk. Rows and units
follow `DOCS/benchmarks/README.md`.

**Commands.** `bench/run.sh pages-markdown` runs, per shape:

- ours: `sublime -q convert styled.pages styled.md --to markdown`

The harness module (`bench/src/pairs/pages_markdown.rs`) exposes `ours` for
in-process runs; inputs come from the `pages-json` pair's generator.

## Threats to validity

- No other tool reads the modern Pages format, so the goals are ours, not
  a peer's; they say whether the path is fast enough to feel free, not
  whether it is the fastest possible.
- Repeated text compresses unusually well: the package is 2.5 MB for 7.5 MB
  of streams where a real resume is 328 KB for 75 KB. MB/s of the package
  is therefore lower than a real document would show; ratios between our
  own paths are trustworthy.
- The scaled document reuses the seed's forty style objects 5,000 times; a
  document with thousands of distinct styles would stress the stylesheet
  reader instead.
- Single machine, sometimes busy; medians of three absorb most of it.
- The Markdown writer is a small part of the cost (22 ms of 170 ms on
  `styled` in-process); this pair measures the document model and the
  package decode more than the writer.

## Results

### 2026-09-24, reachable decode and fast deflate

commit: b6dfbef
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

Phase 3: the document paths decode only the objects reached from the document root (the stylesheet, theme, and view-state hubs are not expanded), the fast deflate level walks one candidate and skips indexing inside long matches, the Word XML drops `w:szCs` and the preserve attribute where nothing needs it, and the paragraph splitter reuses its scratch and no longer recounts UTF-16 units per run. Same inputs as the blocks below.

| Target | Ours | Reference | Result |
|---|---|---|---|
| pages -> markdown, styled (2.5 MB): throughput (MB/s of input) | 26.3 | goal: 50 | FAIL |
| pages -> markdown, styled: peak memory (MB) | 70.0 | goal: <= 64.0 | FAIL |
| pages -> markdown, prose (1.5 MB): throughput (MB/s of input) | 25.1 | goal: 50 | FAIL |
| pages -> markdown, prose: peak memory (MB) | 41.6 | goal: <= 64.0 | PASS |

### 2026-09-24, slim document model

commit: b8c1a66
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

Phase 1 of the performance plan: the document model became a text arena with interned properties, links, revisions, and strings (a run is 64 bytes and owns no heap). Same inputs as the block below.

| Target | Ours | Reference | Result |
|---|---|---|---|
| pages -> markdown, styled (2.5 MB): throughput (MB/s of input) | 24.9 | goal: 50 | FAIL |
| pages -> markdown, styled: peak memory (MB) | 75.0 | goal: <= 64.0 | FAIL |
| pages -> markdown, prose (1.5 MB): throughput (MB/s of input) | 22.1 | goal: 50 | FAIL |
| pages -> markdown, prose: peak memory (MB) | 45.1 | goal: <= 64.0 | PASS |

### 2026-09-24

commit: 8dbe707
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

| Target | Ours | Reference | Result |
|---|---|---|---|
| pages -> markdown, styled (2.5 MB): throughput (MB/s of input) | 15.3 | goal: 50 | FAIL |
| pages -> markdown, styled: peak memory (MB) | 142.0 | goal: <= 64.0 | FAIL |
| pages -> markdown, prose (1.5 MB): throughput (MB/s of input) | 18.1 | goal: 50 | FAIL |
| pages -> markdown, prose: peak memory (MB) | 64.8 | goal: <= 64.0 | FAIL |

## Conclusions

- Failing rows after the three phases: throughput on both shapes (about
  27 MB/s dense, 25 to 30 MB/s prose, against 50) and memory on the dense
  shape (70 MB against 64). Prose memory passes. A real resume converts in
  3 ms at 4.8 MB, inside both goals.
- Where the dense shape's time goes, in-process: package decode 26 ms
  (the body's 300,000 attribute entries), document build 30 ms, the Markdown
  writer about 18 ms, process and I/O the rest. The goal is 50 ms
  for all of it, so the writer is not the problem; the reader is.
- Levers, recorded under Spikes in `STATE.md`: a typed decode of the
  attribute tables straight into vectors (about 12 ms off decode, 15 ms
  off the build, and 25 MB of trees, which passes the memory goal), then
  merging adjacent runs of equal formatting and dropping the source trees
  once the model is built. With the first, this path projects to about
  45 ms on the dense shape, on the goal line. The goals stand.
