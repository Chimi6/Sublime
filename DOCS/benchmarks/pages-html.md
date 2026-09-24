# Pages -> HTML

**Latest** (2026-09-24, reader cursors: pages -> html 39.7 MB/s of input on the dense shape and 38.5 on prose, at 45.5 and 27.9 MB peak; memory PASSES on both shapes, throughput FAILS on both, see Conclusions and `STATE.md`)

## Purpose

A Pages document as HTML through the Markdown event projection and the
HTML writer: the browser view of a document, and the last step before PDF.

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

**Commands.** `bench/run.sh pages-html` runs, per shape:

- ours: `sublime -q convert styled.pages styled.html --to html`

The harness module (`bench/src/pairs/pages_html.rs`) exposes `ours` for
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
- The HTML writer is a small part of the cost; this pair measures the
  document model and the package decode more than the writer.

## Results

### 2026-09-24, reader cursors and one arena copy

commit: 1ed4458
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

Reader round: the attribute tables are read with forward cursors instead of a binary search per run (six per run before), a storage's text is copied into the arena once and runs point into it by offset, paragraphs that are ASCII are split by byte scan, and the Markdown projection caches each style's resolved chain and reuses its scratch. Dense-shape document build 38 -> 25 ms in-process.

| Target | Ours | Reference | Result |
|---|---|---|---|
| pages -> html, styled (2.5 MB): throughput (MB/s of input) | 39.7 | goal: 50 | FAIL |
| pages -> html, styled: peak memory (MB) | 45.5 | goal: <= 64.0 | PASS |
| pages -> html, prose (1.5 MB): throughput (MB/s of input) | 38.5 | goal: 50 | FAIL |
| pages -> html, prose: peak memory (MB) | 27.9 | goal: <= 64.0 | PASS |

### 2026-09-24, typed decode of the attribute tables

commit: 08c4de5
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

Typed decode: on the document paths the twelve attribute tables of a text storage are kept as their encoded bytes in the tree (`Tree::deferred`) and parsed straight into vectors by the reader, twelve bytes an entry instead of three tree entries. Same inputs as the blocks below.

| Target | Ours | Reference | Result |
|---|---|---|---|
| pages -> html, styled (2.5 MB): throughput (MB/s of input) | 31.6 | goal: 50 | FAIL |
| pages -> html, styled: peak memory (MB) | 48.2 | goal: <= 64.0 | PASS |
| pages -> html, prose (1.5 MB): throughput (MB/s of input) | 30.9 | goal: 50 | FAIL |
| pages -> html, prose: peak memory (MB) | 34.9 | goal: <= 64.0 | PASS |

### 2026-09-24, reachable decode and fast deflate

commit: b6dfbef
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

Phase 3: the document paths decode only the objects reached from the document root (the stylesheet, theme, and view-state hubs are not expanded), the fast deflate level walks one candidate and skips indexing inside long matches, the Word XML drops `w:szCs` and the preserve attribute where nothing needs it, and the paragraph splitter reuses its scratch and no longer recounts UTF-16 units per run. Same inputs as the blocks below.

| Target | Ours | Reference | Result |
|---|---|---|---|
| pages -> html, styled (2.5 MB): throughput (MB/s of input) | 28.6 | goal: 50 | FAIL |
| pages -> html, styled: peak memory (MB) | 70.4 | goal: <= 64.0 | FAIL |
| pages -> html, prose (1.5 MB): throughput (MB/s of input) | 28.2 | goal: 50 | FAIL |
| pages -> html, prose: peak memory (MB) | 41.3 | goal: <= 64.0 | PASS |

### 2026-09-24, slim document model

commit: b8c1a66
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

Phase 1 of the performance plan: the document model became a text arena with interned properties, links, revisions, and strings (a run is 64 bytes and owns no heap). Same inputs as the block below.

| Target | Ours | Reference | Result |
|---|---|---|---|
| pages -> html, styled (2.5 MB): throughput (MB/s of input) | 26.0 | goal: 50 | FAIL |
| pages -> html, styled: peak memory (MB) | 74.8 | goal: <= 64.0 | FAIL |
| pages -> html, prose (1.5 MB): throughput (MB/s of input) | 24.6 | goal: 50 | FAIL |
| pages -> html, prose: peak memory (MB) | 45.0 | goal: <= 64.0 | PASS |

### 2026-09-24

commit: 8dbe707
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

| Target | Ours | Reference | Result |
|---|---|---|---|
| pages -> html, styled (2.5 MB): throughput (MB/s of input) | 15.2 | goal: 50 | FAIL |
| pages -> html, styled: peak memory (MB) | 142.3 | goal: <= 64.0 | FAIL |
| pages -> html, prose (1.5 MB): throughput (MB/s of input) | 18.7 | goal: 50 | FAIL |
| pages -> html, prose: peak memory (MB) | 64.3 | goal: <= 64.0 | FAIL |

## Conclusions

- Standing after the typed decode: memory passes on both shapes (48.2 and
  34.9 MB against 64); throughput fails on both (31.6 and 30.9 MB/s
  against 50). A real resume converts in 3 ms at 4.5 MB.
- Where the dense shape's time goes now, in-process: package decode about
  16 ms, document build about 25 ms, the html writer 12 to 18 ms, process
  and I/O the rest, against a 50 ms line. The reader is still the larger
  half; its remaining levers (merging adjacent runs of equal formatting,
  dropping the source trees, the generic tree decode of the rest of the
  body) are under Spikes in `STATE.md`. The goal stands: it is within a
  third on the dense shape and met on any real document.
