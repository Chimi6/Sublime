# Pages -> Word

**Latest** (2026-09-24, reader cursors: pages -> docx 21.5 MB/s of input on the dense shape and 24.6 on prose, at 46.4 and 28.0 MB peak; memory PASSES on both shapes, throughput FAILS on both, see Conclusions and `STATE.md`)

## Purpose

The conversion the flagship is for: a Pages document as a Word file with
its styles by name, lists, tables, images, headers and footers, and tracked
changes. It has to feel instant on a resume and stay inside the binary's
memory habits on a book.

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

**Commands.** `bench/run.sh pages-docx` runs, per shape:

- ours: `sublime -q convert styled.pages styled.docx --to docx`

The harness module (`bench/src/pairs/pages_docx.rs`) exposes `ours` for
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
- The Word output is a ZIP of XML: the writer deflates a 30 MB
  `document.xml` for the dense shape, so this pair carries compression
  work the Markdown, HTML, and text pairs do not.

## Results

### 2026-09-24, reader cursors and one arena copy

commit: 1ed4458
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

Reader round: the attribute tables are read with forward cursors instead of a binary search per run (six per run before), a storage's text is copied into the arena once and runs point into it by offset, paragraphs that are ASCII are split by byte scan, and the Markdown projection caches each style's resolved chain and reuses its scratch. Dense-shape document build 38 -> 25 ms in-process.

| Target | Ours | Reference | Result |
|---|---|---|---|
| pages -> docx, styled (2.5 MB): throughput (MB/s of input) | 21.5 | goal: 50 | FAIL |
| pages -> docx, styled: peak memory (MB) | 46.4 | goal: <= 64.0 | PASS |
| pages -> docx, prose (1.5 MB): throughput (MB/s of input) | 24.6 | goal: 50 | FAIL |
| pages -> docx, prose: peak memory (MB) | 28.0 | goal: <= 64.0 | PASS |

### 2026-09-24, run formatting interned into character styles

commit: 3ed21c5
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

Write-time interning: fonts, sizes, colors, underline, baseline, and language of a run go into one character style per (named style, paragraph formatting, run formatting) triple, based on the named style when there is one, and the run carries `w:rStyle`; the toggle properties Word applies relatively (bold, italic, strike, caps) stay inline. Dense-shape XML 19.3 -> 15.6 MB; the rest of the XML is the run and text elements themselves.

| Target | Ours | Reference | Result |
|---|---|---|---|
| pages -> docx, styled (2.5 MB): throughput (MB/s of input) | 20.0 | goal: 50 | FAIL |
| pages -> docx, styled: peak memory (MB) | 48.2 | goal: <= 64.0 | PASS |
| pages -> docx, prose (1.5 MB): throughput (MB/s of input) | 22.1 | goal: 50 | FAIL |
| pages -> docx, prose: peak memory (MB) | 35.2 | goal: <= 64.0 | PASS |

### 2026-09-24, typed decode of the attribute tables

commit: 08c4de5
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

Typed decode: on the document paths the twelve attribute tables of a text storage are kept as their encoded bytes in the tree (`Tree::deferred`) and parsed straight into vectors by the reader, twelve bytes an entry instead of three tree entries. Same inputs as the blocks below.

| Target | Ours | Reference | Result |
|---|---|---|---|
| pages -> docx, styled (2.5 MB): throughput (MB/s of input) | 18.8 | goal: 50 | FAIL |
| pages -> docx, styled: peak memory (MB) | 48.2 | goal: <= 64.0 | PASS |
| pages -> docx, prose (1.5 MB): throughput (MB/s of input) | 22.6 | goal: 50 | FAIL |
| pages -> docx, prose: peak memory (MB) | 34.8 | goal: <= 64.0 | PASS |

### 2026-09-24, reachable decode and fast deflate

commit: b6dfbef
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

