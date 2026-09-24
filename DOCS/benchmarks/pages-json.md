# Pages <-> pages-json

**Latest** (2026-09-24: pages -> pages-json runs at 30 MB/s of package bytes (90 MB/s of decompressed streams) on the dense shape and 40 (174) on prose, at 45 and 27 MB peak, inside the floor line; pages-json -> pages runs at 328 MB/s of JSON, 18 percent below the forward direction, so its line FAILs)

## Purpose

The lossless layer under every Pages path: the package read into
schema-named object trees and written out as JSON, and the JSON rebuilt
into a byte-identical package. Every document conversion pays this
decode, so its cost is the floor of the flagship, and the round trip is
the correctness oracle for the format (`tests/pages_fixtures.rs`).

## Pass lines

No peer reads the modern Pages format, so the references are the pair's
own floor and its other direction.

| Target | Pass line |
|---|---|
| `pages -> pages-json` throughput | within four times the decompression floor: `bench pages-json floor`, which opens the ZIP and Snappy-decompresses every stream and does nothing else. Ours decodes every object into trees and writes JSON five times the stream size on top, so a factor of four is the line to hold, not a peer to beat |
| `pages-json -> pages` throughput (MB/s of the JSON) | >= `pages -> pages-json` measured over the same JSON bytes: rebuilding the package from JSON (tokenize, encode, Snappy-compress, ZIP) must not cost more than producing it |
| Peak resident memory, either direction | <= 64 MB on these inputs |

## How to read the table

Both directions are ours; nothing in the table is another tool. The
forward rows (`pages -> pages-json`) are judged against the floor, a
harness mode that only unzips and Snappy-decompresses the package; ours
must stay within four times it. The reverse rows (`pages-json -> pages`)
are judged against our own forward direction over the same bytes, so a
FAIL there means "rebuilding the package is slower than dumping it", not
"slower than a competitor". Throughput is always MB/s of the input file,
so the forward rows count package bytes (compressed, 2.5 MB) and the
reverse rows count JSON bytes (33.7 MB); the two are not comparable to
each other, only to their own lines. The `[extra]` row restates the
forward rate per decompressed byte, which is the number to hold against
the text-format pairs.

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

**Commands.** `bench/run.sh pages-json` runs, per shape:

- ours forward: `sublime -q convert styled.pages styled.json --to pages-json`
- ours back: `sublime -q convert styled.json styled-back.pages --from pages-json --to pages`
- floor: `sublime-bench pages-json floor styled.pages styled.raw`
  (`bench/src/pairs/pages_json.rs`)

The forward row prints MB/s of the package and, in parentheses, MB/s of the
decompressed streams, which is the number comparable to the text-format
pairs.

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
- The floor writes 7.9 MB of decompressed bytes to disk while ours writes
  35 MB of JSON; part of the gap is output volume, not decoding.

## Results

### 2026-09-24, standard rows

commit: c60d037
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

Same inputs as the block below, re-run with the standard row names and units; forward and reverse numbers within run noise of it.

| Target | Ours | Reference | Result |
|---|---|---|---|
| pages -> pages-json, styled (2.5 MB): throughput (MB/s of input) | 30.2 | 85.5 floor (unzip + Snappy only); line: ours >= floor / 4 | PASS |
| pages -> pages-json, styled: throughput (MB/s of decompressed streams, 7.5 MB) [extra] | 89.5 | 253.4 floor | n/a |
| pages -> pages-json, styled: peak memory (MB) | 44.5 | 13.8 floor; line: <= 64 | PASS |
| pages-json -> pages, styled (33.7 MB): throughput (MB/s of input) | 328.5 | 401.2 our pages -> pages-json over the same bytes; line: not slower | FAIL |
| pages-json -> pages, styled: peak memory (MB) | 45.9 | line: <= 64 | PASS |
| pages -> pages-json, prose (1.5 MB): throughput (MB/s of input) | 39.7 | 103.4 floor (unzip + Snappy only); line: ours >= floor / 4 | PASS |
| pages -> pages-json, prose: throughput (MB/s of decompressed streams, 6.6 MB) [extra] | 174.3 | 453.4 floor | n/a |
| pages -> pages-json, prose: peak memory (MB) | 27.1 | 10.3 floor; line: <= 64 | PASS |
| pages-json -> pages, prose (16.1 MB): throughput (MB/s of input) | 327.5 | 421.0 our pages -> pages-json over the same bytes; line: not slower | FAIL |
| pages-json -> pages, prose: peak memory (MB) | 30.9 | line: <= 64 | PASS |

### 2026-09-24

commit: 73fd5f5
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

| Target | Ours | Reference | Result |
|---|---|---|---|
| pages -> pages-json, styled: MB/s of the package (decompressed streams) | 31.6 (93.6) | 98.0 (290.4) floor | PASS |
| pages-json -> pages, styled: MB/s of the JSON | 330.2 | 419.4 (pages -> pages-json, same bytes) | FAIL |
| Peak RSS pages -> pages-json, styled (MB) | 44.4 | 13.5 floor; <= 64 | PASS |
| Peak RSS pages-json -> pages, styled (MB) | 46.0 | <= 64 | PASS |
| pages -> pages-json, prose: MB/s of the package (decompressed streams) | 39.4 (172.6) | 70.3 (308.1) floor | PASS |
| pages-json -> pages, prose: MB/s of the JSON | 324.6 | 416.8 (pages -> pages-json, same bytes) | FAIL |
| Peak RSS pages -> pages-json, prose (MB) | 27.4 | 10.3 floor; <= 64 | PASS |
| Peak RSS pages-json -> pages, prose (MB) | 30.5 | <= 64 | PASS |
| Binary size, gnu (bytes) | 1399664 | <= 1400000 (size-budget, what CI checks) | PASS |
| Binary size, musl static (bytes, the release asset) | 1499776 | recorded | n/a |
| Startup above spawn floor (ms, 1 KB file) | -0.823 (spawn 0.561, floor 1.384) | < 1 | PASS |

Forward decode is 3.1x the floor on the dense shape and 1.8x on prose, both
inside the line; per decompressed byte it runs at 94 and 173 MB/s, the
class of our markup-dense Markdown parse. The reverse direction misses its
line by 21 percent on both shapes: rebuilding spends its time in the
Snappy compressor (`io::snappy::encode`) and the JSON tokenizer, and the
compressor is the greedy matcher tuned for ratio.

## Conclusions

- The decode floor is not the problem: 3x of "unzip and decompress" for
  full schema-named trees plus JSON is a healthy ratio. The lever that
  matters is not here but in the document paths, which decode all 570
  preset objects to use 70 (see `pages-docx.md`).
- The reverse direction's miss is the compressor. A faster Snappy match
  mode for the rebuild (skip the 8-byte extension, smaller table) or a
  streaming JSON tokenizer without the token buffer are the candidates;
  recorded as a blocker in `STATE.md`.
