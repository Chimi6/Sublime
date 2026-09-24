# Pages <-> pages-json

**Latest** (2026-09-24: pages -> pages-json 30.0 MB/s of input on the dense shape and 40.6 on prose, at 44.1 and 27.3 MB peak, below the 50 MB/s goal; pages-json -> pages 325.3 and 338.2 MB/s of input at 46 and 31 MB peak, passing both goals)

## Purpose

The lossless layer under every Pages path: the package read into
schema-named object trees and written as JSON, and the JSON rebuilt into a
byte-identical package. Every document conversion pays the forward decode,
so its speed is the floor of the flagship; the round trip is the
correctness oracle for the format (`tests/pages_fixtures.rs`).

## Pass lines

No other tool reads the modern Pages format, so there is no peer to
measure against. Every Pages pair uses the same two goals instead, chosen
so a document feels instant and the binary keeps its memory habits:

| Target | Goal |
|---|---|
| Throughput, every direction and shape | >= 50 MB/s of input. A real resume (328 KB) at that rate is 7 ms, a 10 MB book 0.2 s. It is a third of what the markup-dense Markdown pair does per byte on uncompressed text, which is the discount a compressed, object-graph format earns |
| Peak memory, every direction and shape | <= 64 MB on these inputs. The object trees of the 2.5 MB package take 44 MB; the goal leaves room for a document model or an output buffer, not for both held whole |

The goals are provisional: when a peer implementation appears, it replaces
them as the reference. A row that misses a goal is a FAIL and a blocker in
`STATE.md`; the two directions of a pair are never judged against each
other.

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

**Commands.** `bench/run.sh pages-json` runs, per shape:

- forward: `sublime -q convert styled.pages styled.json --to pages-json`
- back: `sublime -q convert styled.json styled-back.pages --from pages-json --to pages`

The harness module (`bench/src/pairs/pages_json.rs`) holds the generator
and `ours` / `ours-back` modes for in-process runs.

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
- The two directions have different inputs (a 2.5 MB package, a 33.7 MB
  JSON file), so their MB/s are not comparable to each other, only to the
  goal. In seconds the forward direction takes 0.08 s and the reverse
  0.10 s on the dense shape.

## Results

### 2026-09-24, reader cursors and one arena copy

commit: 1ed4458
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

Unchanged in substance (the lossless path does not use the document reader); recorded for the date trail.

| Target | Ours | Reference | Result |
|---|---|---|---|
| pages -> pages-json, styled (2.5 MB): throughput (MB/s of input) | 29.7 | goal: 50 | FAIL |
| pages -> pages-json, styled: peak memory (MB) | 47.9 | goal: <= 64.0 | PASS |
| pages-json -> pages, styled (33.7 MB): throughput (MB/s of input) | 312.4 | goal: 50 | PASS |
| pages-json -> pages, styled: peak memory (MB) | 46.0 | goal: <= 64.0 | PASS |
| pages -> pages-json, prose (1.5 MB): throughput (MB/s of input) | 37.5 | goal: 50 | FAIL |
| pages -> pages-json, prose: peak memory (MB) | 27.2 | goal: <= 64.0 | PASS |
| pages-json -> pages, prose (16.1 MB): throughput (MB/s of input) | 285.3 | goal: 50 | PASS |
| pages-json -> pages, prose: peak memory (MB) | 30.3 | goal: <= 64.0 | PASS |

### 2026-09-24, typed decode of the attribute tables

commit: 08c4de5
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

Unchanged (the lossless path decodes everything, deferring nothing); recorded for the date trail.

| Target | Ours | Reference | Result |
|---|---|---|---|
| pages -> pages-json, styled (2.5 MB): throughput (MB/s of input) | 29.5 | goal: 50 | FAIL |
| pages -> pages-json, styled: peak memory (MB) | 47.9 | goal: <= 64.0 | PASS |
| pages-json -> pages, styled (33.7 MB): throughput (MB/s of input) | 329.5 | goal: 50 | PASS |
| pages-json -> pages, styled: peak memory (MB) | 46.3 | goal: <= 64.0 | PASS |
| pages -> pages-json, prose (1.5 MB): throughput (MB/s of input) | 39.5 | goal: 50 | FAIL |
| pages -> pages-json, prose: peak memory (MB) | 27.6 | goal: <= 64.0 | PASS |
| pages-json -> pages, prose (16.1 MB): throughput (MB/s of input) | 285.3 | goal: 50 | PASS |
| pages-json -> pages, prose: peak memory (MB) | 30.8 | goal: <= 64.0 | PASS |

