# Pages -> Word

**Latest** (2026-09-24: pages -> docx runs at 10.1 MB/s of input on the dense shape and 13.0 MB/s on prose, at 232.0 and 112.9 MB peak; both lines FAIL against our own package round trip (about 31 and 35 MB/s, 45 and 27 MB). Blockers and the 0.6.1 plan are in `STATE.md`)

## Purpose

The conversion the flagship is for: a Pages document as a Word file, with its styles by name, lists, tables, images, headers and footers, and tracked changes. It has to feel instant on a resume and stay inside the binary's memory habits on a book.

## Pass lines

No peer implementation exists. The reference is our own package round
trip, `pages -> pages-json`, on the same input and in the same session: it
decodes every object and writes several times more bytes than any document
path, so a document path that costs more than it is doing avoidable work.
The memory line is the same reference's peak.

| Target | Pass line |
|---|---|
| `pages -> docx` throughput (MB/s of the package) | >= `pages -> pages-json` on the same input |
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

**Commands.** `bench/run.sh pages-docx` runs, per shape:

- ours: `sublime -q convert styled.pages styled.docx --to docx`
- reference: `sublime -q convert styled.pages styled.json --to pages-json`

The pair's harness module (`bench/src/pairs/pages_docx.rs`) exposes `ours`
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
| pages -> docx, styled (2.5 MB): throughput (MB/s of input) | 10.1 | 30.1 our pages -> pages-json on the same input; line: not slower | FAIL |
| pages -> docx, styled: peak memory (MB) | 232.0 | 44.5 our pages -> pages-json on the same input; line: not more | FAIL |
| pages -> docx, prose (1.5 MB): throughput (MB/s of input) | 13.0 | 32.8 our pages -> pages-json on the same input; line: not slower | FAIL |
| pages -> docx, prose: peak memory (MB) | 112.9 | 27.5 our pages -> pages-json on the same input; line: not more | FAIL |

### 2026-09-24

commit: 73fd5f5
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

| Target | Ours | Reference | Result |
|---|---|---|---|
| pages -> docx throughput, styled (MB/s of the package) | 10.1 | 31.1 (pages-json) | FAIL |
| Peak RSS pages -> docx, styled (MB) | 232.3 | 44.6 (pages-json) | FAIL |
| pages -> docx throughput, prose (MB/s of the package) | 13.0 | 40.2 (pages-json) | FAIL |
| Peak RSS pages -> docx, prose (MB) | 113.4 | 27.3 (pages-json) | FAIL |
| Binary size, gnu (bytes) | 1399664 | <= 1400000 (size-budget, what CI checks) | PASS |
| Binary size, musl static (bytes, the release asset) | 1499776 | recorded | n/a |
| Startup above spawn floor (ms, 1 KB file) | -0.300 (spawn 1.214, floor 1.514) | < 1 | PASS |

Both lines fail on both shapes. Throughput is a third of the reference
on the dense shape and a third on prose; memory is 5.2x and 4.2x. The
in-process split on `styled` (from a symbolized build, `eu-stack`
samples): package decode 26 ms, document read 60 ms, Word write 100 ms of
which the Deflate of a 30 MB `document.xml` is most, and the remainder is
the process. Peak memory is the object trees (45 MB), the document model
(48 MB of structs for 170,000 runs at 224 bytes each, plus their strings),
and the rendered XML held whole before it is compressed.

Before this block the reader was quadratic in the number of paragraphs
(every paragraph scanned the whole section and layout tables, and the
style tables, from the start): the same input took 16.9 s. Binary searches
over the sorted tables and a cursor for UTF-16 offsets instead of a
16-byte-per-character table brought it to 0.26 s. That fix is in this
commit; the lines still fail.

## Conclusions

- The model is too fat for this shape: 224 bytes per run before its text.
  Fonts and languages are cloned `String`s on every run; a run should
  hold a style index and only its own overrides, with shared strings
  interned in the style table.
- The Word writer holds the whole `document.xml` and then deflates it;
  streaming the body through the compressor as it is rendered removes the
  30 MB buffer and overlaps the two costs.
- The package decode builds trees for all 570 objects to use about 70;
  decoding an object on first lookup halves the fixed cost for every
  document path (the spike in `STATE.md`).
- These three are the 0.6.1 work. The pass lines stay as they are; a
  document path must not cost more than the lossless layer it sits on.
