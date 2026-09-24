# Pages -> Word

**Latest** (2026-09-24, slim model: pages -> docx 13.1 MB/s of input on the dense shape and 16.4 on prose, at 174.9 and 93.8 MB peak; all four goals FAIL, see Conclusions and `STATE.md`)

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

- Both goals fail on both shapes: a fifth of the throughput goal on the
  dense shape, and 3.6x the memory goal. In-process on `styled`
  (symbolized samples): package decode 26 ms, document read 60 ms, Word
  write 100 ms, most of it deflating the 30 MB `document.xml`. Peak memory
  is the object trees (44 MB), the document model (48 MB of structs for
  170,000 runs at 224 bytes each, plus their strings), and the rendered
  XML held whole before compression.
- Before this block the reader was quadratic in the paragraph count (16.9 s
  on this input); binary searches over the sorted attribute tables and a
  cursor for UTF-16 offsets brought it to 0.25 s. That is in this commit;
  the goals still fail.
- Levers, in order: a slimmer run (a style index and only its own
  overrides, fonts and languages interned in the style table); the Word
  body streamed through the compressor as it is rendered; objects decoded
  on first lookup instead of all 570 up front. Recorded as a blocker in
  `STATE.md` for 0.6.1.
