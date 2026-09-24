# Pages -> plain text

**Latest** (2026-09-24, reader cursors: pages -> text 40.1 MB/s of input on the dense shape and 32.3 on prose, at 45.3 and 28.1 MB peak; memory PASSES on both shapes, throughput FAILS on both, see Conclusions and `STATE.md`)

## Purpose

A Pages document as plain text through the Markdown event projection and
the text writer: the "just give me the text" ask, and the least work any
document path can do. If this path cannot meet the goals, no document path
can.

## Reference

No tool in any language reads the modern Pages (IWA) format, so the Pages
*reader* has no peer and the reference is the shared goal (`pages-json.md`). The
text *writer* is the same one measured against a `pulldown-cmark` text dump in
`markdown-text.md`. Implemented in `bench/src/pairs/pages_text.rs`.

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

**Commands.** `bench/run.sh pages-text` runs, per shape:

- ours: `sublime -q convert styled.pages styled.txt --to text`

The harness module (`bench/src/pairs/pages_text.rs`) exposes `ours` for
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
- The text writer is a few milliseconds; this pair is the clearest
  measure of the document model's own cost.

## Results

### 2026-09-24, reader cursors and one arena copy

commit: 1ed4458
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

Reader round: the attribute tables are read with forward cursors instead of a binary search per run (six per run before), a storage's text is copied into the arena once and runs point into it by offset, paragraphs that are ASCII are split by byte scan, and the Markdown projection caches each style's resolved chain and reuses its scratch. Dense-shape document build 38 -> 25 ms in-process.

| Target | Ours | Reference | Result |
|---|---|---|---|
| pages -> text, styled (2.5 MB): throughput (MB/s of input) | 40.1 | goal: 50 | FAIL |
| pages -> text, styled: peak memory (MB) | 45.3 | goal: <= 64.0 | PASS |
| pages -> text, prose (1.5 MB): throughput (MB/s of input) | 32.3 | goal: 50 | FAIL |
| pages -> text, prose: peak memory (MB) | 28.1 | goal: <= 64.0 | PASS |

### 2026-09-24, typed decode of the attribute tables

commit: 08c4de5
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

Typed decode: on the document paths the twelve attribute tables of a text storage are kept as their encoded bytes in the tree (`Tree::deferred`) and parsed straight into vectors by the reader, twelve bytes an entry instead of three tree entries. Same inputs as the blocks below.

| Target | Ours | Reference | Result |
|---|---|---|---|
| pages -> text, styled (2.5 MB): throughput (MB/s of input) | 31.8 | goal: 50 | FAIL |
| pages -> text, styled: peak memory (MB) | 48.2 | goal: <= 64.0 | PASS |
| pages -> text, prose (1.5 MB): throughput (MB/s of input) | 31.1 | goal: 50 | FAIL |
| pages -> text, prose: peak memory (MB) | 34.9 | goal: <= 64.0 | PASS |

### 2026-09-24, reachable decode and fast deflate

commit: b6dfbef
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

Phase 3: the document paths decode only the objects reached from the document root (the stylesheet, theme, and view-state hubs are not expanded), the fast deflate level walks one candidate and skips indexing inside long matches, the Word XML drops `w:szCs` and the preserve attribute where nothing needs it, and the paragraph splitter reuses its scratch and no longer recounts UTF-16 units per run. Same inputs as the blocks below.

| Target | Ours | Reference | Result |
|---|---|---|---|
| pages -> text, styled (2.5 MB): throughput (MB/s of input) | 27.4 | goal: 50 | FAIL |
| pages -> text, styled: peak memory (MB) | 70.5 | goal: <= 64.0 | FAIL |
| pages -> text, prose (1.5 MB): throughput (MB/s of input) | 29.7 | goal: 50 | FAIL |
| pages -> text, prose: peak memory (MB) | 41.5 | goal: <= 64.0 | PASS |

### 2026-09-24, slim document model

commit: b8c1a66
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

Phase 1 of the performance plan: the document model became a text arena with interned properties, links, revisions, and strings (a run is 64 bytes and owns no heap). Same inputs as the block below.

| Target | Ours | Reference | Result |
|---|---|---|---|
| pages -> text, styled (2.5 MB): throughput (MB/s of input) | 25.9 | goal: 50 | FAIL |
| pages -> text, styled: peak memory (MB) | 74.8 | goal: <= 64.0 | FAIL |
| pages -> text, prose (1.5 MB): throughput (MB/s of input) | 26.4 | goal: 50 | FAIL |
| pages -> text, prose: peak memory (MB) | 44.7 | goal: <= 64.0 | PASS |

### 2026-09-24

commit: 8dbe707
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

| Target | Ours | Reference | Result |
|---|---|---|---|
| pages -> text, styled (2.5 MB): throughput (MB/s of input) | 15.9 | goal: 50 | FAIL |
| pages -> text, styled: peak memory (MB) | 142.3 | goal: <= 64.0 | FAIL |
| pages -> text, prose (1.5 MB): throughput (MB/s of input) | 19.3 | goal: 50 | FAIL |
| pages -> text, prose: peak memory (MB) | 64.5 | goal: <= 64.0 | FAIL |

## Conclusions

- Standing: memory passes on both shapes (45 and 28 MB against 64);
  throughput fails on both (40 and 32 MB/s against 50), a fifth to a
  third short. A real resume converts in 3 ms at 4.5 MB.
- The cheap levers are spent. After cursors, one arena copy, the ASCII
  fast path, cached style chains, 16-byte spans, and a bulk-copy Snappy
  decoder, the dense shape's wall clock is 58 ms against a 50 ms line and
  splits into package decode 10 ms (7.5 of it Snappy), document build
  25 ms, the text writer 11 ms, and about 10 ms of process start, file
  I/O, and first-touch page faults; the last round of micro-changes moved
  none of them.
- What would close the gap is structural, not tuning: a Snappy decoder
  inner loop at 1.5 GB/s instead of 1 (about 3 ms), a document built
  without per-paragraph vectors (fewer pages touched, cheaper drop), and
  ultimately a text path that streams from the storage tables without
  building the model, which trades the hub architecture for the last
  10 ms and is not worth it for this shape. Recorded under Spikes in
  `STATE.md`; the goal stands as a target, met on any real document.