### 2026-09-24, reachable decode and fast deflate

commit: b6dfbef
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

Unchanged by phase 3 (the lossless path still decodes everything); recorded for the date trail.

| Target | Ours | Reference | Result |
|---|---|---|---|
| pages -> pages-json, styled (2.5 MB): throughput (MB/s of input) | 28.5 | goal: 50 | FAIL |
| pages -> pages-json, styled: peak memory (MB) | 48.6 | goal: <= 64.0 | PASS |
| pages-json -> pages, styled (33.7 MB): throughput (MB/s of input) | 330.0 | goal: 50 | PASS |
| pages-json -> pages, styled: peak memory (MB) | 46.0 | goal: <= 64.0 | PASS |
| pages -> pages-json, prose (1.5 MB): throughput (MB/s of input) | 40.4 | goal: 50 | FAIL |
| pages -> pages-json, prose: peak memory (MB) | 28.1 | goal: <= 64.0 | PASS |
| pages-json -> pages, prose (16.1 MB): throughput (MB/s of input) | 323.8 | goal: 50 | PASS |
| pages-json -> pages, prose: peak memory (MB) | 30.6 | goal: <= 64.0 | PASS |

### 2026-09-24, slim document model

commit: b8c1a66
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

Unchanged by the model work (this pair does not build the model); recorded for the date trail.

| Target | Ours | Reference | Result |
|---|---|---|---|
| pages -> pages-json, styled (2.5 MB): throughput (MB/s of input) | 26.7 | goal: 50 | FAIL |
| pages -> pages-json, styled: peak memory (MB) | 44.6 | goal: <= 64.0 | PASS |
| pages-json -> pages, styled (33.7 MB): throughput (MB/s of input) | 324.7 | goal: 50 | PASS |
| pages-json -> pages, styled: peak memory (MB) | 45.9 | goal: <= 64.0 | PASS |
| pages -> pages-json, prose (1.5 MB): throughput (MB/s of input) | 33.6 | goal: 50 | FAIL |
| pages -> pages-json, prose: peak memory (MB) | 27.1 | goal: <= 64.0 | PASS |
| pages-json -> pages, prose (16.1 MB): throughput (MB/s of input) | 322.0 | goal: 50 | PASS |
| pages-json -> pages, prose: peak memory (MB) | 30.9 | goal: <= 64.0 | PASS |

### 2026-09-24

commit: 8dbe707
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

| Target | Ours | Reference | Result |
|---|---|---|---|
| pages -> pages-json, styled (2.5 MB): throughput (MB/s of input) | 30.0 | goal: 50 | FAIL |
| pages -> pages-json, styled: peak memory (MB) | 44.1 | goal: <= 64.0 | PASS |
| pages-json -> pages, styled (33.7 MB): throughput (MB/s of input) | 325.3 | goal: 50 | PASS |
| pages-json -> pages, styled: peak memory (MB) | 46.1 | goal: <= 64.0 | PASS |
| pages -> pages-json, prose (1.5 MB): throughput (MB/s of input) | 40.6 | goal: 50 | FAIL |
| pages -> pages-json, prose: peak memory (MB) | 27.3 | goal: <= 64.0 | PASS |
| pages-json -> pages, prose (16.1 MB): throughput (MB/s of input) | 338.2 | goal: 50 | PASS |
| pages-json -> pages, prose: peak memory (MB) | 30.9 | goal: <= 64.0 | PASS |

## Conclusions

- Failing rows: the forward direction's throughput on both shapes (28 and
  40 MB/s of package bytes against 50). Memory passes everywhere and the
  reverse direction passes with a wide margin (330 MB/s of JSON, the class
  of our JSON tokenizer everywhere else).
- Per decompressed byte the forward direction runs at 84 to 172 MB/s (the
  package is 3x compressed), the class of the markup-dense Markdown parse;
  the goal asks for more because every document path pays this decode
  first.
- Levers, recorded under Spikes in `STATE.md`: the typed decode of the
  attribute tables serves this path too, since a typed table can be
  written as JSON directly (the same key-named entries the tree produces
  today) without ever building the tree entries; behind it, `Tree::decode`
  itself (one entry write per field, the field-slot lookup by number) and
  the JSON writer's per-field work. With the typed table the forward
  direction projects to about 40 MB/s dense and past the goal on prose;
  the last stretch on the dense shape is the tree decode of the rest.