Phase 3: the document paths decode only the objects reached from the document root (the stylesheet, theme, and view-state hubs are not expanded), the fast deflate level walks one candidate and skips indexing inside long matches, the Word XML drops `w:szCs` and the preserve attribute where nothing needs it, and the paragraph splitter reuses its scratch and no longer recounts UTF-16 units per run. Same inputs as the blocks below.

| Target | Ours | Reference | Result |
|---|---|---|---|
| pages -> docx, styled (2.5 MB): throughput (MB/s of input) | 16.9 | goal: 50 | FAIL |
| pages -> docx, styled: peak memory (MB) | 70.0 | goal: <= 64.0 | FAIL |
| pages -> docx, prose (1.5 MB): throughput (MB/s of input) | 21.3 | goal: 50 | FAIL |
| pages -> docx, prose: peak memory (MB) | 41.6 | goal: <= 64.0 | PASS |

### 2026-09-24, streamed Word body

commit: 9dcba8e
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

Phase 2: the body is rendered in 256 KiB parts straight into a streaming deflated ZIP entry (sync-flushed parts, sizes in a data descriptor) at a fast match level, so the XML is never held whole. Same inputs as the blocks below.

| Target | Ours | Reference | Result |
|---|---|---|---|
| pages -> docx, styled (2.5 MB): throughput (MB/s of input) | 15.6 | goal: 50 | FAIL |
| pages -> docx, styled: peak memory (MB) | 74.8 | goal: <= 64.0 | FAIL |
| pages -> docx, prose (1.5 MB): throughput (MB/s of input) | 18.5 | goal: 50 | FAIL |
| pages -> docx, prose: peak memory (MB) | 45.3 | goal: <= 64.0 | PASS |

### 2026-09-24, slim document model

commit: b8c1a66
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

Phase 1 of the performance plan: the document model became a text arena with interned properties, links, revisions, and strings (a run is 64 bytes and owns no heap). Same inputs as the block below.

| Target | Ours | Reference | Result |
|---|---|---|---|
| pages -> docx, styled (2.5 MB): throughput (MB/s of input) | 13.1 | goal: 50 | FAIL |
| pages -> docx, styled: peak memory (MB) | 174.9 | goal: <= 64.0 | FAIL |
| pages -> docx, prose (1.5 MB): throughput (MB/s of input) | 16.4 | goal: 50 | FAIL |
| pages -> docx, prose: peak memory (MB) | 93.8 | goal: <= 64.0 | FAIL |

### 2026-09-24

commit: 8dbe707
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

| Target | Ours | Reference | Result |
|---|---|---|---|
| pages -> docx, styled (2.5 MB): throughput (MB/s of input) | 10.0 | goal: 50 | FAIL |
| pages -> docx, styled: peak memory (MB) | 232.2 | goal: <= 64.0 | FAIL |
| pages -> docx, prose (1.5 MB): throughput (MB/s of input) | 13.3 | goal: 50 | FAIL |
| pages -> docx, prose: peak memory (MB) | 113.3 | goal: <= 64.0 | FAIL |

## Conclusions

- Standing after the typed decode and the interned styles: memory passes
  on both shapes (48.2 and 35.2 MB against 64, from 232 and 113 at the
  start); throughput fails on both (20.0 and 22.1 MB/s against 50, from
  10 and 13). A real resume converts in 3 ms at 4.8 MB.
- Where the dense shape's time goes now, in-process: package decode about
  16 ms, document build about 25 ms, Word render and deflate about
  50 ms for 15.6 MB of XML, against a 50 ms line. The XML is now mostly
  the run and text elements themselves (92 bytes a run on average), so
  the next lever on this path is merging adjacent runs of equal effective
  formatting, which cuts elements as well as bytes; behind it, the deflate
  itself and the reader's levers shared with the text paths (Spikes in
  `STATE.md`). The goal stands: within a factor of two and a half on the
  synthetic dense shape, met on any real document.
